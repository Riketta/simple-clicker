//! Composition root: parses the CLI, loads and validates config, picks
//! concrete adapters, wires the engine, and runs the driver loop.
//!
//! This is the only file that names concrete adapter types. Adapter
//! availability is decided here (Cargo features + platform), never in the
//! domain.

use std::{
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
    thread::sleep,
    time::Duration,
};

use anyhow::{
    Context,
    bail,
};
use clap::Parser;
#[cfg(feature = "versa-kbm")]
use simple_clicker::adapters::driven::versa_kbm::{
    VersaKbmSettings,
    VersaKbmSink,
};
#[cfg(all(windows, feature = "win32"))]
use simple_clicker::adapters::driven::win32::{
    TargetWindow as WinTarget,
    Win32Focus,
    Win32Sink,
};
#[cfg(feature = "hotkeys-rdev")]
use simple_clicker::adapters::driving::rdev_hotkeys::RdevHotkeys;
#[cfg(all(windows, feature = "win32"))]
use simple_clicker::adapters::driving::win32_hotkeys::Win32Hotkeys;
use simple_clicker::{
    adapters::driven::{
        logging_sink::LoggingSink,
        std_time::{
            SystemClock,
            SystemJitter,
        },
    },
    config::{
        HotkeyBackend,
        Settings,
        SinkBackend,
        SinkSettings,
    },
    domain::{
        engine::{
            Actuation,
            Engine,
        },
        key::Key,
    },
    ports::{
        driven::{
            Clock,
            FocusMonitor,
            InputSink,
            Jitter,
            SinkError,
        },
        driving::HotkeySource,
    },
};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[cfg(not(all(windows, feature = "win32")))]
/// Placeholder so the shared builder signatures compile on hosts without
/// the Win32 adapter; never constructed there.
#[allow(dead_code)]
struct WinTarget;

/// Base cadence of the driver loop, in milliseconds. Hotkey polling and
/// actuation both wake at most this often; shorter scheduled events wake
/// the loop sooner.
const POLL_INTERVAL_MS: u64 = 10;

/// Consecutive sink failures tolerated before the driver stops (a broken
/// transport should not spin forever).
const MAX_CONSECUTIVE_SINK_FAILURES: u32 = 10;

/// How often a lost target window is re-scanned for, matching the win32
/// adapters' internal re-acquisition rate limit.
#[cfg(all(windows, feature = "win32"))]
const TARGET_RESCAN: Duration = Duration::from_secs(2);

/// Sub-chunks of the target-wait retry interval, so Ctrl+C stays
/// responsive while waiting for a window.
#[cfg(all(windows, feature = "win32"))]
const WAIT_TICK: Duration = Duration::from_millis(200);

/// Command-line interface.
#[derive(Parser, Debug)]
#[command(
    name = "simple-clicker",
    version,
    about = "Config-driven auto clicker with swappable input adapters (Win32 messages, versa-kbm HID device)."
)]
struct Cli {
    /// Path to the TOML config file.
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,
    /// Log actuations instead of actuating (same as `sink.backend = "log"`).
    #[arg(long)]
    dry_run: bool,
    /// Print every valid key name and exit.
    #[arg(long)]
    list_keys: bool,
    /// Sink override: auto | win32 | versa-kbm | log.
    #[arg(long)]
    sink: Option<String>,
    /// Hotkey backend override: auto | win32 | rdev.
    #[arg(long)]
    hotkeys: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<ExitCode> {
    if cli.list_keys {
        print_keys();
        return Ok(ExitCode::SUCCESS);
    }

    let settings = simple_clicker::config::load(&cli.config)
        .with_context(|| format!("could not load {}", cli.config.display()))?;

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&settings.log_level));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let sink_backend = match cli.sink.as_deref() {
        Some(raw) => SinkBackend::parse(raw).ok_or_else(|| {
            anyhow::anyhow!("--sink: unknown value '{raw}' (valid: auto, win32, versa-kbm, log)")
        })?,
        None => settings.sink.backend,
    };
    let sink_backend = if cli.dry_run { SinkBackend::Log } else { sink_backend };
    let hotkey_backend = match cli.hotkeys.as_deref() {
        Some(raw) => HotkeyBackend::parse(raw).ok_or_else(|| {
            anyhow::anyhow!("--hotkeys: unknown value '{raw}' (valid: auto, win32, rdev)")
        })?,
        None => settings.hotkey_backend,
    };

    // Ctrl+C (and console close) only set a flag; the driver polls it at
    // its own cadence, releases held inputs, and exits with code 130.
    let stop = Arc::new(AtomicBool::new(false));
    ctrlc::set_handler({
        let stop = Arc::clone(&stop);
        move || stop.store(true, Ordering::Relaxed)
    })
    .context("could not install the Ctrl+C handler")?;

    let mut watch = settings.engine.hotkeys();
    watch.sort_unstable();
    watch.dedup();
    let hotkeys = build_hotkeys(hotkey_backend, &watch)?;

    // Target discovery (Win32 only): needed by the win32 sink and by the
    // focus monitor. The `auto` backend resolves during discovery.
    let Some(acquired) = acquire(sink_backend, &settings, &stop)? else {
        info!("Ctrl+C received; stopping before the clicker started");
        return Ok(ExitCode::from(130));
    };
    // A Ctrl+C during the tail of a wait window can still surface here;
    // the sink drops cleanly (the versa-kbm `Drop` releases the device).
    if stop.load(Ordering::Relaxed) {
        info!("Ctrl+C received; stopping");
        return Ok(ExitCode::from(130));
    }
    let sink_backend = acquired.backend;
    let target = acquired.target;
    let sink = acquired.sink;

    let focus = build_focus(target.as_ref());
    if settings.target.require_focus && focus.is_none() {
        bail!(
            "target.require_focus = true but no focus monitor is available \
             (Windows provides one once the target window is found; nothing \
             exists on other hosts); set require_focus = false to run without \
             a focus check"
        );
    }
    if !settings.clicker.global_hotkeys && focus.is_none() {
        bail!(
            "clicker.global_hotkeys = false needs a focus monitor to gate \
             hotkeys (Windows provides one once the target window is found; \
             nothing exists on other hosts)"
        );
    }

    banner(&settings, sink.as_ref(), hotkeys.as_ref(), focus.as_deref());

    let mut unsupported = settings
        .engine
        .configured_keys()
        .into_iter()
        .filter(|key| !sink.supports(*key))
        .map(|key| key.name().to_owned())
        .collect::<Vec<_>>();
    unsupported.sort_unstable();
    unsupported.dedup();
    if !unsupported.is_empty() {
        bail!("input sink does not support these configured keys: {}", unsupported.join(", "));
    }

    let release_all = release_on_stop(&settings.sink, sink_backend);
    let reason = drive(
        DriveStack {
            hotkeys,
            focus,
            sink,
            engine: settings.engine,
            clock: Box::new(SystemClock::new()),
            jitter: Box::new(SystemJitter::new()),
        },
        &stop,
        settings.target.exit_when_lost,
        release_all,
    )?;
    Ok(match reason {
        StopReason::TargetLost => ExitCode::SUCCESS,
        StopReason::Interrupted => ExitCode::from(130),
    })
}

/// Everything the driver loop needs, wired by the composition root.
struct DriveStack {
    hotkeys: Box<dyn HotkeySource>,
    focus: Option<Box<dyn FocusMonitor>>,
    sink: Box<dyn InputSink>,
    engine: Engine,
    clock: Box<dyn Clock>,
    jitter: Box<dyn Jitter>,
}

/// Why the driver loop ended.
#[derive(Debug)]
enum StopReason {
    /// The target window disappeared and `exit_when_lost` asked for a
    /// stop.
    TargetLost,
    /// Ctrl+C (or a console close) was received.
    Interrupted,
}

/// The driver loop: poll hotkeys, advance the engine, deliver actuations,
/// sleep until the next interesting moment. Releases everything on exit.
fn drive(
    stack: DriveStack,
    stop: &AtomicBool,
    exit_when_lost: bool,
    release_all_on_stop: bool,
) -> anyhow::Result<StopReason> {
    let DriveStack { mut hotkeys, mut focus, mut sink, mut engine, clock, mut jitter } = stack;
    let mut consecutive_failures = 0u32;
    let mut target_paused = false;

    let reason = loop {
        if stop.load(Ordering::Relaxed) {
            info!("Ctrl+C received; releasing and stopping");
            break StopReason::Interrupted;
        }
        if exit_when_lost && !sink.is_target_alive() {
            info!("target window is gone; stopping");
            break StopReason::TargetLost;
        }

        let now = clock.now_ms();
        let pressed = hotkeys.poll();
        let focus_now = focus.as_mut().and_then(|monitor| monitor.is_focused());
        let immediate = engine.handle_keys(&pressed, now, focus_now);
        let due = engine.poll(now, focus_now, jitter.as_mut());
        for actuation in immediate.into_iter().chain(due) {
            match sink.actuate(&actuation) {
                Ok(()) => {
                    consecutive_failures = 0;
                    target_paused = false;
                }
                // The target process is gone: not a transport failure. The
                // engine keeps running; actuation resumes (harmless against
                // a restarted process) when the adapter re-acquires a
                // window.
                Err(SinkError::TargetGone) => {
                    if !target_paused {
                        target_paused = true;
                        tracing::warn!(
                            "target window is gone; actuation paused until it returns \
                             (target.exit_when_lost = false)"
                        );
                    }
                }
                Err(err) => {
                    consecutive_failures += 1;
                    tracing::warn!("actuation failed ({consecutive_failures} in a row): {err}");
                    if consecutive_failures >= MAX_CONSECUTIVE_SINK_FAILURES {
                        // The same release policy as a graceful stop: the
                        // `release_all_on_stop` opt-out is honored on every
                        // exit path.
                        release(&mut engine, &mut sink, release_all_on_stop);
                        bail!("input sink keeps failing; stopping (last error: {err})");
                    }
                }
            }
        }

        let sleep_ms = match engine.next_wake() {
            Some(at) => at.saturating_sub(now).clamp(1, POLL_INTERVAL_MS),
            None => POLL_INTERVAL_MS,
        };
        sleep(Duration::from_millis(sleep_ms));
    };

    release(&mut engine, &mut sink, release_all_on_stop);
    Ok(reason)
}

/// The startup banner: version, resolved adapter descriptions, and the
/// engine's initial state.
fn banner(
    settings: &Settings,
    sink: &dyn InputSink,
    hotkeys: &dyn HotkeySource,
    focus: Option<&dyn FocusMonitor>,
) {
    info!("### Simple Clicker v{} ###", env!("CARGO_PKG_VERSION"));
    info!("[!] Sink: {}", sink.describe());
    info!("[!] Hotkeys: {}", hotkeys.describe());
    if let Some(monitor) = focus {
        info!("[!] Focus: {}", monitor.describe());
    }
    if let Some(toggle) = settings.engine.toggle_key() {
        info!("[!] Toggle: [{toggle}]");
    }
    info!("[!] Profiles: {}", settings.engine.profile_summary().join(", "));
    info!("[!] Actions: {}", settings.engine.action_summary().join(", "));
    info!("[!] {}", settings.engine.describe());
}

/// Releases the engine's held state through the sink; `include_release_all`
/// controls whether the final release-all is sent (off when the user asked
/// the versa-kbm device to keep its state).
fn release(engine: &mut Engine, sink: &mut Box<dyn InputSink>, include_release_all: bool) {
    for actuation in engine.halt() {
        if actuation == Actuation::ReleaseAll && !include_release_all {
            continue;
        }
        if let Err(err) = sink.actuate(&actuation)
            && !matches!(err, SinkError::TargetGone)
        {
            // Nothing can stay latched in a dead process; skip silently.
            tracing::warn!("release failed: {err}");
        }
    }
}

/// Whether the final release-all should reach the device.
fn release_on_stop(settings: &SinkSettings, backend: SinkBackend) -> bool {
    backend != SinkBackend::VersaKbm || settings.versa_kbm.release_on_stop
}

/// Resolves `auto` into a concrete hotkey backend.
// The Result keeps the signature uniform across cfg combinations; these
// two variants simply never fail.
#[cfg(all(windows, feature = "win32"))]
#[allow(clippy::unnecessary_wraps)]
fn resolve_auto_hotkeys() -> anyhow::Result<HotkeyBackend> {
    Ok(HotkeyBackend::Win32)
}

/// Resolves `auto` into a concrete hotkey backend.
#[cfg(all(not(all(windows, feature = "win32")), feature = "hotkeys-rdev"))]
#[allow(clippy::unnecessary_wraps)]
fn resolve_auto_hotkeys() -> anyhow::Result<HotkeyBackend> {
    Ok(HotkeyBackend::Rdev)
}

/// Resolves `auto` into a concrete hotkey backend.
#[cfg(all(not(all(windows, feature = "win32")), not(feature = "hotkeys-rdev")))]
fn resolve_auto_hotkeys() -> anyhow::Result<HotkeyBackend> {
    bail!(
        "no hotkey backend compiled; enable --features hotkeys-rdev (or --features win32 on Windows)"
    )
}

/// Builds the hotkey source for the chosen backend.
fn build_hotkeys(backend: HotkeyBackend, watch: &[Key]) -> anyhow::Result<Box<dyn HotkeySource>> {
    #[cfg(not(any(all(windows, feature = "win32"), feature = "hotkeys-rdev")))]
    let _ = watch;
    // Resolve "auto" into a concrete backend first so every arm below has
    // distinct code across all feature combinations.
    let effective = match backend {
        HotkeyBackend::Auto => resolve_auto_hotkeys()?,
        other => other,
    };
    match effective {
        HotkeyBackend::Win32 => {
            #[cfg(all(windows, feature = "win32"))]
            {
                Ok(Box::new(Win32Hotkeys::new(watch)))
            }
            #[cfg(not(all(windows, feature = "win32")))]
            {
                bail!("hotkeys.backend = \"win32\" requires Windows and the \"win32\" feature");
            }
        }
        HotkeyBackend::Rdev => {
            #[cfg(feature = "hotkeys-rdev")]
            {
                Ok(Box::new(
                    RdevHotkeys::new(watch)
                        .map_err(|err| anyhow::anyhow!("rdev hotkeys unavailable: {err}"))?,
                ))
            }
            #[cfg(not(feature = "hotkeys-rdev"))]
            {
                bail!("hotkeys.backend = \"rdev\" requires building with --features hotkeys-rdev");
            }
        }
        // "auto" is resolved above; this arm only satisfies the match.
        HotkeyBackend::Auto => bail!("no hotkey backend available"),
    }
}

/// Builds the focus monitor over the Win32 target, when present.
fn build_focus(target: Option<&WinTarget>) -> Option<Box<dyn FocusMonitor>> {
    #[cfg(all(windows, feature = "win32"))]
    {
        target.map(|t| Box::new(Win32Focus::new(t)) as Box<dyn FocusMonitor>)
    }
    #[cfg(not(all(windows, feature = "win32")))]
    {
        let _ = target;
        None
    }
}

/// What the discovery phase settled on: the concrete backend (`auto`
/// resolved), the discovered window (when one was needed), and the sink.
struct AcquiredSink {
    backend: SinkBackend,
    target: Option<WinTarget>,
    sink: Box<dyn InputSink>,
}

/// Outcome of waiting for a target window.
#[cfg(all(windows, feature = "win32"))]
enum TargetWait {
    /// A window was found (possibly after waiting).
    Found(WinTarget),
    /// None was found and `wait_for_window = false` said not to keep
    /// looking.
    NotFound,
    /// Ctrl+C arrived while waiting.
    Interrupted,
}

/// Whether the focus gate needs a discovered target window even when the
/// sink itself does not.
#[cfg(all(windows, feature = "win32"))]
fn needs_window(settings: &Settings) -> bool {
    settings.target.require_focus || !settings.clicker.global_hotkeys
}

/// Discovers the target, resolves `auto`, and builds the sink. Returns
/// `None` when Ctrl+C arrived while waiting.
#[cfg(all(windows, feature = "win32"))]
fn acquire(
    backend: SinkBackend,
    settings: &Settings,
    stop: &AtomicBool,
) -> anyhow::Result<Option<AcquiredSink>> {
    match backend {
        SinkBackend::Auto => {
            if settings.target.process_names.is_empty() {
                bail!("target.process_names is empty; list the process names to look for");
            }
            auto_acquire(settings, stop)
        }
        SinkBackend::Log => {
            let target = wait_target_if_needed(settings, stop)?;
            Ok(Some(AcquiredSink { backend, target, sink: Box::new(LoggingSink) }))
        }
        SinkBackend::VersaKbm => {
            #[cfg(feature = "versa-kbm")]
            {
                let target = wait_target_if_needed(settings, stop)?;
                let sink = Box::new(open_versa_kbm(&settings.sink.versa_kbm)?);
                Ok(Some(AcquiredSink { backend, target, sink }))
            }
            #[cfg(not(feature = "versa-kbm"))]
            {
                bail!("sink.backend = \"versa-kbm\" requires building with --features versa-kbm")
            }
        }
        SinkBackend::Win32 => {
            let target = match wait_for_target(&settings.target, stop)? {
                TargetWait::Found(target) => target,
                TargetWait::NotFound => {
                    bail!("sink.backend = \"win32\" but no target window was found")
                }
                TargetWait::Interrupted => return Ok(None),
            };
            let sink = Box::new(Win32Sink::new(target.clone()));
            Ok(Some(AcquiredSink { backend, target: Some(target), sink }))
        }
    }
}

/// Non-Windows variant: no target discovery exists; `auto` resolves to
/// the versa-kbm device when one is available.
#[cfg(not(all(windows, feature = "win32")))]
fn acquire(
    backend: SinkBackend,
    settings: &Settings,
    stop: &AtomicBool,
) -> anyhow::Result<Option<AcquiredSink>> {
    let _ = stop;
    let _ = settings;
    match backend {
        SinkBackend::Log => {
            Ok(Some(AcquiredSink { backend, target: None, sink: Box::new(LoggingSink) }))
        }
        SinkBackend::Win32 => {
            bail!("sink.backend = \"win32\" requires Windows and the \"win32\" feature")
        }
        SinkBackend::VersaKbm => {
            #[cfg(feature = "versa-kbm")]
            {
                Ok(Some(AcquiredSink {
                    backend,
                    target: None,
                    sink: Box::new(open_versa_kbm(&settings.sink.versa_kbm)?),
                }))
            }
            #[cfg(not(feature = "versa-kbm"))]
            {
                bail!("sink.backend = \"versa-kbm\" requires building with --features versa-kbm")
            }
        }
        SinkBackend::Auto => {
            #[cfg(feature = "versa-kbm")]
            {
                match open_versa_kbm(&settings.sink.versa_kbm) {
                    Ok(sink) => Ok(Some(AcquiredSink {
                        backend: SinkBackend::VersaKbm,
                        target: None,
                        sink: Box::new(sink),
                    })),
                    Err(err) => bail!("no input sink available for backend \"auto\": {err:#}"),
                }
            }
            #[cfg(not(feature = "versa-kbm"))]
            {
                bail!(
                    "no input sink available for backend \"auto\"; build with --features versa-kbm"
                )
            }
        }
    }
}

/// Waits for the target window only when a focus monitor needs one
/// ([`needs_window`]). `None` is tolerated there: the run-level checks
/// below report the missing monitor precisely.
#[cfg(all(windows, feature = "win32"))]
fn wait_target_if_needed(
    settings: &Settings,
    stop: &AtomicBool,
) -> anyhow::Result<Option<WinTarget>> {
    if !needs_window(settings) {
        return Ok(None);
    }
    match wait_for_target(&settings.target, stop)? {
        TargetWait::Found(target) => Ok(Some(target)),
        TargetWait::NotFound | TargetWait::Interrupted => Ok(None),
    }
}

/// Finds the target window, honoring `wait_for_window` and Ctrl+C.
#[cfg(all(windows, feature = "win32"))]
fn wait_for_target(
    target: &simple_clicker::config::TargetSettings,
    stop: &AtomicBool,
) -> anyhow::Result<TargetWait> {
    if target.process_names.is_empty() {
        bail!("target.process_names is empty; list the process names to look for");
    }
    let mut found = WinTarget::find(&target.process_names);
    while found.is_none() && target.wait_for_window {
        info!(
            "no target window yet (processes: {}); retrying in 2s (Ctrl+C to quit)",
            target.process_names.join(", ")
        );
        if interrupted_sleep(stop) {
            return Ok(TargetWait::Interrupted);
        }
        found = WinTarget::find(&target.process_names);
    }
    match found {
        Some(target) => Ok(TargetWait::Found(target)),
        None => Ok(TargetWait::NotFound),
    }
}

/// Sleeps one retry interval in [`WAIT_TICK`] chunks so Ctrl+C stays
/// responsive. `true` when the stop flag was seen.
#[cfg(all(windows, feature = "win32"))]
fn interrupted_sleep(stop: &AtomicBool) -> bool {
    let ticks = u32::try_from(TARGET_RESCAN.as_millis() / WAIT_TICK.as_millis()).unwrap_or(1);
    for _ in 0..ticks {
        if stop.load(Ordering::Relaxed) {
            return true;
        }
        sleep(WAIT_TICK);
    }
    false
}

/// Resolves `auto` on Windows: prefer the win32 sink once the target
/// window is found; while no window exists, take an available versa-kbm
/// device (unless focus gating still needs the window); keep waiting for
/// the window otherwise.
#[cfg(all(windows, feature = "win32"))]
fn auto_acquire(settings: &Settings, stop: &AtomicBool) -> anyhow::Result<Option<AcquiredSink>> {
    let names = settings.target.process_names.clone();
    #[cfg(feature = "versa-kbm")]
    let mut device_tried = false;
    loop {
        if let Some(target) = WinTarget::find(&names) {
            let sink = Box::new(Win32Sink::new(target.clone()));
            return Ok(Some(AcquiredSink {
                backend: SinkBackend::Win32,
                target: Some(target),
                sink,
            }));
        }
        // No window: the device wins only when focus gating does not
        // still need one.
        #[cfg(feature = "versa-kbm")]
        {
            if !device_tried && !settings.target.require_focus && settings.clicker.global_hotkeys {
                device_tried = true;
                match open_versa_kbm(&settings.sink.versa_kbm) {
                    Ok(sink) => {
                        return Ok(Some(AcquiredSink {
                            backend: SinkBackend::VersaKbm,
                            target: None,
                            sink: Box::new(sink),
                        }));
                    }
                    Err(err) => {
                        info!("versa-kbm sink not available, waiting for the window: {err:#}");
                    }
                }
            }
        }
        if !settings.target.wait_for_window {
            break;
        }
        info!(
            "no target window yet (processes: {}); retrying in 2s (Ctrl+C to quit)",
            names.join(", ")
        );
        if interrupted_sleep(stop) {
            return Ok(None);
        }
    }
    bail!(
        "no input sink available for backend \"auto\" \
         (win32 needs the target window; versa-kbm needs the device and --features versa-kbm)"
    )
}

/// Opens the versa-kbm device.
#[cfg(feature = "versa-kbm")]
fn open_versa_kbm(
    section: &simple_clicker::config::VersaKbmSection,
) -> anyhow::Result<VersaKbmSink> {
    VersaKbmSink::open(VersaKbmSettings {
        vid: section.vid,
        pid: section.pid,
        timeout_ms: section.timeout_ms,
        release_on_stop: section.release_on_stop,
    })
    .map_err(|err| anyhow::anyhow!("could not open the versa-kbm device: {err}"))
}

/// Prints every valid key and button name for config files.
fn print_keys() {
    let names: Vec<&str> = Key::ALL.iter().map(|key| key.name()).collect();
    for row in names.chunks(8) {
        println!("{}", row.join("  "));
    }
    println!(
        "buttons: {}",
        simple_clicker::domain::key::MouseButton::ALL
            .iter()
            .map(|b| b.name())
            .collect::<Vec<_>>()
            .join("  ")
    );
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::VecDeque,
        rc::Rc,
    };

    use simple_clicker::{
        config::VersaKbmSection,
        domain::engine::{
            Action,
            Bindings,
            EngineOptions,
            Millis,
            Press,
            Profile,
            Step,
        },
    };

    use super::*;

    // ------------------------------------------------------------- fakes

    struct FakeClock;

    impl Clock for FakeClock {
        fn now_ms(&self) -> Millis {
            0
        }
    }

    struct FixedJitter(Millis);

    impl Jitter for FixedJitter {
        fn sample_ms(&mut self, min_ms: Millis, max_ms: Millis) -> Millis {
            self.0.clamp(min_ms, max_ms)
        }
    }

    /// Queued presses: each poll pops the next batch, then always empty.
    struct FakeHotkeys {
        presses: VecDeque<Vec<Key>>,
    }

    impl HotkeySource for FakeHotkeys {
        fn poll(&mut self) -> Vec<Key> {
            self.presses.pop_front().unwrap_or_default()
        }

        fn describe(&self) -> String {
            "fake".to_owned()
        }
    }

    #[derive(Clone, Copy)]
    enum Plan {
        Ok,
        TargetGone,
        Transport,
    }

    /// Records every actuation; the plan decides each result. After
    /// `stop_after` actuations the shared stop flag is set, ending the
    /// loop deterministically.
    struct FakeSink {
        plan: Plan,
        sent: Rc<RefCell<Vec<Actuation>>>,
        stop: Arc<AtomicBool>,
        stop_after: usize,
    }

    impl InputSink for FakeSink {
        fn actuate(&mut self, actuation: &Actuation) -> Result<(), SinkError> {
            {
                let mut sent = self.sent.borrow_mut();
                sent.push(*actuation);
                if sent.len() >= self.stop_after {
                    self.stop.store(true, Ordering::Relaxed);
                }
            }
            match self.plan {
                Plan::Ok => Ok(()),
                Plan::TargetGone => Err(SinkError::TargetGone),
                Plan::Transport => Err(SinkError::Transport("boom".to_owned())),
            }
        }

        fn supports(&self, _key: Key) -> bool {
            true
        }

        fn describe(&self) -> String {
            "fake".to_owned()
        }
    }

    fn fake_sink(plan: Plan, stop: &Arc<AtomicBool>, stop_after: usize) -> FakeSink {
        FakeSink {
            plan,
            sent: Rc::new(RefCell::new(Vec::new())),
            stop: Arc::clone(stop),
            stop_after,
        }
    }

    /// A zero-interval, zero-hold tap engine toggled by Numpad4: each loop
    /// iteration bursts 8 actuations (the burst cap) once toggled on.
    fn tap_engine() -> Engine {
        Engine::new(
            vec![Profile { name: "super".to_owned(), interval_min_ms: 0, interval_max_ms: 0 }],
            vec![Action { name: "tap".to_owned(), steps: vec![Step::hold(Press::Key(Key::Q), 0)] }],
            Bindings::new(Some(Key::Numpad4), vec![], vec![]),
            EngineOptions { added_delay_ms: 0, require_focus: false, global_hotkeys: true },
        )
    }

    fn stack(sink: FakeSink) -> DriveStack {
        DriveStack {
            hotkeys: Box::new(FakeHotkeys { presses: VecDeque::from(vec![vec![Key::Numpad4]]) }),
            focus: None,
            sink: Box::new(sink),
            engine: tap_engine(),
            clock: Box::new(FakeClock),
            jitter: Box::new(FixedJitter(0)),
        }
    }

    // ------------------------------------------------------------- tests

    #[test]
    fn driver_delivers_actuations_until_stopped() {
        let stop = Arc::new(AtomicBool::new(false));
        let sink = fake_sink(Plan::Ok, &stop, 4);
        let sent = Rc::clone(&sink.sent);
        let result = drive(stack(sink), &stop, false, true);
        assert!(
            matches!(&result, Ok(StopReason::Interrupted)),
            "a working sink must run until stopped: {result:?}"
        );
        let sent = sent.borrow();
        assert_eq!(sent.first(), Some(&Actuation::KeyDown(Key::Q)), "taps must reach the sink");
        assert!(sent.contains(&Actuation::ReleaseAll), "the stop must release: {sent:?}");
    }

    #[test]
    fn driver_survives_target_gone_and_releases_on_stop() {
        let stop = Arc::new(AtomicBool::new(false));
        let sink = fake_sink(Plan::TargetGone, &stop, 3);
        let sent = Rc::clone(&sink.sent);
        let result = drive(stack(sink), &stop, false, true);
        assert!(
            matches!(&result, Ok(StopReason::Interrupted)),
            "TargetGone must pause, not fail the loop: {result:?}"
        );
        let sent = sent.borrow();
        assert!(sent.contains(&Actuation::ReleaseAll), "the stop must release: {sent:?}");
        // One 8-actuation burst (the 3rd actuation set the stop flag) plus
        // the final release-all.
        assert_eq!(sent.len(), 9, "burst + release-all expected, got {sent:?}");
    }

    #[test]
    fn driver_bails_after_ten_consecutive_transport_failures() {
        let stop = Arc::new(AtomicBool::new(false));
        let sink = fake_sink(Plan::Transport, &stop, usize::MAX);
        let sent = Rc::clone(&sink.sent);
        let result = drive(stack(sink), &stop, false, true);
        assert!(
            matches!(&result, Err(err) if err.to_string().contains("input sink keeps failing")),
            "ten transport failures must stop the loop: {result:?}"
        );
        let sent = sent.borrow();
        assert_eq!(
            sent.last(),
            Some(&Actuation::ReleaseAll),
            "the bail must release before stopping: {sent:?}"
        );
        assert_eq!(
            sent.len(),
            usize::try_from(MAX_CONSECUTIVE_SINK_FAILURES).unwrap_or(usize::MAX) + 1,
            "10 failures + the release-all expected, got {sent:?}"
        );
    }

    #[test]
    fn driver_bail_release_honors_the_release_all_opt_out() {
        let stop = Arc::new(AtomicBool::new(false));
        let sink = fake_sink(Plan::Transport, &stop, usize::MAX);
        let sent = Rc::clone(&sink.sent);
        let result = drive(stack(sink), &stop, false, false);
        assert!(result.is_err(), "the loop must still bail: {result:?}");
        let sent = sent.borrow();
        assert!(
            !sent.contains(&Actuation::ReleaseAll),
            "release_all_on_stop = false must suppress the release-all: {sent:?}"
        );
    }

    #[test]
    fn release_on_stop_defaults_true_except_the_versa_kbm_opt_out() {
        let versa = |release_on_stop: bool| SinkSettings {
            backend: SinkBackend::Auto,
            versa_kbm: VersaKbmSection { release_on_stop, ..VersaKbmSection::default() },
        };
        assert!(release_on_stop(&versa(true), SinkBackend::VersaKbm));
        assert!(!release_on_stop(&versa(false), SinkBackend::VersaKbm));
        assert!(
            release_on_stop(&versa(false), SinkBackend::Win32),
            "window messages latch nothing after exit"
        );
        assert!(release_on_stop(&versa(false), SinkBackend::Log));
        assert!(release_on_stop(&versa(false), SinkBackend::Auto));
    }

    #[cfg(all(windows, feature = "win32"))]
    #[test]
    fn needs_window_follows_the_focus_gate_requirements() {
        use simple_clicker::config::{
            ClickerSettings,
            TargetSettings,
        };

        let settings = |require_focus: bool, global_hotkeys: bool| Settings {
            target: TargetSettings {
                process_names: vec![],
                wait_for_window: true,
                exit_when_lost: true,
                require_focus,
            },
            clicker: ClickerSettings { global_hotkeys },
            engine: tap_engine(),
            sink: SinkSettings {
                backend: SinkBackend::Auto,
                versa_kbm: VersaKbmSection::default(),
            },
            hotkey_backend: HotkeyBackend::Auto,
            log_level: "info".to_owned(),
        };
        assert!(!needs_window(&settings(false, true)), "pure global hotkeying needs no window");
        assert!(needs_window(&settings(true, true)), "a focus gate needs one");
        assert!(needs_window(&settings(false, false)), "focus-gated hotkeys need one");
    }
}
