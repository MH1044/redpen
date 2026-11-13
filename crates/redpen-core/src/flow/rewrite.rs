//! The rewrite flow's first step (docs/ARCHITECTURE.md §3): the hotkey's capture. The popup,
//! the check and the rewrite actions that follow it are wired by module 06.

use std::time::Duration;

use serde::Serialize;

use super::wait_for_keys_up;
use crate::clock::{Clock, Sleeper};
use crate::platform::{Capture, CaptureError, Point, Rect, WindowRef, check_allowed};
use crate::store::Settings;

/// A captured selection: the text, the window it came from (where Accept types the result), and
/// where the popup goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Captured {
    pub window: WindowRef,
    pub text: String,
    pub caret: Option<Rect>,
    pub cursor: Point,
    /// When the capture finished, from the injected clock.
    pub captured_at_ms: i64,
}

/// Reads the selection in the window in front.
///
/// In order: the window in front; the refusals of [`check_allowed`] (a blocklisted process, an
/// elevated window, a password field), before any synthetic key; the wait for modifier keys to
/// be up, after which the window in front is checked again if the wait took any time; then
/// `read_selection` with `capture.copy_timeout_ms`. An empty selection is
/// [`CaptureError::NothingCopied`].
pub async fn capture_session(
    cap: &dyn Capture,
    s: &Settings,
    clock: &dyn Clock,
    sleeper: &dyn Sleeper,
) -> Result<Captured, CaptureError> {
    let started = clock.now_ms();
    let result = capture(cap, s, clock, sleeper).await;
    let elapsed_ms = clock.now_ms().saturating_sub(started);
    match &result {
        Ok(c) => tracing::info!(bytes = c.text.len(), elapsed_ms, "capture_read"),
        Err(e) => tracing::info!(kind = e.kind(), elapsed_ms, "capture_stopped"),
    }
    result
}

async fn capture(
    cap: &dyn Capture,
    s: &Settings,
    clock: &dyn Clock,
    sleeper: &dyn Sleeper,
) -> Result<Captured, CaptureError> {
    let mut fg = cap.foreground()?;
    check_allowed(&fg, &s.privacy.blocklist)?;
    if wait_for_keys_up(cap, sleeper).await? > 0 {
        // A held Alt can switch windows; look at whatever is in front now.
        fg = cap.foreground()?;
        check_allowed(&fg, &s.privacy.blocklist)?;
    }
    let timeout = Duration::from_millis(s.capture.copy_timeout_ms.into());
    let text = cap.read_selection(timeout)?;
    if text.is_empty() {
        return Err(CaptureError::NothingCopied);
    }
    Ok(Captured {
        window: fg.window,
        text,
        caret: fg.caret,
        cursor: fg.cursor,
        captured_at_ms: clock.now_ms(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{FakeClock, FakeSleeper};
    use crate::platform::{FakeCapture, Foreground};

    fn notepad() -> Foreground {
        Foreground {
            window: WindowRef { id: 5, pid: 50 },
            process: "notepad.exe".into(),
            caret: Some(Rect {
                x: 10,
                y: 20,
                width: 1,
                height: 16,
            }),
            cursor: Point { x: 3, y: 4 },
            ..Foreground::default()
        }
    }

    #[tokio::test]
    async fn reads_the_selection_with_the_settings_timeout() {
        let fake = FakeCapture::with_foreground(notepad()).with_selection("wrold");
        let mut s = Settings::default();
        s.capture.copy_timeout_ms = 650;
        let clock = FakeClock::at(1_000);
        let got = capture_session(&fake, &s, &clock, &FakeSleeper::new())
            .await
            .unwrap();
        assert_eq!(
            got,
            Captured {
                window: notepad().window,
                text: "wrold".into(),
                caret: notepad().caret,
                cursor: Point { x: 3, y: 4 },
                captured_at_ms: 1_000,
            }
        );
        assert_eq!(
            fake.call_names(),
            ["foreground", "modifiers_down", "read_selection"]
        );
        assert_eq!(
            fake.calls()[2],
            crate::platform::CaptureCall::ReadSelection {
                timeout: Duration::from_millis(650)
            }
        );
    }

    #[tokio::test]
    async fn a_refusal_comes_before_any_key() {
        let mut fg = notepad();
        fg.is_password = true;
        let fake = FakeCapture::with_foreground(fg).with_selection("secret");
        let err = capture_session(
            &fake,
            &Settings::default(),
            &FakeClock::at(0),
            &FakeSleeper::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(err, CaptureError::PasswordField);
        assert_eq!(fake.call_names(), ["foreground"]);
    }

    #[tokio::test]
    async fn the_window_is_checked_again_after_waiting_for_keys() {
        let fake = FakeCapture::with_foreground(notepad()).with_selection("text");
        fake.hold_modifiers(3);
        // While the keys were held, a password manager came to the front.
        fake.script_foreground(Ok(notepad()));
        fake.script_foreground(Ok(Foreground {
            process: "bitwarden.exe".into(),
            ..notepad()
        }));
        let err = capture_session(
            &fake,
            &Settings::default(),
            &FakeClock::at(0),
            &FakeSleeper::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CaptureError::Blocklisted { .. }));
        assert_eq!(fake.count("read_selection"), 0);
    }

    #[tokio::test]
    async fn an_empty_copy_is_nothing_copied() {
        let fake = FakeCapture::with_foreground(notepad());
        fake.script_read_selection(Ok(String::new()));
        let err = capture_session(
            &fake,
            &Settings::default(),
            &FakeClock::at(0),
            &FakeSleeper::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(err, CaptureError::NothingCopied);
    }
}
