//! Standard-library time adapters: the process monotonic clock and a
//! thread-local RNG jitter source. Always compiled.

use std::time::Instant;

use rand::Rng;

use crate::{
    domain::engine::Millis,
    ports::driven::{
        Clock,
        Jitter,
    },
};

/// [`Clock`] over `std::time::Instant`, measured from construction.
#[derive(Clone, Copy, Debug)]
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    /// Anchors the clock at construction time.
    #[must_use]
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> Millis {
        let elapsed = self.start.elapsed().as_millis();
        // u128 -> u64: a process would have to run for ~584 million years
        // to overflow; saturate instead of panicking on the impossible.
        u64::try_from(elapsed).unwrap_or(u64::MAX)
    }
}

/// [`Jitter`] over the `rand` thread RNG: uniform in the inclusive range.
#[derive(Debug, Default)]
pub struct SystemJitter {
    rng: rand::rngs::ThreadRng,
}

impl SystemJitter {
    /// Creates the jitter source (seeds the thread RNG lazily).
    #[must_use]
    pub fn new() -> Self {
        Self { rng: rand::rng() }
    }
}

impl Jitter for SystemJitter {
    fn sample_ms(&mut self, min_ms: Millis, max_ms: Millis) -> Millis {
        if max_ms <= min_ms {
            return min_ms;
        }
        self.rng.random_range(min_ms..=max_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Clock,
        SystemClock,
        SystemJitter,
    };
    use crate::ports::driven::Jitter;

    #[test]
    fn clock_marches_forward() {
        let clock = SystemClock::new();
        let first = clock.now_ms();
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert!(clock.now_ms() >= first, "clock must not go backwards");
    }

    #[test]
    fn jitter_respects_degenerate_ranges() {
        let mut jitter = SystemJitter::new();
        assert_eq!(jitter.sample_ms(5, 5), 5);
        assert_eq!(jitter.sample_ms(7, 3), 7, "min wins when min > max");
    }

    #[test]
    fn jitter_stays_in_range() {
        let mut jitter = SystemJitter::new();
        for _ in 0..100 {
            let sample = jitter.sample_ms(10, 30);
            assert!((10..=30).contains(&sample), "sample {sample} outside 10..=30");
        }
    }
}
