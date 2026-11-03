//! The input-sink port: where actuations are delivered.

use crate::domain::{
    engine::Actuation,
    key::Key,
};

/// Failures an input sink can report.
#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    /// The transport or OS call failed; the message carries the adapter's
    /// specifics. The engine treats this as "this actuation may not have
    /// landed" and keeps going.
    #[error("input transport failed: {0}")]
    Transport(String),
    /// The sink cannot express this key in its code space.
    #[error("key {0} is not supported by this input adapter")]
    Unsupported(Key),
    /// The sink's bound target window is gone (its process exited). Not a
    /// transport failure: the driver pauses instead of counting it, and
    /// actuation resumes once the sink re-acquires a target (or stops,
    /// per `target.exit_when_lost`).
    #[error("target window is gone")]
    TargetGone,
}

/// Outbound port: delivers the engine's actuations to some virtual keyboard
/// and mouse (window messages, a HID hardware device, a logger, ...).
///
/// Sinks must never leave input latched on a best-effort path:
/// [`Actuation::ReleaseAll`] releases everything the sink may hold, and
/// every down actuation is paired by the engine with an up (immediately,
/// on the schedule, or at shutdown via
/// [`Actuation::ReleaseAll`]). Window-bound sinks re-acquire a restarted
/// target themselves (rate-limited) and report
/// [`SinkError::TargetGone`] while none is reachable.
pub trait InputSink {
    /// Delivers one actuation. Errors do not stop the clicker by
    /// themselves; the driver counts consecutive failures.
    ///
    /// # Errors
    /// Transport-specific failures are reported as [`SinkError`]; the
    /// driver decides whether to keep going.
    fn actuate(&mut self, actuation: &Actuation) -> Result<(), SinkError>;

    /// Whether this sink can express `key` in its code space (the driver
    /// validates the whole config at startup and fails fast otherwise).
    #[must_use]
    fn supports(&self, key: Key) -> bool;

    /// A human-readable description of the backend for the startup banner.
    #[must_use]
    fn describe(&self) -> String;

    /// Whether the sink's target is still reachable: sinks bound to a
    /// specific window report its liveness, everything else reports
    /// `true`. The driver uses this for `target.exit_when_lost`.
    fn is_target_alive(&self) -> bool {
        true
    }
}
