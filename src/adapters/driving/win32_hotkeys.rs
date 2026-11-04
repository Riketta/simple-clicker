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
            if edge_pressed(&mut entry.was_down, down) {
                pressed.push(entry.key);
            }
        }
        pressed
    }

    fn describe(&self) -> String {
        format!("win32 (GetAsyncKeyState, global; {} watched keys)", self.watched.len())
    }
}

/// One watched key's edge detector: reports `true` exactly on the
/// released-to-pressed transition, so a key held down is reported once.
fn edge_pressed(was_down: &mut bool, down: bool) -> bool {
    let pressed = down && !*was_down;
    *was_down = down;
    pressed
}

#[cfg(test)]
mod tests {
    use super::{
        Win32Hotkeys,
        edge_pressed,
    };
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

    #[test]
    fn a_held_key_is_reported_once_until_released() {
        // (was_down, down) -> pressed: the full transition table.
        for (was_down, down, expected) in
            [(false, false, false), (false, true, true), (true, true, false), (true, false, false)]
        {
            let mut was_down = was_down;
            assert_eq!(edge_pressed(&mut was_down, down), expected, "from {was_down:?}");
            assert_eq!(was_down, down, "the state must track the current press");
        }
    }

    #[test]
    fn a_repress_is_reported_again() {
        let mut was_down = false;
        assert!(edge_pressed(&mut was_down, true), "first press");
        assert!(!edge_pressed(&mut was_down, true), "still held");
        assert!(!edge_pressed(&mut was_down, false), "release");
        assert!(edge_pressed(&mut was_down, true), "second press");
    }
}
