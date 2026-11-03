//! TOML configuration: file schema, loading, and validation into domain
//! values.
//!
//! `config.example.toml` is the checked-in, documented template. A test
//! parses it, so keys that leave the schema fail the gate; keys missing
//! from the example are fine and take their defaults.

use std::{
    collections::HashSet,
    fs,
    path::Path,
};

use serde::Deserialize;

use crate::domain::{
    engine::{
        Action,
        Bindings,
        Engine,
        EngineOptions,
        Press,
        Profile,
        Step,
    },
    key::{
        Key,
        MouseButton,
    },
};

/// Everything wrong with a config file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("failed to read config file {path}: {source}")]
    Read {
        /// The path attempted.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file is not valid TOML or does not match the schema.
    #[error("failed to parse config file {path}: {source}")]
    Parse {
        /// The path attempted.
        path: String,
        /// The underlying TOML error.
        #[source]
        source: toml::de::Error,
    },
    /// The file parsed but the values do not make sense together.
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

// ---------------------------------------------------------------- schema

/// Bounds in milliseconds.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeMs {
    /// Inclusive minimum, in milliseconds.
    pub min: u64,
    /// Inclusive maximum, in milliseconds.
    pub max: u64,
}

/// `[[profile]]`: a pacing profile.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSection {
    /// Config-facing name (referenced by `default_profile`).
    pub name: String,
    /// Hotkey that selects this profile.
    #[serde(default)]
    pub select: Option<String>,
    /// Gap between repetitions.
    pub interval_ms: RangeMs,
}

/// One repetition step inside `[[action]]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepSection {
    /// Key to press (mutually exclusive with `button`; both absent means a
    /// pause step).
    #[serde(default)]
    pub key: Option<String>,
    /// Mouse button to click (mutually exclusive with `key`).
    #[serde(default)]
    pub button: Option<String>,
    /// Hold time for key/button presses, or the duration for a pause.
    #[serde(default)]
    pub hold_ms: Option<RangeMs>,
}

/// `[[action]]`: the spammed sequence.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionSection {
    /// Config-facing name (referenced by `default_action`).
    pub name: String,
    /// Hotkey that selects this action.
    #[serde(default)]
    pub select: Option<String>,
    /// The steps of one repetition.
    #[serde(default)]
    pub steps: Vec<StepSection>,
}

/// `[target]`: which window the clicker drives.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetSection {
    /// Process names to look for (`.exe` optional, case-insensitive).
    #[serde(default)]
    pub process_names: Vec<String>,
    /// Keep looking instead of failing when no process matches.
    #[serde(default = "default_true")]
    pub wait_for_window: bool,
    /// Stop when the target window disappears.
    #[serde(default = "default_true")]
    pub exit_when_lost: bool,
    /// Only spam while the target window is focused.
    #[serde(default = "default_true")]
    pub require_focus: bool,
}

impl Default for TargetSection {
    fn default() -> Self {
        Self {
            process_names: Vec::new(),
            wait_for_window: true,
            exit_when_lost: true,
            require_focus: true,
        }
    }
}

/// `[clicker]`: engine knobs and defaults.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClickerSection {
    /// Extra milliseconds added to every repetition gap (the original
    /// tool's `--addeddelay=`).
    #[serde(default)]
    pub added_delay_ms: u64,
    /// Hotkeys are recognized even when the target is not focused.
    #[serde(default = "default_true")]
    pub global_hotkeys: bool,
    /// Hotkey that toggles clicking on and off.
    #[serde(default)]
    pub toggle_key: Option<String>,
    /// Profile selected at startup (default: the first one).
    #[serde(default)]
    pub default_profile: Option<String>,
    /// Action selected at startup (default: the first one).
    #[serde(default)]
    pub default_action: Option<String>,
}

impl Default for ClickerSection {
    fn default() -> Self {
        Self {
            added_delay_ms: 0,
            global_hotkeys: true,
            toggle_key: None,
            default_profile: None,
            default_action: None,
        }
    }
}

/// `[sink.versa-kbm]`: device connection settings.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersaKbmSection {
    /// USB vendor id.
    #[serde(default = "default_vid")]
    pub vid: u16,
    /// USB product id.
    #[serde(default = "default_pid")]
    pub pid: u16,
    /// Per-command ack timeout, in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// Release everything on the device when the clicker stops.
    #[serde(default = "default_true")]
    pub release_on_stop: bool,
}

impl Default for VersaKbmSection {
    fn default() -> Self {
        Self {
            vid: default_vid(),
            pid: default_pid(),
            timeout_ms: default_timeout_ms(),
            release_on_stop: true,
        }
    }
}

/// `[sink]`: the output adapter.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SinkSection {
    /// `auto`, `win32`, `versa-kbm`, or `log`.
    #[serde(default)]
    pub backend: Option<String>,
    /// versa-kbm device settings.
    #[serde(default, rename = "versa-kbm")]
    pub versa_kbm: VersaKbmSection,
}

/// `[hotkeys]`: the input adapter for hotkey presses.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotkeysSection {
    /// `auto`, `win32`, or `rdev`.
    #[serde(default)]
    pub backend: Option<String>,
}

/// `[logging]`: where diagnostics go.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingSection {
    /// Default filter when `RUST_LOG` is unset.
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for LoggingSection {
    fn default() -> Self {
        Self { level: default_log_level() }
    }
}

/// The whole file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    /// Target window settings.
    #[serde(default)]
    pub target: TargetSection,
    /// Engine knobs.
    #[serde(default)]
    pub clicker: ClickerSection,
    /// Output adapter.
    #[serde(default)]
    pub sink: SinkSection,
    /// Hotkey input adapter.
    #[serde(default)]
    pub hotkeys: HotkeysSection,
    /// Logging settings.
    #[serde(default)]
    pub logging: LoggingSection,
    /// Pacing profiles.
    #[serde(default)]
    pub profile: Vec<ProfileSection>,
    /// Spammed sequences.
    #[serde(default)]
    pub action: Vec<ActionSection>,
}

fn default_true() -> bool {
    true
}

const fn default_vid() -> u16 {
    0x258a
}

const fn default_pid() -> u16 {
    0x0100
}

const fn default_timeout_ms() -> u64 {
    250
}

fn default_log_level() -> String {
    "info".to_owned()
}

// -------------------------------------------------------------- resolved

/// Which input sink to build (composition-root selection).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SinkBackend {
    /// Pick the best available sink (win32 when the target window is
    /// found, otherwise versa-kbm).
    Auto,
    /// Win32 window-message sink (Windows only).
    Win32,
    /// versa-kbm HID device sink.
    VersaKbm,
    /// Dry-run logger.
    Log,
}

impl SinkBackend {
    /// Parses the config/CLI value; `None` when unknown.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "win32" => Some(Self::Win32),
            "versa-kbm" | "versa_kbm" => Some(Self::VersaKbm),
            "log" => Some(Self::Log),
            _ => None,
        }
    }
}

/// Which hotkey source to build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyBackend {
    /// Pick the best available source (win32 on Windows, rdev elsewhere).
    Auto,
    /// Win32 `GetAsyncKeyState` polling.
    Win32,
    /// `rdev` global listener.
    Rdev,
}

impl HotkeyBackend {
    /// Parses the config/CLI value; `None` when unknown.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "win32" => Some(Self::Win32),
            "rdev" => Some(Self::Rdev),
            _ => None,
        }
    }
}

/// Resolved `[target]` settings.
#[derive(Clone, Debug)]
pub struct TargetSettings {
    /// Process names to look for.
    pub process_names: Vec<String>,
    /// Keep looking instead of failing when none matches.
    pub wait_for_window: bool,
    /// Stop when the target window disappears.
    pub exit_when_lost: bool,
    /// Only spam while the target window is focused.
    pub require_focus: bool,
}

/// Resolved `[clicker]` settings that are not engine state.
#[derive(Clone, Copy, Debug)]
pub struct ClickerSettings {
    /// Hotkeys work even when the target is not focused.
    pub global_hotkeys: bool,
}

/// Resolved `[sink]` settings.
#[derive(Clone, Debug)]
pub struct SinkSettings {
    /// Which sink backend to build.
    pub backend: SinkBackend,
    /// versa-kbm device settings.
    pub versa_kbm: VersaKbmSection,
}

/// The fully validated configuration: everything the composition root
/// needs to wire adapters and start the engine.
pub struct Settings {
    /// Target window settings.
    pub target: TargetSettings,
    /// Clicker settings that are not engine state.
    pub clicker: ClickerSettings,
    /// The configured engine, ready to run.
    pub engine: Engine,
    /// Output adapter selection.
    pub sink: SinkSettings,
    /// Hotkey source selection.
    pub hotkey_backend: HotkeyBackend,
    /// Default log filter (overridden by `RUST_LOG`).
    pub log_level: String,
}

impl core::fmt::Debug for Settings {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Settings")
            .field("target", &self.target)
            .field("clicker", &self.clicker)
            .field("sink", &self.sink)
            .field("hotkey_backend", &self.hotkey_backend)
            .field("log_level", &self.log_level)
            .finish_non_exhaustive()
    }
}

// --------------------------------------------------------------- loading

/// Loads and validates the config file at `path`.
///
/// # Errors
/// [`ConfigError`] on read, parse, or validation failure.
pub fn load(path: &Path) -> Result<Settings, ConfigError> {
    let path_display = path.display().to_string();
    let raw = fs::read_to_string(path)
        .map_err(|source| ConfigError::Read { path: path_display.clone(), source })?;
    let file: FileConfig =
        toml::from_str(&raw).map_err(|source| ConfigError::Parse { path: path_display, source })?;
    resolve(file)
}

/// Resolves a parsed file into validated [`Settings`].
fn resolve(file: FileConfig) -> Result<Settings, ConfigError> {
    let toggle = parse_optional_key("clicker.toggle_key", file.clicker.toggle_key.as_deref())?;
    let mut hotkeys_seen: HashSet<Key> = HashSet::new();
    if let Some(key) = toggle {
        hotkeys_seen.insert(key);
    }
    let mut names_seen: HashSet<&str> = HashSet::new();

    let (profiles, profile_bindings) =
        resolve_profiles(&file.profile, &mut hotkeys_seen, &mut names_seen)?;
    let (actions, action_bindings) =
        resolve_actions(&file.action, &mut hotkeys_seen, &mut names_seen)?;
    let default_profile = resolve_default(
        file.clicker.default_profile.as_ref(),
        &profile_names(&profiles),
        "clicker.default_profile",
    )?;
    let default_action = resolve_default(
        file.clicker.default_action.as_ref(),
        &action_names(&actions),
        "clicker.default_action",
    )?;

    let bindings = Bindings::new(toggle, profile_bindings, action_bindings);
    let mut engine = Engine::new(
        profiles,
        actions,
        bindings,
        EngineOptions {
            added_delay_ms: file.clicker.added_delay_ms,
            require_focus: file.target.require_focus,
            global_hotkeys: file.clicker.global_hotkeys,
        },
    );
    engine.select_defaults(default_profile, default_action);

    let sink_backend = file.sink.backend.as_deref().map_or(Ok(SinkBackend::Auto), |raw| {
        SinkBackend::parse(raw).ok_or_else(|| {
            invalid_choice("sink.backend", raw, &["auto", "win32", "versa-kbm", "log"])
        })
    })?;
    let hotkey_backend =
        file.hotkeys.backend.as_deref().map_or(Ok(HotkeyBackend::Auto), |raw| {
            HotkeyBackend::parse(raw)
                .ok_or_else(|| invalid_choice("hotkeys.backend", raw, &["auto", "win32", "rdev"]))
        })?;

    Ok(Settings {
        target: TargetSettings {
            process_names: file.target.process_names,
            wait_for_window: file.target.wait_for_window,
            exit_when_lost: file.target.exit_when_lost,
            require_focus: file.target.require_focus,
        },
        clicker: ClickerSettings { global_hotkeys: file.clicker.global_hotkeys },
        engine,
        sink: SinkSettings { backend: sink_backend, versa_kbm: file.sink.versa_kbm },
        hotkey_backend,
        log_level: file.logging.level,
    })
}

/// Profiles plus their select-hotkey bindings, in file order.
type ProfileSet = (Vec<Profile>, Vec<(Key, usize)>);

/// Actions plus their select-hotkey bindings, in file order.
type ActionSet = (Vec<Action>, Vec<(Key, usize)>);

/// Resolves the `[[profile]]` sections and their select hotkeys, guarding
/// against duplicate hotkey bindings and duplicate names across the whole
/// file.
fn resolve_profiles<'a>(
    sections: &'a [ProfileSection],
    hotkeys_seen: &mut HashSet<Key>,
    names_seen: &mut HashSet<&'a str>,
) -> Result<ProfileSet, ConfigError> {
    let mut profiles = Vec::new();
    let mut bindings = Vec::new();
    for (idx, section) in sections.iter().enumerate() {
        if !names_seen.insert(section.name.as_str()) {
            return Err(ConfigError::Invalid(format!(
                "profile name '{}' is used more than once",
                section.name
            )));
        }
        validate_range(&format!("profile '{}' interval_ms", section.name), section.interval_ms)?;
        let select = parse_optional_key(
            &format!("profile '{}' select", section.name),
            section.select.as_deref(),
        )?;
        if let Some(key) = select {
            if !hotkeys_seen.insert(key) {
                return Err(ConfigError::Invalid(format!("hotkey {key} is bound more than once")));
            }
            bindings.push((key, idx));
        }
        profiles.push(Profile {
            name: section.name.clone(),
            interval_min_ms: section.interval_ms.min,
            interval_max_ms: section.interval_ms.max,
        });
    }
    if profiles.is_empty() {
        return Err(ConfigError::Invalid("at least one [[profile]] is required".to_owned()));
    }
    Ok((profiles, bindings))
}

/// Resolves the `[[action]]` sections and their select hotkeys, guarding
/// against duplicate hotkey bindings and duplicate names across the whole
/// file.
fn resolve_actions<'a>(
    sections: &'a [ActionSection],
    hotkeys_seen: &mut HashSet<Key>,
    names_seen: &mut HashSet<&'a str>,
) -> Result<ActionSet, ConfigError> {
    let mut actions = Vec::new();
    let mut bindings = Vec::new();
    for (idx, section) in sections.iter().enumerate() {
        if !names_seen.insert(section.name.as_str()) {
            return Err(ConfigError::Invalid(format!(
                "action name '{}' is used more than once",
                section.name
            )));
        }
        let mut steps = Vec::new();
        for (step_idx, step) in section.steps.iter().enumerate() {
            steps.push(build_step(
                &format!("action '{}' step {}", section.name, step_idx + 1),
                step,
            )?);
        }
        if steps.is_empty() {
            return Err(ConfigError::Invalid(format!("action '{}' has no steps", section.name)));
        }
        let select = parse_optional_key(
            &format!("action '{}' select", section.name),
            section.select.as_deref(),
        )?;
        if let Some(key) = select {
            if !hotkeys_seen.insert(key) {
                return Err(ConfigError::Invalid(format!("hotkey {key} is bound more than once")));
            }
            bindings.push((key, idx));
        }
        actions.push(Action { name: section.name.clone(), steps });
    }
    if actions.is_empty() {
        return Err(ConfigError::Invalid("at least one [[action]] is required".to_owned()));
    }
    Ok((actions, bindings))
}

/// Builds one repetition step from config.
fn build_step(context: &str, step: &StepSection) -> Result<Step, ConfigError> {
    let press = match (step.key.as_deref(), step.button.as_deref()) {
        (Some(raw), None) => {
            let key = parse_key(context, raw)?;
            Press::Key(key)
        }
        (None, Some(raw)) => {
            let button = parse_button(context, raw)?;
            Press::Click(button)
        }
        (Some(_), Some(_)) => {
            return Err(ConfigError::Invalid(format!(
                "{context}: key and button are mutually exclusive"
            )));
        }
        (None, None) => {
            if step.hold_ms.is_none() {
                return Err(ConfigError::Invalid(format!(
                    "{context}: a pause step needs hold_ms (key/button presses default to a zero hold)"
                )));
            }
            Press::Pause
        }
    };
    let (hold_min, hold_max) = match step.hold_ms {
        Some(range) => {
            validate_range(context, range)?;
            (range.min, range.max)
        }
        None => (0, 0),
    };
    Ok(Step { press, hold_min_ms: hold_min, hold_max_ms: hold_max })
}

/// Parses one optional key name with its config path for error messages.
fn parse_optional_key(context: &str, raw: Option<&str>) -> Result<Option<Key>, ConfigError> {
    raw.map(|name| parse_key(context, name)).transpose()
}

/// Parses one key name.
fn parse_key(context: &str, raw: &str) -> Result<Key, ConfigError> {
    Key::from_name(raw).ok_or_else(|| {
        ConfigError::Invalid(format!(
            "{context}: unknown key '{raw}' (run with --list-keys to see valid names)"
        ))
    })
}

/// Parses one mouse button name.
fn parse_button(context: &str, raw: &str) -> Result<MouseButton, ConfigError> {
    MouseButton::from_name(raw).ok_or_else(|| {
        ConfigError::Invalid(format!(
            "{context}: unknown button '{raw}' (valid: left, right, middle)"
        ))
    })
}

/// Rejects inverted ranges.
fn validate_range(context: &str, range: RangeMs) -> Result<(), ConfigError> {
    if range.min > range.max {
        return Err(ConfigError::Invalid(format!(
            "{context}: min {} is greater than max {}",
            range.min, range.max
        )));
    }
    Ok(())
}

/// Resolves a name-based default into an index (absent: first entry).
fn resolve_default(
    name: Option<&String>,
    names: &[String],
    field: &str,
) -> Result<usize, ConfigError> {
    match name {
        None => Ok(0),
        Some(wanted) => names.iter().position(|candidate| candidate == wanted).ok_or_else(|| {
            ConfigError::Invalid(format!(
                "{field}: unknown name '{wanted}' (configured: {})",
                names.join(", ")
            ))
        }),
    }
}

/// The configured profile names, in order.
fn profile_names(profiles: &[Profile]) -> Vec<String> {
    profiles.iter().map(|profile| profile.name.clone()).collect()
}

/// The configured action names, in order.
fn action_names(actions: &[Action]) -> Vec<String> {
    actions.iter().map(|action| action.name.clone()).collect()
}

/// Builds an "invalid choice" error listing the valid values.
fn invalid_choice(field: &str, raw: &str, valid: &[&str]) -> ConfigError {
    ConfigError::Invalid(format!("{field}: unknown value '{raw}' (valid: {})", valid.join(", ")))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        ConfigError,
        FileConfig,
        load,
        resolve,
    };

    const BASE: &str = r#"
        [[profile]]
        name = "human"
        select = "Numpad6"
        interval_ms = { min = 15, max = 60 }

        [[action]]
        name = "spell"
        select = "Numpad2"
        steps = [{ key = "Q" }]
    "#;

    #[test]
    fn the_committed_example_file_is_valid() {
        let raw = include_str!("../config.example.toml");
        let parsed = toml::from_str::<FileConfig>(raw);
        assert!(parsed.is_ok(), "example config must parse: {:?}", parsed.as_ref().err());
        if let Ok(file) = parsed {
            let resolved = resolve(file);
            assert!(
                resolved.is_ok(),
                "example config must validate: {:?}",
                resolved.as_ref().err()
            );
            if let Ok(settings) = resolved {
                assert_eq!(settings.target.process_names.first().map(String::as_str), Some("Wow"));
                assert!(!settings.engine.is_running());
            }
        }
    }

    #[test]
    fn happy_path_resolves_defaults_and_bindings() {
        let parsed = toml::from_str::<FileConfig>(BASE);
        assert!(parsed.is_ok(), "fixture must parse: {:?}", parsed.as_ref().err());
        if let Ok(file) = parsed {
            let resolved = resolve(file);
            assert!(resolved.is_ok(), "fixture must validate: {:?}", resolved.as_ref().err());
            if let Ok(settings) = resolved {
                assert!(settings.target.require_focus);
                assert!(!settings.engine.is_running());
                assert_eq!(settings.engine.describe(), "paused");
                let keys = settings.engine.configured_keys();
                assert!(keys.contains(&crate::domain::key::Key::Numpad2));
            }
        }
    }

    #[test]
    fn duplicated_hotkeys_are_rejected() {
        let parsed = toml::from_str::<FileConfig>(BASE);
        assert!(parsed.is_ok(), "fixture must parse: {:?}", parsed.as_ref().err());
        if let Ok(mut file) = parsed
            && let Some(profile) = file.profile.first_mut()
        {
            profile.select = Some("Numpad2".to_owned()); // same as the action's select
            let resolved = resolve(file);
            assert!(
                matches!(resolved, Err(ConfigError::Invalid(ref message)) if message.contains("more than once")),
                "expected a duplicate-hotkey rejection, got {resolved:?}"
            );
        }
    }

    #[test]
    fn duplicated_names_are_rejected() {
        let dup_profile = r#"
            [[profile]]
            name = "p"
            interval_ms = { min = 0, max = 0 }

            [[profile]]
            name = "p"
            interval_ms = { min = 1, max = 1 }

            [[action]]
            name = "a"
            steps = [{ key = "Q" }]
        "#;
        let dup_action = r#"
            [[profile]]
            name = "p"
            interval_ms = { min = 0, max = 0 }

            [[action]]
            name = "a"
            steps = [{ key = "Q" }]

            [[action]]
            name = "a"
            steps = [{ key = "W" }]
        "#;
        for (raw, section) in [(dup_profile, "profile"), (dup_action, "action")] {
            let parsed = toml::from_str::<FileConfig>(raw);
            assert!(parsed.is_ok(), "fixture must parse: {:?}", parsed.as_ref().err());
            if let Ok(file) = parsed {
                let resolved = resolve(file);
                assert!(
                    matches!(resolved, Err(ConfigError::Invalid(ref message))
                        if message.contains(&format!("{section} name 'p' is used"))
                            || message.contains(&format!("{section} name 'a' is used"))),
                    "expected a duplicate-{section}-name rejection, got {resolved:?}"
                );
            }
        }
    }

    #[test]
    fn unknown_key_names_are_rejected_with_hint() {
        let parsed = toml::from_str::<FileConfig>(BASE);
        assert!(parsed.is_ok(), "fixture must parse: {:?}", parsed.as_ref().err());
        if let Ok(mut file) = parsed {
            file.clicker.toggle_key = Some("Numpad42".to_owned());
            let resolved = resolve(file);
            assert!(
                matches!(resolved, Err(ConfigError::Invalid(ref message)) if message.contains("--list-keys")),
                "expected an unknown-key rejection, got {resolved:?}"
            );
        }
    }

    #[test]
    fn key_and_button_are_mutually_exclusive() {
        let raw = r#"
            [[profile]]
            name = "p"
            interval_ms = { min = 0, max = 0 }

            [[action]]
            name = "a"
            steps = [{ key = "Q", button = "left" }]
        "#;
        let parsed = toml::from_str::<FileConfig>(raw);
        assert!(parsed.is_ok(), "fixture must parse: {:?}", parsed.as_ref().err());
        if let Ok(file) = parsed {
            let resolved = resolve(file);
            assert!(
                matches!(resolved, Err(ConfigError::Invalid(ref message)) if message.contains("mutually exclusive")),
                "expected a mutual-exclusion rejection, got {resolved:?}"
            );
        }
    }

    #[test]
    fn inverted_ranges_are_rejected() {
        let raw = r#"
            [[profile]]
            name = "p"
            interval_ms = { min = 50, max = 10 }

            [[action]]
            name = "a"
            steps = [{ key = "Q" }]
        "#;
        let parsed = toml::from_str::<FileConfig>(raw);
        assert!(parsed.is_ok(), "fixture must parse: {:?}", parsed.as_ref().err());
        if let Ok(file) = parsed {
            let resolved = resolve(file);
            assert!(
                matches!(resolved, Err(ConfigError::Invalid(ref message)) if message.contains("greater than")),
                "expected an inverted-range rejection, got {resolved:?}"
            );
        }
    }

    #[test]
    fn pause_steps_require_a_hold() {
        let raw = r#"
            [[profile]]
            name = "p"
            interval_ms = { min = 0, max = 0 }

            [[action]]
            name = "a"
            steps = [{ }]
        "#;
        let parsed = toml::from_str::<FileConfig>(raw);
        assert!(parsed.is_ok(), "fixture must parse: {:?}", parsed.as_ref().err());
        if let Ok(file) = parsed {
            let resolved = resolve(file);
            assert!(
                matches!(resolved, Err(ConfigError::Invalid(ref message)) if message.contains("hold_ms")),
                "expected a missing-hold rejection, got {resolved:?}"
            );
        }
    }

    #[test]
    fn unknown_backend_values_are_rejected() {
        let parsed = toml::from_str::<FileConfig>(BASE);
        assert!(parsed.is_ok(), "fixture must parse: {:?}", parsed.as_ref().err());
        if let Ok(mut file) = parsed {
            file.sink.backend = Some("telepathy".to_owned());
            let resolved = resolve(file);
            assert!(
                matches!(resolved, Err(ConfigError::Invalid(ref message)) if message.contains("sink.backend")),
                "expected a backend rejection, got {resolved:?}"
            );
        }
    }

    #[test]
    fn load_reports_missing_file_as_read_error() {
        let error = load(Path::new("./does/not/exist.toml"));
        assert!(matches!(error, Err(ConfigError::Read { .. })));
    }

    #[test]
    fn default_profile_must_exist() {
        let parsed = toml::from_str::<FileConfig>(BASE);
        assert!(parsed.is_ok(), "fixture must parse: {:?}", parsed.as_ref().err());
        if let Ok(mut file) = parsed {
            file.clicker.default_profile = Some("nope".to_owned());
            let resolved = resolve(file);
            assert!(
                matches!(resolved, Err(ConfigError::Invalid(ref message)) if message.contains("default_profile")),
                "expected a default-profile rejection, got {resolved:?}"
            );
        }
    }
}
