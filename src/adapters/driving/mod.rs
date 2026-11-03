//! Driving (inbound) adapters: hotkey sources.

#[cfg(feature = "hotkeys-rdev")]
pub mod rdev_hotkeys;

#[cfg(all(windows, feature = "win32"))]
pub mod win32_hotkeys;
