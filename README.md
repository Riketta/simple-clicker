# SimpleClicker (Rust)

A config-driven auto clicker for tagging mobs in World of Warcraft — a Rust
rewrite of the original C# SimpleClicker, generalized around a hexagonal
architecture: the clicker core is platform-blind, and every input surface is
a swappable adapter.

## What it does

While running, the tool watches a set of hotkeys and, when toggled on,
spams the selected **action** (a sequence of key presses, mouse clicks, and
pauses) into the target application with the pacing of the selected
**profile** — a `super` mode with no delay, or a `human` mode that jitters
the gap between repetitions. Everything (hotkeys, profiles, actions,
adapter selection) lives in `config.toml`.

### Default hotkeys (as configured in `config.example.toml`)

| Key | Effect |
|---|---|
| `Numpad4` | Toggle clicking on/off |
| `Numpad5` | Profile: super (no input delay) |
| `Numpad6` | Profile: human (15–60 ms jitter) |
| `Numpad2` | Action: spam spell key `Q` |
| `Numpad3` | Action: spam interact key `Numpad0` |

## Adapters

| Port | Adapter | Platforms | Notes |
|---|---|---|---|
| Input sink | `win32` (`--features win32`, default on Windows) | Windows | Keyboard via `WM_KEYDOWN`/`WM_KEYUP` messages into the target window — works while the game is in the background. Mouse clicks are global `SendInput` (the real cursor). |
| Input sink | `versa-kbm` (`--features versa-kbm`, default) | Windows, Linux, macOS | Actuates through the [versa-kbm](https://github.com/Riketta/versa-kbm) RP2040 HID device: the inputs land in whatever window is focused on the machine the device is plugged into. Input goes to the *focused* window. |
| Input sink | `log` (always built) | all | Dry-run: logs actuations, touches nothing (`--dry-run`). |
| Hotkeys | `win32` (Windows) | Windows | `GetAsyncKeyState` polling — global, so hotkeys work while the game is focused or not. |
| Hotkeys | `rdev` (`--features hotkeys-rdev`) | Linux/X11, macOS, Windows | Global event listener. macOS needs Accessibility permissions; Wayland is not supported. |
| Focus monitor | `win32` (Windows) | Windows | Compares the foreground window with the target. |

`backend = "auto"` picks the best compiled-in adapter: `win32` on Windows
once the target window is found; while no window exists yet, `versa-kbm`
when the device is present (unless `require_focus` or `global_hotkeys =
false` still needs the window for the focus monitor). When the target
window disappears, `exit_when_lost = true` (default) stops the clicker;
with `false`, actuation pauses, the win32 adapters re-scan for the window
every 2s, and clicking resumes when it returns.

### Cross-platform honesty

- **Keyboard into a background window** (the original tool's trick) is only
  possible through Win32 window messages — that adapter is Windows-only by
  nature.
- **versa-kbm is fully cross-platform** on the host side (`hidapi`), but the
  device actuates the focused window, so it cannot drive a background
  window anywhere.
- On non-Windows hosts there is no focus monitor: `target.require_focus`
  must be `false` there (startup fails with that hint otherwise).

## Usage

```sh
cargo run -- --config config.toml          # default sink/hotkey backends
cargo run -- --dry-run                     # log actuations only
cargo run -- --list-keys                   # print valid key names
cargo run -- --sink versa-kbm --hotkeys rdev
```

1. `cp config.example.toml config.toml` and adjust process names, hotkeys,
   profiles, and actions.
2. Build with the features you need:

```sh
cargo build --release --all-features                       # everything
cargo build --release --no-default-features --features versa-kbm,hotkeys-rdev    # no Win32
cargo build --release                                      # Win32 + versa-kbm
```

Key names follow `config.example.toml`; run `--list-keys` for the full
list. Actions support `key` steps (with `hold_ms`), `button` steps
(`left`/`right`/`middle`), and pause steps (no `key`/`button`, `hold_ms`
required) — a repetition walks the steps in order, then waits the profile's
interval (jitter-sampled plus `clicker.added_delay_ms`, the old
`--addeddelay=` argument).

### The original tagging macro still works

```
/cleartarget
/tar reef crawler
/tar fine
/stopmacro [noexists][dead]
/script SetRaidTarget("target", 6);
/cast Judgement
```

Bind it to a key, select the `spell` action, and hold the line.

## versa-kbm notes

- The device must be **armed** (`vkbm arm`) with the hardware BOOTSEL
  switch on; the sink reports both states at startup and translates
  `ERR DISABLED`/`ERR DISARMED` refusals into actionable log lines.
- `sink.versa-kbm.release_on_stop = true` (default) releases every held
  key and button on the device when the clicker stops — including on
  Ctrl+C, and a `Drop` handler on the sink releases as a last resort
  (panics, unusual signals).
- Toggle-off always releases immediately, so no inputs stay latched.

## Building, testing, the gate

The local gate IS the CI gate (see `AGENTS.md` for the exact commands):
nightly rustfmt, clippy `-D warnings` across feature combinations
(default / no-default / all-features), tests, build, and `cargo deny check`.

Linux build dependencies for the adapters: `libudev-dev` (hidapi),
`libx11-dev libxi-dev libxtst-dev` (rdev).

## Limitations

- Mouse clicks through the `win32` sink move the real cursor (global
  input), matching the original tool's `mouse_event` behavior.
- The `win32` sink sends keyboard messages synchronously
  (`SendMessageW`): a hung target window can stall the clicker loop. The
  versa-kbm sink has no such path.
- With `clicker.global_hotkeys = false`, the toggle key is only honored
  while the target window is confirmed focused — refocus the target to
  toggle off.
- The `versa-kbm` control channel has no authentication — see the
  [versa-kbm security notes](https://github.com/Riketta/versa-kbm#security--limitations).
- A hard kill (taskkill, SIGKILL, power loss) skips the final release;
  the engine's taps are millisecond-short, and `vkbm release` / the
  BOOTSEL switch clean up.
- `rdev` cannot observe keys on Wayland sessions.

## Roadmap

- X11 focus monitor for Linux (so `require_focus` works with the
  versa-kbm sink there).
- Absolute-positioned message-based mouse clicks for the win32 sink
  (background clicks at client coordinates).
- A sequence editor / profile hot-reload.

## Origin

Rewrite of [SimpleClicker](https://github.com/Riketta/SimpleClicker) (C#, `SendMessage`
spam with hardcoded hotkeys). The Rust version keeps the defaults and
behavior, and adds config-driven actions, profiles, and swappable input
adapters. MIT licensed, like the original.
