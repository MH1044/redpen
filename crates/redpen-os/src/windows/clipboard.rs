//! The Windows clipboard (docs/modules/05-capture-and-replace-windows.md): saving what it holds
//! and putting it back, reading and writing text, and the marks that keep Redpen's temporary
//! entry out of clipboard history and cloud sync (invariant 5).
//!
//! **What is saved.** Every format whose data is memory (an `HGLOBAL`), byte for byte, in the
//! order Windows lists them. Left out, because their data is a drawing object rather than memory:
//! `CF_BITMAP` (Windows makes it again from the DIB), `CF_METAFILEPICT`, `CF_PALETTE`,
//! `CF_ENHMETAFILE`, `CF_OWNERDISPLAY` and their `CF_DSP…` twins, the private range
//! (`0x0200`–`0x02FF`) and the drawing-object range (`0x0300`–`0x03FF`). A delayed-render format
//! is rendered by its app when Redpen asks for it (Windows offers no documented way to tell one
//! apart before asking); one its app does not render (the app has closed, for example) is left
//! out. Above [`MAX_SAVED_BYTES`] in total, only `CF_UNICODETEXT` is kept, and one log line
//! records the size left out, nothing else.
//!
//! **Opening.** `OpenClipboard` is tried [`OPEN_ATTEMPTS`] times, [`OPEN_RETRY`] apart, then
//! gives [`CaptureError::ClipboardBusy`]. Writes are owned by a message-only window made for the
//! write and destroyed after it, so no later clipboard message ever waits on Redpen.
//!
//! **Text** goes on the clipboard with Windows line endings (CR LF) and comes off it with LF.

use std::time::Duration;

use ::windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use ::windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use ::windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use ::windows::Win32::System::Ole::CF_UNICODETEXT;
use ::windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
};
use ::windows::core::{HSTRING, w};
use redpen_core::platform::CaptureError;

use super::{last_error, os_error};

/// How many times `OpenClipboard` is tried before [`CaptureError::ClipboardBusy`].
pub const OPEN_ATTEMPTS: u32 = 5;

/// The wait between two tries of `OpenClipboard`.
pub const OPEN_RETRY: Duration = Duration::from_millis(20);

/// Above this many bytes in total, only `CF_UNICODETEXT` is saved: 64 MB.
pub const MAX_SAVED_BYTES: usize = 64 * 1024 * 1024;

/// `CF_UNICODETEXT`, as a format id.
pub const UNICODE_TEXT: u32 = CF_UNICODETEXT.0 as u32;

/// The registered formats that keep an entry out of clipboard history and cloud sync, set on
/// Redpen's temporary entry: the first with any data, the other two with the `DWORD` 0.
pub const EXCLUSION_FORMATS: [&str; 3] = [
    "ExcludeClipboardContentFromMonitorProcessing",
    "CanIncludeInClipboardHistory",
    "CanUploadToCloudClipboard",
];

/// Standard formats whose data is a drawing object, or nothing, rather than memory.
const NOT_MEMORY: [u32; 8] = [
    2,    // CF_BITMAP
    3,    // CF_METAFILEPICT
    9,    // CF_PALETTE
    14,   // CF_ENHMETAFILE
    0x80, // CF_OWNERDISPLAY
    0x82, // CF_DSPBITMAP
    0x83, // CF_DSPMETAFILEPICT
    0x8E, // CF_DSPENHMETAFILE
];

/// Whether a format's data is saved and restored: memory formats only.
pub fn is_saved_format(id: u32) -> bool {
    !NOT_MEMORY.contains(&id) && !(0x0200..=0x03FF).contains(&id)
}

/// What the clipboard held: each saved format's id and bytes, in the order Windows listed
/// them, and how many bytes were left out for being over [`MAX_SAVED_BYTES`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Saved {
    pub formats: Vec<(u32, Vec<u8>)>,
    pub skipped_bytes: usize,
}

impl Saved {
    /// The bytes saved for a format.
    pub fn get(&self, id: u32) -> Option<&[u8]> {
        self.formats
            .iter()
            .find(|(f, _)| *f == id)
            .map(|(_, b)| b.as_slice())
    }
}

/// The kind of entry [`set_text`] makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// An ordinary entry: Copy, which the user asked for.
    Normal,
    /// Redpen's own entry during a paste, marked with [`EXCLUSION_FORMATS`].
    Temporary,
}

/// The clipboard's sequence number, which moves whenever its contents change.
pub fn sequence_number() -> u32 {
    // SAFETY: no arguments, no state.
    unsafe { GetClipboardSequenceNumber() }
}

/// The id of a registered format, registering it when needed.
pub fn format_id(name: &str) -> u32 {
    // SAFETY: the name is a valid, NUL-terminated wide string for the call's duration.
    unsafe { RegisterClipboardFormatW(&HSTRING::from(name)) }
}

/// Every saved format of the clipboard now.
pub fn save() -> Result<Saved, CaptureError> {
    let _open = Open::new(None)?;
    let mut listed = Vec::new();
    let mut format = 0;
    loop {
        // SAFETY: the clipboard is open on this thread.
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }
        if is_saved_format(format) {
            listed.push(format);
        }
    }
    // Ask for each format's data in turn. Once the total is over the limit, only the text is
    // still asked for: asking for a format Windows makes from another (`CF_DIBV5` from a DIB)
    // makes it convert the data, which is not worth it for a format that will not be kept.
    let mut found: Vec<(u32, HGLOBAL, usize)> = Vec::new();
    let mut total = 0usize;
    for format in listed {
        if total > MAX_SAVED_BYTES && format != UNICODE_TEXT {
            continue;
        }
        // SAFETY: the clipboard is open; the handle stays valid until it is closed.
        let Ok(handle) = (unsafe { GetClipboardData(format) }) else {
            continue;
        };
        let memory = HGLOBAL(handle.0);
        // SAFETY: a memory format's handle is an HGLOBAL; GlobalSize answers 0 for anything else.
        let size = unsafe { GlobalSize(memory) };
        if size > 0 {
            total = total.saturating_add(size);
            found.push((format, memory, size));
        }
    }
    let keep_all = total <= MAX_SAVED_BYTES;
    let mut saved = Saved::default();
    for (format, memory, size) in found {
        if !keep_all && format != UNICODE_TEXT {
            saved.skipped_bytes += size;
            continue;
        }
        if let Some(bytes) = read_global(memory, size) {
            saved.formats.push((format, bytes));
        }
    }
    if saved.skipped_bytes > 0 {
        tracing::info!(
            skipped_bytes = saved.skipped_bytes,
            "clipboard_save_over_limit"
        );
    }
    Ok(saved)
}

/// Replaces the clipboard's contents with `saved`, format by format.
pub fn restore(saved: &Saved) -> Result<(), CaptureError> {
    let owner = Owner::new()?;
    let _open = Open::new(Some(owner.0))?;
    // SAFETY: the clipboard is open on this thread, owned by `owner`.
    unsafe { EmptyClipboard() }.map_err(|e| os_error(&e))?;
    let mut first_error = None;
    for (format, bytes) in &saved.formats {
        if let Err(e) = set_bytes(*format, bytes) {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// The clipboard's text, if it holds any.
pub fn read_text() -> Result<Option<String>, CaptureError> {
    let _open = Open::new(None)?;
    // SAFETY: the clipboard is open on this thread.
    if unsafe { IsClipboardFormatAvailable(UNICODE_TEXT) }.is_err() {
        return Ok(None);
    }
    // SAFETY: as above; the handle stays valid until the clipboard is closed.
    let Ok(handle) = (unsafe { GetClipboardData(UNICODE_TEXT) }) else {
        return Ok(None);
    };
    let memory = HGLOBAL(handle.0);
    // SAFETY: CF_UNICODETEXT's data is an HGLOBAL.
    let size = unsafe { GlobalSize(memory) };
    let Some(bytes) = read_global(memory, size) else {
        return Ok(None);
    };
    Ok(Some(from_clipboard_text(&bytes)))
}

/// Puts `text` on the clipboard as its only contents; a [`Entry::Temporary`] entry also carries
/// the [`EXCLUSION_FORMATS`].
pub fn set_text(text: &str, entry: Entry) -> Result<(), CaptureError> {
    let owner = Owner::new()?;
    let _open = Open::new(Some(owner.0))?;
    // SAFETY: the clipboard is open on this thread, owned by `owner`.
    unsafe { EmptyClipboard() }.map_err(|e| os_error(&e))?;
    if entry == Entry::Temporary {
        let zero = 0u32.to_le_bytes();
        for name in EXCLUSION_FORMATS {
            set_bytes(format_id(name), &zero)?;
        }
    }
    set_bytes(UNICODE_TEXT, &to_clipboard_text(text))
}

/// `text` as `CF_UNICODETEXT` data: CR LF line endings, UTF-16 little-endian, NUL-terminated.
pub fn to_clipboard_text(text: &str) -> Vec<u8> {
    let mut crlf = String::with_capacity(text.len() + 2);
    let mut previous = '\0';
    for c in text.chars() {
        if c == '\n' && previous != '\r' {
            crlf.push('\r');
        }
        crlf.push(c);
        previous = c;
    }
    crlf.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// `CF_UNICODETEXT` data as text: up to the first NUL, CR LF as LF, unpaired surrogates as U+FFFD.
pub fn from_clipboard_text(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .take_while(|&unit| unit != 0)
        .collect();
    String::from_utf16_lossy(&units).replace("\r\n", "\n")
}

/// Copies `size` bytes out of a global memory block.
fn read_global(memory: HGLOBAL, size: usize) -> Option<Vec<u8>> {
    // SAFETY: `memory` is a live HGLOBAL of at least `size` bytes (its GlobalSize); it is locked
    // for the copy and unlocked after.
    unsafe {
        let ptr = GlobalLock(memory) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
        let _ = GlobalUnlock(memory);
        Some(bytes)
    }
}

/// Puts `bytes` on the open clipboard as `format`, in a new global memory block that the
/// clipboard then owns.
fn set_bytes(format: u32, bytes: &[u8]) -> Result<(), CaptureError> {
    // SAFETY: a new block of at least `bytes.len()` bytes is locked, filled, unlocked, and
    // handed to the clipboard, or freed when the clipboard refuses it.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).map_err(|e| os_error(&e))?;
        let ptr = GlobalLock(memory) as *mut u8;
        if ptr.is_null() {
            let error = last_error();
            let _ = GlobalFree(Some(memory));
            return Err(error);
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(memory);
        if let Err(e) = SetClipboardData(format, Some(HANDLE(memory.0))) {
            let _ = GlobalFree(Some(memory));
            return Err(os_error(&e));
        }
    }
    Ok(())
}

/// The clipboard, open on this thread until dropped.
struct Open;

impl Open {
    fn new(owner: Option<HWND>) -> Result<Self, CaptureError> {
        for attempt in 0..OPEN_ATTEMPTS {
            if attempt > 0 {
                std::thread::sleep(OPEN_RETRY);
            }
            // SAFETY: `owner`, when given, is a window of this thread.
            if unsafe { OpenClipboard(owner) }.is_ok() {
                return Ok(Open);
            }
        }
        tracing::info!(attempts = OPEN_ATTEMPTS, "clipboard_busy");
        Err(CaptureError::ClipboardBusy)
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: the clipboard was opened on this thread by `Open::new`.
        let _ = unsafe { CloseClipboard() };
    }
}

/// A message-only window that owns the clipboard for one write, destroyed when dropped. The
/// data stays on the clipboard after it is gone.
struct Owner(HWND);

impl Owner {
    fn new() -> Result<Self, CaptureError> {
        // SAFETY: creates a message-only window of the predefined STATIC class on this thread.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }
        .map_err(|e| os_error(&e))?;
        Ok(Owner(hwnd))
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        // SAFETY: the window was created on this thread by `Owner::new`.
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_object_formats_are_not_saved() {
        for id in [
            2, 3, 9, 14, 0x80, 0x82, 0x83, 0x8E, 0x0200, 0x02FF, 0x0300, 0x03FF,
        ] {
            assert!(!is_saved_format(id), "{id:#x}");
        }
        // CF_TEXT, CF_DIB, CF_UNICODETEXT, CF_HDROP, CF_LOCALE, CF_DIBV5, CF_DSPTEXT, and a
        // registered format such as "HTML Format".
        for id in [1, 8, 13, 15, 16, 17, 0x81, 0xC0F0] {
            assert!(is_saved_format(id), "{id:#x}");
        }
    }

    #[test]
    fn text_goes_on_with_crlf_and_comes_off_with_lf() {
        let bytes = to_clipboard_text("a\nb\r\nc");
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&p| u16::from_le_bytes(p))
            .collect();
        assert_eq!(units, "a\r\nb\r\nc\0".encode_utf16().collect::<Vec<_>>());
        assert_eq!(from_clipboard_text(&bytes), "a\nb\nc");
    }

    #[test]
    fn text_stops_at_the_first_nul_and_keeps_astral_characters() {
        let mut bytes = to_clipboard_text("héllo 👍");
        bytes.extend_from_slice(&[0x41, 0x00, 0x00, 0x00]);
        assert_eq!(from_clipboard_text(&bytes), "héllo 👍");
        assert_eq!(from_clipboard_text(&[]), "");
        // An odd trailing byte is ignored.
        assert_eq!(from_clipboard_text(&[0x41, 0x00, 0x42]), "A");
    }

    #[test]
    fn the_limit_is_64_mb() {
        assert_eq!(MAX_SAVED_BYTES, 67_108_864);
        assert_eq!(UNICODE_TEXT, 13);
    }

    #[test]
    fn saved_formats_are_found_by_id() {
        let saved = Saved {
            formats: vec![(13, vec![1, 0]), (8, vec![2])],
            skipped_bytes: 0,
        };
        assert_eq!(saved.get(8), Some(&[2u8][..]));
        assert_eq!(saved.get(1), None);
    }
}
