//! The clicker engine: a pure state machine turning hotkey commands and
//! elapsed time into a schedule of actuations.
//!
//! The engine never reads the clock or generates randomness and never
//! touches a platform API. The driver loop feeds it monotonic milliseconds
//! (via a [`Clock`](crate::ports::driven::Clock) adapter), freshly pressed
//! hotkeys (via a [`HotkeySource`](crate::ports::driving::HotkeySource)
//! adapter) and a [`Jitter`](crate::ports::driven::Jitter) adapter, and
//! forwards every returned [`Actuation`] to an
//! [`InputSink`](crate::ports::driven::InputSink) adapter.
//!
//! Timing model: a repetition walks the selected action's steps in order;
//! each step presses (down at `t`, up at `t + hold`), a pause step just
//! elapses its duration. After the final step's release, the next
//! repetition starts at `t + gap`, where `gap` is the selected profile's
//! interval sampled through the jitter port plus the configured
//! added delay. Toggling off releases everything immediately.

use std::fmt;

use tracing::info;

use crate::{
    domain::key::{
        Key,
        MouseButton,
    },
    ports::driven::Jitter,
};

/// Monotonic milliseconds since an arbitrary epoch, as fed in by the clock
/// adapter.
pub type Millis = u64;

/// One concrete output event the driver must deliver to the input sink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actuation {
    /// Press a key down.
    KeyDown(Key),
    /// Release a key.
    KeyUp(Key),
    /// Press a mouse button down.
    MouseDown(MouseButton),
    /// Release a mouse button.
    MouseUp(MouseButton),
    /// Release every key and button the sink may hold.
    ReleaseAll,
}

impl fmt::Display for Actuation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Actuation::KeyDown(key) => write!(f, "key down {key}"),
            Actuation::KeyUp(key) => write!(f, "key up {key}"),
            Actuation::MouseDown(button) => write!(f, "mouse down {button}"),
            Actuation::MouseUp(button) => write!(f, "mouse up {button}"),
            Actuation::ReleaseAll => f.write_str("release all"),
        }
    }
}

/// What a repetition step does when it fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// Press and release a key.
    Key(Key),
    /// Click (press and release) a mouse button.
    Click(MouseButton),
    /// Wait without actuating.
    Pause,
}

/// One step inside an [`Action`].
#[derive(Clone, Debug)]
pub struct Step {
    /// What to do when the step fires.
    pub press: Press,
    /// Inclusive bounds (milliseconds) sampled through the jitter port: the
    /// hold time for key/button presses, the duration for pauses.
    pub hold_min_ms: Millis,
    /// Upper hold bound; equal to [`Step::hold_min_ms`] means a constant.
    pub hold_max_ms: Millis,
}

impl Step {
    /// A step that presses `what` with a constant hold.
    #[must_use]
    pub const fn hold(press: Press, hold_ms: Millis) -> Self {
        Self { press, hold_min_ms: hold_ms, hold_max_ms: hold_ms }
    }

    /// A step that presses `what` with a hold sampled in the given range.
    #[must_use]
    pub const fn hold_range(press: Press, hold_min_ms: Millis, hold_max_ms: Millis) -> Self {
        Self { press, hold_min_ms, hold_max_ms }
    }
}

/// A pacing profile: how long to wait between repetitions of the selected
/// action. `Super` in the original tool is a profile with a zero interval;
/// `Human` samples jitter into the interval range.
#[derive(Clone, Debug)]
pub struct Profile {
    /// Config-facing name.
    pub name: String,
    /// Inclusive minimum gap between repetitions, in milliseconds.
    pub interval_min_ms: Millis,
    /// Inclusive maximum gap between repetitions, in milliseconds.
    pub interval_max_ms: Millis,
}

/// A named sequence of steps spammed while selected.
#[derive(Clone, Debug)]
pub struct Action {
    /// Config-facing name.
    pub name: String,
    /// The steps one repetition walks, in order.
    pub steps: Vec<Step>,
}

/// A hotkey command resolved from config bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Toggle clicking on/off.
    Toggle,
    /// Select the profile at this index.
    SelectProfile(usize),
    /// Select the action at this index.
    SelectAction(usize),
}

/// Hotkey-to-command bindings, built from validated config.
#[derive(Clone, Debug, Default)]
pub struct Bindings {
    toggle: Option<Key>,
    profiles: Vec<(Key, usize)>,
    actions: Vec<(Key, usize)>,
}

impl Bindings {
    /// Builds bindings from validated config: every `(key, index)` pair
    /// selects the profile/action at `index`; every key must be unique
    /// across all three lists (checked by the config layer).
    #[must_use]
    pub fn new(
        toggle: Option<Key>,
        profiles: Vec<(Key, usize)>,
        actions: Vec<(Key, usize)>,
    ) -> Self {
        Self { toggle, profiles, actions }
    }

    /// Resolves a freshly pressed key into a command.
    #[must_use]
    pub fn resolve(&self, key: Key) -> Option<Command> {
        if self.toggle == Some(key) {
            return Some(Command::Toggle);
        }
        for &(hotkey, idx) in &self.profiles {
            if hotkey == key {
                return Some(Command::SelectProfile(idx));
            }
        }
        for &(hotkey, idx) in &self.actions {
            if hotkey == key {
                return Some(Command::SelectAction(idx));
            }
        }
        None
    }

    /// Every hotkey the bindings listen for; the watch list for hotkey
    /// adapters and the support check for input sinks.
    #[must_use]
    pub fn hotkeys(&self) -> Vec<Key> {
        let mut keys = Vec::new();
        if let Some(toggle) = self.toggle {
            keys.push(toggle);
        }
        keys.extend(self.profiles.iter().map(|&(key, _)| key));
        keys.extend(self.actions.iter().map(|&(key, _)| key));
        keys
    }
}

/// Behavioral knobs that are pure configuration, not state.
#[derive(Clone, Debug)]
pub struct EngineOptions {
    /// Extra milliseconds added to every repetition gap (the old
    /// `--addeddelay=` argument).
    pub added_delay_ms: Millis,
    /// Only actuate while the focus monitor reports the target focused.
    pub require_focus: bool,
    /// Hotkeys work even when the target is not focused.
    pub global_hotkeys: bool,
}

/// The clicker state machine.
pub struct Engine {
    profiles: Vec<Profile>,
    actions: Vec<Action>,
    bindings: Bindings,
    options: EngineOptions,

    running: bool,
    profile_idx: usize,
    action_idx: usize,
    step_idx: usize,
    pending: Option<Scheduled>,
    next_fire: Option<Millis>,
    last_focus_warn: Option<Millis>,
}

struct Scheduled {
    act: Option<Actuation>,
    at: Millis,
}

impl Engine {
    /// Builds an engine over validated config. Empty profile/action lists
    /// are tolerated (the engine then never fires) so the config layer is
    /// the single validation point.
    #[must_use]
    pub fn new(
        profiles: Vec<Profile>,
        actions: Vec<Action>,
        bindings: Bindings,
        options: EngineOptions,
    ) -> Self {
        Self {
            profiles,
            actions,
            bindings,
            options,
            running: false,
            profile_idx: 0,
            action_idx: 0,
            step_idx: 0,
            pending: None,
            next_fire: None,
            last_focus_warn: None,
        }
    }

    /// Selects the starting profile and action by index (defaults resolved
    /// by the config layer).
    pub fn select_defaults(&mut self, profile_idx: usize, action_idx: usize) {
        self.profile_idx = profile_idx;
        self.action_idx = action_idx;
    }

    /// Processes freshly pressed hotkeys and returns the immediate
    /// actuations they trigger (releases from a toggle-off, plus
    /// [`Actuation::ReleaseAll`]).
    ///
    /// `focus` is the focus monitor's verdict for the target window;
    /// `None` means unknown. When `global_hotkeys` is off, hotkeys are
    /// only honored while the target is confirmed focused.
    pub fn handle_keys(
        &mut self,
        keys: &[Key],
        now: Millis,
        focus: Option<bool>,
    ) -> Vec<Actuation> {
        let mut out = Vec::new();
        if !(self.options.global_hotkeys || focus == Some(true)) {
            return out;
        }
        for &key in keys {
            match self.bindings.resolve(key) {
                Some(Command::Toggle) => {
                    self.running = !self.running;
                    if self.running {
                        self.next_fire = Some(now);
                        self.step_idx = 0;
                        info!("[+] clicking on: {} / {}", self.profile_name(), self.action_name());
                    } else {
                        info!("[-] clicking off");
                        out.extend(self.halt());
                    }
                }
                Some(Command::SelectProfile(idx))
                    if self.profiles.get(idx).is_some() && self.profile_idx != idx =>
                {
                    self.profile_idx = idx;
                    info!("[p] profile: {}", self.profile_name());
                }
                Some(Command::SelectAction(idx))
                    if self.actions.get(idx).is_some() && self.action_idx != idx =>
                {
                    self.action_idx = idx;
                    self.step_idx = 0;
                    info!("[a] action: {}", self.action_name());
                }
                _ => {}
            }
        }
        out
    }

    /// Advances the timeline to `now` and returns every actuation due at
    /// exactly `now`. Releases fire regardless of the focus gate (a held
    /// key must always be let go); presses only fire while the gate is
    /// open.
    pub fn poll(
        &mut self,
        now: Millis,
        focus: Option<bool>,
        jitter: &mut dyn Jitter,
    ) -> Vec<Actuation> {
        let mut out = Vec::new();
        if self.options.require_focus && focus.is_none() {
            self.warn_focus_unknown(now);
        }
        let gate_open = !self.options.require_focus || focus == Some(true);
        let burst_cap = self.step_count() * 2 + 4;
        for _ in 0..burst_cap.max(8) {
            if self.pending.as_ref().is_some_and(|sched| now < sched.at) {
                break;
            }
            if let Some(sched) = self.pending.take() {
                if let Some(act) = sched.act {
                    out.push(act);
                }
                if self.step_idx >= self.step_count() {
                    // Repetition complete: schedule the next one after the
                    // profile gap (jitter-sampled plus the added delay).
                    self.step_idx = 0;
                    let gap = self.sample_gap(jitter);
                    self.next_fire = Some(now.saturating_add(gap));
                    if gap > 0 {
                        break;
                    }
                }
                continue;
            }
            if !self.running || !gate_open || self.next_fire.is_none_or(|at| now < at) {
                break;
            }
            let Some(step) = self
                .actions
                .get(self.action_idx)
                .and_then(|action| action.steps.get(self.step_idx))
            else {
                break;
            };
            let step = step.clone();
            match step.press {
                Press::Pause => {
                    let wait = Self::sample_hold(&step, jitter);
                    self.pending = Some(Scheduled { act: None, at: now.saturating_add(wait) });
                }
                Press::Key(key) => {
                    out.push(Actuation::KeyDown(key));
                    let hold = Self::sample_hold(&step, jitter);
                    self.pending = Some(Scheduled {
                        act: Some(Actuation::KeyUp(key)),
                        at: now.saturating_add(hold),
                    });
                }
                Press::Click(button) => {
                    out.push(Actuation::MouseDown(button));
                    let hold = Self::sample_hold(&step, jitter);
                    self.pending = Some(Scheduled {
                        act: Some(Actuation::MouseUp(button)),
                        at: now.saturating_add(hold),
                    });
                }
            }
            self.step_idx += 1;
        }
        out
    }

    /// Immediate actuations for a graceful stop: release anything held and
    /// release-all at the sink. Idempotent.
    pub fn halt(&mut self) -> Vec<Actuation> {
        let mut out = Vec::new();
        if let Some(act) = self.pending.take().and_then(|sched| sched.act) {
            out.push(act);
        }
        out.push(Actuation::ReleaseAll);
        out
    }

    /// When the driver should wake up next (its own poll cadence when
    /// nothing is scheduled).
    #[must_use]
    pub fn next_wake(&self) -> Option<Millis> {
        let pending_at = self.pending.as_ref().map(|sched| sched.at);
        let fire_at = if self.running { self.next_fire } else { None };
        match (pending_at, fire_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    /// Whether clicking is currently enabled.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// The toggle hotkey, for the startup banner.
    #[must_use]
    pub fn toggle_key(&self) -> Option<Key> {
        self.bindings.toggle
    }

    /// One-line state summary for the banner and status logs.
    #[must_use]
    pub fn describe(&self) -> String {
        if !self.running {
            return "paused".to_owned();
        }
        let count = self.step_count();
        if count == 0 {
            format!("clicking {} / {}", self.profile_name(), self.action_name())
        } else {
            format!(
                "clicking {} / {} (step {}/{})",
                self.profile_name(),
                self.action_name(),
                self.step_idx.min(count - 1) + 1,
                count
            )
        }
    }

    /// Human-readable names of the configured profiles and their hotkeys.
    #[must_use]
    pub fn profile_summary(&self) -> Vec<String> {
        let bindings: Vec<Option<Key>> = self
            .bindings
            .profiles
            .iter()
            .copied()
            .enumerate()
            .map(|(i, (key, idx))| if idx == i { Some(key) } else { None })
            .collect();
        self.profiles
            .iter()
            .enumerate()
            .map(|(i, profile)| match bindings.get(i).copied().flatten() {
                Some(key) => format!("{} [{}]", profile.name, key),
                None => format!("{} [unbound]", profile.name),
            })
            .collect()
    }

    /// Human-readable names of the configured actions and their hotkeys.
    #[must_use]
    pub fn action_summary(&self) -> Vec<String> {
        let bindings: Vec<Option<Key>> = self
            .bindings
            .actions
            .iter()
            .copied()
            .enumerate()
            .map(|(i, (key, idx))| if idx == i { Some(key) } else { None })
            .collect();
        self.actions
            .iter()
            .enumerate()
            .map(|(i, action)| match bindings.get(i).copied().flatten() {
                Some(key) => format!("{} [{}]", action.name, key),
                None => format!("{} [unbound]", action.name),
            })
            .collect()
    }

    /// Every key configured anywhere (spam keys and hotkeys); the sink
    /// support check runs over this list.
    #[must_use]
    pub fn configured_keys(&self) -> Vec<Key> {
        let mut keys = self.bindings.hotkeys();
        for action in &self.actions {
            for step in &action.steps {
                match step.press {
                    Press::Key(key) => keys.push(key),
                    Press::Click(_) | Press::Pause => {}
                }
            }
        }
        keys
    }

    fn step_count(&self) -> usize {
        self.actions.get(self.action_idx).map_or(0, |action| action.steps.len())
    }

    fn profile_name(&self) -> &str {
        self.profiles.get(self.profile_idx).map_or("?", |p| p.name.as_str())
    }

    fn action_name(&self) -> &str {
        self.actions.get(self.action_idx).map_or("?", |a| a.name.as_str())
    }

    fn sample_gap(&self, jitter: &mut dyn Jitter) -> Millis {
        let (min, max) = self
            .profiles
            .get(self.profile_idx)
            .map_or((0, 0), |p| (p.interval_min_ms, p.interval_max_ms));
        jitter.sample_ms(min, max).saturating_add(self.options.added_delay_ms)
    }

    /// Samples one step's hold (or pause duration) from its bounds.
    fn sample_hold(step: &Step, jitter: &mut dyn Jitter) -> Millis {
        jitter.sample_ms(step.hold_min_ms, step.hold_max_ms)
    }

    fn warn_focus_unknown(&mut self, now: Millis) {
        let due = self
            .last_focus_warn
            .is_none_or(|last| now.saturating_sub(last) >= FOCUS_WARN_PERIOD_MS);
        if due && self.running {
            self.last_focus_warn = Some(now);
            tracing::warn!(
                "focus state unknown; pausing actuation until the target focus is readable"
            );
        }
    }
}

/// How often (in milliseconds) the engine re-warns about an unreadable
/// focus state while running.
const FOCUS_WARN_PERIOD_MS: Millis = 5_000;

#[cfg(test)]
mod tests {
    use super::{
        Action,
        Actuation,
        Bindings,
        Command,
        Engine,
        EngineOptions,
        Key,
        Millis,
        MouseButton,
        Press,
        Profile,
        Step,
    };
    use crate::ports::driven::Jitter;

    /// Deterministic jitter: returns the configured value clamped into the
    /// requested range.
    struct FakeJitter {
        value: Millis,
    }

    impl FakeJitter {
        fn fixed(value: Millis) -> Self {
            Self { value }
        }
    }

    impl Jitter for FakeJitter {
        fn sample_ms(&mut self, min_ms: Millis, max_ms: Millis) -> Millis {
            self.value.clamp(min_ms, max_ms)
        }
    }

    fn super_profile() -> Profile {
        Profile { name: "super".to_owned(), interval_min_ms: 0, interval_max_ms: 0 }
    }

    fn human_profile() -> Profile {
        Profile { name: "human".to_owned(), interval_min_ms: 10, interval_max_ms: 30 }
    }

    fn single_key_action(name: &str, key: Key) -> Action {
        Action { name: name.to_owned(), steps: vec![Step::hold(Press::Key(key), 0)] }
    }

    fn engine(profiles: Vec<Profile>, actions: Vec<Action>) -> Engine {
        Engine::new(
            profiles,
            actions,
            Bindings::new(Some(Key::Numpad4), vec![], vec![]),
            EngineOptions { added_delay_ms: 0, require_focus: false, global_hotkeys: true },
        )
    }

    fn toggle_on(engine: &mut Engine, now: Millis) -> Vec<Actuation> {
        engine.handle_keys(&[Key::Numpad4], now, Some(true))
    }

    fn toggle_off(engine: &mut Engine, now: Millis) -> Vec<Actuation> {
        engine.handle_keys(&[Key::Numpad4], now, Some(true))
    }

    #[test]
    fn super_mode_taps_at_full_speed() {
        let mut eng = engine(vec![super_profile()], vec![single_key_action("spell", Key::Q)]);
        toggle_on(&mut eng, 100);
        let out = eng.poll(100, Some(true), &mut FakeJitter::fixed(0));
        // Zero hold and zero gap: the burst cap bounds one poll's output,
        // and every pair must be a whole down/up tap.
        assert_eq!(out.len(), 8);
        assert_eq!(out.len() % 2, 0);
        for pair in out.chunks(2) {
            if let [first, second] = pair {
                assert_eq!(*first, Actuation::KeyDown(Key::Q), "down must precede up: {out:?}");
                assert_eq!(*second, Actuation::KeyUp(Key::Q), "down must precede up: {out:?}");
            }
        }
        // The next repetition is already due.
        assert_eq!(eng.next_wake(), Some(100));
    }

    #[test]
    fn human_mode_gaps_repetitions_with_jitter_and_added_delay() {
        let mut eng = Engine::new(
            vec![human_profile()],
            vec![single_key_action("spell", Key::Q)],
            Bindings::new(Some(Key::Numpad4), vec![], vec![]),
            EngineOptions { added_delay_ms: 5, require_focus: false, global_hotkeys: true },
        );
        toggle_on(&mut eng, 0);
        // Hold is 0, so down and up land in the same poll; the gap sample
        // is 20 (clamped into 10..=30) + 5 added.
        let out = eng.poll(0, Some(true), &mut FakeJitter::fixed(20));
        assert_eq!(out, vec![Actuation::KeyDown(Key::Q), Actuation::KeyUp(Key::Q)]);
        assert_eq!(eng.next_wake(), Some(25));
        assert_eq!(eng.poll(24, Some(true), &mut FakeJitter::fixed(20)), []);
        // At 25 the next repetition fires and, with the zero hold, also
        // completes within the same burst; the following gap is 25 again.
        assert_eq!(
            eng.poll(25, Some(true), &mut FakeJitter::fixed(20)),
            vec![Actuation::KeyDown(Key::Q), Actuation::KeyUp(Key::Q)]
        );
        assert_eq!(eng.next_wake(), Some(50));
    }

    #[test]
    fn holds_release_after_the_sampled_hold_time() {
        let mut eng = engine(
            vec![super_profile()],
            vec![Action {
                name: "hold".to_owned(),
                steps: vec![Step::hold_range(Press::Key(Key::Q), 40, 60)],
            }],
        );
        toggle_on(&mut eng, 0);
        let out = eng.poll(0, Some(true), &mut FakeJitter::fixed(50));
        assert_eq!(out, vec![Actuation::KeyDown(Key::Q)]);
        assert_eq!(eng.poll(49, Some(true), &mut FakeJitter::fixed(50)), []);
        // At t=50 the release fires and, because the profile gap is zero,
        // the next repetition's press fires in the same burst.
        assert_eq!(
            eng.poll(50, Some(true), &mut FakeJitter::fixed(50)),
            vec![Actuation::KeyUp(Key::Q), Actuation::KeyDown(Key::Q)]
        );
        assert_eq!(eng.poll(99, Some(true), &mut FakeJitter::fixed(50)), []);
    }

    #[test]
    fn multi_step_action_walks_steps_in_order_then_pauses() {
        let mut eng = engine(
            vec![super_profile()],
            vec![Action {
                name: "combo".to_owned(),
                steps: vec![
                    Step::hold(Press::Key(Key::Q), 5),
                    Step::hold(Press::Pause, 10),
                    Step::hold(Press::Click(MouseButton::Left), 0),
                ],
            }],
        );
        toggle_on(&mut eng, 0);
        assert_eq!(
            eng.poll(0, Some(true), &mut FakeJitter::fixed(0)),
            vec![Actuation::KeyDown(Key::Q)]
        );
        // Nothing happens while the key is held.
        assert_eq!(eng.poll(4, Some(true), &mut FakeJitter::fixed(0)), []);
        // The release fires, then the pause step schedules itself.
        assert_eq!(
            eng.poll(5, Some(true), &mut FakeJitter::fixed(0)),
            vec![Actuation::KeyUp(Key::Q)]
        );
        assert_eq!(eng.poll(14, Some(true), &mut FakeJitter::fixed(0)), []);
        // Pause elapses silently; the click fires and completes the
        // repetition, and with the zero gap the next repetition's first
        // step fires in the same burst.
        assert_eq!(
            eng.poll(15, Some(true), &mut FakeJitter::fixed(0)),
            vec![
                Actuation::MouseDown(MouseButton::Left),
                Actuation::MouseUp(MouseButton::Left),
                Actuation::KeyDown(Key::Q)
            ]
        );
        assert_eq!(eng.poll(19, Some(true), &mut FakeJitter::fixed(0)), []);
        assert_eq!(
            eng.poll(20, Some(true), &mut FakeJitter::fixed(0)),
            vec![Actuation::KeyUp(Key::Q)]
        );
    }

    #[test]
    fn toggle_off_releases_everything_immediately() {
        let mut eng = engine(
            vec![super_profile()],
            vec![Action {
                name: "hold".to_owned(),
                steps: vec![Step::hold(Press::Key(Key::Q), 50)],
            }],
        );
        toggle_on(&mut eng, 0);
        assert_eq!(
            eng.poll(0, Some(true), &mut FakeJitter::fixed(0)),
            vec![Actuation::KeyDown(Key::Q)]
        );
        let out = toggle_off(&mut eng, 10);
        assert_eq!(out, vec![Actuation::KeyUp(Key::Q), Actuation::ReleaseAll]);
        assert!(!eng.is_running());
        assert_eq!(eng.poll(60, Some(true), &mut FakeJitter::fixed(0)), []);
        // Halting again is idempotent: just the release-all.
        assert_eq!(eng.halt(), vec![Actuation::ReleaseAll]);
    }

    #[test]
    fn focus_gate_blocks_presses_but_not_releases() {
        let mut eng = Engine::new(
            vec![super_profile()],
            vec![Action {
                name: "hold".to_owned(),
                steps: vec![Step::hold(Press::Key(Key::Q), 10)],
            }],
            Bindings::new(Some(Key::Numpad4), vec![], vec![]),
            EngineOptions { added_delay_ms: 0, require_focus: true, global_hotkeys: true },
        );
        toggle_on(&mut eng, 0);
        assert_eq!(eng.poll(0, None, &mut FakeJitter::fixed(0)), [], "unknown focus blocks");
        assert_eq!(eng.poll(0, Some(false), &mut FakeJitter::fixed(0)), [], "unfocused blocks");
        let out = eng.poll(0, Some(true), &mut FakeJitter::fixed(0));
        assert_eq!(out, vec![Actuation::KeyDown(Key::Q)]);
        // Focus lost while the key is held: the release still fires.
        assert_eq!(
            eng.poll(10, Some(false), &mut FakeJitter::fixed(0)),
            vec![Actuation::KeyUp(Key::Q)]
        );
        assert_eq!(eng.poll(10, Some(false), &mut FakeJitter::fixed(0)), []);
    }

    #[test]
    fn non_global_hotkeys_need_focus() {
        let mut eng = Engine::new(
            vec![super_profile()],
            vec![single_key_action("spell", Key::Q)],
            Bindings::new(Some(Key::Numpad4), vec![], vec![]),
            EngineOptions { added_delay_ms: 0, require_focus: false, global_hotkeys: false },
        );
        // Ignored: target not focused.
        let ignored = eng.handle_keys(&[Key::Numpad4], 0, Some(false));
        assert!(ignored.is_empty() && !eng.is_running());
        // Honored: target focused.
        eng.handle_keys(&[Key::Numpad4], 0, Some(true));
        assert!(eng.is_running());
    }

    #[test]
    fn select_hotkeys_switch_profile_and_action() {
        // Default profile is the human one (index 0) so every poll yields
        // exactly one tap followed by a scheduled gap.
        let mut eng = Engine::new(
            vec![human_profile(), super_profile()],
            vec![single_key_action("spell", Key::Q), single_key_action("interact", Key::Numpad0)],
            Bindings::new(
                Some(Key::Numpad4),
                vec![(Key::Numpad5, 0), (Key::Numpad6, 1)],
                vec![(Key::Numpad2, 0), (Key::Numpad3, 1)],
            ),
            EngineOptions { added_delay_ms: 0, require_focus: false, global_hotkeys: true },
        );
        toggle_on(&mut eng, 0);
        // Jitter 30: hold clamps to the 0..=0 range, the gap to 10..=30.
        let out = eng.poll(0, Some(true), &mut FakeJitter::fixed(30));
        assert_eq!(out, vec![Actuation::KeyDown(Key::Q), Actuation::KeyUp(Key::Q)]);
        assert_eq!(eng.next_wake(), Some(30));
        // Switch the spammed action: the next repetition presses Numpad0.
        assert_eq!(eng.handle_keys(&[Key::Numpad3], 0, Some(true)), Vec::<Actuation>::new());
        assert_eq!(
            eng.poll(30, Some(true), &mut FakeJitter::fixed(30)),
            vec![Actuation::KeyDown(Key::Numpad0), Actuation::KeyUp(Key::Numpad0)]
        );
        // Switch to the super profile: the next poll bursts taps with no
        // gap between repetitions.
        eng.handle_keys(&[Key::Numpad6], 0, Some(true));
        let burst = eng.poll(60, Some(true), &mut FakeJitter::fixed(30));
        assert_eq!(burst.len(), 8, "super profile must burst taps, got {burst:?}");
    }

    #[test]
    fn bindings_resolve_and_enumerate() {
        let bindings =
            Bindings::new(Some(Key::Numpad4), vec![(Key::Numpad5, 0)], vec![(Key::Numpad2, 1)]);
        assert_eq!(bindings.resolve(Key::Numpad4), Some(Command::Toggle));
        assert_eq!(bindings.resolve(Key::Numpad5), Some(Command::SelectProfile(0)));
        assert_eq!(bindings.resolve(Key::Numpad2), Some(Command::SelectAction(1)));
        assert_eq!(bindings.resolve(Key::Q), None);
        assert_eq!(bindings.hotkeys(), vec![Key::Numpad4, Key::Numpad5, Key::Numpad2]);
    }

    #[test]
    fn summaries_describe_bindings() {
        let eng = Engine::new(
            vec![super_profile(), human_profile()],
            vec![single_key_action("spell", Key::Q)],
            Bindings::new(Some(Key::Numpad4), vec![(Key::Numpad5, 0)], vec![(Key::Numpad2, 0)]),
            EngineOptions { added_delay_ms: 0, require_focus: false, global_hotkeys: true },
        );
        assert_eq!(eng.profile_summary(), vec!["super [Numpad5]", "human [unbound]"]);
        assert_eq!(eng.action_summary(), vec!["spell [Numpad2]"]);
        assert_eq!(eng.describe(), "paused");
        // Configured keys cover hotkeys plus spam keys.
        let eng2 = engine(vec![super_profile()], vec![single_key_action("spell", Key::Q)]);
        let mut keys = eng2.configured_keys();
        keys.sort_unstable();
        keys.dedup();
        assert!(keys.contains(&Key::Q));
        assert!(keys.contains(&Key::Numpad4));
    }

    #[test]
    fn empty_action_never_fires() {
        let mut eng =
            engine(vec![super_profile()], vec![Action { name: "empty".to_owned(), steps: vec![] }]);
        toggle_on(&mut eng, 0);
        assert_eq!(eng.poll(0, Some(true), &mut FakeJitter::fixed(0)), []);
        assert_eq!(eng.poll(100, Some(true), &mut FakeJitter::fixed(0)), []);
    }
}
