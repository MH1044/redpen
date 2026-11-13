# Module 05 references

Where the expected values of `crates/redpen-core/tests/05_capture.rs` (bullets 1 to 9, with
fakes) and `crates/redpen-os/tests/05_capture_windows.rs` (bullets 10 to 17, on Windows) come
from. None of them was taken from Redpen's own output. Module 05 has no command-line command, so
this folder holds no golden case.

## Messages

docs/ARCHITECTURE.md §8 and the spec's Messages, compared with each error's `Display`:

| Error | Message |
|---|---|
| `PasswordField` | Redpen doesn't read or type into password fields. |
| `Blocklisted {app}` | Redpen is turned off for this app (Settings → Privacy). |
| `Elevated` | This window runs as administrator, so Windows doesn't let Redpen read or type into it. |
| `NothingCopied` | Nothing was copied. Select some text first. |
| `KeysHeld` | Release the shortcut keys, then try again. |
| `FocusChanged` | The window you started in is no longer in front, so nothing was typed. Copy the text instead. |
| `WindowGone` | The window you started in has closed. Copy the text instead. |
| `ClipboardBusy` | Another app is using the clipboard. Try again. |
| `NothingToUndo` | There is nothing to undo. |
| `Os("5")` | Windows refused the request (code 5). |
| `SecureInput` | Another app is protecting what you type right now, so Redpen can't read or type. |

`Blocklisted` carries the process name in `app`; the message never shows it (§8: logs and messages
name no process). `SecureInput` is refused (invariant 3); its message and its place in the order
(last, after the password field) are the spec's Design notes, decided during implementation.

## Order of the checks

The Scope's order for `check_allowed`: blocklisted process, then elevated window, then password
field; the Design notes put secure input last. docs/ARCHITECTURE.md §3 puts `foreground` and the refusal checks before the modifier keys and
`read_selection`, so a refused capture sends no key and never waits for the keys. Invariants 3 and
4 (§7): no synthetic copy before the checks, and the same checks again before a paste.

## Timings

- **Key wait** (Design notes): `modifiers_down` is checked once, then polled every 10 ms through the
  `Sleeper`, at most 100 times. The fake sleeper moves the fake clock by each wait, and the fake
  reports the keys down while the clock is before the release time:
  - held 1.2 s: down at the checks at 0, 10, …, 1000 ms (101 checks), so 100 waits of 10 ms
    (1 s), then `KeysHeld`;
  - held 300 ms: down at 0, 10, …, 290 ms (30 checks), up at the check at 300 ms, so 30 waits,
    then the copy;
  - held 1 s: up at the 100th poll, at 1000 ms, so 100 waits, then the copy.
- **Copy timeout** 400 ms and **restore delay** 250 ms: `capture.copy_timeout_ms` and
  `capture.paste_restore_delay_ms` of docs/ARCHITECTURE.md §4. The fakes check that the values reach
  `read_selection` and `paste`, and that changed settings change them.
- **Focus wait** 50 ms: the Scope's `paste` bullet, after bringing the target to the front.
- **Undo** lasts 2 minutes (120 000 ms) on the injected clock. The tests undo at 30 s and 60 s
  (inside) and at 121 s (outside), and at the boundary the Design notes set: at exactly
  120 000 ms a replace still counts (once: a second Undo is `NothingToUndo`), at 120 001 ms it
  does not.
- **Clipboard busy**: `OpenClipboard` 5 attempts, 20 ms apart, so at least 4 × 20 = 80 ms.
  Windows' default timer ticks every 15.6 ms and may end each wait early by part of a tick, so the
  test accepts 60 ms or more.
- **Nothing copied on Windows**: 400 ms polled every 10 ms; the test accepts 350 ms to 3 s for the
  same reason.
- **p95** (bullet 16): nearest rank over 20 runs, the 19th of the sorted times
  (ceil(0.95 × 20) = 19), at most 150 ms.
- The fake clock starts at 1 790 000 000 000 ms, 2026-09-21T14:13:20Z (`date -u`), as in module 02.

## Default blocklist

docs/ARCHITECTURE.md §4: `1password.exe`, `bitwarden.exe`, `keepass.exe`, `keepassxc.exe`,
`dashlane.exe`, `enpass.exe`, `nordpass.exe`, `proton pass.exe`. Process names are lower case
(Scope: `foreground()` lower-cases the executable name).

## Clipboard fixtures (Windows)

The test writes and reads the clipboard through the Win32 functions itself (`OpenClipboard`,
`SetClipboardData`, `GetClipboardData`, `GlobalAlloc`), never through Redpen, so each expected
value is true by construction.

- Standard formats (Windows SDK, `WinUser.h`): `CF_DIB` = 8, `CF_UNICODETEXT` = 13.
- Registered formats, by name: `HTML Format` (Microsoft, "HTML Clipboard Format"), and
  `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` and
  `CanUploadToCloudClipboard` (Microsoft, "Clipboard Formats", cloud clipboard and clipboard
  history formats). The last two hold a DWORD; 0 keeps the entry out of history and off the cloud
  clipboard. The first is checked for presence only.
- `keep me` as `CF_UNICODETEXT`: UTF-16LE with its terminator, 16 bytes.
- The `HTML Format` entry: a `Version:0.9` header with four 10-digit byte offsets (105 bytes),
  then `<html><body>`, CR LF, `<!--StartFragment-->`, the fragment `<b>keep me</b>`,
  `<!--EndFragment-->`, CR LF, `</body></html>` and a NUL. StartHTML 105, StartFragment 139,
  EndFragment 153, EndHTML 187; 188 bytes in all. The test computes the offsets the same way.
- The 2×2 DIB: a 40-byte BITMAPINFOHEADER (32 bits per pixel, `BI_RGB`, `biSizeImage` 16) and
  four pixels in blue-green-red-unused order (red, green, blue, white): 56 bytes.
- "The same bytes per format" (bullets 10 and 11): each format is read back after the test puts
  it and again after the capture or the replace; the two reads must be equal.
- The 70 MB image (bullet 17): a DIB of 4096 × 4480 pixels at 4 bytes each, 73 400 320 bytes of
  pixels (70 × 1 048 576, 70 MiB), 73 400 360 bytes with its header, generated inside the test
  (docs/TESTING.md §2). It is above the 64 MB limit whether a megabyte is 1 000 000 or 1 048 576
  bytes. The log line must carry a size of at least 73 400 320 (docs/ARCHITECTURE.md §8: sizes in
  bytes) and no line may carry `keep me`, the selection, `hello` or the helper's name.
- The 300-word selection (bullet 16): `The quick brown fox jumps over the lazy dog today.` (10
  words, 50 characters) 30 times, joined by spaces: 300 words, 1 529 characters.

## The helper window

`redpen-testwin` (Scope) shows one window with a normal EDIT and an `ES_PASSWORD` EDIT. The tests
use this line protocol on its standard input and output:

- it prints `ready` once its window is shown;
- then it reads one command per line and answers each with one line:
  - `focus normal` or `focus password`: brings its window to the front with that EDIT focused;
    answers `ok`;
  - `set <text>`: the rest of the line becomes the focused EDIT's text; answers `ok`;
  - `select <start> <end>`: selects that range of the focused EDIT, in UTF-16 offsets; answers
    `ok`;
  - `get`: answers the focused EDIT's text;
  - a command it cannot carry out answers `error: <reason>`;
- it exits at the end of its input.

The test reads the EDITs' screen rectangles and the window in front itself (`EnumChildWindows`,
`GetWindowRect`, `GetForegroundWindow`), and expects `foreground()` to name `redpen-testwin.exe`,
the helper's process id, and its window handle as the window id.

## Desktop tests

The Windows tests type real keys and replace the clipboard, so they run only with
`REDPEN_DESKTOP_TESTS=1`. Without it each prints "Set REDPEN_DESKTOP_TESTS=1 to run the desktop
tests; they type into the window in front and replace the clipboard." and passes (Design notes).
They hold one lock, so two never run at once.
