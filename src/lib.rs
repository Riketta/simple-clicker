//! `SimpleClicker` in Rust: a config-driven auto clicker that spams key and
//! mouse actions into a target application through swappable input adapters
//! (Win32 window messages, or the versa-kbm USB HID device).
//!
//! The layout follows classic hexagonal ports & adapters with a strict
//! one-way dependency direction: `adapters -> ports -> domain`. See
//! `AGENTS.md` for the invariants and `README.md` for what the tool does
//! today.

pub mod adapters;
pub mod config;
pub mod domain;
pub mod ports;
