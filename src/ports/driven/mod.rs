//! Driven (outbound) ports: the core's queries and sinks.

pub mod clock;
pub mod focus;
pub mod input;
pub mod jitter;

pub use clock::Clock;
pub use focus::FocusMonitor;
pub use input::{
    InputSink,
    SinkError,
};
pub use jitter::Jitter;
