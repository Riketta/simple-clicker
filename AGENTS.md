# AGENTS Instructions

SimpleClicker in Rust is a config-driven auto clicker that spams key and
mouse actions into a target application (typically World of Warcraft)
through swappable input adapters: Win32 window messages or the versa-kbm
RP2040 HID device. This file is the map and the rulebook for working in
this repo: architecture invariants, conventions, and the development gate.
It is deliberately NOT a feature manual — `README.md` documents what the
project does today, and rustdoc documents module contracts. Where this
file and the code disagree, the code wins; then fix the doc.

Keep this file small, durable, and always correct: global facts and rules
only. No version numbers, dependency lists, feature inventories, or
anything README/rustdoc already owns — those go stale here.

## Architecture

Classic hexagonal ports & adapters, single crate. Dependency direction is
strictly one-way:

```
adapters -> ports -> domain -> (nothing)
```

- `domain/` is pure: no I/O, no platform crates, no clock or randomness
  reads, no adapter types. Time arrives as `Millis` from the `Clock` port,
  randomness as the `Jitter` port; the engine is fully deterministic and
  unit-tested with fake ports.
- `ports/` are trait contracts plus DTOs. They may reference domain types,
  never adapter types. `driving` = sources that wake the core (hotkeys);
  `driven` = the core's sinks and queries (input sink, focus monitor,
  clock, jitter).
- `adapters/` are thin: validate, translate between the neutral vocabulary
  (`domain::key::Key`, `domain::engine::Actuation`) and the platform's
  code spaces (VK codes, HID usages, `rdev` keys), delegate. No business
  decisions in adapters; this is the only place platform/OS/transport
  crates are touched.
- `src/main.rs` is the composition root and the only file naming concrete
  adapter types. Adapter availability is decided there via Cargo features
  and `cfg`; feature gating lives in `Cargo.toml`, and a plain
  `cargo build` (no default features) must succeed on every host.
- Failure semantics: sink/hotkey failures are reported through port error
  enums and never panic; `SinkError::TargetGone` (lost target window) is
  not counted as a transport failure — the driver pauses actuation
  (`exit_when_lost = false`; the win32 adapters re-acquire the window
  themselves) or stops (`true`). Focus reads that cannot be answered
  return `None` and the engine withholds presses but always releases.
  Ctrl+C is a polled stop flag: the driver releases held inputs and exits
  with code 130.

## Layout

| Path | Contents |
|---|---|
| `src/main.rs` | composition root: adapter selection, validation, driver loop |
| `src/domain/` | pure core: `Key`/`MouseButton` vocabulary, the engine state machine |
| `src/ports/driving/` | inbound contracts (`HotkeySource`) |
| `src/ports/driven/` | outbound contracts (`InputSink`, `FocusMonitor`, `Clock`, `Jitter`) |
| `src/adapters/driving/` | hotkey adapters (win32 polling, rdev listener) |
| `src/adapters/driven/` | sinks (win32 messages, versa-kbm HID, dry-run logger), clock/jitter |
| `src/config.rs` | TOML schema, loader, validation into domain values |
| `config.example.toml` | checked-in documented template; kept in sync by a test |

## Conventions

- Errors: `anyhow` + `.context()` at the composition root only; concrete
  error enums (thiserror) in domain, ports, and adapters. No
  `unwrap`/`expect`/`panic` anywhere — the lint ladder denies them,
  including tests.
- Logging: `tracing`, structured fields; level from `[logging]` with
  `RUST_LOG` override.
- `config.example.toml` is a contract: update it in the same change that
  alters the schema in `src/config.rs` — a test parses the example file,
  so keys that leave the schema fail the gate; keys missing from the
  example take their defaults.
- The domain speaks only in neutral key/button values; adding a new input
  backend means adding an adapter plus a mapping table, never a domain
  change.
- Platform differences belong in adapters and config, never as branches
  inside domain code.

## Build & CI

The local gate IS the CI gate. After the LAST edit of a change, run
exactly, in order (feature-gated project: the clippy ladder repeats for
no-default and all-features):

1. `cargo +nightly fmt --all -- --check`
2. `cargo clippy --all-targets --all-features --locked -- -D warnings`
3. `cargo clippy --all-targets --no-default-features --locked -- -D warnings`
4. `cargo clippy --all-targets --locked -- -D warnings`
5. `cargo test --no-default-features --locked`
6. `cargo test --locked`
7. `cargo build --locked`

Plus `cargo deny check` when touching dependencies.

Rules: read the clippy exit status and `error:` lines, not the warning
count; `cargo test` does not execute clippy lints; dependency changes ship
an updated lockfile (every cargo gate runs `--locked`).

The versa-kbm adapter depends on the GitHub repo `Riketta/versa-kbm`
(branch `main`, git dependency, pinned to an exact commit by the
lockfile); bump it with `cargo update -p vkbm-cli`. To test local
versa-kbm changes, temporarily swap the dependency back to a path.

## Documentation maintenance

- `README.md` describes what the project does today; the Roadmap section
  is the only forward-looking part. Update it in the same change that adds
  or alters features, config, or layout.
- Keep this file's invariants, layout, and gate in sync in the same change
  that alters them. Re-check completeness periodically: no reference to
  files or commands that do not exist.
