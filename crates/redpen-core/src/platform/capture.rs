//! The capture boundary (docs/modules/05-capture-and-replace-windows.md): what the window in front
//! is, the refusals that run before any synthetic key (invariants 3 and 4), and the errors a user
//! sees when capture or Accept stops.
//!
//! [`Capture`] is implemented per operating system in `redpen-os` and by
//! [`FakeCapture`](super::FakeCapture) in tests. The flows in [`crate::flow`] drive it and never
//! touch the operating system themselves (invariant 10).

use std::time::Duration;

use serde::Serialize;

/// A top-level window: its handle (an `HWND` on Windows) and the id of the process that owns it.
/// The two together name one window, so a handle Windows reuses for a new window in another
/// process does not count as the window Redpen captured from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
pub struct WindowRef {
    pub id: u64,
    pub pid: u32,
}

/// A rectangle on the screen, in physical pixels: the top-left corner and the size.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// A point on the screen, in physical pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// The window in front, and what the refusals need to know about it.
///
/// `process` is the lower-case executable name (`notepad.exe`); it is compared with the
/// blocklist and is never written to a log (docs/ARCHITECTURE.md §8). `caret` is where the text
/// cursor is, when the app shows a system caret; the popup falls back to `cursor`, the mouse.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Foreground {
    pub window: WindowRef,
    pub process: String,
    /// The focused control is a password field.
    pub is_password: bool,
    /// The system is protecting keyboard input (a password prompt holds the keyboard). Windows
    /// has no such mode for ordinary windows, so it is always false there.
    pub secure_input: bool,
    /// The window's process runs at a higher integrity level than Redpen (as administrator), or
    /// Windows refused to say.
    pub elevated: bool,
    pub caret: Option<Rect>,
    pub cursor: Point,
}

/// Reading the selection in the app in front and typing text back, through the clipboard
/// (ADR-0003). Every method is a single step; the order, the refusals and the waits between them
/// belong to the flows ([`crate::flow::rewrite::capture_session`], [`crate::flow::deliver`]).
pub trait Capture: Send + Sync {
    /// The window in front now.
    fn foreground(&self) -> Result<Foreground, CaptureError>;

    /// The same facts about a window that may not be in front; `None` when the window is gone.
    /// The password flag is only known for the window in front: for any other it is false, so
    /// the flows check again once the window is in front.
    fn inspect(&self, w: &WindowRef) -> Result<Option<Foreground>, CaptureError>;

    /// Whether Ctrl, Alt, Shift or a Windows key is down now.
    fn modifiers_down(&self) -> bool;

    /// Copies the selection of the window in front (Ctrl+C) and returns it, leaving the
    /// clipboard as it was. [`CaptureError::NothingCopied`] when the clipboard did not change
    /// within `timeout`, or holds no text.
    fn read_selection(&self, timeout: Duration) -> Result<String, CaptureError>;

    /// Brings `w` to the front; true when it is in front.
    fn focus(&self, w: &WindowRef) -> Result<bool, CaptureError>;

    /// Types `text` into the window in front through the clipboard (Ctrl+V), the temporary entry
    /// marked to stay out of clipboard history and cloud sync, then after `restore_delay` puts
    /// back what the clipboard held.
    fn paste(&self, text: &str, restore_delay: Duration) -> Result<(), CaptureError>;

    /// Sends one Ctrl+Z to the window in front.
    fn send_undo(&self) -> Result<(), CaptureError>;

    /// Puts `text` on the clipboard as an ordinary entry (Copy, which the user asked for).
    fn set_clipboard_text(&self, text: &str) -> Result<(), CaptureError>;
}

/// Why capture, Accept, Copy or Undo stopped. `Display` is the message the interface shows; the
/// interface receives it as `{"kind", "message"}` plus the variant's fields (`app`, `code`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    #[error("Redpen doesn't read or type into password fields.")]
    PasswordField,
    #[error("Another app is protecting what you type right now, so Redpen can't read or type.")]
    SecureInput,
    /// `app` is the process name on the blocklist. It is shown nowhere in the message and never
    /// logged.
    #[error("Redpen is turned off for this app (Settings → Privacy).")]
    Blocklisted { app: String },
    #[error(
        "This window runs as administrator, so Windows doesn't let Redpen read or type into it."
    )]
    Elevated,
    #[error("Nothing was copied. Select some text first.")]
    NothingCopied,
    #[error("Release the shortcut keys, then try again.")]
    KeysHeld,
    #[error(
        "The window you started in is no longer in front, so nothing was typed. Copy the text instead."
    )]
    FocusChanged,
    #[error("The window you started in has closed. Copy the text instead.")]
    WindowGone,
    #[error("Another app is using the clipboard. Try again.")]
    ClipboardBusy,
    #[error("There is nothing to undo.")]
    NothingToUndo,
    /// The operating system refused a call. The field is the numeric code only (a Win32 error
    /// in decimal, or an `HRESULT` in hexadecimal), never the system's own text.
    #[error("Windows refused the request (code {0}).")]
    Os(String),
}

impl CaptureError {
    /// An [`CaptureError::Os`] for a numeric code.
    pub fn os(code: impl std::fmt::Display) -> Self {
        CaptureError::Os(code.to_string())
    }

    /// The variant's name, as the interface and the logs report it.
    pub fn kind(&self) -> &'static str {
        match self {
            CaptureError::PasswordField => "PasswordField",
            CaptureError::SecureInput => "SecureInput",
            CaptureError::Blocklisted { .. } => "Blocklisted",
            CaptureError::Elevated => "Elevated",
            CaptureError::NothingCopied => "NothingCopied",
            CaptureError::KeysHeld => "KeysHeld",
            CaptureError::FocusChanged => "FocusChanged",
            CaptureError::WindowGone => "WindowGone",
            CaptureError::ClipboardBusy => "ClipboardBusy",
            CaptureError::NothingToUndo => "NothingToUndo",
            CaptureError::Os(_) => "Os",
        }
    }

    /// Whether this is one of the refusals of [`check_allowed`]: the window in front must not be
    /// read or typed into.
    pub fn is_refusal(&self) -> bool {
        matches!(
            self,
            CaptureError::PasswordField
                | CaptureError::SecureInput
                | CaptureError::Blocklisted { .. }
                | CaptureError::Elevated
        )
    }

    /// `{"kind", "message"}` plus the variant's fields.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("an error serialises")
    }
}

/// Serialised as `{"kind", "message"}`, then `app` for `Blocklisted` and `code` for `Os`.
impl Serialize for CaptureError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("kind", self.kind())?;
        map.serialize_entry("message", &self.to_string())?;
        match self {
            CaptureError::Blocklisted { app } => map.serialize_entry("app", app)?,
            CaptureError::Os(code) => map.serialize_entry("code", code)?,
            _ => {}
        }
        map.end()
    }
}

/// The refusals, in this order: a blocklisted process, an elevated window, a password field,
/// secure input. Process names compare without regard to case or surrounding spaces, so a
/// blocklist entry typed as `KeePass.exe` still matches `keepass.exe`.
pub fn check_allowed(fg: &Foreground, blocklist: &[String]) -> Result<(), CaptureError> {
    let process = fg.process.trim().to_lowercase();
    if !process.is_empty()
        && blocklist
            .iter()
            .any(|entry| entry.trim().to_lowercase() == process)
    {
        return Err(CaptureError::Blocklisted { app: process });
    }
    if fg.elevated {
        return Err(CaptureError::Elevated);
    }
    if fg.is_password {
        return Err(CaptureError::PasswordField);
    }
    if fg.secure_input {
        return Err(CaptureError::SecureInput);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::settings::DEFAULT_BLOCKLIST;
    use serde_json::json;

    fn front(process: &str) -> Foreground {
        Foreground {
            window: WindowRef { id: 7, pid: 70 },
            process: process.into(),
            ..Foreground::default()
        }
    }

    fn defaults() -> Vec<String> {
        DEFAULT_BLOCKLIST.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn messages_are_the_spec_strings() {
        let cases = [
            (
                CaptureError::PasswordField,
                "Redpen doesn't read or type into password fields.",
            ),
            (
                CaptureError::SecureInput,
                "Another app is protecting what you type right now, so Redpen can't read or type.",
            ),
            (
                CaptureError::Blocklisted {
                    app: "1password.exe".into(),
                },
                "Redpen is turned off for this app (Settings → Privacy).",
            ),
            (
                CaptureError::Elevated,
                "This window runs as administrator, so Windows doesn't let Redpen read or type into it.",
            ),
            (
                CaptureError::NothingCopied,
                "Nothing was copied. Select some text first.",
            ),
            (
                CaptureError::KeysHeld,
                "Release the shortcut keys, then try again.",
            ),
            (
                CaptureError::FocusChanged,
                "The window you started in is no longer in front, so nothing was typed. Copy the text instead.",
            ),
            (
                CaptureError::WindowGone,
                "The window you started in has closed. Copy the text instead.",
            ),
            (
                CaptureError::ClipboardBusy,
                "Another app is using the clipboard. Try again.",
            ),
            (CaptureError::NothingToUndo, "There is nothing to undo."),
            (CaptureError::os(5), "Windows refused the request (code 5)."),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message, "{error:?}");
        }
    }

    #[test]
    fn errors_serialise_with_kind_message_and_fields() {
        assert_eq!(
            CaptureError::Blocklisted {
                app: "bitwarden.exe".into()
            }
            .to_json(),
            json!({"kind": "Blocklisted",
                   "message": "Redpen is turned off for this app (Settings → Privacy).",
                   "app": "bitwarden.exe"})
        );
        assert_eq!(
            CaptureError::os(1400).to_json(),
            json!({"kind": "Os", "message": "Windows refused the request (code 1400).",
                   "code": "1400"})
        );
        assert_eq!(
            CaptureError::KeysHeld.to_json(),
            json!({"kind": "KeysHeld", "message": "Release the shortcut keys, then try again."})
        );
    }

    #[test]
    fn the_message_never_names_the_app() {
        let e = CaptureError::Blocklisted {
            app: "keepassxc.exe".into(),
        };
        assert!(!e.to_string().contains("keepassxc"));
    }

    #[test]
    fn every_default_blocklist_entry_is_refused() {
        for name in DEFAULT_BLOCKLIST {
            assert_eq!(
                check_allowed(&front(name), &defaults()),
                Err(CaptureError::Blocklisted { app: name.into() }),
                "{name}"
            );
        }
        assert_eq!(check_allowed(&front("notepad.exe"), &defaults()), Ok(()));
    }

    #[test]
    fn blocklist_matching_ignores_case_and_spaces() {
        let list = vec![" KeePass.EXE ".to_string()];
        assert_eq!(
            check_allowed(&front("keepass.exe"), &list),
            Err(CaptureError::Blocklisted {
                app: "keepass.exe".into()
            })
        );
        assert_eq!(
            check_allowed(&front("KEEPASS.EXE"), &list),
            Err(CaptureError::Blocklisted {
                app: "keepass.exe".into()
            })
        );
        // A name that only contains a blocklisted one is a different program.
        assert_eq!(check_allowed(&front("notkeepass.exe"), &list), Ok(()));
        // An unknown process name never matches an empty entry.
        assert_eq!(check_allowed(&front(""), &[String::new()]), Ok(()));
    }

    #[test]
    fn refusals_run_in_order() {
        let mut fg = front("1password.exe");
        fg.elevated = true;
        fg.is_password = true;
        fg.secure_input = true;
        assert!(matches!(
            check_allowed(&fg, &defaults()),
            Err(CaptureError::Blocklisted { .. })
        ));
        fg.process = "notepad.exe".into();
        assert_eq!(check_allowed(&fg, &defaults()), Err(CaptureError::Elevated));
        fg.elevated = false;
        assert_eq!(
            check_allowed(&fg, &defaults()),
            Err(CaptureError::PasswordField)
        );
        fg.is_password = false;
        assert_eq!(
            check_allowed(&fg, &defaults()),
            Err(CaptureError::SecureInput)
        );
        fg.secure_input = false;
        assert_eq!(check_allowed(&fg, &defaults()), Ok(()));
    }

    #[test]
    fn refusals_are_told_apart_from_other_errors() {
        assert!(CaptureError::PasswordField.is_refusal());
        assert!(CaptureError::Elevated.is_refusal());
        assert!(
            CaptureError::Blocklisted {
                app: "enpass.exe".into()
            }
            .is_refusal()
        );
        assert!(CaptureError::SecureInput.is_refusal());
        assert!(!CaptureError::NothingCopied.is_refusal());
        assert!(!CaptureError::os(5).is_refusal());
    }
}
