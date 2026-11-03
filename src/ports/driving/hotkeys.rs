//! The hotkey port: a source of freshly pressed hotkeys.

use crate::domain::key::Key;

/// Failures while setting a hotkey source up (polling itself never fails:
/// a broken source simply stops reporting presses).
#[derive(Debug, thiserror::Error)]
pub enum HotkeyError {
    /// The backend could not start (device grab failed, listener thread
    /// died, OS API refused).
    #[error("hotkey backend failed to start: {0}")]
    Start(String),
}

/// Inbound port: reports keys newly pressed since the last poll
/// (edge-detected by the adapter; a key held down is reported once).
///
/// The adapter owns the edge detection and the watch set (the engine's
/// configured hotkeys); the domain decides what the presses mean.
pub trait HotkeySource {
    /// Drains the presses that happened since the previous call. Returns
    /// them in arrival order; never blocks.
    fn poll(&mut self) -> Vec<Key>;

    /// A human-readable description of the backend for the startup banner.
    #[must_use]
    fn describe(&self) -> String;
}
