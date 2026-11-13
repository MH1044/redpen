//! Module 05 tests on Windows (docs/modules/05-capture-and-replace-windows.md, Acceptance tests 10
//! to 17), with the helper window `redpen-testwin`. A test named `tNN_…` covers bullet NN; a test
//! named `e_…` covers an error path or a Scope bullet on the real desktop. Bullets 1 to 9 are the
//! fake tests in `crates/redpen-core/tests/05_capture.rs`.
//!
//! These tests type real keys and replace the clipboard, so they run only with
//! `REDPEN_DESKTOP_TESTS=1` (Design notes): in CI's `windows-integration` job, and on a developer's
//! computer only on purpose, with nothing else open. Without it each test prints the skip message
//! and passes. They share one lock, so two of them never run at once.
//!
//! Expected values are true by construction: the text typed into the helper window, the clipboard
//! formats this file puts on the clipboard itself (through the Win32 functions declared in
//! `mod ffi`, never through Redpen), and the timings and limits of the spec.
//! `tests/cases/05-capture-and-replace-windows/REFERENCE.md` has the byte layouts, the arithmetic
//! and the helper's line protocol.
//!
//! The spec does not name the Windows `Capture` type, `Captured`'s fields or `Rect`'s fields; those
//! are gathered in `mod api`, so a different name changes one line there and no expected value.
#![cfg(windows)]

use std::future::Future;
use std::io::{BufRead, BufReader, Write};
use std::pin::Pin;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use redpen_core::clock::{Sleeper, SystemClock};
use redpen_core::flow::deliver::deliver;
use redpen_core::flow::rewrite::capture_session;
use redpen_core::platform::{Capture, CaptureError, check_allowed};
use redpen_core::store::Settings;

use ffi::Hwnd;

// ---------------------------------------------------------------------------------------------
// The names the spec leaves open.

mod api {
    use redpen_core::flow::rewrite::Captured;
    use redpen_core::platform::Rect;
    use redpen_os::windows::WindowsCapture;

    /// The Windows `Capture`.
    pub fn capture() -> WindowsCapture {
        WindowsCapture::new()
    }

    /// The selected text `capture_session` read.
    pub fn captured_text(c: &Captured) -> &str {
        &c.text
    }

    /// A rectangle as left, top, right and bottom, in screen pixels.
    pub fn ltrb(r: &Rect) -> (i64, i64, i64, i64) {
        let (x, y) = (i64::from(r.x), i64::from(r.y));
        (x, y, x + i64::from(r.width), y + i64::from(r.height))
    }
}

// ---------------------------------------------------------------------------------------------
// Constants of the spec and of docs/ARCHITECTURE.md §4 and §8.

const SKIP: &str = "Set REDPEN_DESKTOP_TESTS=1 to run the desktop tests; they type into the window in front and replace the clipboard.";

const MSG_PASSWORD: &str = "Redpen doesn't read or type into password fields.";
const MSG_NOTHING_COPIED: &str = "Nothing was copied. Select some text first.";
const MSG_WINDOW_GONE: &str = "The window you started in has closed. Copy the text instead.";
const MSG_CLIPBOARD_BUSY: &str = "Another app is using the clipboard. Try again.";

/// `copy_timeout_ms` and `paste_restore_delay_ms` (docs/ARCHITECTURE.md §4).
const COPY_TIMEOUT: Duration = Duration::from_millis(400);
const RESTORE_DELAY: Duration = Duration::from_millis(250);

/// The helper's executable, as `foreground()` names it (lower case).
const HELPER_EXE: &str = "redpen-testwin.exe";
const ORIGINAL: &str = "hello wrold";
const SELECTION: &str = "wrold";
const FIXED: &str = "hello world";
const REPLACEMENT: &str = "world";
const KEEP_ME: &str = "keep me";

/// Registered clipboard formats (REFERENCE.md).
const HTML_FORMAT: &str = "HTML Format";
const EXCLUDE_FROM_MONITORS: &str = "ExcludeClipboardContentFromMonitorProcessing";
const IN_HISTORY: &str = "CanIncludeInClipboardHistory";
const TO_CLOUD: &str = "CanUploadToCloudClipboard";

/// The 70 MB image of bullet 17: 4096 × 4480 pixels at 4 bytes each, 73 400 320 bytes (70 MiB),
/// after a 40-byte BITMAPINFOHEADER.
const BIG_WIDTH: i32 = 4096;
const BIG_HEIGHT: i32 = 4480;
const BIG_PIXEL_BYTES: u64 = 73_400_320;

// ---------------------------------------------------------------------------------------------
// The gate, and running the async flows without a runtime.

static DESKTOP: Mutex<()> = Mutex::new(());

/// The lock that keeps desktop tests apart, or `None` (after printing why) when they are off.
fn desktop() -> Option<MutexGuard<'static, ()>> {
    if std::env::var("REDPEN_DESKTOP_TESTS").as_deref() != Ok("1") {
        eprintln!("{SKIP}");
        return None;
    }
    Some(DESKTOP.lock().unwrap_or_else(|p| p.into_inner()))
}

/// Polls a future to completion on this thread. The flows wait only through the `Sleeper`, and
/// `ThreadSleeper` blocks, so each poll makes progress.
fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

/// A sleeper that blocks the calling thread for the whole duration (the trait's `async_trait`
/// form, written out).
struct ThreadSleeper;

impl Sleeper for ThreadSleeper {
    fn sleep<'life0, 'async_trait>(
        &'life0 self,
        d: Duration,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move { thread::sleep(d) })
    }
}

fn refused<T>(r: Result<T, CaptureError>) -> CaptureError {
    match r {
        Ok(_) => panic!("expected a refusal, got Ok"),
        Err(e) => e,
    }
}

fn ok<T>(r: Result<T, CaptureError>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("expected Ok, got {e:?}: {e}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Win32, declared here so the test checks the clipboard by its own route.

mod ffi {
    use std::ffi::c_void;

    pub type Hwnd = *mut c_void;
    pub type Handle = *mut c_void;
    pub type Bool = i32;
    pub type WndProc = unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize;
    pub type EnumProc = unsafe extern "system" fn(Hwnd, isize) -> Bool;

    pub const CF_DIB: u32 = 8;
    pub const CF_UNICODETEXT: u32 = 13;
    pub const GMEM_MOVEABLE: u32 = 0x0002;
    pub const GMEM_ZEROINIT: u32 = 0x0040;
    pub const WM_CLIPBOARDUPDATE: u32 = 0x031D;
    pub const PM_REMOVE: u32 = 0x0001;
    pub const GWL_STYLE: i32 = -16;
    pub const ES_PASSWORD: i32 = 0x0020;
    pub const HWND_MESSAGE: isize = -3;

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Rect32 {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    #[repr(C)]
    #[allow(dead_code, reason = "filled in and read by Windows")]
    pub struct Msg {
        pub hwnd: Hwnd,
        pub message: u32,
        pub wparam: usize,
        pub lparam: isize,
        pub time: u32,
        pub pt_x: i32,
        pub pt_y: i32,
        pub private: u32,
    }

    #[repr(C)]
    #[allow(dead_code, reason = "read by Windows")]
    pub struct WndClassW {
        pub style: u32,
        pub wnd_proc: Option<WndProc>,
        pub cls_extra: i32,
        pub wnd_extra: i32,
        pub instance: Handle,
        pub icon: Handle,
        pub cursor: Handle,
        pub background: Handle,
        pub menu_name: *const u16,
        pub class_name: *const u16,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn OpenClipboard(owner: Hwnd) -> Bool;
        pub fn CloseClipboard() -> Bool;
        pub fn EmptyClipboard() -> Bool;
        pub fn SetClipboardData(format: u32, mem: Handle) -> Handle;
        pub fn GetClipboardData(format: u32) -> Handle;
        pub fn IsClipboardFormatAvailable(format: u32) -> Bool;
        pub fn GetClipboardSequenceNumber() -> u32;
        pub fn RegisterClipboardFormatW(name: *const u16) -> u32;
        pub fn AddClipboardFormatListener(hwnd: Hwnd) -> Bool;
        pub fn RemoveClipboardFormatListener(hwnd: Hwnd) -> Bool;
        pub fn GetForegroundWindow() -> Hwnd;
        pub fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        pub fn AllowSetForegroundWindow(pid: u32) -> Bool;
        pub fn EnumChildWindows(parent: Hwnd, callback: EnumProc, lparam: isize) -> Bool;
        pub fn IsWindow(hwnd: Hwnd) -> Bool;
        pub fn GetClassNameW(hwnd: Hwnd, name: *mut u16, max: i32) -> i32;
        pub fn GetWindowLongW(hwnd: Hwnd, index: i32) -> i32;
        pub fn GetWindowRect(hwnd: Hwnd, rect: *mut Rect32) -> Bool;
        pub fn RegisterClassW(class: *const WndClassW) -> u16;
        pub fn CreateWindowExW(
            ex_style: u32,
            class: *const u16,
            title: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: Hwnd,
            menu: Handle,
            instance: Handle,
            param: *const c_void,
        ) -> Hwnd;
        pub fn DestroyWindow(hwnd: Hwnd) -> Bool;
        pub fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
        pub fn PeekMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32, remove: u32) -> Bool;
        pub fn DispatchMessageW(msg: *const Msg) -> isize;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
        pub fn GlobalLock(mem: Handle) -> *mut c_void;
        pub fn GlobalUnlock(mem: Handle) -> Bool;
        pub fn GlobalSize(mem: Handle) -> usize;
        pub fn GlobalFree(mem: Handle) -> Handle;
        pub fn GetModuleHandleW(name: *const u16) -> Handle;
    }
}

/// The test's own clipboard access.
mod clip {
    use std::ptr;
    use std::thread;
    use std::time::Duration;

    use super::ffi::{self, Hwnd};

    /// A NUL-terminated UTF-16 string.
    pub fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn register(name: &str) -> u32 {
        let id = unsafe { ffi::RegisterClipboardFormatW(wide(name).as_ptr()) };
        assert_ne!(id, 0, "the clipboard format {name} could not be registered");
        id
    }

    pub fn sequence() -> u32 {
        unsafe { ffi::GetClipboardSequenceNumber() }
    }

    pub fn available(format: u32) -> bool {
        unsafe { ffi::IsClipboardFormatAvailable(format) != 0 }
    }

    /// A message-only window on this thread, to own the clipboard; destroyed when dropped.
    pub struct MessageWindow(pub Hwnd);

    impl MessageWindow {
        pub fn create() -> Self {
            let class = wide("STATIC");
            let title = wide("");
            let hwnd = unsafe {
                ffi::CreateWindowExW(
                    0,
                    class.as_ptr(),
                    title.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    ffi::HWND_MESSAGE as Hwnd,
                    ptr::null_mut(),
                    ffi::GetModuleHandleW(ptr::null()),
                    ptr::null(),
                )
            };
            assert!(!hwnd.is_null(), "a message-only window could not be made");
            Self(hwnd)
        }
    }

    impl Drop for MessageWindow {
        fn drop(&mut self) {
            unsafe { ffi::DestroyWindow(self.0) };
        }
    }

    /// The clipboard, open on this thread until dropped.
    pub struct Open;

    impl Drop for Open {
        fn drop(&mut self) {
            unsafe { ffi::CloseClipboard() };
        }
    }

    /// Opens the clipboard, waiting up to 2 s for another program to close it.
    pub fn open(owner: Hwnd) -> Open {
        for _ in 0..200 {
            if unsafe { ffi::OpenClipboard(owner) } != 0 {
                return Open;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the test could not open the clipboard within 2 s");
    }

    /// One format's data: bytes, or a header followed by zeros up to `total` bytes.
    pub enum Data {
        Bytes(Vec<u8>),
        HeaderThenZeros { header: Vec<u8>, total: usize },
    }

    /// Empties the clipboard and puts `entries` on it, owned by a window that is then destroyed
    /// (the data stays: none of it is delayed-render).
    pub fn put(entries: &[(u32, Data)]) {
        let owner = MessageWindow::create();
        {
            let _open = open(owner.0);
            assert_ne!(unsafe { ffi::EmptyClipboard() }, 0, "EmptyClipboard failed");
            for (format, data) in entries {
                let (head, total) = match data {
                    Data::Bytes(b) => (b.as_slice(), b.len()),
                    Data::HeaderThenZeros { header, total } => (header.as_slice(), *total),
                };
                unsafe {
                    let h = ffi::GlobalAlloc(ffi::GMEM_MOVEABLE | ffi::GMEM_ZEROINIT, total);
                    assert!(!h.is_null(), "GlobalAlloc({total}) failed");
                    let p = ffi::GlobalLock(h).cast::<u8>();
                    assert!(!p.is_null(), "GlobalLock failed");
                    ptr::copy_nonoverlapping(head.as_ptr(), p, head.len());
                    ffi::GlobalUnlock(h);
                    if ffi::SetClipboardData(*format, h).is_null() {
                        ffi::GlobalFree(h);
                        panic!("SetClipboardData({format}) failed");
                    }
                }
            }
        }
        drop(owner);
    }

    /// Reads one format of the clipboard this thread has open; `None` when it is not there.
    pub fn read_open(format: u32) -> Option<Vec<u8>> {
        unsafe {
            let h = ffi::GetClipboardData(format);
            if h.is_null() {
                return None;
            }
            let size = ffi::GlobalSize(h);
            let p = ffi::GlobalLock(h).cast::<u8>().cast_const();
            if p.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(p, size).to_vec();
            ffi::GlobalUnlock(h);
            Some(bytes)
        }
    }

    /// The bytes of each format, read under one open.
    pub fn read(formats: &[u32]) -> Vec<Option<Vec<u8>>> {
        let _open = open(ptr::null_mut());
        formats.iter().map(|&f| read_open(f)).collect()
    }
}

use clip::Data;

/// `s` as CF_UNICODETEXT: UTF-16LE with its terminator.
fn unicode_text(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// CF_UNICODETEXT bytes back to text, up to the terminator.
fn text_of(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// A CF_HTML entry ("HTML Clipboard Format"): a header of 10-digit byte offsets, then the
/// document, then a NUL. The offsets are computed, so the header is right by construction.
fn cf_html(fragment: &str) -> Vec<u8> {
    const PRE: &str = "<html><body>\r\n<!--StartFragment-->";
    const POST: &str = "<!--EndFragment-->\r\n</body></html>";
    let header = |start_html: usize, end_html: usize, start_frag: usize, end_frag: usize| {
        format!(
            "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_frag:010}\r\nEndFragment:{end_frag:010}\r\n"
        )
    };
    let start_html = header(0, 0, 0, 0).len();
    let start_frag = start_html + PRE.len();
    let end_frag = start_frag + fragment.len();
    let end_html = end_frag + POST.len();
    let mut out = header(start_html, end_html, start_frag, end_frag).into_bytes();
    out.extend_from_slice(PRE.as_bytes());
    out.extend_from_slice(fragment.as_bytes());
    out.extend_from_slice(POST.as_bytes());
    out.push(0);
    out
}

/// A BITMAPINFOHEADER (40 bytes, little-endian) for a 32-bit, uncompressed (BI_RGB) image.
fn dib_header(width: i32, height: i32) -> Vec<u8> {
    let image = u32::try_from(width * height * 4).expect("the image size fits a DWORD");
    let mut h = Vec::with_capacity(40);
    h.extend_from_slice(&40u32.to_le_bytes()); // biSize
    h.extend_from_slice(&width.to_le_bytes()); // biWidth
    h.extend_from_slice(&height.to_le_bytes()); // biHeight (bottom-up)
    h.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    h.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    h.extend_from_slice(&0u32.to_le_bytes()); // biCompression: BI_RGB
    h.extend_from_slice(&image.to_le_bytes()); // biSizeImage
    h.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    h.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    h.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    h.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    assert_eq!(h.len(), 40);
    h
}

/// A 2×2 DIB: red, green, blue and white, as blue-green-red-unused bytes. 56 bytes.
fn dib_2x2() -> Vec<u8> {
    let mut d = dib_header(2, 2);
    d.extend_from_slice(&[0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0]);
    assert_eq!(d.len(), 56);
    d
}

/// What the clipboard held before a capture: each format's name, number and bytes as read back.
struct Snapshot {
    entries: Vec<(&'static str, u32, Vec<u8>)>,
}

/// Puts `keep me`, an HTML Format fragment and a 2×2 DIB on the clipboard (bullet 10) and reads
/// them back.
fn put_keep_me() -> Snapshot {
    let written = vec![
        ("CF_UNICODETEXT", ffi::CF_UNICODETEXT, unicode_text(KEEP_ME)),
        (
            HTML_FORMAT,
            clip::register(HTML_FORMAT),
            cf_html("<b>keep me</b>"),
        ),
        ("CF_DIB", ffi::CF_DIB, dib_2x2()),
    ];
    let entries: Vec<(u32, Data)> = written
        .iter()
        .map(|(_, f, b)| (*f, Data::Bytes(b.clone())))
        .collect();
    clip::put(&entries);
    let formats: Vec<u32> = written.iter().map(|(_, f, _)| *f).collect();
    let read = clip::read(&formats);
    let entries = written
        .into_iter()
        .zip(read)
        .map(|((name, format, bytes), got)| {
            let got = got.unwrap_or_else(|| panic!("{name} is not on the clipboard after put"));
            assert!(
                got.starts_with(&bytes),
                "{name} reads back differently from what was put"
            );
            (name, format, got)
        })
        .collect();
    Snapshot { entries }
}

/// Invariant 5: every format of the snapshot holds the same bytes.
fn assert_restored(before: &Snapshot) {
    let formats: Vec<u32> = before.entries.iter().map(|(_, f, _)| *f).collect();
    let after = clip::read(&formats);
    for ((name, _, bytes), got) in before.entries.iter().zip(after) {
        let got = got.unwrap_or_else(|| panic!("{name} is missing from the restored clipboard"));
        assert!(
            &got == bytes,
            "{name} was not restored byte for byte: {} bytes before, {} after",
            bytes.len(),
            got.len()
        );
    }
}

/// The 300-word selection of bullet 16: a 10-word sentence, 30 times (1 529 characters).
fn three_hundred_words() -> String {
    ["The quick brown fox jumps over the lazy dog today."; 30].join(" ")
}

// ---------------------------------------------------------------------------------------------
// The helper window. Protocol (REFERENCE.md): `ready` once the window is shown, then one line
// in, one line out: `ok` (or `error: …`) for `focus`, `set` and `select`, the focused EDIT's
// text for `get`.

struct TestWin {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

impl TestWin {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_redpen-testwin"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("redpen-testwin starts");
        // The process that started the helper may hand it the foreground.
        unsafe { ffi::AllowSetForegroundWindow(child.id()) };
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line.trim_end_matches('\r').to_string()).is_err() {
                    break;
                }
            }
        });
        let mut w = Self {
            child,
            stdin,
            lines: rx,
        };
        let first = w.reply("start");
        assert_eq!(
            first, "ready",
            "redpen-testwin prints `ready` once its window is shown"
        );
        w
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn reply(&mut self, what: &str) -> String {
        match self.lines.recv_timeout(Duration::from_secs(10)) {
            Ok(line) => line,
            Err(_) => panic!(
                "redpen-testwin gave no answer to `{what}` within 10 s; it answers every command with one line"
            ),
        }
    }

    fn cmd(&mut self, line: &str) -> String {
        writeln!(self.stdin, "{line}")
            .and_then(|()| self.stdin.flush())
            .expect("redpen-testwin reads its input");
        let reply = self.reply(line);
        assert!(
            !reply.starts_with("error"),
            "redpen-testwin refused `{line}`: {reply}"
        );
        reply
    }

    fn get(&mut self) -> String {
        self.cmd("get")
    }

    /// Focuses the `normal` or `password` EDIT, gives it `text` and the selection `start..end`
    /// (UTF-16 offsets), and waits until the helper's window is in front. Returns that window.
    fn prepare(&mut self, which: &str, text: &str, start: usize, end: usize) -> Hwnd {
        self.cmd(&format!("focus {which}"));
        self.cmd(&format!("set {text}"));
        self.cmd(&format!("select {start} {end}"));
        assert_eq!(self.get(), text, "redpen-testwin did not take the text");
        self.wait_front()
    }

    /// The helper's top-level window once it is in front.
    fn wait_front(&self) -> Hwnd {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let hwnd = unsafe { ffi::GetForegroundWindow() };
            let mut pid = 0u32;
            if !hwnd.is_null() {
                unsafe { ffi::GetWindowThreadProcessId(hwnd, &mut pid) };
            }
            if pid == self.pid() {
                return hwnd;
            }
            assert!(
                Instant::now() < deadline,
                "redpen-testwin's window did not come to the front (process {pid} is in front); docs/TESTING.md §6"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Asks for the focused EDIT's text until it is `expected`, for up to 3 s.
    fn wait_text(&mut self, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let text = self.get();
            if text == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the EDIT reads {text:?}, not {expected:?}"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// The screen rectangle of the EDIT without `ES_PASSWORD` in `top`.
    fn normal_edit_rect(&self, top: Hwnd) -> ffi::Rect32 {
        let edits: Vec<Hwnd> = children(top)
            .into_iter()
            .filter(|&h| class_name(h).eq_ignore_ascii_case("edit"))
            .filter(|&h| unsafe { ffi::GetWindowLongW(h, ffi::GWL_STYLE) } & ffi::ES_PASSWORD == 0)
            .collect();
        assert_eq!(edits.len(), 1, "redpen-testwin has one normal EDIT");
        let mut r = ffi::Rect32::default();
        assert_ne!(unsafe { ffi::GetWindowRect(edits[0], &mut r) }, 0);
        r
    }

    /// Ends the helper and waits until `top` no longer exists.
    fn close(&mut self, top: Hwnd) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let deadline = Instant::now() + Duration::from_secs(5);
        while unsafe { ffi::IsWindow(top) } != 0 {
            assert!(Instant::now() < deadline, "the helper's window outlived it");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for TestWin {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

unsafe extern "system" fn collect_child(hwnd: Hwnd, lparam: isize) -> ffi::Bool {
    let found = unsafe { &mut *(lparam as *mut Vec<Hwnd>) };
    found.push(hwnd);
    1
}

fn children(parent: Hwnd) -> Vec<Hwnd> {
    let mut found: Vec<Hwnd> = Vec::new();
    unsafe { ffi::EnumChildWindows(parent, collect_child, ptr::addr_of_mut!(found) as isize) };
    found
}

fn class_name(hwnd: Hwnd) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { ffi::GetClassNameW(hwnd, buf.as_mut_ptr(), 64) };
    String::from_utf16_lossy(&buf[..usize::try_from(n).unwrap_or(0)])
}

// ---------------------------------------------------------------------------------------------
// Another program holding the clipboard open (bullet 14), and a clipboard listener (bullet 13).

/// A thread that opens the clipboard with its own window and keeps it open until dropped.
struct Holder {
    release: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Holder {
    fn start() -> Self {
        let (opened_tx, opened_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let thread = thread::spawn(move || {
            let owner = clip::MessageWindow::create();
            let open = clip::open(owner.0);
            let _ = opened_tx.send(());
            let _ = release_rx.recv_timeout(Duration::from_secs(30));
            drop(open);
            drop(owner);
        });
        opened_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the holder opened the clipboard");
        Self {
            release: release_tx,
            thread: Some(thread),
        }
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.release.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// One clipboard change as the listener saw it.
#[derive(Debug, Clone)]
struct Seen {
    text: Option<String>,
    exclude_from_monitors: bool,
    in_history: Option<Vec<u8>>,
    to_cloud: Option<Vec<u8>>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

fn seen() -> MutexGuard<'static, Vec<Seen>> {
    SEEN.lock().unwrap_or_else(|p| p.into_inner())
}

/// Reads the entry that was just put on the clipboard.
fn read_entry(owner: Hwnd) -> Option<Seen> {
    for _ in 0..100 {
        if unsafe { ffi::OpenClipboard(owner) } != 0 {
            let entry = Seen {
                text: clip::read_open(ffi::CF_UNICODETEXT).map(|b| text_of(&b)),
                exclude_from_monitors: clip::available(clip::register(EXCLUDE_FROM_MONITORS)),
                in_history: clip::read_open(clip::register(IN_HISTORY)),
                to_cloud: clip::read_open(clip::register(TO_CLOUD)),
            };
            unsafe { ffi::CloseClipboard() };
            return Some(entry);
        }
        thread::sleep(Duration::from_millis(1));
    }
    None
}

unsafe extern "system" fn listener_proc(
    hwnd: Hwnd,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if msg == ffi::WM_CLIPBOARDUPDATE {
        if let Some(entry) = read_entry(hwnd) {
            seen().push(entry);
        }
        return 0;
    }
    unsafe { ffi::DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A clipboard format listener on its own thread, with a message loop.
struct Listener {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Listener {
    fn start() -> Self {
        seen().clear();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            let class = clip::wide("RedpenTestClipboardListener");
            let title = clip::wide("");
            unsafe {
                let instance = ffi::GetModuleHandleW(ptr::null());
                let wc = ffi::WndClassW {
                    style: 0,
                    wnd_proc: Some(listener_proc),
                    cls_extra: 0,
                    wnd_extra: 0,
                    instance,
                    icon: ptr::null_mut(),
                    cursor: ptr::null_mut(),
                    background: ptr::null_mut(),
                    menu_name: ptr::null(),
                    class_name: class.as_ptr(),
                };
                // A second registration in the same process fails and the first one stays.
                ffi::RegisterClassW(&wc);
                let hwnd = ffi::CreateWindowExW(
                    0,
                    class.as_ptr(),
                    title.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    ffi::HWND_MESSAGE as Hwnd,
                    ptr::null_mut(),
                    instance,
                    ptr::null(),
                );
                let listening = !hwnd.is_null() && ffi::AddClipboardFormatListener(hwnd) != 0;
                let _ = ready_tx.send(listening);
                let mut msg: ffi::Msg = std::mem::zeroed();
                while !flag.load(Ordering::SeqCst) {
                    while ffi::PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, ffi::PM_REMOVE) != 0 {
                        ffi::DispatchMessageW(&msg);
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                if !hwnd.is_null() {
                    ffi::RemoveClipboardFormatListener(hwnd);
                    ffi::DestroyWindow(hwnd);
                }
            }
        });
        assert_eq!(
            ready_rx.recv_timeout(Duration::from_secs(10)),
            Ok(true),
            "the test's clipboard listener could not start"
        );
        Self {
            stop,
            thread: Some(thread),
        }
    }

    /// Stops listening; every change seen, in order.
    fn finish(mut self) -> Vec<Seen> {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        seen().clone()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The DWORD at the start of a format's data.
fn dword(data: Option<&[u8]>) -> Option<u32> {
    data.filter(|b| b.len() >= 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

// ---------------------------------------------------------------------------------------------
// A `tracing` subscriber that keeps every event and span as one line of text (bullet 17).

#[derive(Clone, Default)]
struct LogLines {
    lines: Arc<Mutex<Vec<String>>>,
}

impl LogLines {
    fn push(&self, line: String) {
        self.lines
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(line);
    }

    fn all(&self) -> Vec<String> {
        self.lines.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

struct Fields(String);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write as _;
        let _ = write!(self.0, " {}={:?}", field.name(), value);
    }
}

impl tracing::Subscriber for LogLines {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let mut f = Fields(span.metadata().name().to_string());
        span.record(&mut f);
        self.push(f.0);
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        let mut f = Fields(String::new());
        values.record(&mut f);
        self.push(f.0);
    }

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut f = Fields(String::new());
        event.record(&mut f);
        self.push(f.0);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

/// Every run of digits in `line`, as a number.
fn numbers(line: &str) -> Vec<u64> {
    line.split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Bullets 10 to 17.

#[test]
fn t10_read_selection_returns_wrold_and_restores_every_format_byte_for_byte() {
    let Some(_desktop) = desktop() else { return };
    let mut tw = TestWin::start();
    let before = put_keep_me();
    tw.prepare("normal", ORIGINAL, 6, 11);
    let cap = api::capture();
    let text = ok(cap.read_selection(COPY_TIMEOUT));
    assert_eq!(text, SELECTION);
    assert_restored(&before);
    assert_eq!(tw.get(), ORIGINAL, "a copy changed the EDIT");
}

#[test]
fn t11_deliver_types_world_into_the_selection_and_restores_the_clipboard() {
    let Some(_desktop) = desktop() else { return };
    let mut tw = TestWin::start();
    let before = put_keep_me();
    tw.prepare("normal", ORIGINAL, 6, 11);
    let cap = api::capture();
    let fg = ok(cap.foreground());
    assert_eq!(fg.window.pid, tw.pid());
    ok(block_on(deliver(
        &cap,
        &fg.window,
        REPLACEMENT,
        &Settings::default(),
        &ThreadSleeper,
    )));
    tw.wait_text(FIXED);
    assert_restored(&before);
}

#[test]
fn t12_password_edit_is_refused_without_a_copy() {
    let Some(_desktop) = desktop() else { return };
    let mut tw = TestWin::start();
    tw.prepare("password", "secret1", 0, 7);
    let cap = api::capture();
    let fg = ok(cap.foreground());
    assert!(
        fg.is_password,
        "UI Automation did not report the ES_PASSWORD EDIT"
    );
    let e = refused(check_allowed(&fg, &Settings::default().privacy.blocklist));
    assert!(matches!(e, CaptureError::PasswordField), "{e:?}");

    let seq = clip::sequence();
    let e = refused(block_on(capture_session(
        &cap,
        &Settings::default(),
        &SystemClock,
        &ThreadSleeper,
    )));
    assert!(matches!(e, CaptureError::PasswordField), "{e:?}");
    assert_eq!(e.to_string(), MSG_PASSWORD);
    assert_eq!(
        clip::sequence(),
        seq,
        "the clipboard changed: a Ctrl+C was sent"
    );
    assert_eq!(tw.get(), "secret1", "the password EDIT changed");
}

#[test]
fn t13_paste_marks_the_temporary_entry_with_the_three_exclusion_formats() {
    let Some(_desktop) = desktop() else { return };
    let mut tw = TestWin::start();
    put_keep_me();
    tw.prepare("normal", ORIGINAL, 6, 11);
    let listener = Listener::start();
    let cap = api::capture();
    ok(cap.paste(REPLACEMENT, RESTORE_DELAY));
    thread::sleep(Duration::from_millis(200));
    let seen = listener.finish();
    let entry = seen
        .iter()
        .find(|s| s.text.as_deref() == Some(REPLACEMENT))
        .unwrap_or_else(|| panic!("the listener never saw Redpen's temporary entry: {seen:?}"));
    assert!(
        entry.exclude_from_monitors,
        "{EXCLUDE_FROM_MONITORS} is missing: {entry:?}"
    );
    assert_eq!(
        dword(entry.in_history.as_deref()),
        Some(0),
        "{IN_HISTORY}: {entry:?}"
    );
    assert_eq!(
        dword(entry.to_cloud.as_deref()),
        Some(0),
        "{TO_CLOUD}: {entry:?}"
    );
}

#[test]
fn t14_clipboard_held_open_is_clipboard_busy_after_five_attempts() {
    let Some(_desktop) = desktop() else { return };
    let mut tw = TestWin::start();
    tw.prepare("normal", ORIGINAL, 6, 11);
    let cap = api::capture();
    let holder = Holder::start();
    let seq = clip::sequence();
    let started = Instant::now();
    let r = cap.read_selection(COPY_TIMEOUT);
    let elapsed = started.elapsed();
    drop(holder);
    let e = refused(r);
    assert!(matches!(e, CaptureError::ClipboardBusy), "{e:?}");
    assert_eq!(e.to_string(), MSG_CLIPBOARD_BUSY);
    // Five attempts 20 ms apart wait four times; Windows' 15.6 ms timer may cut each short.
    assert!(
        elapsed >= Duration::from_millis(60),
        "gave up after {elapsed:?}, before five attempts"
    );
    assert_eq!(clip::sequence(), seq);
    assert_eq!(tw.get(), ORIGINAL);
}

#[test]
fn t15_foreground_names_the_helper_and_puts_the_caret_inside_the_edit() {
    let Some(_desktop) = desktop() else { return };
    let mut tw = TestWin::start();
    let top = tw.prepare("normal", ORIGINAL, 6, 11);
    let cap = api::capture();
    let fg = ok(cap.foreground());
    assert_eq!(fg.process, HELPER_EXE);
    assert_eq!(fg.window.pid, tw.pid());
    assert_eq!(fg.window.id, top as usize as u64, "the window handle");
    assert!(!fg.is_password);
    assert!(!fg.secure_input);
    assert!(
        !fg.elevated,
        "a window at Redpen's own integrity level is not elevated"
    );
    let edit = tw.normal_edit_rect(top);
    let caret = fg.caret.as_ref().expect("a caret rectangle");
    let (left, top_edge, right, bottom) = api::ltrb(caret);
    assert!(
        left >= i64::from(edit.left)
            && top_edge >= i64::from(edit.top)
            && right <= i64::from(edit.right)
            && bottom <= i64::from(edit.bottom),
        "caret {:?} is outside the EDIT {edit:?}",
        (left, top_edge, right, bottom)
    );
    ok(check_allowed(&fg, &Settings::default().privacy.blocklist));
}

#[test]
fn t16_capturing_300_words_takes_at_most_150_ms_at_p95_over_20_runs() {
    let Some(_desktop) = desktop() else { return };
    let text = three_hundred_words();
    assert_eq!(text.split_whitespace().count(), 300);
    let mut tw = TestWin::start();
    tw.prepare("normal", &text, 0, text.encode_utf16().count());
    let cap = api::capture();
    let s = Settings::default();
    let mut times = Vec::with_capacity(20);
    for run in 0..20 {
        let started = Instant::now();
        let r = block_on(capture_session(&cap, &s, &SystemClock, &ThreadSleeper));
        times.push(started.elapsed());
        let captured = r.unwrap_or_else(|e| panic!("run {run}: {e}"));
        assert_eq!(api::captured_text(&captured), text, "run {run}");
    }
    times.sort();
    // Nearest rank: the 19th of 20 sorted times (ceil(0.95 × 20) = 19).
    let p95 = times[18];
    assert!(
        p95 <= Duration::from_millis(150),
        "p95 {p95:?} over {times:?}"
    );
}

#[test]
fn t17_a_70_mb_image_is_not_saved_and_the_earlier_text_is_kept() {
    let Some(_desktop) = desktop() else { return };
    let mut tw = TestWin::start();
    let total = 40 + usize::try_from(BIG_PIXEL_BYTES).unwrap();
    clip::put(&[
        (ffi::CF_UNICODETEXT, Data::Bytes(unicode_text(KEEP_ME))),
        (
            ffi::CF_DIB,
            Data::HeaderThenZeros {
                header: dib_header(BIG_WIDTH, BIG_HEIGHT),
                total,
            },
        ),
    ]);
    assert!(clip::available(ffi::CF_DIB));
    tw.prepare("normal", ORIGINAL, 6, 11);
    let cap = api::capture();
    let log = LogLines::default();
    let r = tracing::subscriber::with_default(log.clone(), || cap.read_selection(COPY_TIMEOUT));
    assert_eq!(ok(r), SELECTION);

    let after = clip::read(&[ffi::CF_UNICODETEXT]);
    let text = after[0]
        .as_deref()
        .map(text_of)
        .expect("text on the clipboard after the capture");
    assert_eq!(text, KEEP_ME);
    assert!(
        !clip::available(ffi::CF_DIB),
        "the 70 MB image was saved and put back"
    );

    let lines = log.all();
    assert!(
        lines
            .iter()
            .any(|l| numbers(l).iter().any(|&n| n >= BIG_PIXEL_BYTES)),
        "no log line records the skipped size in bytes: {lines:?}"
    );
    for line in &lines {
        for content in [KEEP_ME, SELECTION, "hello", "redpen-testwin"] {
            assert!(
                !line.contains(content),
                "a log line carries {content:?}: {line}"
            );
        }
    }
    // Leave the clipboard small for the next test.
    clip::put(&[]);
}

// ---------------------------------------------------------------------------------------------
// Error paths and Scope bullets on the real desktop.

#[test]
fn e06_empty_selection_is_nothing_copied_after_the_timeout_and_the_clipboard_is_restored() {
    let Some(_desktop) = desktop() else { return };
    let mut tw = TestWin::start();
    let before = put_keep_me();
    // An EDIT with an empty selection copies nothing, so the sequence number never changes.
    tw.prepare("normal", ORIGINAL, 3, 3);
    let cap = api::capture();
    let started = Instant::now();
    let r = cap.read_selection(COPY_TIMEOUT);
    let elapsed = started.elapsed();
    let e = refused(r);
    assert!(matches!(e, CaptureError::NothingCopied), "{e:?}");
    assert_eq!(e.to_string(), MSG_NOTHING_COPIED);
    // `copy_timeout_ms` is 400; the 10 ms polls may each be cut short by Windows' timer.
    assert!(
        elapsed >= Duration::from_millis(350),
        "gave up after {elapsed:?}, before the copy timeout"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "waited {elapsed:?} for a copy"
    );
    assert_restored(&before);
}

#[test]
fn e07_deliver_to_the_closed_helper_is_window_gone() {
    let Some(_desktop) = desktop() else { return };
    let mut tw = TestWin::start();
    let top = tw.prepare("normal", ORIGINAL, 6, 11);
    let cap = api::capture();
    let fg = ok(cap.foreground());
    let target = fg.window;
    match ok(cap.inspect(&target)) {
        Some(found) => assert_eq!(found.process, HELPER_EXE),
        None => panic!("inspect lost a window that exists"),
    }
    tw.close(top);
    assert!(
        ok(cap.inspect(&target)).is_none(),
        "inspect found a closed window"
    );
    let seq = clip::sequence();
    let e = refused(block_on(deliver(
        &cap,
        &target,
        REPLACEMENT,
        &Settings::default(),
        &ThreadSleeper,
    )));
    assert!(matches!(e, CaptureError::WindowGone), "{e:?}");
    assert_eq!(e.to_string(), MSG_WINDOW_GONE);
    assert_eq!(
        clip::sequence(),
        seq,
        "the clipboard changed with nowhere to paste"
    );
}

#[test]
fn e08_deliver_into_the_password_edit_is_refused_and_nothing_is_typed() {
    let Some(_desktop) = desktop() else { return };
    let mut tw = TestWin::start();
    tw.prepare("password", "secret1", 0, 7);
    let cap = api::capture();
    let fg = ok(cap.foreground());
    let seq = clip::sequence();
    let e = refused(block_on(deliver(
        &cap,
        &fg.window,
        REPLACEMENT,
        &Settings::default(),
        &ThreadSleeper,
    )));
    assert!(matches!(e, CaptureError::PasswordField), "{e:?}");
    assert_eq!(e.to_string(), MSG_PASSWORD);
    assert_eq!(
        clip::sequence(),
        seq,
        "the clipboard changed: a paste was prepared"
    );
    assert_eq!(tw.get(), "secret1", "the password EDIT changed");
}

#[test]
fn e09_send_undo_reverts_the_replace() {
    let Some(_desktop) = desktop() else { return };
    let mut tw = TestWin::start();
    put_keep_me();
    tw.prepare("normal", ORIGINAL, 6, 11);
    let cap = api::capture();
    let fg = ok(cap.foreground());
    ok(block_on(deliver(
        &cap,
        &fg.window,
        REPLACEMENT,
        &Settings::default(),
        &ThreadSleeper,
    )));
    tw.wait_text(FIXED);
    tw.wait_front();
    ok(cap.send_undo());
    tw.wait_text(ORIGINAL);
}

#[test]
fn e_copy_puts_a_normal_entry_on_the_clipboard() {
    let Some(_desktop) = desktop() else { return };
    put_keep_me();
    let cap = api::capture();
    ok(cap.set_clipboard_text("copied by Redpen"));
    let after = clip::read(&[ffi::CF_UNICODETEXT]);
    let text = after[0]
        .as_deref()
        .map(text_of)
        .expect("text on the clipboard");
    assert_eq!(text, "copied by Redpen");
    // The user asked for this entry: it is not marked to stay out of history or monitors.
    for name in [EXCLUDE_FROM_MONITORS, IN_HISTORY, TO_CLOUD] {
        assert!(
            !clip::available(clip::register(name)),
            "{name} is on a Copy entry"
        );
    }
}

#[test]
fn e14_copy_while_the_clipboard_is_held_is_clipboard_busy() {
    let Some(_desktop) = desktop() else { return };
    let cap = api::capture();
    let holder = Holder::start();
    let r = cap.set_clipboard_text("copied by Redpen");
    drop(holder);
    let e = refused(r);
    assert!(matches!(e, CaptureError::ClipboardBusy), "{e:?}");
    assert_eq!(e.to_string(), MSG_CLIPBOARD_BUSY);
}

#[test]
fn e_no_modifier_is_down_when_no_key_is_held() {
    let Some(_desktop) = desktop() else { return };
    assert!(!api::capture().modifiers_down());
}
