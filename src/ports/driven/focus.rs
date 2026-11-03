//! The focus-monitor port: can the target window receive input right now?

/// Outbound port: reports whether the target window currently has focus.
pub trait FocusMonitor {
    /// `Some(true)` when the target is focused, `Some(false)` when another
    /// window is, `None` when the state cannot be determined (the engine
    /// then degrades safely: presses are withheld, releases still fire).
    fn is_focused(&self) -> Option<bool>;

    /// A human-readable description of the backend for the startup banner.
    #[must_use]
    fn describe(&self) -> String;
}
