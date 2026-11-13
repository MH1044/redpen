//! Module 05 tests with fakes (docs/modules/05-capture-and-replace-windows.md, Acceptance tests 1
//! to 9). A test named `tNN_…` covers bullet NN; a test named `e_…` covers an error path, a message
//! or a Scope bullet the numbered list leaves implicit. Bullets 10 to 17 run on Windows, in
//! `crates/redpen-os/tests/05_capture_windows.rs`.
//!
//! Expected values come from the spec (Scope, Interfaces, Messages and Design notes),
//! docs/ARCHITECTURE.md §3 (the order of the rewrite flow), §4 (the default blocklist and the
//! capture settings), §5 (the results of `undo_last_replace`), §7 (invariants 3 to 5) and §8 (the
//! messages), with the arithmetic in `tests/cases/05-capture-and-replace-windows/REFERENCE.md`.
//! Nothing here is taken from Redpen's own output.
//!
//! The fake is `Desk`, a `Capture` written here (the Design notes allow a test its own): a small
//! model of a desktop with windows (process name, password field, elevation, whether it can be
//! brought to the front), the window in front, a clipboard, and keys held until a time on a
//! `FakeClock`. It records every call in order, with the window in front when a key was sent.
//! Because it answers every method from one model, a flow may ask in the order it chooses
//! (`inspect` or `foreground` after `focus`, for example) and the expected results still hold. The
//! sleeper moves the same clock forward, so "keys held for 1.2 s" means the same thing to a flow
//! that counts polls (the Design notes) and to one that reads the clock.
//!
//! The spec does not name `Captured`'s fields, the undo buffer's calls or `Point`'s fields; those
//! are gathered in `mod api`, so a different name changes one line there and no expected value.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use redpen_core::clock::{Clock, FakeClock, Sleeper};
use redpen_core::flow::deliver::deliver;
use redpen_core::flow::rewrite::capture_session;
use redpen_core::platform::{
    Capture, CaptureError, FakeCapture, Foreground, WindowRef, check_allowed,
};
use redpen_core::store::Settings;

// ---------------------------------------------------------------------------------------------
// The calls the spec leaves unnamed. Everything else in this file uses only the names of the
// spec's Interfaces and of modules 00 and 02.

mod api {
    use redpen_core::clock::{Clock, Sleeper};
    use redpen_core::flow::deliver::{UndoBuffer, UndoOutcome};
    use redpen_core::flow::rewrite::Captured;
    use redpen_core::platform::{Capture, CaptureError, Point, WindowRef};

    /// The selected text `capture_session` read.
    pub fn captured_text(c: &Captured) -> &str {
        &c.text
    }

    /// A cursor position, in screen pixels.
    pub fn point(x: i32, y: i32) -> Point {
        Point { x, y }
    }

    /// An empty undo buffer (Scope: "Undo buffer in memory").
    pub fn undo_buffer() -> UndoBuffer {
        UndoBuffer::default()
    }

    /// Remembers the last replace: the window and the text it held before, at the clock's time.
    pub fn remember(buf: &UndoBuffer, window: WindowRef, original: &str, clock: &dyn Clock) {
        buf.remember(window, original.to_string(), clock);
    }

    /// Undoes the last replace; the result as `undo_last_replace` names it (docs/ARCHITECTURE.md
    /// §5): `"undone"` or `"on_clipboard"`.
    pub async fn undo(
        buf: &UndoBuffer,
        cap: &dyn Capture,
        clock: &dyn Clock,
        sleeper: &dyn Sleeper,
    ) -> Result<&'static str, CaptureError> {
        buf.undo(cap, clock, sleeper).await.map(|o| match o {
            UndoOutcome::Undone => "undone",
            UndoOutcome::OnClipboard => "on_clipboard",
        })
    }

    /// `CaptureError::Os` for a Windows error code.
    pub fn os_error(code: u32) -> CaptureError {
        CaptureError::Os(code.to_string())
    }
}

// ---------------------------------------------------------------------------------------------
// Messages: docs/ARCHITECTURE.md §8 and the spec's Messages.

const MSG_PASSWORD: &str = "Redpen doesn't read or type into password fields.";
const MSG_BLOCKLISTED: &str = "Redpen is turned off for this app (Settings → Privacy).";
const MSG_ELEVATED: &str =
    "This window runs as administrator, so Windows doesn't let Redpen read or type into it.";
const MSG_NOTHING_COPIED: &str = "Nothing was copied. Select some text first.";
const MSG_KEYS_HELD: &str = "Release the shortcut keys, then try again.";
const MSG_FOCUS_CHANGED: &str =
    "The window you started in is no longer in front, so nothing was typed. Copy the text instead.";
const MSG_WINDOW_GONE: &str = "The window you started in has closed. Copy the text instead.";
const MSG_CLIPBOARD_BUSY: &str = "Another app is using the clipboard. Try again.";
const MSG_NOTHING_TO_UNDO: &str = "There is nothing to undo.";
const MSG_OS_5: &str = "Windows refused the request (code 5).";
/// The spec's Design notes (secure input, decided during implementation).
const MSG_SECURE_INPUT: &str =
    "Another app is protecting what you type right now, so Redpen can't read or type.";

/// The default blocklist of docs/ARCHITECTURE.md §4.
const ARCH_BLOCKLIST: [&str; 8] = [
    "1password.exe",
    "bitwarden.exe",
    "keepass.exe",
    "keepassxc.exe",
    "dashlane.exe",
    "enpass.exe",
    "nordpass.exe",
    "proton pass.exe",
];

/// docs/ARCHITECTURE.md §4: `copy_timeout_ms` 400 and `paste_restore_delay_ms` 250.
const COPY_TIMEOUT: Duration = Duration::from_millis(400);
const RESTORE_DELAY: Duration = Duration::from_millis(250);
/// Scope: after bringing the target to the front, wait 50 ms.
const FOCUS_WAIT: Duration = Duration::from_millis(50);
/// Design notes: the key wait polls every 10 ms, at most 100 times after the first check.
const POLL: Duration = Duration::from_millis(10);
/// 2026-09-21T14:13:20Z, the fake clock's start (as in module 02).
const T0: i64 = 1_790_000_000_000;

/// What the fake clipboard holds before every test.
const CLIPBOARD_BEFORE: &str = "keep me";
/// The selection in `hello wrold`, and the rewrite that replaces it. Undo gives the selection
/// back.
const SELECTION: &str = "wrold";
const REPLACEMENT: &str = "world";

// ---------------------------------------------------------------------------------------------
// The desktop model.

/// One top-level window: its handle and process, and what `foreground`/`inspect` report for the
/// control that has the keyboard focus in it.
#[derive(Debug, Clone, Copy)]
struct Win {
    id: u64,
    pid: u32,
    process: &'static str,
    is_password: bool,
    secure_input: bool,
    elevated: bool,
    /// Whether `focus` can bring it to the front.
    focusable: bool,
}

const fn win(id: u64, pid: u32, process: &'static str) -> Win {
    Win {
        id,
        pid,
        process,
        is_password: false,
        secure_input: false,
        elevated: false,
        focusable: true,
    }
}

/// An ordinary editor.
const NOTEPAD: Win = win(0x1001, 4100, "notepad.exe");
/// Redpen's popup, in front while a rewrite is shown (Scope, `paste`).
const POPUP: Win = win(0x2002, 4200, "redpen.exe");
/// A password manager on the default blocklist.
const VAULT: Win = win(0x3003, 4300, "1password.exe");

impl Win {
    fn password(mut self) -> Self {
        self.is_password = true;
        self
    }

    fn elevated(mut self) -> Self {
        self.elevated = true;
        self
    }

    fn secure(mut self) -> Self {
        self.secure_input = true;
        self
    }

    /// A window `focus` cannot bring to the front.
    fn stuck(mut self) -> Self {
        self.focusable = false;
        self
    }

    fn r(&self) -> WindowRef {
        WindowRef {
            id: self.id,
            pid: self.pid,
        }
    }

    fn fg(&self) -> Foreground {
        Foreground {
            window: self.r(),
            process: self.process.to_string(),
            is_password: self.is_password,
            secure_input: self.secure_input,
            elevated: self.elevated,
            caret: None,
            cursor: api::point(640, 360),
        }
    }
}

/// One call on the fake, in order. `front` is the window in front when a key was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Foreground,
    Inspect(u64),
    ModifiersDown,
    ReadSelection(Duration),
    Focus(u64),
    Paste {
        text: String,
        restore_delay: Duration,
        front: u64,
    },
    SendUndo {
        front: u64,
    },
    SetClipboardText(String),
}

struct DeskState {
    /// The windows that exist.
    windows: Vec<Win>,
    /// The id of the window in front.
    front: u64,
    /// The clipboard's text.
    clipboard: String,
    calls: Vec<Call>,
}

struct Desk {
    clock: Arc<FakeClock>,
    /// Ctrl, Alt, Shift or Windows is down while the clock reads less than this.
    keys_up_at_ms: i64,
    /// What Ctrl+C copies; `None` when the copy never changes the clipboard's sequence number.
    selection: Option<String>,
    state: Mutex<DeskState>,
}

impl Desk {
    /// `front` in front and nothing else open; the selection is `wrold`, no key is held.
    fn new(clock: &Arc<FakeClock>, front: Win) -> Self {
        Self {
            clock: Arc::clone(clock),
            keys_up_at_ms: i64::MIN,
            selection: Some(SELECTION.to_string()),
            state: Mutex::new(DeskState {
                windows: vec![front],
                front: front.id,
                clipboard: CLIPBOARD_BEFORE.to_string(),
                calls: Vec::new(),
            }),
        }
    }

    /// Another window that exists, behind the one in front.
    fn with(self, w: Win) -> Self {
        self.lock().windows.push(w);
        self
    }

    /// A copy that never changes the clipboard's sequence number.
    fn copying_nothing(mut self) -> Self {
        self.selection = None;
        self
    }

    /// The modifier keys are down from now for `d` on the fake clock.
    fn keys_held_for(mut self, d: Duration) -> Self {
        self.keys_up_at_ms = self.clock.now_ms() + i64::try_from(d.as_millis()).unwrap();
        self
    }

    fn lock(&self) -> MutexGuard<'_, DeskState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn calls(&self) -> Vec<Call> {
        self.lock().calls.clone()
    }

    fn clipboard(&self) -> String {
        self.lock().clipboard.clone()
    }

    fn reads(&self) -> Vec<Duration> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::ReadSelection(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    fn pastes(&self) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|c| matches!(c, Call::Paste { .. }))
            .collect()
    }

    fn undos(&self) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|c| matches!(c, Call::SendUndo { .. }))
            .collect()
    }

    fn clipboard_writes(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::SetClipboardText(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    /// Invariants 3 and 4: no synthetic key and no clipboard write happened.
    fn assert_no_key_sent(&self) {
        let calls = self.calls();
        let keyed: Vec<&Call> = calls
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    Call::ReadSelection(_)
                        | Call::Paste { .. }
                        | Call::SendUndo { .. }
                        | Call::SetClipboardText(_)
                )
            })
            .collect();
        assert!(
            keyed.is_empty(),
            "a key or a clipboard write was sent: {calls:?}"
        );
        assert_eq!(self.clipboard(), CLIPBOARD_BEFORE);
    }
}

impl Capture for Desk {
    fn foreground(&self) -> Result<Foreground, CaptureError> {
        let mut s = self.lock();
        s.calls.push(Call::Foreground);
        let front = s.front;
        let w = s
            .windows
            .iter()
            .find(|w| w.id == front)
            .copied()
            .expect("the window in front exists");
        Ok(w.fg())
    }

    fn inspect(&self, w: &WindowRef) -> Result<Option<Foreground>, CaptureError> {
        let mut s = self.lock();
        s.calls.push(Call::Inspect(w.id));
        Ok(s.windows.iter().find(|x| x.id == w.id).map(Win::fg))
    }

    fn modifiers_down(&self) -> bool {
        self.lock().calls.push(Call::ModifiersDown);
        self.clock.now_ms() < self.keys_up_at_ms
    }

    fn read_selection(&self, timeout: Duration) -> Result<String, CaptureError> {
        self.lock().calls.push(Call::ReadSelection(timeout));
        // The clipboard route saves the clipboard, copies, reads and restores it, so the fake's
        // clipboard holds what it held whichever way the copy went.
        match &self.selection {
            Some(text) => Ok(text.clone()),
            None => Err(CaptureError::NothingCopied),
        }
    }

    fn focus(&self, w: &WindowRef) -> Result<bool, CaptureError> {
        let mut s = self.lock();
        s.calls.push(Call::Focus(w.id));
        let front = s.front;
        match s.windows.iter().find(|x| x.id == w.id).copied() {
            Some(x) if x.id == front || x.focusable => {
                s.front = x.id;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn paste(&self, text: &str, restore_delay: Duration) -> Result<(), CaptureError> {
        let mut s = self.lock();
        let front = s.front;
        s.calls.push(Call::Paste {
            text: text.to_string(),
            restore_delay,
            front,
        });
        // The temporary entry is replaced by the saved clipboard after `restore_delay`.
        Ok(())
    }

    fn send_undo(&self) -> Result<(), CaptureError> {
        let mut s = self.lock();
        let front = s.front;
        s.calls.push(Call::SendUndo { front });
        Ok(())
    }

    fn set_clipboard_text(&self, text: &str) -> Result<(), CaptureError> {
        let mut s = self.lock();
        s.calls.push(Call::SetClipboardText(text.to_string()));
        s.clipboard = text.to_string();
        Ok(())
    }
}

/// A sleeper that returns at once, records every wait and moves the fake clock by it.
struct Ticks {
    clock: Arc<FakeClock>,
    sleeps: Mutex<Vec<Duration>>,
}

impl Ticks {
    fn sleeps(&self) -> Vec<Duration> {
        self.sleeps
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

#[async_trait::async_trait]
impl Sleeper for Ticks {
    async fn sleep(&self, d: Duration) {
        self.sleeps
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(d);
        self.clock.advance(d);
    }
}

/// A fake clock at `T0` and a sleeper that moves it.
fn time() -> (Arc<FakeClock>, Ticks) {
    let clock = Arc::new(FakeClock::at(T0));
    let sleeper = Ticks {
        clock: Arc::clone(&clock),
        sleeps: Mutex::new(Vec::new()),
    };
    (clock, sleeper)
}

fn blocklist(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
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

fn assert_message(e: &CaptureError, message: &str) {
    assert_eq!(e.to_string(), message, "{e:?}");
}

// ---------------------------------------------------------------------------------------------
// Bullets 1 to 4: refusals before any synthetic key (invariant 3), in the Scope's order.

#[tokio::test]
async fn t01_foreground_1password_is_blocklisted_and_nothing_is_read() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, VAULT);
    let e = refused(capture_session(&desk, &Settings::default(), &*clock, &sleeper).await);
    match &e {
        CaptureError::Blocklisted { app } => assert_eq!(app, "1password.exe"),
        other => panic!("expected Blocklisted, got {other:?}"),
    }
    assert_message(&e, MSG_BLOCKLISTED);
    assert!(desk.reads().is_empty(), "{:?}", desk.calls());
    desk.assert_no_key_sent();
}

#[test]
fn t01_check_allowed_refuses_1password_with_the_default_blocklist() {
    let e = refused(check_allowed(
        &VAULT.fg(),
        &Settings::default().privacy.blocklist,
    ));
    assert!(
        matches!(&e, CaptureError::Blocklisted { app } if app == "1password.exe"),
        "{e:?}"
    );
    assert_message(&e, MSG_BLOCKLISTED);
}

#[tokio::test]
async fn t02_password_field_is_refused_and_nothing_is_read() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD.password());
    let e = refused(capture_session(&desk, &Settings::default(), &*clock, &sleeper).await);
    assert!(matches!(e, CaptureError::PasswordField), "{e:?}");
    assert_message(&e, MSG_PASSWORD);
    assert!(desk.reads().is_empty(), "{:?}", desk.calls());
    desk.assert_no_key_sent();
}

#[tokio::test]
async fn t03_elevated_window_is_refused_and_nothing_is_read() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD.elevated());
    let e = refused(capture_session(&desk, &Settings::default(), &*clock, &sleeper).await);
    assert!(matches!(e, CaptureError::Elevated), "{e:?}");
    assert_message(&e, MSG_ELEVATED);
    assert!(desk.reads().is_empty(), "{:?}", desk.calls());
    desk.assert_no_key_sent();
}

#[tokio::test]
async fn t04_blocklisted_elevated_and_password_together_give_blocklisted() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, VAULT.elevated().password());
    let e = refused(capture_session(&desk, &Settings::default(), &*clock, &sleeper).await);
    assert!(
        matches!(&e, CaptureError::Blocklisted { app } if app == "1password.exe"),
        "{e:?}"
    );
    assert_message(&e, MSG_BLOCKLISTED);
    desk.assert_no_key_sent();
}

#[test]
fn t04_check_allowed_order_is_blocklisted_then_elevated_then_password() {
    let list = blocklist(&ARCH_BLOCKLIST);
    let all = refused(check_allowed(&VAULT.elevated().password().fg(), &list));
    assert!(matches!(all, CaptureError::Blocklisted { .. }), "{all:?}");
    let blocked_elevated = refused(check_allowed(&VAULT.elevated().fg(), &list));
    assert!(
        matches!(blocked_elevated, CaptureError::Blocklisted { .. }),
        "{blocked_elevated:?}"
    );
    let blocked_password = refused(check_allowed(&VAULT.password().fg(), &list));
    assert!(
        matches!(blocked_password, CaptureError::Blocklisted { .. }),
        "{blocked_password:?}"
    );
    let elevated_password = refused(check_allowed(&NOTEPAD.elevated().password().fg(), &list));
    assert!(
        matches!(elevated_password, CaptureError::Elevated),
        "{elevated_password:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// Bullet 5: the key wait (Design notes: 10 ms polls through the Sleeper, at most 100 after the
// first check). REFERENCE.md counts the polls.

#[tokio::test]
async fn t05_modifiers_down_for_1_2_s_is_keys_held() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD).keys_held_for(Duration::from_millis(1_200));
    let e = refused(capture_session(&desk, &Settings::default(), &*clock, &sleeper).await);
    assert!(matches!(e, CaptureError::KeysHeld), "{e:?}");
    assert_message(&e, MSG_KEYS_HELD);
    assert!(desk.reads().is_empty(), "{:?}", desk.calls());
    desk.assert_no_key_sent();
    // The first check and 100 more, 10 ms apart: 1 s of waiting, then the refusal.
    assert_eq!(sleeper.sleeps(), vec![POLL; 100]);
    let checks = desk
        .calls()
        .iter()
        .filter(|c| matches!(c, Call::ModifiersDown))
        .count();
    assert_eq!(checks, 101);
}

#[tokio::test]
async fn t05_modifiers_down_for_300_ms_then_the_capture_proceeds() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD).keys_held_for(Duration::from_millis(300));
    let captured = ok(capture_session(&desk, &Settings::default(), &*clock, &sleeper).await);
    assert_eq!(api::captured_text(&captured), SELECTION);
    // Down at 0, 10, …, 290 ms; up at the check at 300 ms.
    assert_eq!(sleeper.sleeps(), vec![POLL; 30]);
    assert_eq!(desk.reads(), vec![COPY_TIMEOUT]);
    // Ctrl+C is sent only once the keys are up: no check of the keys after the copy.
    let calls = desk.calls();
    let read_at = calls
        .iter()
        .position(|c| matches!(c, Call::ReadSelection(_)))
        .unwrap();
    let last_check = calls
        .iter()
        .rposition(|c| matches!(c, Call::ModifiersDown))
        .unwrap();
    assert!(last_check < read_at, "{calls:?}");
}

#[tokio::test]
async fn t05_keys_released_on_the_last_poll_still_capture() {
    // Down for exactly 1 s: up at the 100th poll after the first check.
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD).keys_held_for(Duration::from_millis(1_000));
    let captured = ok(capture_session(&desk, &Settings::default(), &*clock, &sleeper).await);
    assert_eq!(api::captured_text(&captured), SELECTION);
    assert_eq!(sleeper.sleeps(), vec![POLL; 100]);
    assert_eq!(desk.reads(), vec![COPY_TIMEOUT]);
}

#[tokio::test]
async fn e05_keys_already_up_capture_without_waiting() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD);
    let captured = ok(capture_session(&desk, &Settings::default(), &*clock, &sleeper).await);
    assert_eq!(api::captured_text(&captured), SELECTION);
    assert!(sleeper.sleeps().is_empty(), "{:?}", sleeper.sleeps());
    // One copy, and nothing written: the capture leaves the clipboard as it was (invariant 5).
    assert_eq!(desk.reads(), vec![COPY_TIMEOUT]);
    assert!(desk.pastes().is_empty());
    assert!(desk.undos().is_empty());
    assert!(desk.clipboard_writes().is_empty());
    assert_eq!(desk.clipboard(), CLIPBOARD_BEFORE);
}

#[tokio::test]
async fn e_refusals_come_before_the_key_wait() {
    // docs/ARCHITECTURE.md §3: foreground and the refusal checks, then the modifier keys.
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, VAULT).keys_held_for(Duration::from_millis(300));
    let e = refused(capture_session(&desk, &Settings::default(), &*clock, &sleeper).await);
    assert!(matches!(e, CaptureError::Blocklisted { .. }), "{e:?}");
    assert!(sleeper.sleeps().is_empty(), "{:?}", sleeper.sleeps());
    desk.assert_no_key_sent();
}

// ---------------------------------------------------------------------------------------------
// Bullet 6: a copy that never changes the sequence number.

#[tokio::test]
async fn t06_copy_without_a_sequence_change_is_nothing_copied_and_the_clipboard_is_restored() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD).copying_nothing();
    let e = refused(capture_session(&desk, &Settings::default(), &*clock, &sleeper).await);
    assert!(matches!(e, CaptureError::NothingCopied), "{e:?}");
    assert_message(&e, MSG_NOTHING_COPIED);
    // The copy waited `copy_timeout_ms`, 400 ms, once.
    assert_eq!(desk.reads(), vec![COPY_TIMEOUT]);
    // Nothing was written afterwards: the clipboard holds what it held.
    assert!(desk.pastes().is_empty());
    assert!(desk.clipboard_writes().is_empty());
    assert_eq!(desk.clipboard(), CLIPBOARD_BEFORE);
}

#[tokio::test]
async fn e06_copy_timeout_comes_from_the_settings() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD);
    let mut s = Settings::default();
    s.capture.copy_timeout_ms = 250;
    ok(capture_session(&desk, &s, &*clock, &sleeper).await);
    assert_eq!(desk.reads(), vec![Duration::from_millis(250)]);
}

// ---------------------------------------------------------------------------------------------
// Bullet 7: `deliver` brings the captured window to the front, or refuses.

#[tokio::test]
async fn t07_deliver_to_a_window_that_can_be_focused_pastes_once() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, POPUP).with(NOTEPAD);
    ok(deliver(
        &desk,
        &NOTEPAD.r(),
        REPLACEMENT,
        &Settings::default(),
        &sleeper,
    )
    .await);
    assert_eq!(
        desk.pastes(),
        vec![Call::Paste {
            text: REPLACEMENT.to_string(),
            restore_delay: RESTORE_DELAY,
            front: NOTEPAD.id,
        }]
    );
    assert!(
        sleeper.sleeps().contains(&FOCUS_WAIT),
        "no 50 ms wait after bringing the window to the front: {:?}",
        sleeper.sleeps()
    );
    assert!(desk.clipboard_writes().is_empty());
    assert_eq!(desk.clipboard(), CLIPBOARD_BEFORE);
}

#[tokio::test]
async fn t07_deliver_to_a_closed_window_is_window_gone_and_nothing_is_pasted() {
    let (clock, sleeper) = time();
    // NOTEPAD was captured, then closed: only the popup exists.
    let desk = Desk::new(&clock, POPUP);
    let e = refused(
        deliver(
            &desk,
            &NOTEPAD.r(),
            REPLACEMENT,
            &Settings::default(),
            &sleeper,
        )
        .await,
    );
    assert!(matches!(e, CaptureError::WindowGone), "{e:?}");
    assert_message(&e, MSG_WINDOW_GONE);
    assert!(desk.pastes().is_empty(), "{:?}", desk.calls());
    desk.assert_no_key_sent();
}

#[tokio::test]
async fn t07_deliver_to_a_window_that_cannot_come_to_the_front_is_focus_changed() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, POPUP).with(NOTEPAD.stuck());
    let e = refused(
        deliver(
            &desk,
            &NOTEPAD.r(),
            REPLACEMENT,
            &Settings::default(),
            &sleeper,
        )
        .await,
    );
    assert!(matches!(e, CaptureError::FocusChanged), "{e:?}");
    assert_message(&e, MSG_FOCUS_CHANGED);
    assert!(desk.pastes().is_empty(), "{:?}", desk.calls());
    desk.assert_no_key_sent();
}

#[tokio::test]
async fn e07_deliver_to_the_window_already_in_front_pastes_once() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD);
    ok(deliver(
        &desk,
        &NOTEPAD.r(),
        REPLACEMENT,
        &Settings::default(),
        &sleeper,
    )
    .await);
    assert_eq!(
        desk.pastes(),
        vec![Call::Paste {
            text: REPLACEMENT.to_string(),
            restore_delay: RESTORE_DELAY,
            front: NOTEPAD.id,
        }]
    );
}

#[tokio::test]
async fn e07_paste_restore_delay_comes_from_the_settings() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, POPUP).with(NOTEPAD);
    let mut s = Settings::default();
    s.capture.paste_restore_delay_ms = 400;
    ok(deliver(&desk, &NOTEPAD.r(), REPLACEMENT, &s, &sleeper).await);
    assert_eq!(
        desk.pastes(),
        vec![Call::Paste {
            text: REPLACEMENT.to_string(),
            restore_delay: Duration::from_millis(400),
            front: NOTEPAD.id,
        }]
    );
}

// ---------------------------------------------------------------------------------------------
// Bullet 8: `deliver` checks the target again (invariant 4).

#[tokio::test]
async fn t08_deliver_when_the_focused_control_became_a_password_field_is_refused() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, POPUP).with(NOTEPAD.password());
    let e = refused(
        deliver(
            &desk,
            &NOTEPAD.r(),
            REPLACEMENT,
            &Settings::default(),
            &sleeper,
        )
        .await,
    );
    assert!(matches!(e, CaptureError::PasswordField), "{e:?}");
    assert_message(&e, MSG_PASSWORD);
    assert!(desk.pastes().is_empty(), "{:?}", desk.calls());
    desk.assert_no_key_sent();
}

#[tokio::test]
async fn t08_deliver_into_a_password_field_already_in_front_is_refused() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD.password());
    let e = refused(
        deliver(
            &desk,
            &NOTEPAD.r(),
            REPLACEMENT,
            &Settings::default(),
            &sleeper,
        )
        .await,
    );
    assert!(matches!(e, CaptureError::PasswordField), "{e:?}");
    desk.assert_no_key_sent();
}

#[tokio::test]
async fn e08_deliver_runs_every_check_again() {
    // Elevated target.
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, POPUP).with(NOTEPAD.elevated());
    let e = refused(
        deliver(
            &desk,
            &NOTEPAD.r(),
            REPLACEMENT,
            &Settings::default(),
            &sleeper,
        )
        .await,
    );
    assert!(matches!(e, CaptureError::Elevated), "{e:?}");
    desk.assert_no_key_sent();

    // A process added to the blocklist after the capture.
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, POPUP).with(NOTEPAD);
    let mut s = Settings::default();
    s.privacy.blocklist.push("notepad.exe".to_string());
    let e = refused(deliver(&desk, &NOTEPAD.r(), REPLACEMENT, &s, &sleeper).await);
    assert!(
        matches!(&e, CaptureError::Blocklisted { app } if app == "notepad.exe"),
        "{e:?}"
    );
    desk.assert_no_key_sent();
}

// ---------------------------------------------------------------------------------------------
// Bullet 9: Undo (Scope: the last replace, for 2 minutes on the injected clock).

#[tokio::test]
async fn t09_undo_within_two_minutes_with_the_window_in_front_sends_one_ctrl_z() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD);
    let buf = api::undo_buffer();
    api::remember(&buf, NOTEPAD.r(), SELECTION, &*clock);
    clock.advance(Duration::from_secs(60));
    let result = ok(api::undo(&buf, &desk, &*clock, &sleeper).await);
    assert_eq!(result, "undone");
    assert_eq!(desk.undos(), vec![Call::SendUndo { front: NOTEPAD.id }]);
    assert!(desk.clipboard_writes().is_empty());
    assert!(desk.pastes().is_empty());
}

#[tokio::test]
async fn t09_undo_brings_the_window_to_the_front_then_sends_one_ctrl_z() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, POPUP).with(NOTEPAD);
    let buf = api::undo_buffer();
    api::remember(&buf, NOTEPAD.r(), SELECTION, &*clock);
    clock.advance(Duration::from_secs(30));
    let result = ok(api::undo(&buf, &desk, &*clock, &sleeper).await);
    assert_eq!(result, "undone");
    assert_eq!(desk.undos(), vec![Call::SendUndo { front: NOTEPAD.id }]);
    assert!(desk.clipboard_writes().is_empty());
}

#[tokio::test]
async fn t09_undo_with_the_window_gone_puts_the_original_on_the_clipboard() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, POPUP);
    let buf = api::undo_buffer();
    api::remember(&buf, NOTEPAD.r(), SELECTION, &*clock);
    clock.advance(Duration::from_secs(60));
    let result = ok(api::undo(&buf, &desk, &*clock, &sleeper).await);
    assert_eq!(result, "on_clipboard");
    assert!(desk.undos().is_empty(), "{:?}", desk.calls());
    assert_eq!(desk.clipboard_writes(), vec![SELECTION.to_string()]);
    assert_eq!(desk.clipboard(), SELECTION);
}

#[tokio::test]
async fn e09_undo_when_the_window_cannot_come_to_the_front_puts_the_original_on_the_clipboard() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, POPUP).with(NOTEPAD.stuck());
    let buf = api::undo_buffer();
    api::remember(&buf, NOTEPAD.r(), SELECTION, &*clock);
    clock.advance(Duration::from_secs(60));
    let result = ok(api::undo(&buf, &desk, &*clock, &sleeper).await);
    assert_eq!(result, "on_clipboard");
    assert!(desk.undos().is_empty(), "{:?}", desk.calls());
    assert_eq!(desk.clipboard_writes(), vec![SELECTION.to_string()]);
}

#[tokio::test]
async fn t09_undo_after_two_minutes_is_nothing_to_undo() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD);
    let buf = api::undo_buffer();
    api::remember(&buf, NOTEPAD.r(), SELECTION, &*clock);
    clock.advance(Duration::from_secs(121));
    let e = refused(api::undo(&buf, &desk, &*clock, &sleeper).await);
    assert!(matches!(e, CaptureError::NothingToUndo), "{e:?}");
    assert_message(&e, MSG_NOTHING_TO_UNDO);
    assert!(desk.undos().is_empty(), "{:?}", desk.calls());
    assert!(desk.clipboard_writes().is_empty());
}

#[tokio::test]
async fn e09_undo_at_exactly_two_minutes_still_undoes_once() {
    // Design notes: undoable while at most 2 minutes old on the clock (120 000 ms still counts),
    // once.
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD);
    let buf = api::undo_buffer();
    api::remember(&buf, NOTEPAD.r(), SELECTION, &*clock);
    clock.advance(Duration::from_millis(120_000));
    let result = ok(api::undo(&buf, &desk, &*clock, &sleeper).await);
    assert_eq!(result, "undone");
    let again = refused(api::undo(&buf, &desk, &*clock, &sleeper).await);
    assert!(matches!(again, CaptureError::NothingToUndo), "{again:?}");
    assert_eq!(desk.undos(), vec![Call::SendUndo { front: NOTEPAD.id }]);
}

#[tokio::test]
async fn e09_undo_one_millisecond_after_two_minutes_is_nothing_to_undo() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD);
    let buf = api::undo_buffer();
    api::remember(&buf, NOTEPAD.r(), SELECTION, &*clock);
    clock.advance(Duration::from_millis(120_001));
    let e = refused(api::undo(&buf, &desk, &*clock, &sleeper).await);
    assert!(matches!(e, CaptureError::NothingToUndo), "{e:?}");
    desk.assert_no_key_sent();
}

#[tokio::test]
async fn e09_undo_with_nothing_replaced_is_nothing_to_undo() {
    let (clock, sleeper) = time();
    let desk = Desk::new(&clock, NOTEPAD);
    let buf = api::undo_buffer();
    let e = refused(api::undo(&buf, &desk, &*clock, &sleeper).await);
    assert!(matches!(e, CaptureError::NothingToUndo), "{e:?}");
    assert_message(&e, MSG_NOTHING_TO_UNDO);
    desk.assert_no_key_sent();
}

// ---------------------------------------------------------------------------------------------
// The other checks of the Scope, and every message.

#[test]
fn e_check_allowed_refuses_every_default_blocklist_name() {
    let list = blocklist(&ARCH_BLOCKLIST);
    for (i, name) in (0u32..).zip(ARCH_BLOCKLIST) {
        let w = win(0x4000 + u64::from(i), 5000 + i, name);
        let e = refused(check_allowed(&w.fg(), &list));
        assert!(
            matches!(&e, CaptureError::Blocklisted { app } if app == name),
            "{name}: {e:?}"
        );
    }
}

#[test]
fn e_check_allowed_lets_an_ordinary_field_through() {
    ok(check_allowed(&NOTEPAD.fg(), &blocklist(&ARCH_BLOCKLIST)));
    ok(check_allowed(
        &NOTEPAD.fg(),
        &Settings::default().privacy.blocklist,
    ));
}

#[test]
fn e_check_allowed_follows_the_blocklist_it_is_given() {
    // A password manager the user took off the list is no longer refused for its name.
    ok(check_allowed(&VAULT.fg(), &[]));
    let e = refused(check_allowed(&NOTEPAD.fg(), &blocklist(&["notepad.exe"])));
    assert!(matches!(e, CaptureError::Blocklisted { .. }), "{e:?}");
}

#[test]
fn e_secure_input_is_refused() {
    // Invariant 3: secure input is refused like a password field.
    let e = refused(check_allowed(&NOTEPAD.secure().fg(), &[]));
    assert!(matches!(e, CaptureError::SecureInput), "{e:?}");
    assert_message(&e, MSG_SECURE_INPUT);
}

#[test]
fn e_secure_input_is_checked_last() {
    // Design notes: `check_allowed` checks secure input last, after the password field.
    let list = blocklist(&ARCH_BLOCKLIST);
    let blocked = refused(check_allowed(&VAULT.secure().fg(), &list));
    assert!(
        matches!(blocked, CaptureError::Blocklisted { .. }),
        "{blocked:?}"
    );
    let elevated = refused(check_allowed(&NOTEPAD.elevated().secure().fg(), &list));
    assert!(matches!(elevated, CaptureError::Elevated), "{elevated:?}");
    let password = refused(check_allowed(&NOTEPAD.password().secure().fg(), &list));
    assert!(
        matches!(password, CaptureError::PasswordField),
        "{password:?}"
    );
}

#[test]
fn e_messages_of_every_capture_error() {
    let cases: Vec<(CaptureError, &str)> = vec![
        (CaptureError::PasswordField, MSG_PASSWORD),
        (CaptureError::SecureInput, MSG_SECURE_INPUT),
        (
            CaptureError::Blocklisted {
                app: "1password.exe".to_string(),
            },
            MSG_BLOCKLISTED,
        ),
        (CaptureError::Elevated, MSG_ELEVATED),
        (CaptureError::NothingCopied, MSG_NOTHING_COPIED),
        (CaptureError::KeysHeld, MSG_KEYS_HELD),
        (CaptureError::FocusChanged, MSG_FOCUS_CHANGED),
        (CaptureError::WindowGone, MSG_WINDOW_GONE),
        (CaptureError::ClipboardBusy, MSG_CLIPBOARD_BUSY),
        (CaptureError::NothingToUndo, MSG_NOTHING_TO_UNDO),
        (api::os_error(5), MSG_OS_5),
    ];
    for (e, message) in &cases {
        assert_message(e, message);
    }
    // The blocklist message never names the app (docs/ARCHITECTURE.md §8: no process names).
    let e = CaptureError::Blocklisted {
        app: "keepassxc.exe".to_string(),
    };
    assert_message(&e, MSG_BLOCKLISTED);
}

#[test]
fn e_fake_capture_implements_capture() {
    fn is_capture<T: Capture + Send + Sync>() {}
    is_capture::<FakeCapture>();
}
