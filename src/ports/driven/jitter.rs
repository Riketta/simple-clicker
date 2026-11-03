//! The jitter port: the engine's only source of randomness.

use crate::domain::engine::Millis;

/// Outbound port: samples a value from an inclusive millisecond range.
/// Adapters wrap a real RNG; tests substitute deterministic values.
pub trait Jitter {
    /// Returns one value in `min_ms..=max_ms` (inclusive). When
    /// `min_ms >= max_ms` the result must be exactly `min_ms`.
    fn sample_ms(&mut self, min_ms: Millis, max_ms: Millis) -> Millis;
}
