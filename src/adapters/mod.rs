//! Adapter implementations for the ports.
//!
//! This is the only place platform, OS, and transport crates are touched.
//! Adapters are thin: validate, translate to and from the neutral domain
//! vocabulary, delegate; no business decisions live here. Platform-specific
//! modules are gated by target and Cargo feature, so a plain
//! `cargo build` (no default features) succeeds on every host.

pub mod driven;
pub mod driving;
