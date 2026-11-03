//! Driven (outbound) adapters: input sinks, focus monitoring, time.

pub mod logging_sink;
pub mod std_time;

#[cfg(all(windows, feature = "win32"))]
pub mod win32;

#[cfg(feature = "versa-kbm")]
pub mod versa_kbm;
