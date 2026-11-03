//! `rdev` driving adapter: a global hotkey listener for hosts without the
//! Win32 backend (Linux/X11, macOS; Windows also supported, where the
//! native backend is usually preferred).
//!
//! Platform caveats: Linux/X11 needs a running X server (Wayland sessions
//! are not supported by `rdev`'s listener), and macOS requires the process
//! to be granted Accessibility input permissions.

use std::{
    collections::HashSet,
    sync::mpsc::{
        Receiver,
        sync_channel,
    },
};

use tracing::warn;

use crate::{
    domain::key::Key,
    ports::driving::{
        HotkeyError,
        HotkeySource,
    },
};

/// The `rdev` hotkey source: a background listener thread forwards
/// key-press events over a bounded channel; [`HotkeySource::poll`] drains
/// it.
#[derive(Debug)]
pub struct RdevHotkeys {
    rx: Receiver<Key>,
}

impl RdevHotkeys {
    /// Starts the global listener, watching exactly the configured hotkeys.
    ///
    /// # Errors
    /// [`HotkeyError::Start`] when the listener thread cannot spawn or the
    /// platform hook cannot be installed.
    pub fn new(watched_keys: &[Key]) -> Result<Self, HotkeyError> {
        let (tx, rx) = sync_channel(256);
        let watch: HashSet<Key> = watched_keys.iter().copied().collect();
        std::thread::Builder::new()
            .name("rdev-hotkeys".to_owned())
            .spawn(move || {
                let result = rdev::listen(move |event: rdev::Event| {
                    if let rdev::EventType::KeyPress(key) = event.event_type
                        && let Some(domain_key) = map_key(key)
                        && watch.contains(&domain_key)
                    {
                        // Drop instead of blocking the listen thread when
                        // a burst fills the channel.
                        let _ = tx.try_send(domain_key);
                    }
                });
                if let Err(err) = result {
                    warn!("rdev hotkey listener stopped: {err:?}");
                }
            })
            .map_err(|err| HotkeyError::Start(err.to_string()))?;
        Ok(Self { rx })
    }
}

impl HotkeySource for RdevHotkeys {
    fn poll(&mut self) -> Vec<Key> {
        let mut pressed = Vec::new();
        while let Ok(key) = self.rx.try_recv() {
            pressed.push(key);
        }
        pressed
    }

    fn describe(&self) -> String {
        "rdev (global event listener)".to_owned()
    }
}

/// Maps an `rdev` key into the domain vocabulary; unrepresentable keys
/// (media, IME, meta) are dropped.
// A total mapping table over the source enum is inherently long; the
// length is mechanical, not a design smell.
#[allow(clippy::too_many_lines)]
#[must_use]
fn map_key(key: rdev::Key) -> Option<Key> {
    match key {
        rdev::Key::KeyA => Some(Key::A),
        rdev::Key::KeyB => Some(Key::B),
        rdev::Key::KeyC => Some(Key::C),
        rdev::Key::KeyD => Some(Key::D),
        rdev::Key::KeyE => Some(Key::E),
        rdev::Key::KeyF => Some(Key::F),
        rdev::Key::KeyG => Some(Key::G),
        rdev::Key::KeyH => Some(Key::H),
        rdev::Key::KeyI => Some(Key::I),
        rdev::Key::KeyJ => Some(Key::J),
        rdev::Key::KeyK => Some(Key::K),
        rdev::Key::KeyL => Some(Key::L),
        rdev::Key::KeyM => Some(Key::M),
        rdev::Key::KeyN => Some(Key::N),
        rdev::Key::KeyO => Some(Key::O),
        rdev::Key::KeyP => Some(Key::P),
        rdev::Key::KeyQ => Some(Key::Q),
        rdev::Key::KeyR => Some(Key::R),
        rdev::Key::KeyS => Some(Key::S),
        rdev::Key::KeyT => Some(Key::T),
        rdev::Key::KeyU => Some(Key::U),
        rdev::Key::KeyV => Some(Key::V),
        rdev::Key::KeyW => Some(Key::W),
        rdev::Key::KeyX => Some(Key::X),
        rdev::Key::KeyY => Some(Key::Y),
        rdev::Key::KeyZ => Some(Key::Z),
        rdev::Key::Num0 => Some(Key::Num0),
        rdev::Key::Num1 => Some(Key::Num1),
        rdev::Key::Num2 => Some(Key::Num2),
        rdev::Key::Num3 => Some(Key::Num3),
        rdev::Key::Num4 => Some(Key::Num4),
        rdev::Key::Num5 => Some(Key::Num5),
        rdev::Key::Num6 => Some(Key::Num6),
        rdev::Key::Num7 => Some(Key::Num7),
        rdev::Key::Num8 => Some(Key::Num8),
        rdev::Key::Num9 => Some(Key::Num9),
        rdev::Key::F1 => Some(Key::F1),
        rdev::Key::F2 => Some(Key::F2),
        rdev::Key::F3 => Some(Key::F3),
        rdev::Key::F4 => Some(Key::F4),
        rdev::Key::F5 => Some(Key::F5),
        rdev::Key::F6 => Some(Key::F6),
        rdev::Key::F7 => Some(Key::F7),
        rdev::Key::F8 => Some(Key::F8),
        rdev::Key::F9 => Some(Key::F9),
        rdev::Key::F10 => Some(Key::F10),
        rdev::Key::F11 => Some(Key::F11),
        rdev::Key::F12 => Some(Key::F12),
        rdev::Key::Kp0 => Some(Key::Numpad0),
        rdev::Key::Kp1 => Some(Key::Numpad1),
        rdev::Key::Kp2 => Some(Key::Numpad2),
        rdev::Key::Kp3 => Some(Key::Numpad3),
        rdev::Key::Kp4 => Some(Key::Numpad4),
        rdev::Key::Kp5 => Some(Key::Numpad5),
        rdev::Key::Kp6 => Some(Key::Numpad6),
        rdev::Key::Kp7 => Some(Key::Numpad7),
        rdev::Key::Kp8 => Some(Key::Numpad8),
        rdev::Key::Kp9 => Some(Key::Numpad9),
        rdev::Key::KpReturn => Some(Key::NumpadEnter),
        rdev::Key::KpMinus => Some(Key::NumpadSubtract),
        rdev::Key::KpPlus => Some(Key::NumpadAdd),
        rdev::Key::KpMultiply => Some(Key::NumpadMultiply),
        rdev::Key::KpDivide => Some(Key::NumpadDivide),
        rdev::Key::Space => Some(Key::Space),
        rdev::Key::Tab => Some(Key::Tab),
        rdev::Key::Return => Some(Key::Enter),
        rdev::Key::Escape => Some(Key::Escape),
        rdev::Key::Backspace => Some(Key::Backspace),
        rdev::Key::LeftArrow => Some(Key::Left),
        rdev::Key::RightArrow => Some(Key::Right),
        rdev::Key::UpArrow => Some(Key::Up),
        rdev::Key::DownArrow => Some(Key::Down),
        rdev::Key::Insert => Some(Key::Insert),
        rdev::Key::Delete => Some(Key::Delete),
        rdev::Key::Home => Some(Key::Home),
        rdev::Key::End => Some(Key::End),
        rdev::Key::PageUp => Some(Key::PageUp),
        rdev::Key::PageDown => Some(Key::PageDown),
        rdev::Key::ShiftLeft => Some(Key::LShift),
        rdev::Key::ShiftRight => Some(Key::RShift),
        rdev::Key::ControlLeft => Some(Key::LCtrl),
        rdev::Key::ControlRight => Some(Key::RCtrl),
        rdev::Key::Alt => Some(Key::LAlt),
        rdev::Key::AltGr => Some(Key::RAlt),
        rdev::Key::Minus => Some(Key::Minus),
        rdev::Key::Equal => Some(Key::Equals),
        rdev::Key::LeftBracket => Some(Key::LBracket),
        rdev::Key::RightBracket => Some(Key::RBracket),
        rdev::Key::BackSlash => Some(Key::Backslash),
        rdev::Key::SemiColon => Some(Key::Semicolon),
        rdev::Key::Quote => Some(Key::Apostrophe),
        rdev::Key::BackQuote => Some(Key::Backquote),
        rdev::Key::Comma => Some(Key::Comma),
        rdev::Key::Dot => Some(Key::Period),
        rdev::Key::Slash => Some(Key::Slash),
        // Meta keys, lock keys, media keys and anything unmapped carry no
        // meaning for the clicker; ignore them.
        rdev::Key::CapsLock
        | rdev::Key::MetaLeft
        | rdev::Key::MetaRight
        | rdev::Key::NumLock
        | rdev::Key::PrintScreen
        | rdev::Key::ScrollLock
        | rdev::Key::Pause
        | rdev::Key::IntlBackslash
        | rdev::Key::KpDelete
        | rdev::Key::Function
        | rdev::Key::Unknown(_) => None,
    }
}
