//! The clock port: monotonic time as fed into the engine.

use crate::domain::engine::Millis;

/// Outbound port: the engine's only source of time. Adapters wrap the
/// platform clock; tests substitute deterministic values.
pub trait Clock {
    /// Current monotonic time in milliseconds from an arbitrary epoch
    /// (only differences matter). Must be non-decreasing between calls.
    fn now_ms(&self) -> Millis;
}
