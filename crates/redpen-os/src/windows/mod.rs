//! Capture on Windows (docs/modules/05-capture-and-replace-windows.md): the clipboard route of
//! ADR-0003, with UI Automation, the process token and the window's thread deciding beforehand
//! whether Redpen may read or type at all.
//!
//! - [`foreground`]: the window in front, its process, elevation, caret and the mouse.
//! - [`uia`]: whether the focused control is a password field.
//! - [`clipboard`]: saving and restoring every format whose data is memory, reading and writing
//!   text, and the marks that keep Redpen's temporary entry out of clipboard history.
//! - [`input`]: the modifier keys and the synthetic Ctrl+C, Ctrl+V and Ctrl+Z.
//!
//! [`WindowsCapture`] puts them together behind `redpen_core::platform::Capture`. Its calls block
//! the calling thread (the copy wait, the restore delay); the flows run them from async code
//! that may block for that long.

pub mod clipboard;
pub mod foreground;
pub mod input;
pub mod uia;

use std::time::{Duration, Instant};

use ::windows::Win32::Foundation::{GetLastError, HWND};
use ::windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
};
use ::windows::core::HRESULT;
use redpen_core::platform::{Capture, CaptureError, Foreground, WindowRef};

/// How often `read_selection` looks at the clipboard's sequence number.
pub const COPY_POLL: Duration = Duration::from_millis(10);

/// The [`Capture`] of Windows.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsCapture;

impl WindowsCapture {
    pub fn new() -> Self {
        Self
    }
}

impl Capture for WindowsCapture {
    fn foreground(&self) -> Result<Foreground, CaptureError> {
        foreground::foreground()
    }

    fn inspect(&self, w: &WindowRef) -> Result<Option<Foreground>, CaptureError> {
        foreground::inspect(w)
    }

    fn modifiers_down(&self) -> bool {
        input::modifiers_down()
    }

    fn read_selection(&self, timeout: Duration) -> Result<String, CaptureError> {
        let started = Instant::now();
        let saved = clipboard::save()?;
        let before = clipboard::sequence_number();
        input::send_ctrl(input::Key::C)?;
        let copied = wait_for_copy(before, timeout);
        // Put back what the clipboard held, unless the copy never happened (the clipboard is then
        // untouched and writing it again would only add an entry to clipboard history).
        if clipboard::sequence_number() != before {
            clipboard::restore(&saved)?;
        }
        tracing::debug!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            copied = copied.as_ref().map(String::len).unwrap_or(0),
            "read_selection"
        );
        match copied {
            Some(text) if !text.is_empty() => Ok(text),
            _ => Err(CaptureError::NothingCopied),
        }
    }

    fn focus(&self, w: &WindowRef) -> Result<bool, CaptureError> {
        if !foreground::exists(w) {
            return Err(CaptureError::WindowGone);
        }
        let hwnd = to_hwnd(w);
        // SAFETY: plain Win32 calls on a window handle; a stale handle only makes them fail.
        unsafe {
            if GetForegroundWindow() == hwnd {
                return Ok(true);
            }
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            // Allowed because Redpen's popup is in front when Accept or Undo runs.
            let _ = SetForegroundWindow(hwnd);
            Ok(GetForegroundWindow() == hwnd)
        }
    }

    fn paste(&self, text: &str, restore_delay: Duration) -> Result<(), CaptureError> {
        let saved = clipboard::save()?;
        let before = clipboard::sequence_number();
        let typed = clipboard::set_text(text, clipboard::Entry::Temporary)
            .and_then(|()| input::send_ctrl(input::Key::V))
            .map(|()| std::thread::sleep(restore_delay));
        // Put back what the clipboard held whenever this call changed it, even when the paste
        // itself failed half way.
        let restored = if clipboard::sequence_number() != before {
            clipboard::restore(&saved)
        } else {
            Ok(())
        };
        tracing::debug!(bytes = text.len(), "paste");
        typed.and(restored)
    }

    fn send_undo(&self) -> Result<(), CaptureError> {
        input::send_ctrl(input::Key::Z)
    }

    fn set_clipboard_text(&self, text: &str) -> Result<(), CaptureError> {
        clipboard::set_text(text, clipboard::Entry::Normal)
    }
}

/// Polls the clipboard every [`COPY_POLL`] until its sequence number moves away from `before`
/// and it holds text, or `timeout` passes. Returns the text, if any came.
fn wait_for_copy(before: u32, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    loop {
        if clipboard::sequence_number() != before {
            // An app may empty the clipboard and set its text in two steps: keep waiting until
            // there is text, or the time is up.
            if let Ok(Some(text)) = clipboard::read_text() {
                return Some(text);
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(COPY_POLL);
    }
}

/// The window handle a [`WindowRef`] names.
pub(crate) fn to_hwnd(w: &WindowRef) -> HWND {
    HWND(w.id as usize as *mut core::ffi::c_void)
}

/// The [`WindowRef`] of a window handle owned by process `pid`.
pub(crate) fn window_ref(hwnd: HWND, pid: u32) -> WindowRef {
    WindowRef {
        id: hwnd.0 as usize as u64,
        pid,
    }
}

/// An error from a `windows` call as [`CaptureError::Os`]: a Win32 error in decimal (`5`), any
/// other `HRESULT` in hexadecimal (`0x80004005`).
pub(crate) fn os_error(e: &::windows::core::Error) -> CaptureError {
    hresult_error(e.code())
}

/// [`os_error`] for a bare `HRESULT`.
pub(crate) fn hresult_error(hr: HRESULT) -> CaptureError {
    let bits = hr.0 as u32;
    // FACILITY_WIN32 (7) with the failure bit: a Win32 error code wrapped as an HRESULT.
    if bits & 0xFFFF_0000 == 0x8007_0000 {
        CaptureError::os(bits & 0xFFFF)
    } else {
        CaptureError::os(format!("0x{bits:08X}"))
    }
}

/// The calling thread's last Win32 error as [`CaptureError::Os`].
pub(crate) fn last_error() -> CaptureError {
    // SAFETY: reads a thread-local value.
    let code = unsafe { GetLastError() };
    CaptureError::os(code.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_codes_are_numbers_only() {
        let denied = ::windows::core::Error::from_hresult(HRESULT(0x8007_0005_u32 as i32));
        assert_eq!(os_error(&denied), CaptureError::os(5));
        assert_eq!(
            hresult_error(HRESULT(0x8004_0201_u32 as i32)),
            CaptureError::Os("0x80040201".into())
        );
        assert_eq!(
            CaptureError::os(5).to_string(),
            "Windows refused the request (code 5)."
        );
    }

    #[test]
    fn window_refs_round_trip() {
        let w = WindowRef {
            id: 0x0001_02F4,
            pid: 4242,
        };
        assert_eq!(window_ref(to_hwnd(&w), w.pid), w);
    }
}
