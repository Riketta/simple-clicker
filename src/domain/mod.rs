//! Pure core: the clicker state machine and its value types.
//!
//! No I/O, no platform crates, no clock or randomness reads: time and
//! randomness arrive as [`crate::ports::driven`] handles. The domain speaks
//! only in neutral values ([`key::Key`], [`engine::Actuation`]); every
//! platform difference lives in adapters.

pub mod engine;
pub mod key;
