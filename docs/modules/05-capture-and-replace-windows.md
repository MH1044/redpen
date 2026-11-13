# 05: Capture and replace on Windows

## Goal

On Windows, Redpen can read the selection in the app in front and later type text back into that
window, through the clipboard route, leaving the user's clipboard as it was. It refuses, before any
synthetic key is sent, password fields, blocklisted apps and elevated windows, and checks again
before typing. This is the privacy boundary for rewrites (06).

## Depends on

00.

## Scope

- `Capture` trait and `FakeCapture` in `redpen-core::platform`; the Windows implementation in
  `redpen-os`.
- `foreground()`: window handle and process id (`GetForegroundWindow`,
  `GetWindowThreadProcessId`), lower-case executable name (`QueryFullProcessImageNameW`), password
  flag (UI Automation focused element, `CurrentIsPassword`), elevation (target token integrity level
  above Redpen's; access denied counts as elevated), caret rectangle (`GetGUIThreadInfo`) and
  cursor position.
- `check_allowed`, in this order: blocklisted process, elevated window, password field. The
  default blocklist is in docs/ARCHITECTURE.md §4 (process names verified on a real machine here).
- Waiting for Ctrl, Alt, Shift and Windows keys to be up (`GetAsyncKeyState`, every 10 ms, at most
  1 s) before any synthetic input.
- Clipboard save and restore: every format whose data is an `HGLOBAL`; skipped and named in the
  HANDBOOK: `CF_BITMAP` (Windows synthesises it from the DIB), `CF_ENHMETAFILE`,
  `CF_METAFILEPICT`, `CF_PALETTE`, `CF_OWNERDISPLAY` and delayed-render formats; above 64 MB in
  total only `CF_UNICODETEXT` is kept. `OpenClipboard` is retried 5 times, 20 ms apart.
- `read_selection`: record `GetClipboardSequenceNumber`, send Ctrl+C with `SendInput`, poll every
  10 ms until the number changes or `copy_timeout_ms` (400) passes, read `CF_UNICODETEXT`, restore.
- `paste(target, text)`: bring `target` to the front if needed (Redpen's popup is in front, so
  `SetForegroundWindow` is allowed), wait 50 ms, confirm it is in front, run `check_allowed` on it
  again, save the clipboard, set the text with the formats `ExcludeClipboardContentFromMonitorProcessing`,
  `CanIncludeInClipboardHistory` (0) and `CanUploadToCloudClipboard` (0), send Ctrl+V, wait
  `paste_restore_delay_ms` (250), restore.
- `set_clipboard_text` for Copy (a normal clipboard entry, since the user asked for it).
- Undo buffer in memory: the last replace (window, original text), for 2 minutes. Undo sends one
  Ctrl+Z to that window when it is in front or can be brought there; otherwise the original goes on
  the clipboard.
- `flow::rewrite::capture_session` and `flow::deliver` (the replace path Accept uses) over the
  trait.
- `redpen-testwin` (Windows test helper): a window with a normal EDIT and an `ES_PASSWORD` EDIT,
  driven over stdin (`focus normal|password`, `set <text>`, `select <start> <end>`, `get`).

## Files

- `crates/redpen-core/src/platform/{mod.rs, capture.rs, fake.rs}`, `crates/redpen-core/src/flow/{mod.rs, rewrite.rs, deliver.rs}`
- `crates/redpen-os/src/{lib.rs, windows/mod.rs, windows/foreground.rs, windows/uia.rs,
  windows/clipboard.rs, windows/input.rs}`, `crates/redpen-os/src/bin/redpen-testwin.rs`
- `.github/workflows/ci.yml` (the `windows-integration` job)
- `THIRD_PARTY.md` (`windows`)
- Tests: `crates/redpen-core/tests/05_capture.rs` (fakes), `crates/redpen-os/tests/05_capture_windows.rs`

## Interfaces

```rust
pub struct WindowRef { pub id: u64, pub pid: u32 }
pub struct Foreground { pub window: WindowRef, pub process: String, pub is_password: bool,
    pub secure_input: bool, pub elevated: bool, pub caret: Option<Rect>, pub cursor: Point }
pub trait Capture: Send + Sync {
    fn foreground(&self) -> Result<Foreground, CaptureError>;
    fn inspect(&self, w: &WindowRef) -> Result<Option<Foreground>, CaptureError>; // None: window gone
    fn modifiers_down(&self) -> bool;
    fn read_selection(&self, timeout: Duration) -> Result<String, CaptureError>;
    fn focus(&self, w: &WindowRef) -> Result<bool, CaptureError>;               // true when in front
    fn paste(&self, text: &str, restore_delay: Duration) -> Result<(), CaptureError>;
    fn send_undo(&self) -> Result<(), CaptureError>;
    fn set_clipboard_text(&self, text: &str) -> Result<(), CaptureError>;
}
pub fn check_allowed(fg: &Foreground, blocklist: &[String]) -> Result<(), CaptureError>;
pub async fn capture_session(cap: &dyn Capture, s: &Settings, clock: &dyn Clock,
                             sleeper: &dyn Sleeper) -> Result<Captured, CaptureError>;
pub async fn deliver(cap: &dyn Capture, target: &WindowRef, text: &str, s: &Settings,
                     sleeper: &dyn Sleeper) -> Result<(), CaptureError>;
pub enum CaptureError { PasswordField, SecureInput, Blocklisted { app: String }, Elevated,
    NothingCopied, KeysHeld, FocusChanged, WindowGone, ClipboardBusy, NothingToUndo, Os(String) }
```

Messages: those of docs/ARCHITECTURE.md §8, plus `ClipboardBusy` "Another app is using the
clipboard. Try again." and `NothingToUndo` "There is nothing to undo." `Os` shows "Windows refused
the request (code 5)." with the numeric code only.

## Out of scope

- The hotkey and popup: module 06.
- UIA TextPattern capture without the clipboard: later (ADR-0003).
- Linux: module 07 (there is no macOS build).

## Design notes

- Decided: refusals happen before any synthetic key; `deliver` re-checks the target because focus
  can move while a rewrite runs (docs/ARCHITECTURE.md §7, invariants 3 to 5).
- The HANDBOOK records ADR-0003's limits: elevated windows, apps that ignore synthetic input,
  terminals where Ctrl+C interrupts (Windows Terminal copies a selection with Ctrl+C; the classic
  console does not).
- Chromium-based apps build their accessibility tree when a UI Automation client first asks; the
  first query may lack the password flag. This is measured during implementation, with a record
  of whether a second query is needed.
- Decided before implementation:
  - **Dependency**: `windows` 0.62 (MIT OR Apache-2.0), a Windows-only dependency of `redpen-os`
    with the features `Win32_Foundation`, `Win32_UI_WindowsAndMessaging`,
    `Win32_UI_Input_KeyboardAndMouse`, `Win32_UI_Accessibility`, `Win32_System_DataExchange`,
    `Win32_System_Memory`, `Win32_System_Threading`, `Win32_Security`, `Win32_System_Com`,
    `Win32_Graphics_Gdi` and `Win32_System_Ole` (checked against `deny.toml` and built). On Linux
    `redpen-os` stays empty until module 07.
  - **The Windows tests type real keys and replace the clipboard**, so they run only with
    `REDPEN_DESKTOP_TESTS=1`: in CI's `windows-integration` job on `windows-latest`, and on a
    developer's computer only on purpose, with nothing else open, because a synthetic Ctrl+V lands
    in whatever window is in front. Without the variable each one skips and says "Set
    REDPEN_DESKTOP_TESTS=1 to run the desktop tests; they type into the window in front and replace
    the clipboard." `verify` never sets it.
  - **The key wait** polls `modifiers_down` through the `Sleeper` every 10 ms, at most 100 times
    after the first check (1 s). It counts polls and never reads the clock, so a fake sleeper alone
    drives it: test 5's 1.2 s and 300 ms are keys held for 120 and 30 polls.
  - **The undo buffer** times its 2 minutes with the injected `Clock`.
  - **The default blocklist's process names** come from each vendor's documentation, since no
    password manager is installed where Redpen is built; checking each on a computer that has it
    installed joins the manual checklist of docs/TESTING.md §8.
  - **Fakes**: `FakeCapture` is scripted call by call and records every call in order; tests may
    also implement `Capture` themselves.
- Decided during implementation:
  - **Chromium needs a second query (measured).** On a fresh Edge 157 process (three runs) and
    Chrome 156 process (one run) showing a page whose password field had the focus, the first UI
    Automation query answered with the browser's frame (class `View` in Edge,
    `Chrome_WidgetWin_1` in Chrome) and no password flag every time; a second query straight
    after answered with the password field every time. The first answer's class differs between
    browsers, so the question is asked a second time whenever the first answer is not a password
    field, on the same client and thread (about 4 ms; the first query of a process costs about
    50 ms while UI Automation loads, later ones 3 to 5 ms). A classic edit box with `ES_PASSWORD`
    counts as a password field without asking.
  - **UI Automation runs on a thread of its own** in the multithreaded apartment for every
    question, so it never waits on Redpen's own windows and never changes the caller's COM state,
    with connection and call timeouts of 1 s. When it cannot answer, the capture stops with `Os`
    and its code rather than guess. Only the window in front has a password flag: `inspect` of a
    window behind others reports false, which is why `deliver` checks again once the target is in
    front. An elevated window is not asked (it is refused first, and Windows would not let Redpen
    read it anyway).
  - **Elevation** compares the last sub-authority of the token's integrity label with Redpen's
    own. When Windows refuses to open the process, its name is empty and it counts as elevated;
    when it refuses to open the token (the usual case for a window running as administrator), it
    counts as elevated and keeps the name Windows gave, so the blocklist is still checked first.
  - **Secure input** is always false on Windows: ordinary windows have no such mode, and the secure
    desktop of an administrator prompt is out of reach of Redpen's shortcut. Its message, for the
    platforms that have it: "Another app is protecting what you type right now, so Redpen can't
    read or type." `check_allowed` checks it last, after the password field.
  - **The blocklist** matches whole process names, ignoring case and surrounding spaces.
  - **Types the spec leaves open**: `Rect {x, y, width, height}` and `Point {x, y}` in physical
    screen pixels; `Captured {window, text, caret, cursor, captured_at_ms}`. `CaptureError`
    serialises as `{kind, message}` plus `app` for `Blocklisted` and `code` for `Os`; the code is a
    Win32 error in decimal (`5`) or any other `HRESULT` in hexadecimal (`0x80040201`).
  - **`capture_session`** checks the window in front again after the key wait when it had to wait
    (a held Alt can switch windows), and treats an empty copy as `NothingCopied`.
  - **`deliver`**, in order: the target must exist (`WindowGone`) and pass the checks as far as
    they are known while it is behind the popup; it is brought to the front if it is not there
    (`FocusChanged` when it will not come); 50 ms; the key wait, since Ctrl+Enter accepts; the
    target must be the window in front (`FocusChanged`); the checks again; `paste`. A window is
    its handle and its process id together, so a handle Windows reuses is not the same window.
  - **Undo** (`UndoBuffer::remember`, `UndoBuffer::undo`): a replace stays undoable while at most
    2 minutes old on the clock (120 000 ms still counts), once. It takes the same steps as Accept
    before sending Ctrl+Z; when the window has closed, will not come to the front, or now has a
    password field (or secure input, or runs elevated), the original goes on the clipboard as an
    ordinary entry and the result is `on_clipboard`. The blocklist is not asked again: the replace
    passed it, and the same handle and process id are the same program. Keys still held, or an
    `Os` error, leave the replace in the buffer so Undo can be tried again.
  - **What the clipboard save leaves out** besides the five formats of the Scope: their `CF_DSP…`
    twins, the private range (`0x0200`–`0x02FF`) and the drawing-object range
    (`0x0300`–`0x03FF`), whose data is not memory Redpen can copy. 64 MB is 67 108 864 bytes.
    Formats are asked for in the order Windows lists them; once those asked for are over the
    limit, only `CF_UNICODETEXT` is still asked for (asking for a format Windows makes from
    another, `CF_DIBV5` from a DIB, makes it convert the data). One log line
    `clipboard_save_over_limit` then carries `skipped_bytes`, the size of what was left out, and
    nothing else.
  - **Delayed-render formats are rendered, not skipped.** Windows has no documented way to tell a
    delayed-render format from another before asking for its data, and asking makes its app
    render it. So Redpen keeps what the app renders and leaves out a format whose app does not
    (one that has closed, for example). The HANDBOOK says so.
  - **Restoring only what changed.** `read_selection` and `paste` put the saved contents back only
    when the clipboard's sequence number moved; a copy that never happened leaves the clipboard
    untouched, so clipboard history gains no second copy of it. The three exclusion formats carry
    the `DWORD` 0 and go on before the text; Copy's entry carries none of them.
  - **Writes are owned by a message-only window** made for the write and destroyed after it, so
    no later clipboard message ever waits on Redpen; the data stays on the clipboard.
  - **After the sequence number moves**, `read_selection` keeps polling until `CF_UNICODETEXT` is
    there, within the same timeout, because an app may empty the clipboard and fill it in two
    steps. Text comes off the clipboard with CR LF turned into LF, and goes on with LF turned into
    CR LF, as `CF_UNICODETEXT` expects.
  - **Keys** are sent with their virtual-key and scan codes; a partial `SendInput` releases Ctrl
    and the key. `focus` restores a minimised window before `SetForegroundWindow`.
  - **The app's own copy is an ordinary clipboard entry.** The Ctrl+C makes the app copy the
    selection as it would for the user, so Windows clipboard history (Win+V), when it is on, lists
    the selection; Redpen restores the clipboard afterwards but cannot mark an entry another app
    made. The HANDBOOK says so.
  - **`redpen-testwin`** prints `ready` once its window is shown and answers every line with one
    line (`ok`, `error: …`, or the value). Besides the Scope's commands it takes `get normal|password`,
    `rect [normal|password]`, `hwnd` and `quit`, and `focus` joins the input of the window in front
    for a moment so a background process can take the front. On Linux it only says it runs on
    Windows.
  - **CI**: the `windows-integration` job runs `cargo test -p redpen-os --test 05_capture_windows
    -- --test-threads=1` with `REDPEN_DESKTOP_TESTS=1`, then the launch smoke.

## Acceptance tests

Fake tests in `crates/redpen-core/tests/05_capture.rs` (a `FakeCapture` that records every call);
Windows tests in `crates/redpen-os/tests/05_capture_windows.rs` with `redpen-testwin`, run by CI
`windows-integration` (or locally with `REDPEN_DESKTOP_TESTS=1`, docs/TESTING.md §6).

1. Foreground `1password.exe` → `Blocklisted` with its message; the fake saw no `read_selection`.
2. Password field → `PasswordField`; no `read_selection`.
3. Elevated → `Elevated`; no `read_selection`.
4. Blocklisted, elevated and password all true → `Blocklisted` (the order).
5. Modifiers down for 1.2 s (fake clock) → `KeysHeld`; down for 300 ms → capture proceeds.
6. The fake's copy never changes the sequence → `NothingCopied` after 400 ms, the clipboard
   restored.
7. `deliver` to a window that exists and can be focused → one `paste`; to a closed window →
   `WindowGone`; to one that cannot be brought to the front → `FocusChanged`; no `paste` in either.
8. `deliver` when the target's focused control became a password field → `PasswordField`, no
   `paste`.
9. Undo within 2 minutes with the window in front → one `send_undo`; with the window gone → the
   original on the clipboard and result `on_clipboard`; after 2 minutes → `NothingToUndo`.
10. Windows: EDIT text `hello wrold`, selection 6 to 11 → `read_selection` returns `wrold`; the
    clipboard held text `keep me`, an HTML Format fragment and a 2×2 DIB before, and holds the same
    bytes per format after.
11. Windows: `deliver` of `world` to that selection → the EDIT reads `hello world`; the clipboard is
    restored as in 10.
12. Windows: the password EDIT focused → `PasswordField`, the EDIT unchanged, and the clipboard
    sequence number unchanged (no Ctrl+C was sent).
13. Windows: during `paste`, a clipboard listener in the test sees the three exclusion formats on
    Redpen's temporary entry.
14. Windows: the test holds the clipboard open → `ClipboardBusy` with its message after 5 attempts.
15. Windows: `foreground()` names `redpen-testwin.exe` and returns a caret rectangle inside the EDIT.
16. Windows: capturing a 300-word selection takes at most 150 ms at p95 over 20 runs.
17. Windows: a 70 MB clipboard image is not saved; after `read_selection` the clipboard holds the
    earlier text, and the log line records the skipped size only.

## Status

Done (2026-11-13)
