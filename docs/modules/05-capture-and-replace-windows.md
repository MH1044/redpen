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
- Further decisions are filled in here during implementation.

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

Planned
