//! Win32 driving adapter: hotkey polling through `GetAsyncKeyState`.
//!
//! `GetAsyncKeyState` reads global key state, so hotkeys are recognized
//! even when the clicker (or the game) is not focused — an intentional
//! improvement over the original tool, whose hotkey reads only worked
//! while the game was focused.

use crate::{
    adapters::driven::win32::vk_code,
    domain::key::Key,
    ports::driving::HotkeySource,
};

/// One watched key and its previous global down-state (for edge
/// detection).
#[derive(Debug)]
struct Watched {
    key: Key,
    vk: u16,
    was_down: bool,
}

/// The Win32 hotkey source.
#[derive(Debug)]
pub struct Win32Hotkeys {
    watched: Vec<Watched>,
}

impl Win32Hotkeys {
    /// Watches exactly the configured hotkeys; other keys are ignored.
    #[must_use]
    pub fn new(watched_keys: &[Key]) -> Self {
        Self {
            watched: watched_keys
                .iter()
                .map(|&key| Watched { key, vk: vk_code(key), was_down: false })
                .collect(),
        }
    }
}

impl HotkeySource for Win32Hotkeys {
    fn poll(&mut self) -> Vec<Key> {
        let mut pressed = Vec::new();
        for entry in &mut self.watched {
            // SAFETY: the call reads global key state and takes no
            // pointers. The high bit (negative i16) means currently down.
            let down = unsafe {
                windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(i32::from(
                    entry.vk,
                ))
            } < 0;
            if down && !entry.was_down {
                pressed.push(entry.key);
            }
            entry.was_down = down;
        }
        pressed
    }

    fn describe(&self) -> String {
        format!("win32 (GetAsyncKeyState, global; {} watched keys)", self.watched.len())
    }
}

#[cfg(test)]
mod tests {
    use super::Win32Hotkeys;
    use crate::{
        domain::key::Key,
        ports::driving::HotkeySource,
    };

    #[test]
    fn poll_reports_nothing_without_keys_held() {
        let mut hotkeys = Win32Hotkeys::new(&[Key::Numpad4, Key::Q]);
        // Nothing is pressed in a test environment; a single poll must be
        // empty rather than misfiring on initialization.
        assert_eq!(hotkeys.poll(), []);
    }
}
