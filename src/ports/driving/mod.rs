//! Driving (inbound) ports: sources that drive the core.

pub mod hotkeys;

pub use hotkeys::{
    HotkeyError,
    HotkeySource,
};
