//! versa-kbm driving adapter: the input sink over the RP2040 HID device's
//! vendor control channel.
//!
//! The device actuates the *focused* window on whatever machine its USB
//! plugs into, so pairing this sink with `require_focus = true` needs a
//! focus monitor (only the Win32 backend provides one today). Works on
//! Windows, Linux, and macOS through `hidapi`.

use std::time::Duration;

use hidapi::HidApi;
use tracing::{
    info,
    warn,
};
use vkbm_cli::Client;
use vkbm_proto::{
    Button,
    Command,
    ErrorCode,
    Reply,
    info_flags,
};

use crate::{
    domain::{
        engine::Actuation,
        key::{
            Key,
            MouseButton,
        },
    },
    ports::driven::{
        InputSink,
        SinkError,
    },
};

/// Connection settings from config.
#[derive(Clone, Copy, Debug)]
pub struct VersaKbmSettings {
    /// USB vendor id to look for.
    pub vid: u16,
    /// USB product id to look for.
    pub pid: u16,
    /// Per-command ack timeout in milliseconds.
    pub timeout_ms: u64,
    /// Send `ReleaseAll` to the device when the clicker stops.
    pub release_on_stop: bool,
}

/// Failures while opening the device.
#[derive(Debug, thiserror::Error)]
pub enum VersaError {
    /// HID initialization failed (no USB stack access, permissions).
    #[error("HID subsystem init failed: {0}")]
    Hid(String),
    /// The device handshake failed or the control channel was not found.
    #[error("versa-kbm device: {0}")]
    Client(#[from] vkbm_cli::Error),
}

/// The versa-kbm input sink.
pub struct VersaKbmSink {
    client: Client,
    timeout: Duration,
    /// Whether the sink should release the device on shutdown.
    release_on_stop: bool,
    /// Set once a `ReleaseAll` has been accepted, so the [`Drop`] handler
    /// does not send a redundant one on the normal exit path.
    released: bool,
    summary: String,
}

impl VersaKbmSink {
    /// Finds the device and opens its vendor control channel (one
    /// sacrificial ping inside `Client::open` burns any stale replay
    /// slot).
    ///
    /// # Errors
    /// [`VersaError`] on HID init or handshake failure.
    pub fn open(settings: VersaKbmSettings) -> Result<Self, VersaError> {
        let api = HidApi::new().map_err(|err| VersaError::Hid(err.to_string()))?;
        let mut client = Client::open(&api, settings.vid, settings.pid)?;
        let summary =
            match client.request(&Command::Info, Duration::from_millis(settings.timeout_ms)) {
                Ok(Reply::Info { major, minor, flags }) => {
                    format!(
                        "versa-kbm {:04x}:{:04x} fw {major}.{minor}.x switch={} arm={}",
                        settings.vid,
                        settings.pid,
                        flags & info_flags::SWITCH_ON != 0,
                        flags & info_flags::ARMED != 0,
                    )
                }
                Ok(_) => format!(
                    "versa-kbm {:04x}:{:04x} (info reply unexpected)",
                    settings.vid, settings.pid
                ),
                Err(err) => {
                    warn!("device info query failed, continuing without it: {err}");
                    format!("versa-kbm {:04x}:{:04x}", settings.vid, settings.pid)
                }
            };
        Ok(Self {
            client,
            timeout: Duration::from_millis(settings.timeout_ms),
            release_on_stop: settings.release_on_stop,
            released: false,
            summary,
        })
    }

    fn request(&mut self, command: Command) -> Result<(), SinkError> {
        match self.client.request(&command, self.timeout) {
            Ok(reply) => translate_reply(reply, command),
            Err(err) => Err(SinkError::Transport(err.to_string())),
        }
    }
}

/// Translates one device reply: `Echo` means the device is in dry-run
/// mode (accepted, not actuated), and well-known rejections get
/// actionable messages.
fn translate_reply(reply: Reply, command: Command) -> Result<(), SinkError> {
    match reply {
        Reply::Ok | Reply::Echo => Ok(()),
        Reply::Err(code) => Err(SinkError::Transport(match code {
            ErrorCode::Disabled => "hardware switch is off (BOOTSEL)".to_owned(),
            ErrorCode::Disarmed => "device is disarmed (send `vkbm arm`)".to_owned(),
            other => format!("device refused {command:?}: {other:?}"),
        })),
        Reply::Info { .. } => Err(SinkError::Transport("unexpected Info reply".to_owned())),
    }
}

impl InputSink for VersaKbmSink {
    fn actuate(&mut self, actuation: &Actuation) -> Result<(), SinkError> {
        match *actuation {
            Actuation::KeyDown(key) => {
                self.released = false;
                self.request(Command::KeyDown { usage: hid_usage(key) })
            }
            // Any up after a release-all means input is latched again, so
            // the `Drop` release must re-arm (the engine pairs every down
            // with an up, so this is also the normal per-tap pattern).
            Actuation::KeyUp(key) => {
                self.released = false;
                self.request(Command::KeyUp { usage: hid_usage(key) })
            }
            Actuation::MouseDown(button) => {
                self.released = false;
                self.request(Command::MouseDown { button: map_button(button) })
            }
            Actuation::MouseUp(button) => {
                self.released = false;
                self.request(Command::MouseUp { button: map_button(button) })
            }
            Actuation::ReleaseAll => {
                self.released = true;
                self.request(Command::ReleaseAll)
            }
        }
    }

    fn supports(&self, _key: Key) -> bool {
        // hid_usage is total over Key::ALL.
        true
    }

    fn describe(&self) -> String {
        self.summary.clone()
    }
}

impl Drop for VersaKbmSink {
    /// Best-effort release (per [`VersaKbmSettings::release_on_stop`]) on
    /// paths that skip the driver's normal shutdown: panics, early
    /// returns, or an unusual signal. Harmless when nothing is held.
    fn drop(&mut self) {
        if !self.release_on_stop || self.released {
            return;
        }
        match self.client.request(&Command::ReleaseAll, self.timeout) {
            Ok(_) => info!("device inputs released"),
            Err(err) => warn!("release on drop failed: {err}"),
        }
    }
}

/// Maps a domain button onto the wire vocabulary.
const fn map_button(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
    }
}

/// Maps a domain key onto its HID keyboard usage code (usage page 0x07).
/// Total over [`Key::ALL`].
#[must_use]
const fn hid_usage(key: Key) -> u8 {
    match key {
        Key::A => 0x04,
        Key::B => 0x05,
        Key::C => 0x06,
        Key::D => 0x07,
        Key::E => 0x08,
        Key::F => 0x09,
        Key::G => 0x0A,
        Key::H => 0x0B,
        Key::I => 0x0C,
        Key::J => 0x0D,
        Key::K => 0x0E,
        Key::L => 0x0F,
        Key::M => 0x10,
        Key::N => 0x11,
        Key::O => 0x12,
        Key::P => 0x13,
        Key::Q => 0x14,
        Key::R => 0x15,
        Key::S => 0x16,
        Key::T => 0x17,
        Key::U => 0x18,
        Key::V => 0x19,
        Key::W => 0x1A,
        Key::X => 0x1B,
        Key::Y => 0x1C,
        Key::Z => 0x1D,
        Key::Num1 => 0x1E,
        Key::Num2 => 0x1F,
        Key::Num3 => 0x20,
        Key::Num4 => 0x21,
        Key::Num5 => 0x22,
        Key::Num6 => 0x23,
        Key::Num7 => 0x24,
        Key::Num8 => 0x25,
        Key::Num9 => 0x26,
        Key::Num0 => 0x27,
        Key::NumpadEnter => 0x58,
        Key::Enter => 0x28,
        Key::Escape => 0x29,
        Key::Backspace => 0x2A,
        Key::Tab => 0x2B,
        Key::Space => 0x2C,
        Key::Minus => 0x2D,
        Key::Equals => 0x2E,
        Key::LBracket => 0x2F,
        Key::RBracket => 0x30,
        Key::Backslash => 0x31,
        Key::Semicolon => 0x33,
        Key::Apostrophe => 0x34,
        Key::Backquote => 0x35,
        Key::Comma => 0x36,
        Key::Period => 0x37,
        Key::Slash => 0x38,
        Key::F1 => 0x3A,
        Key::F2 => 0x3B,
        Key::F3 => 0x3C,
        Key::F4 => 0x3D,
        Key::F5 => 0x3E,
        Key::F6 => 0x3F,
        Key::F7 => 0x40,
        Key::F8 => 0x41,
        Key::F9 => 0x42,
        Key::F10 => 0x43,
        Key::F11 => 0x44,
        Key::F12 => 0x45,
        Key::Right => 0x4F,
        Key::Left => 0x50,
        Key::Down => 0x51,
        Key::Up => 0x52,
        Key::NumpadDivide => 0x54,
        Key::NumpadMultiply => 0x55,
        Key::NumpadSubtract => 0x56,
        Key::NumpadAdd => 0x57,
        Key::Numpad1 => 0x59,
        Key::Numpad2 => 0x5A,
        Key::Numpad3 => 0x5B,
        Key::Numpad4 => 0x5C,
        Key::Numpad5 => 0x5D,
        Key::Numpad6 => 0x5E,
        Key::Numpad7 => 0x5F,
        Key::Numpad8 => 0x60,
        Key::Numpad9 => 0x61,
        Key::Numpad0 => 0x62,
        Key::NumpadDecimal => 0x63,
        Key::LCtrl => 0xE0,
        Key::LShift => 0xE1,
        Key::LAlt => 0xE2,
        Key::RCtrl => 0xE4,
        Key::RShift => 0xE5,
        Key::RAlt => 0xE6,
        Key::Insert => 0x49,
        Key::Home => 0x4A,
        Key::PageUp => 0x4B,
        Key::Delete => 0x4C,
        Key::End => 0x4D,
        Key::PageDown => 0x4E,
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::key::Key;
    use crate::ports::driven::SinkError;
    use vkbm_proto::keyboard::KeyboardState;
    use vkbm_proto::{
        Command,
        ErrorCode,
        Reply,
    };

    use super::{
        hid_usage,
        translate_reply,
    };

    #[test]
    fn every_usage_is_reportable_on_the_device_descriptor() {
        // The firmware's own state machine refuses usages outside the
        // flashed descriptor (0x04..=0xDD regular, 0xE0..=0xE7 modifiers);
        // the whole mapping must stay inside that vocabulary.
        for &key in Key::ALL {
            let usage = hid_usage(key);
            assert!(
                KeyboardState::is_valid_usage(usage),
                "usage {usage:#04x} for {} is not reportable on the device",
                key.name()
            );
        }
    }

    #[test]
    fn spot_check_well_known_usages() {
        assert_eq!(hid_usage(Key::Q), 0x14);
        assert_eq!(hid_usage(Key::Numpad0), 0x62);
        assert_eq!(hid_usage(Key::Numpad4), 0x5C);
        assert_eq!(hid_usage(Key::Space), 0x2C);
        assert_eq!(hid_usage(Key::Escape), 0x29);
        assert_eq!(hid_usage(Key::LShift), 0xE1);
    }

    #[test]
    fn replies_translate_into_sink_results() {
        let command = Command::ReleaseAll;
        assert!(translate_reply(Reply::Ok, command).is_ok(), "an explicit Ok must succeed");
        assert!(
            translate_reply(Reply::Echo, command).is_ok(),
            "Echo (dry-run ack) must be treated as success"
        );
        let disabled = translate_reply(Reply::Err(ErrorCode::Disabled), command);
        assert!(
            matches!(&disabled, Err(SinkError::Transport(message)) if message.contains("BOOTSEL")),
            "Disabled must point at the hardware switch, got {disabled:?}"
        );
        let disarmed = translate_reply(Reply::Err(ErrorCode::Disarmed), command);
        assert!(
            matches!(&disarmed, Err(SinkError::Transport(message)) if message.contains("vkbm arm")),
            "Disarmed must suggest `vkbm arm`, got {disarmed:?}"
        );
        let refused = translate_reply(Reply::Err(ErrorCode::KeyboardFull), command);
        assert!(
            matches!(&refused, Err(SinkError::Transport(message)) if message.contains("refused")),
            "other rejections must carry the refusal, got {refused:?}"
        );
        let info = translate_reply(Reply::Info { major: 1, minor: 0, flags: 0 }, command);
        assert!(
            matches!(&info, Err(SinkError::Transport(message)) if message.contains("unexpected Info")),
            "an out-of-place Info must be a transport error, got {info:?}"
        );
    }
}
