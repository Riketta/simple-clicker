//! Platform-neutral key and mouse-button identifiers.
//!
//! The domain speaks only in these values; every adapter maps them into its
//! own code space (Windows virtual-key codes, HID usage codes, `rdev` keys).
//! Variant names are deliberately self-documenting; the canonical config
//! names are the [`Key::name`] strings, printable with `--list-keys`.

use core::fmt;

/// A keyboard key, identified by its physical meaning rather than any
/// platform code.
///
/// The variant list is an identifier table; the names are the
/// documentation (see [`Key::name`] and `--list-keys`).
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    NumpadAdd,
    NumpadSubtract,
    NumpadMultiply,
    NumpadDivide,
    NumpadDecimal,
    NumpadEnter,
    Space,
    Tab,
    Enter,
    Escape,
    Backspace,
    Left,
    Right,
    Up,
    Down,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    LShift,
    RShift,
    LCtrl,
    RCtrl,
    LAlt,
    RAlt,
    Minus,
    Equals,
    LBracket,
    RBracket,
    Backslash,
    Semicolon,
    Apostrophe,
    Backquote,
    Comma,
    Period,
    Slash,
}

impl Key {
    /// Every key the domain knows, in listing order.
    pub const ALL: &'static [Key] = &[
        Key::A,
        Key::B,
        Key::C,
        Key::D,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
        Key::K,
        Key::L,
        Key::M,
        Key::N,
        Key::O,
        Key::P,
        Key::Q,
        Key::R,
        Key::S,
        Key::T,
        Key::U,
        Key::V,
        Key::W,
        Key::X,
        Key::Y,
        Key::Z,
        Key::Num0,
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
        Key::Numpad0,
        Key::Numpad1,
        Key::Numpad2,
        Key::Numpad3,
        Key::Numpad4,
        Key::Numpad5,
        Key::Numpad6,
        Key::Numpad7,
        Key::Numpad8,
        Key::Numpad9,
        Key::NumpadAdd,
        Key::NumpadSubtract,
        Key::NumpadMultiply,
        Key::NumpadDivide,
        Key::NumpadDecimal,
        Key::NumpadEnter,
        Key::Space,
        Key::Tab,
        Key::Enter,
        Key::Escape,
        Key::Backspace,
        Key::Left,
        Key::Right,
        Key::Up,
        Key::Down,
        Key::Insert,
        Key::Delete,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
        Key::LShift,
        Key::RShift,
        Key::LCtrl,
        Key::RCtrl,
        Key::LAlt,
        Key::RAlt,
        Key::Minus,
        Key::Equals,
        Key::LBracket,
        Key::RBracket,
        Key::Backslash,
        Key::Semicolon,
        Key::Apostrophe,
        Key::Backquote,
        Key::Comma,
        Key::Period,
        Key::Slash,
    ];

    /// The canonical config name for this key (the form accepted by
    /// [`Key::from_name`], printed by `--list-keys`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Key::A => "A",
            Key::B => "B",
            Key::C => "C",
            Key::D => "D",
            Key::E => "E",
            Key::F => "F",
            Key::G => "G",
            Key::H => "H",
            Key::I => "I",
            Key::J => "J",
            Key::K => "K",
            Key::L => "L",
            Key::M => "M",
            Key::N => "N",
            Key::O => "O",
            Key::P => "P",
            Key::Q => "Q",
            Key::R => "R",
            Key::S => "S",
            Key::T => "T",
            Key::U => "U",
            Key::V => "V",
            Key::W => "W",
            Key::X => "X",
            Key::Y => "Y",
            Key::Z => "Z",
            Key::Num0 => "0",
            Key::Num1 => "1",
            Key::Num2 => "2",
            Key::Num3 => "3",
            Key::Num4 => "4",
            Key::Num5 => "5",
            Key::Num6 => "6",
            Key::Num7 => "7",
            Key::Num8 => "8",
            Key::Num9 => "9",
            Key::F1 => "F1",
            Key::F2 => "F2",
            Key::F3 => "F3",
            Key::F4 => "F4",
            Key::F5 => "F5",
            Key::F6 => "F6",
            Key::F7 => "F7",
            Key::F8 => "F8",
            Key::F9 => "F9",
            Key::F10 => "F10",
            Key::F11 => "F11",
            Key::F12 => "F12",
            Key::Numpad0 => "Numpad0",
            Key::Numpad1 => "Numpad1",
            Key::Numpad2 => "Numpad2",
            Key::Numpad3 => "Numpad3",
            Key::Numpad4 => "Numpad4",
            Key::Numpad5 => "Numpad5",
            Key::Numpad6 => "Numpad6",
            Key::Numpad7 => "Numpad7",
            Key::Numpad8 => "Numpad8",
            Key::Numpad9 => "Numpad9",
            Key::NumpadAdd => "NumpadAdd",
            Key::NumpadSubtract => "NumpadSubtract",
            Key::NumpadMultiply => "NumpadMultiply",
            Key::NumpadDivide => "NumpadDivide",
            Key::NumpadDecimal => "NumpadDecimal",
            Key::NumpadEnter => "NumpadEnter",
            Key::Space => "Space",
            Key::Tab => "Tab",
            Key::Enter => "Enter",
            Key::Escape => "Escape",
            Key::Backspace => "Backspace",
            Key::Left => "Left",
            Key::Right => "Right",
            Key::Up => "Up",
            Key::Down => "Down",
            Key::Insert => "Insert",
            Key::Delete => "Delete",
            Key::Home => "Home",
            Key::End => "End",
            Key::PageUp => "PageUp",
            Key::PageDown => "PageDown",
            Key::LShift => "LShift",
            Key::RShift => "RShift",
            Key::LCtrl => "LCtrl",
            Key::RCtrl => "RCtrl",
            Key::LAlt => "LAlt",
            Key::RAlt => "RAlt",
            Key::Minus => "Minus",
            Key::Equals => "Equals",
            Key::LBracket => "LBracket",
            Key::RBracket => "RBracket",
            Key::Backslash => "Backslash",
            Key::Semicolon => "Semicolon",
            Key::Apostrophe => "Apostrophe",
            Key::Backquote => "Backquote",
            Key::Comma => "Comma",
            Key::Period => "Period",
            Key::Slash => "Slash",
        }
    }

    /// Parses a key from its config name. Matching is case-insensitive and
    /// accepts a few common aliases (`Esc`, `Del`, arrows with an `Arrow`
    /// suffix, ...).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let norm = name.trim().to_ascii_lowercase();
        for &key in Key::ALL {
            if key.name().to_ascii_lowercase() == norm {
                return Some(key);
            }
        }
        match norm.as_str() {
            "esc" => Some(Key::Escape),
            "del" => Some(Key::Delete),
            "ins" => Some(Key::Insert),
            "pgup" | "pageup" => Some(Key::PageUp),
            "pgdn" | "pgdown" | "pagedown" => Some(Key::PageDown),
            "return" => Some(Key::Enter),
            "leftarrow" | "arrowleft" => Some(Key::Left),
            "rightarrow" | "arrowright" => Some(Key::Right),
            "uparrow" | "arrowup" => Some(Key::Up),
            "downarrow" | "arrowdown" => Some(Key::Down),
            "lshift" | "shift" => Some(Key::LShift),
            "lcontrol" | "lctrl" | "control" | "ctrl" => Some(Key::LCtrl),
            "lmenu" | "lalt" | "alt" => Some(Key::LAlt),
            "rcontrol" => Some(Key::RCtrl),
            "rmenu" => Some(Key::RAlt),
            "plus" | "add" => Some(Key::NumpadAdd),
            "subtract" => Some(Key::NumpadSubtract),
            "multiply" => Some(Key::NumpadMultiply),
            "divide" => Some(Key::NumpadDivide),
            "decimal" => Some(Key::NumpadDecimal),
            "backquote" | "grave" | "oem3" => Some(Key::Backquote),
            "oemplus" | "equal" => Some(Key::Equals),
            "oemminus" | "dash" => Some(Key::Minus),
            "oemcomma" => Some(Key::Comma),
            "oemperiod" | "dot" => Some(Key::Period),
            "oem1" => Some(Key::Semicolon),
            "oem7" | "quote" => Some(Key::Apostrophe),
            "oem2" => Some(Key::Slash),
            "oem4" | "bracketleft" => Some(Key::LBracket),
            "oem6" | "bracketright" => Some(Key::RBracket),
            "oem5" => Some(Key::Backslash),
            _ => None,
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A mouse button.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    /// Every button the domain knows.
    pub const ALL: &'static [MouseButton] =
        &[MouseButton::Left, MouseButton::Right, MouseButton::Middle];

    /// The canonical config name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
        }
    }

    /// Parses a button from its config name (case-insensitive).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "left" => Some(MouseButton::Left),
            "right" => Some(MouseButton::Right),
            "middle" | "mid" => Some(MouseButton::Middle),
            _ => None,
        }
    }
}

impl fmt::Display for MouseButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{
        Key,
        MouseButton,
    };

    #[test]
    fn every_key_round_trips_through_its_name() {
        for &key in Key::ALL {
            assert_eq!(
                Key::from_name(key.name()),
                Some(key),
                "name {} must round-trip",
                key.name()
            );
        }
    }

    #[test]
    fn every_button_round_trips_through_its_name() {
        for &button in MouseButton::ALL {
            assert_eq!(MouseButton::from_name(button.name()), Some(button));
        }
    }

    #[test]
    fn names_and_aliases_are_unique() {
        let mut seen = HashSet::new();
        for &key in Key::ALL {
            assert!(seen.insert(key.name().to_ascii_lowercase()), "duplicate name {}", key.name());
        }
        // Aliases must parse back to the same key through the canonical
        // name: no alias may shadow a canonical name with another key.
        for alias in ["Esc", "Return", "Shift", "Plus"] {
            let resolved = Key::from_name(alias);
            assert!(resolved.is_some(), "alias {alias} must parse");
            if let Some(key) = resolved {
                assert_eq!(
                    Key::from_name(key.name()),
                    Some(key),
                    "alias {alias} must be consistent"
                );
            }
        }
    }

    #[test]
    fn unknown_names_are_rejected() {
        assert_eq!(Key::from_name("DoesNotExist"), None);
        assert_eq!(Key::from_name(""), None);
        assert_eq!(MouseButton::from_name("hyper"), None);
    }

    #[test]
    fn parsing_is_case_insensitive() {
        assert_eq!(Key::from_name("numpad4"), Some(Key::Numpad4));
        assert_eq!(Key::from_name("NUMPAD4"), Some(Key::Numpad4));
        assert_eq!(Key::from_name(" Esc "), Some(Key::Escape));
    }
}
