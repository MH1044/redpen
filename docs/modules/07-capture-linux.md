# 07: Capture on Linux

## Goal

Checking and rewriting work on Linux X11 with the same privacy rules as on Windows, and on Wayland
as far as the desktop allows, with the limits stated in the app rather than hidden. Redpen does not
ship a macOS build (docs/PRODUCT.md §2).

## Depends on

05, 06.

## Scope

- **Linux X11 `Capture`**: foreground window by `_NET_ACTIVE_WINDOW`, process by `_NET_WM_PID` and
  `/proc/<pid>/comm`; Ctrl+C and Ctrl+V through XTest (`x11rb`); CLIPBOARD save and restore of text,
  HTML and images through `arboard`, with Redpen keeping ownership until the restored contents are
  served; password fields refused through AT-SPI (role `password text`) when the accessibility bus
  is reachable; otherwise Settings → Privacy says "On Linux, Redpen can't always detect password
  fields. Keep your password manager in the blocklist."
- **Linux Wayland** (detected from `XDG_SESSION_TYPE` or `WAYLAND_DISPLAY`): global shortcuts
  through the desktop portal's GlobalShortcuts interface where offered; no synthetic input. The check
  hotkey works on the clipboard: "On Wayland, copy the text first (Ctrl+C), then press the
  shortcut." Results go to the clipboard with "On Wayland, Redpen can't type into other apps. The
  text is on your clipboard: press Ctrl+V."
- The default Linux blocklist (`1password`, `bitwarden`, `keepassxc`, `seahorse`), verified here
  against each app's process name.
- The manual checklists for X11 and one Wayland desktop (docs/TESTING.md §8). CI already runs
  `verify` on `ubuntu-latest` from module 00.

## Files

- `crates/redpen-os/src/{linux/mod.rs, linux/x11.rs, linux/atspi.rs, linux/wayland.rs}`
- `crates/redpen-core/src/store/settings.rs` (defaults per platform)
- `docs/TESTING.md` §8 (the two new lists)
- `THIRD_PARTY.md` (`x11rb`, `arboard`, the AT-SPI and portal crates chosen, each licence verified)
- Tests: `crates/redpen-core/tests/07_platforms.rs` (fakes), `crates/redpen-os/tests/07_linux.rs`

## Interfaces

The `Capture` trait is unchanged (module 05). Added:

```rust
pub enum Platform { Windows, LinuxX11, LinuxWayland }
pub fn detect_platform(env: &dyn Env) -> Platform;
pub fn default_hotkeys(p: Platform) -> Hotkeys;
pub trait Capture { /* … */ fn can_type(&self) -> bool; }          // false on Wayland
```

Messages: the two Wayland messages and the Linux password note above.

## Out of scope

- A macOS build: GSpot already covers macOS (docs/PRODUCT.md §2).
- Installers: module 10.
- Wayland synthetic input through the RemoteDesktop portal: later.

## Design notes

- Decided: where Redpen cannot check for a secure field it does not type; on Wayland it never
  types at all, so the only text it reads is text the user copied.
- Further decisions are filled in here during implementation.

## Acceptance tests

Fake tests in `crates/redpen-core/tests/07_platforms.rs`; real-API tests in
`crates/redpen-os/tests/07_linux.rs` (`ubuntu-latest` under `xvfb-run`); the rest in the manual
checklists.

1. X11 real (`xvfb-run`): CLIPBOARD text and HTML are identical after a save and restore cycle.
2. X11 fake: `_NET_WM_PID` 4242 with `/proc/4242/comm` `keepassxc` → the process is `keepassxc`,
   capture is refused with `Blocklisted` (the default list) and no Ctrl+C is sent.
3. X11 fake: the AT-SPI role of the focused element is `password text` → `PasswordField`; no Ctrl+C
   sent.
4. X11 fake: the focused element turns into a password field between capture and Accept → nothing
   pasted, Copy offered.
5. Wayland fake (`XDG_SESSION_TYPE=wayland`): `can_type()` is false; Accept puts the result on the
   clipboard and shows the Wayland message; no synthetic-input call.
6. Linux fake with no AT-SPI bus → the password note is shown in Settings → Privacy.
7. `default_hotkeys(LinuxX11)` → Ctrl+Alt+R, as on Windows; `detect_platform` with
   `WAYLAND_DISPLAY` set → `LinuxWayland`.
8. `verify` passes on `ubuntu-latest` with the Linux capture compiled in.

## Status

Planned
