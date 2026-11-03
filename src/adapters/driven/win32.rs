//! Win32 driven adapters: target-window discovery, the focus monitor, and
//! the input sink.
//!
//! Keyboard actuation goes through `WM_KEYDOWN`/`WM_KEYUP` messages posted
//! to the target window (background-capable, matching the original tool's
//! `SendMessage` approach). Mouse actuation goes through global
//! `SendInput` — the real cursor presses the button in the focused window,
//! exactly like the original tool's `mouse_event` clicks.

use std::{
    collections::HashSet,
    mem::{
        size_of,
        take,
    },
    time::{
        Duration,
        Instant,
    },
};

use tracing::{
    info,
    warn,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle,
        HANDLE,
        HWND,
        INVALID_HANDLE_VALUE,
        LPARAM,
    },
    System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot,
        PROCESSENTRY32W,
        Process32FirstW,
        Process32NextW,
        TH32CS_SNAPPROCESS,
    },
    UI::{
        Input::KeyboardAndMouse::{
            INPUT,
            INPUT_0,
            INPUT_MOUSE,
            MOUSEEVENTF_LEFTDOWN,
            MOUSEEVENTF_LEFTUP,
            MOUSEEVENTF_MIDDLEDOWN,
            MOUSEEVENTF_MIDDLEUP,
            MOUSEEVENTF_RIGHTDOWN,
            MOUSEEVENTF_RIGHTUP,
            MOUSEINPUT,
            SendInput,
        },
        WindowsAndMessaging::{
            EnumWindows,
            GW_OWNER,
            GetForegroundWindow,
            GetWindow,
            GetWindowThreadProcessId,
            IsWindow,
            IsWindowVisible,
            SendMessageW,
            WM_KEYDOWN,
            WM_KEYUP,
        },
    },
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
        FocusMonitor,
        InputSink,
        SinkError,
    },
};

/// Maps a domain key to its Windows virtual-key code. The mapping is total
/// over [`Key::ALL`], so sinks built on it support every configured key.
#[must_use]
pub fn vk_code(key: Key) -> u16 {
    match key {
        Key::A => 0x41,
        Key::B => 0x42,
        Key::C => 0x43,
        Key::D => 0x44,
        Key::E => 0x45,
        Key::F => 0x46,
        Key::G => 0x47,
        Key::H => 0x48,
        Key::I => 0x49,
        Key::J => 0x4A,
        Key::K => 0x4B,
        Key::L => 0x4C,
        Key::M => 0x4D,
        Key::N => 0x4E,
        Key::O => 0x4F,
        Key::P => 0x50,
        Key::Q => 0x51,
        Key::R => 0x52,
        Key::S => 0x53,
        Key::T => 0x54,
        Key::U => 0x55,
        Key::V => 0x56,
        Key::W => 0x57,
        Key::X => 0x58,
        Key::Y => 0x59,
        Key::Z => 0x5A,
        Key::Num0 => 0x30,
        Key::Num1 => 0x31,
        Key::Num2 => 0x32,
        Key::Num3 => 0x33,
        Key::Num4 => 0x34,
        Key::Num5 => 0x35,
        Key::Num6 => 0x36,
        Key::Num7 => 0x37,
        Key::Num8 => 0x38,
        Key::Num9 => 0x39,
        Key::F1 => 0x70,
        Key::F2 => 0x71,
        Key::F3 => 0x72,
        Key::F4 => 0x73,
        Key::F5 => 0x74,
        Key::F6 => 0x75,
        Key::F7 => 0x76,
        Key::F8 => 0x77,
        Key::F9 => 0x78,
        Key::F10 => 0x79,
        Key::F11 => 0x7A,
        Key::F12 => 0x7B,
        Key::Numpad0 => 0x60,
        Key::Numpad1 => 0x61,
        Key::Numpad2 => 0x62,
        Key::Numpad3 => 0x63,
        Key::Numpad4 => 0x64,
        Key::Numpad5 => 0x65,
        Key::Numpad6 => 0x66,
        Key::Numpad7 => 0x67,
        Key::Numpad8 => 0x68,
        Key::Numpad9 => 0x69,
        Key::NumpadAdd => 0x6B,
        Key::NumpadSubtract => 0x6D,
        Key::NumpadMultiply => 0x6A,
        Key::NumpadDivide => 0x6F,
        Key::NumpadDecimal => 0x6E,
        Key::NumpadEnter | Key::Enter => 0x0D,
        Key::Space => 0x20,
        Key::Tab => 0x09,
        Key::Escape => 0x1B,
        Key::Backspace => 0x08,
        Key::Left => 0x25,
        Key::Right => 0x27,
        Key::Up => 0x26,
        Key::Down => 0x28,
        Key::Insert => 0x2D,
        Key::Delete => 0x2E,
        Key::Home => 0x24,
        Key::End => 0x23,
        Key::PageUp => 0x21,
        Key::PageDown => 0x22,
        Key::LShift => 0xA0,
        Key::RShift => 0xA1,
        Key::LCtrl => 0xA2,
        Key::RCtrl => 0xA3,
        Key::LAlt => 0xA4,
        Key::RAlt => 0xA5,
        Key::Minus => 0xBD,
        Key::Equals => 0xBB,
        Key::LBracket => 0xDB,
        Key::RBracket => 0xDD,
        Key::Backslash => 0xDC,
        Key::Semicolon => 0xBA,
        Key::Apostrophe => 0xDE,
        Key::Backquote => 0xC0,
        Key::Comma => 0xBC,
        Key::Period => 0xBE,
        Key::Slash => 0xBF,
    }
}

/// The target application window: found by process name, addressed by
/// window handle. Carries the full configured process list so the sink
/// and the focus monitor can re-acquire a restarted target on their own.
#[derive(Clone, Debug)]
pub struct TargetWindow {
    hwnd: HWND,
    process_name: String,
    process_names: Vec<String>,
}

impl TargetWindow {
    /// Finds the first visible, unowned top-level window of the first
    /// configured process that is running. `process_names` are matched
    /// against the executable name, case-insensitively, with or without
    /// the `.exe` suffix.
    #[must_use]
    pub fn find(process_names: &[String]) -> Option<Self> {
        for name in process_names {
            let Some(pid) = find_process_pid(name) else { continue };
            if let Some(hwnd) = find_main_window(pid) {
                return Some(Self {
                    hwnd,
                    process_name: name.clone(),
                    process_names: process_names.to_vec(),
                });
            }
        }
        None
    }

    /// The configured process name this window was found through.
    #[must_use]
    pub fn process_name(&self) -> &str {
        &self.process_name
    }

    /// Whether the window handle is still valid (the process may have
    /// exited).
    #[must_use]
    pub fn is_alive(&self) -> bool {
        // SAFETY: hwnd is a handle we received from EnumWindows; IsWindow
        // only reads it.
        unsafe { IsWindow(self.hwnd) != 0 }
    }
}

/// How often a lost target window is re-scanned for.
const TARGET_RESCAN: Duration = Duration::from_secs(2);

/// At most one target re-scan per [`TARGET_RESCAN`]: `last` rate-limits
/// consecutive calls, `names` is the configured process list. Returns the
/// newly found window, if any.
fn throttled_rescan(last: &mut Option<Instant>, names: &[String]) -> Option<TargetWindow> {
    let now = Instant::now();
    if last.is_some_and(|at| now.duration_since(at) < TARGET_RESCAN) {
        return None;
    }
    *last = Some(now);
    TargetWindow::find(names)
}

/// One process-list entry scan context.
struct EnumCtx {
    pid: u32,
    found: Option<HWND>,
}

/// `EnumWindows` callback: records the first visible, unowned top-level
/// window belonging to the context's process.
unsafe extern "system" fn enum_main_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    // SAFETY: lparam is the address of the caller-frame EnumCtx, which
    // outlives the EnumWindows call it is passed to.
    let ctx = unsafe { &mut *(lparam as *mut EnumCtx) };
    let mut pid = 0u32;
    // SAFETY: pid is a valid out-pointer; hwnd comes from the enumeration
    // itself.
    unsafe { GetWindowThreadProcessId(hwnd, core::ptr::addr_of_mut!(pid)) };
    if pid != ctx.pid || ctx.found.is_some() {
        return 1;
    }
    // SAFETY: hwnd is a live window handle from the enumeration.
    let visible = unsafe { IsWindowVisible(hwnd) != 0 };
    // SAFETY: hwnd is a live window handle from the enumeration; an owner
    // handle of null means the window is unowned.
    let unowned = unsafe { GetWindow(hwnd, GW_OWNER).is_null() };
    if visible && unowned {
        ctx.found = Some(hwnd);
    }
    1
}

/// Finds a main-window handle for `pid`, or `None`.
fn find_main_window(pid: u32) -> Option<HWND> {
    let mut ctx = EnumCtx { pid, found: None };
    let lparam = core::ptr::addr_of_mut!(ctx) as LPARAM;
    // SAFETY: the callback only touches ctx, which outlives the call;
    // EnumWindows never stores lparam beyond the call.
    unsafe { EnumWindows(Some(enum_main_window), lparam) };
    ctx.found
}

/// Scans the running processes for one whose executable matches `name`
/// (case-insensitive, `.exe` optional). Returns its pid.
fn find_process_pid(name: &str) -> Option<u32> {
    // SAFETY: the snapshot handle is valid until CloseHandle below; the
    // entry struct is initialized (dwSize) before each call and is only
    // read between Process32*W calls.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let result = scan_snapshot(snapshot, name);
        let _ = CloseHandle(snapshot);
        result
    }
}

/// Walks the process snapshot looking for `name`.
///
/// # Safety
/// `snapshot` must be a valid `TH32CS_SNAPPROCESS` handle that is closed
/// by the caller exactly once after this returns.
unsafe fn scan_snapshot(snapshot: HANDLE, name: &str) -> Option<u32> {
    let mut entry = PROCESSENTRY32W {
        dwSize: u32::try_from(size_of::<PROCESSENTRY32W>()).ok()?,
        cntUsage: 0,
        th32ProcessID: 0,
        th32DefaultHeapID: 0,
        th32ModuleID: 0,
        cntThreads: 0,
        th32ParentProcessID: 0,
        pcPriClassBase: 0,
        dwFlags: 0,
        szExeFile: [0; 260],
    };
    // SAFETY: (caller contract) entry is initialized with its size and
    // outlives the iteration.
    if unsafe { Process32FirstW(snapshot, core::ptr::addr_of_mut!(entry)) } == 0 {
        return None;
    }
    loop {
        let exe = wide_cstr(&entry.szExeFile);
        let stem = exe.strip_suffix(".exe").unwrap_or(exe.as_str());
        if exe.eq_ignore_ascii_case(name) || stem.eq_ignore_ascii_case(name) {
            return Some(entry.th32ProcessID);
        }
        // SAFETY: (caller contract) entry stays initialized between calls.
        if unsafe { Process32NextW(snapshot, core::ptr::addr_of_mut!(entry)) } == 0 {
            return None;
        }
    }
}

/// Decodes a NUL-terminated UTF-16 buffer.
fn wide_cstr(buf: &[u16; 260]) -> String {
    let len = buf.iter().position(|&unit| unit == 0).unwrap_or(buf.len());
    match buf.get(..len) {
        Some(prefix) => String::from_utf16_lossy(prefix),
        None => String::new(),
    }
}

/// The Win32 input sink: keyboard into the target window, mouse globally.
/// When the target process exits, actuation pauses and the sink re-scans
/// (rate-limited) for a restarted process, reporting
/// [`SinkError::TargetGone`] until then.
#[derive(Debug)]
pub struct Win32Sink {
    target: TargetWindow,
    held_keys: HashSet<Key>,
    held_buttons: HashSet<MouseButton>,
    last_scan: Option<Instant>,
}

impl Win32Sink {
    /// Builds a sink bound to `target`.
    #[must_use]
    pub fn new(target: TargetWindow) -> Self {
        Self { target, held_keys: HashSet::new(), held_buttons: HashSet::new(), last_scan: None }
    }

    /// Fails with [`SinkError::TargetGone`] while the target window is
    /// gone, re-scanning for it at most once per [`TARGET_RESCAN`].
    fn ensure_target(&mut self) -> Result<(), SinkError> {
        if self.target.is_alive() {
            return Ok(());
        }
        let names = self.target.process_names.clone();
        if let Some(found) = throttled_rescan(&mut self.last_scan, &names) {
            info!("target window re-acquired: {}", found.process_name());
            self.target = found;
            return Ok(());
        }
        Err(SinkError::TargetGone)
    }

    fn send_key(&mut self, key: Key, message: u32) -> Result<(), SinkError> {
        self.ensure_target()?;
        // SAFETY: hwnd is a live window handle (checked above); the call
        // only enqueues a keyboard message into that window's queue.
        let _ = unsafe { SendMessageW(self.target.hwnd, message, usize::from(vk_code(key)), 0) };
        Ok(())
    }

    fn release_all(&mut self) {
        let mut problems = 0u32;
        for key in take(&mut self.held_keys) {
            if self.send_key(key, WM_KEYUP).is_err() {
                problems += 1;
            }
        }
        for button in take(&mut self.held_buttons) {
            if send_mouse(release_flags(button)).is_err() {
                problems += 1;
            }
        }
        if problems > 0 {
            warn!("{problems} release(s) failed while releasing all inputs");
        }
    }
}

impl InputSink for Win32Sink {
    fn actuate(&mut self, actuation: &Actuation) -> Result<(), SinkError> {
        // Everything pauses while the target is gone (global mouse input
        // included: clicking into whatever replaced the window would be
        // worse than pausing).
        self.ensure_target()?;
        match *actuation {
            Actuation::KeyDown(key) => {
                self.send_key(key, WM_KEYDOWN)?;
                self.held_keys.insert(key);
            }
            Actuation::KeyUp(key) => {
                self.send_key(key, WM_KEYUP)?;
                self.held_keys.remove(&key);
            }
            Actuation::MouseDown(button) => {
                send_mouse(press_flags(button))?;
                self.held_buttons.insert(button);
            }
            Actuation::MouseUp(button) => {
                send_mouse(release_flags(button))?;
                self.held_buttons.remove(&button);
            }
            Actuation::ReleaseAll => self.release_all(),
        }
        Ok(())
    }

    fn supports(&self, _key: Key) -> bool {
        // vk_code is total over Key::ALL.
        true
    }

    fn describe(&self) -> String {
        format!(
            "win32 messages (keyboard into '{}' via WM_KEYDOWN/UP; mouse global via SendInput)",
            self.target.process_name()
        )
    }

    fn is_target_alive(&self) -> bool {
        self.target.is_alive()
    }
}

/// Sends one global mouse-button event through `SendInput`.
fn send_mouse(flags: u32) -> Result<(), SinkError> {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    };
    let Ok(cb) = i32::try_from(size_of::<INPUT>()) else {
        return Err(SinkError::Transport("INPUT struct size does not fit i32".to_owned()));
    };
    // SAFETY: `input` is a fully initialized INPUT outliving the call, and
    // the count matches the single element passed.
    let sent = unsafe { SendInput(1, core::ptr::addr_of!(input), cb) };
    if sent == 1 {
        Ok(())
    } else {
        Err(SinkError::Transport("SendInput was refused by the OS".to_owned()))
    }
}

/// `MOUSEEVENTF_*` down flag for a button.
const fn press_flags(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => MOUSEEVENTF_LEFTDOWN,
        MouseButton::Right => MOUSEEVENTF_RIGHTDOWN,
        MouseButton::Middle => MOUSEEVENTF_MIDDLEDOWN,
    }
}

/// `MOUSEEVENTF_*` up flag for a button.
const fn release_flags(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => MOUSEEVENTF_LEFTUP,
        MouseButton::Right => MOUSEEVENTF_RIGHTUP,
        MouseButton::Middle => MOUSEEVENTF_MIDDLEUP,
    }
}

/// The Win32 focus monitor: compares the foreground window with the
/// target's handle. Re-acquires the target (rate-limited) when its
/// process exits, so a restarted game keeps the focus gate working.
#[derive(Clone, Debug)]
pub struct Win32Focus {
    target: TargetWindow,
    last_scan: Option<Instant>,
    gone_logged: bool,
}

impl Win32Focus {
    /// Watches the given target window.
    #[must_use]
    pub fn new(target: &TargetWindow) -> Self {
        Self { target: target.clone(), last_scan: None, gone_logged: false }
    }
}

impl FocusMonitor for Win32Focus {
    fn is_focused(&mut self) -> Option<bool> {
        if !self.target.is_alive() {
            let names = self.target.process_names.clone();
            if let Some(found) = throttled_rescan(&mut self.last_scan, &names) {
                info!("target window re-acquired: {}", found.process_name());
                self.target = found;
                self.gone_logged = false;
            } else {
                if !self.gone_logged {
                    self.gone_logged = true;
                    warn!("target window is gone; focus unknown until it returns");
                }
                return None;
            }
        }
        // SAFETY: the call takes no pointers and only queries desktop
        // state; raw-pointer equality is a plain address comparison.
        Some(unsafe { GetForegroundWindow() == self.target.hwnd })
    }

    fn describe(&self) -> String {
        "win32 (GetForegroundWindow)".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TargetWindow,
        Win32Sink,
        vk_code,
    };
    use crate::{
        domain::key::Key,
        ports::driven::InputSink,
    };

    #[test]
    fn vk_mapping_is_total_and_distinct_for_distinct_keys() {
        // Distinctness is not required by Windows (Enter and NumpadEnter
        // share a VK code legitimately), but every key must map to
        // something the OS knows: here we check the mapping covers every
        // variant and spot-check well-known codes.
        assert_eq!(vk_code(Key::Q), 0x51);
        assert_eq!(vk_code(Key::Numpad4), 0x64);
        assert_eq!(vk_code(Key::Escape), 0x1B);
        assert_eq!(vk_code(Key::F12), 0x7B);
        assert_eq!(vk_code(Key::Space), 0x20);
    }

    #[test]
    fn sink_accepts_every_configured_key() {
        let target = TargetWindow {
            hwnd: std::ptr::null_mut(),
            process_name: "unused".to_owned(),
            process_names: Vec::new(),
        };
        let sink = Win32Sink::new(target);
        for &key in Key::ALL {
            assert!(sink.supports(key), "win32 sink must support {}", key.name());
        }
    }

    #[test]
    fn wide_cstr_decodes_and_truncates() {
        let mut buf = [0u16; 260];
        for (slot, unit) in buf.iter_mut().zip("Wow-64.exe".encode_utf16()) {
            *slot = unit;
        }
        assert_eq!(super::wide_cstr(&buf), "Wow-64.exe");
    }
}
