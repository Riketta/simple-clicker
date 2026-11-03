//! Composition root: parses the CLI, loads and validates config, picks
//! concrete adapters, wires the engine, and runs the driver loop.
//!
//! This is the only file that names concrete adapter types. Adapter
//! availability is decided here (Cargo features + platform), never in the
//! domain.

use std::{
    path::PathBuf,
    process::ExitCode,
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
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    if cli.list_keys {
        print_keys();
        return Ok(());
    }

    let settings = simple_clicker::config::load(&cli.config)
        .with_context(|| format!("could not load {}", cli.config.display()))?;

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&settings.log_level));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let mut sink_backend = match cli.sink.as_deref() {
        Some(raw) => SinkBackend::parse(raw).ok_or_else(|| {
            anyhow::anyhow!("--sink: unknown value '{raw}' (valid: auto, win32, versa-kbm, log)")
        })?,
        None => settings.sink.backend,
    };
    let hotkey_backend = match cli.hotkeys.as_deref() {
        Some(raw) => HotkeyBackend::parse(raw).ok_or_else(|| {
            anyhow::anyhow!("--hotkeys: unknown value '{raw}' (valid: auto, win32, rdev)")
        })?,
        None => settings.hotkey_backend,
    };
    if cli.dry_run {
        sink_backend = SinkBackend::Log;
    }

    let mut watch = settings.engine.configured_keys();
    watch.sort_unstable();
    watch.dedup();
    let hotkeys = build_hotkeys(hotkey_backend, &watch)?;

    // Target discovery (Win32 only): needed by the win32 sink and by the
    // focus monitor.
    #[cfg(all(windows, feature = "win32"))]
    let wants_target = !matches!(sink_backend, SinkBackend::VersaKbm | SinkBackend::Log)
        || settings.target.require_focus;
    #[cfg(all(windows, feature = "win32"))]
    let target: Option<WinTarget> =
        if wants_target { find_target(&settings.target)? } else { None };
    #[cfg(not(all(windows, feature = "win32")))]
    let target: Option<WinTarget> = None;

    let focus = build_focus(target.as_ref());
    if settings.target.require_focus && focus.is_none() {
        bail!(
            "target.require_focus = true but no focus monitor is available \
             (only the Win32 adapter provides one, on Windows); \
             set require_focus = false to run without a focus check"
        );
    }
    if !settings.clicker.global_hotkeys && focus.is_none() {
        bail!(
            "clicker.global_hotkeys = false needs a focus monitor to gate hotkeys \
             (only the Win32 adapter provides one, on Windows)"
        );
    }

    let sink = build_sink(sink_backend, target.as_ref(), &settings)?;

    info!("### Simple Clicker v{} ###", env!("CARGO_PKG_VERSION"));
    info!("[!] Sink: {}", sink.describe());
    info!("[!] Hotkeys: {}", hotkeys.describe());
    if let Some(monitor) = &focus {
        info!("[!] Focus: {}", monitor.describe());
    }
    if let Some(toggle) = settings.engine.toggle_key() {
        info!("[!] Toggle: [{toggle}]");
    }
    info!("[!] Profiles: {}", settings.engine.profile_summary().join(", "));
    info!("[!] Actions: {}", settings.engine.action_summary().join(", "));
    info!("[!] {}", settings.engine.describe());

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

    let release_all = release_on_stop(&settings, sink_backend);
    drive(
        DriveStack { hotkeys, focus, sink, engine: settings.engine },
        settings.target.exit_when_lost,
        release_all,
    )
}

/// Everything the driver loop needs, wired by the composition root.
struct DriveStack {
    hotkeys: Box<dyn HotkeySource>,
    focus: Option<Box<dyn FocusMonitor>>,
    sink: Box<dyn InputSink>,
    engine: Engine,
}

/// The driver loop: poll hotkeys, advance the engine, deliver actuations,
/// sleep until the next interesting moment. Releases everything on exit.
fn drive(stack: DriveStack, exit_when_lost: bool, release_all_on_stop: bool) -> anyhow::Result<()> {
    let DriveStack { mut hotkeys, focus, mut sink, mut engine } = stack;
    let clock = SystemClock::new();
    let mut jitter = SystemJitter::new();
    let mut consecutive_failures = 0u32;
    let mut target_lost = false;

    loop {
        let now = clock.now_ms();
        let pressed = hotkeys.poll();
        let focus_now = focus.as_ref().and_then(|monitor| monitor.is_focused());
        let immediate = engine.handle_keys(&pressed, now, focus_now);
        let due = engine.poll(now, focus_now, &mut jitter);
        for actuation in immediate.into_iter().chain(due) {
            match sink.actuate(&actuation) {
                Ok(()) => consecutive_failures = 0,
                Err(err) => {
                    consecutive_failures += 1;
                    tracing::warn!("actuation failed ({consecutive_failures} in a row): {err}");
                    if consecutive_failures >= MAX_CONSECUTIVE_SINK_FAILURES {
                        release(&mut engine, &mut sink, true);
                        bail!("input sink keeps failing; stopping (last error: {err})");
                    }
                }
            }
        }

        if exit_when_lost {
            if !sink.is_target_alive() {
                info!("target window is gone; stopping");
                break;
            }
        } else if !target_lost && !sink.is_target_alive() {
            target_lost = true;
            tracing::warn!("target window is gone; continuing (target.exit_when_lost = false)");
        }

        let sleep_ms = match engine.next_wake() {
            Some(at) => at.saturating_sub(now).clamp(1, POLL_INTERVAL_MS),
            None => POLL_INTERVAL_MS,
        };
        sleep(Duration::from_millis(sleep_ms));
    }

    release(&mut engine, &mut sink, release_all_on_stop);
    Ok(())
}

/// Releases the engine's held state through the sink; `include_release_all`
/// controls whether the final release-all is sent (off when the user asked
/// the versa-kbm device to keep its state).
fn release(engine: &mut Engine, sink: &mut Box<dyn InputSink>, include_release_all: bool) {
    for actuation in engine.halt() {
        if actuation == Actuation::ReleaseAll && !include_release_all {
            continue;
        }
        if let Err(err) = sink.actuate(&actuation) {
            tracing::warn!("release failed: {err}");
        }
    }
}

/// Whether the final release-all should reach the device.
fn release_on_stop(settings: &Settings, backend: SinkBackend) -> bool {
    backend != SinkBackend::VersaKbm || settings.sink.versa_kbm.release_on_stop
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

/// Finds the target window, honoring `wait_for_window`.
#[cfg(all(windows, feature = "win32"))]
fn find_target(
    target: &simple_clicker::config::TargetSettings,
) -> anyhow::Result<Option<WinTarget>> {
    if target.process_names.is_empty() {
        bail!("target.process_names is empty; list the process names to look for");
    }
    let mut found = WinTarget::find(&target.process_names);
    while found.is_none() && target.wait_for_window {
        info!(
            "no target window yet (processes: {}); retrying in 2s (Ctrl+C to quit)",
            target.process_names.join(", ")
        );
        sleep(Duration::from_secs(2));
        found = WinTarget::find(&target.process_names);
    }
    Ok(found)
}

/// Builds the input sink for the chosen backend.
fn build_sink(
    backend: SinkBackend,
    target: Option<&WinTarget>,
    settings: &Settings,
) -> anyhow::Result<Box<dyn InputSink>> {
    #[cfg(not(feature = "versa-kbm"))]
    let _ = settings;
    #[cfg(not(all(windows, feature = "win32")))]
    let _ = target;
    match backend {
        SinkBackend::Log => Ok(Box::new(LoggingSink)),
        SinkBackend::Win32 => {
            #[cfg(all(windows, feature = "win32"))]
            {
                let Some(win_target) = target else {
                    bail!("sink.backend = \"win32\" but no target window was found");
                };
                Ok(Box::new(Win32Sink::new(win_target.clone())))
            }
            #[cfg(not(all(windows, feature = "win32")))]
            {
                bail!("sink.backend = \"win32\" requires Windows and the \"win32\" feature");
            }
        }
        SinkBackend::VersaKbm => {
            #[cfg(feature = "versa-kbm")]
            {
                Ok(Box::new(open_versa_kbm(&settings.sink.versa_kbm)?))
            }
            #[cfg(not(feature = "versa-kbm"))]
            {
                bail!("sink.backend = \"versa-kbm\" requires building with --features versa-kbm");
            }
        }
        SinkBackend::Auto => {
            #[cfg(all(windows, feature = "win32"))]
            if let Some(win_target) = target {
                return Ok(Box::new(Win32Sink::new(win_target.clone())));
            }
            #[cfg(feature = "versa-kbm")]
            match open_versa_kbm(&settings.sink.versa_kbm) {
                Ok(sink) => return Ok(Box::new(sink)),
                Err(err) => info!("versa-kbm sink not available, trying other backends: {err:#}"),
            }
            bail!(
                "no input sink available for backend \"auto\" \
                 (win32 needs the target window; versa-kbm needs the device and --features versa-kbm)"
            );
        }
    }
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
