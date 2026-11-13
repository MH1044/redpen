//! Accept and Undo (docs/modules/05-capture-and-replace-windows.md): typing the result into the
//! window the selection came from, after checking it again, and taking the last replace back.

use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;

use super::wait_for_keys_up;
use crate::clock::{Clock, Sleeper};
use crate::platform::{Capture, CaptureError, WindowRef, check_allowed};
use crate::store::Settings;

/// How long Accept and Undo wait after bringing a window to the front before checking it.
pub const FOCUS_SETTLE: Duration = Duration::from_millis(50);

/// How long the last replace can be undone.
pub const UNDO_LIFETIME: Duration = Duration::from_secs(120);

/// Types `text` into `target`, replacing its selection (Accept).
///
/// In order: `target` must still exist ([`CaptureError::WindowGone`]) and pass the refusals as
/// far as they are known while it is behind the popup; it is brought to the front if it is not
/// there ([`CaptureError::FocusChanged`] when it cannot be); then a 50 ms wait, the wait for
/// modifier keys to be up, a check that `target` is the window in front, the refusals again on
/// the window as it is now (its focused control may have become a password field while the
/// rewrite ran), and only then `paste` with `capture.paste_restore_delay_ms`.
pub async fn deliver(
    cap: &dyn Capture,
    target: &WindowRef,
    text: &str,
    s: &Settings,
    sleeper: &dyn Sleeper,
) -> Result<(), CaptureError> {
    let result = deliver_inner(cap, target, text, s, sleeper).await;
    match &result {
        Ok(()) => tracing::info!(bytes = text.len(), "deliver_pasted"),
        Err(e) => tracing::info!(kind = e.kind(), "deliver_stopped"),
    }
    result
}

async fn deliver_inner(
    cap: &dyn Capture,
    target: &WindowRef,
    text: &str,
    s: &Settings,
    sleeper: &dyn Sleeper,
) -> Result<(), CaptureError> {
    bring_to_front(cap, target, &s.privacy.blocklist, sleeper).await?;
    let restore_delay = Duration::from_millis(s.capture.paste_restore_delay_ms.into());
    cap.paste(text, restore_delay)
}

/// Everything before the synthetic key of Accept and Undo: `target` exists, is allowed, is in
/// front, and no modifier key is down.
async fn bring_to_front(
    cap: &dyn Capture,
    target: &WindowRef,
    blocklist: &[String],
    sleeper: &dyn Sleeper,
) -> Result<(), CaptureError> {
    let info = cap.inspect(target)?.ok_or(CaptureError::WindowGone)?;
    check_allowed(&info, blocklist)?;
    let in_front = cap
        .foreground()
        .map(|fg| fg.window == *target)
        .unwrap_or(false);
    if !in_front && !cap.focus(target)? {
        return Err(CaptureError::FocusChanged);
    }
    sleeper.sleep(FOCUS_SETTLE).await;
    wait_for_keys_up(cap, sleeper).await?;
    let fg = cap.foreground()?;
    if fg.window != *target {
        return Err(CaptureError::FocusChanged);
    }
    check_allowed(&fg, blocklist)
}

/// What Undo did: sent Ctrl+Z to the window, or put the original text on the clipboard because
/// the window was gone or could not be typed into. Serialised as `"undone"` or `"on_clipboard"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoOutcome {
    Undone,
    OnClipboard,
}

#[derive(Debug, Clone)]
struct LastReplace {
    window: WindowRef,
    original: String,
    at_ms: i64,
}

/// The last replace, kept in memory only, for [`UNDO_LIFETIME`] as told by the injected clock.
#[derive(Debug, Default)]
pub struct UndoBuffer {
    last: Mutex<Option<LastReplace>>,
}

impl UndoBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    fn slot(&self) -> MutexGuard<'_, Option<LastReplace>> {
        self.last.lock().expect("undo buffer is not poisoned")
    }

    /// Remembers a replace in `window` whose selection was `original`, in place of any earlier
    /// one.
    pub fn remember(&self, window: WindowRef, original: impl Into<String>, clock: &dyn Clock) {
        *self.slot() = Some(LastReplace {
            window,
            original: original.into(),
            at_ms: clock.now_ms(),
        });
    }

    /// Whether a replace can be undone now.
    pub fn can_undo(&self, clock: &dyn Clock) -> bool {
        self.slot().as_ref().is_some_and(|last| fresh(last, clock))
    }

    /// Forgets the last replace.
    pub fn clear(&self) {
        *self.slot() = None;
    }

    /// Takes back the last replace, once.
    ///
    /// With the window in front, or brought there, and still allowed: one Ctrl+Z
    /// ([`UndoOutcome::Undone`]). With the window gone, behind another that will not give way,
    /// or now refused (a password field, secure input, elevated): the original text on the
    /// clipboard ([`UndoOutcome::OnClipboard`]). The blocklist is not asked again: the replace
    /// passed it, and a window with the same handle and process id is the same program. Nothing
    /// remembered, or remembered more than two minutes ago: [`CaptureError::NothingToUndo`].
    /// Any other error leaves the replace in the buffer, so Undo can be tried again.
    pub async fn undo(
        &self,
        cap: &dyn Capture,
        clock: &dyn Clock,
        sleeper: &dyn Sleeper,
    ) -> Result<UndoOutcome, CaptureError> {
        let last = {
            let mut slot = self.slot();
            match slot.take() {
                Some(last) if fresh(&last, clock) => last,
                _ => return Err(CaptureError::NothingToUndo),
            }
        };
        let result = undo_into(cap, &last, sleeper).await;
        match &result {
            Ok(outcome) => tracing::info!(outcome = ?outcome, "undo"),
            Err(e) => {
                tracing::info!(kind = e.kind(), "undo_stopped");
                let mut slot = self.slot();
                if slot.is_none() {
                    *slot = Some(last);
                }
            }
        }
        result
    }
}

fn fresh(last: &LastReplace, clock: &dyn Clock) -> bool {
    let lifetime = i64::try_from(UNDO_LIFETIME.as_millis()).unwrap_or(i64::MAX);
    clock.now_ms().saturating_sub(last.at_ms) <= lifetime
}

async fn undo_into(
    cap: &dyn Capture,
    last: &LastReplace,
    sleeper: &dyn Sleeper,
) -> Result<UndoOutcome, CaptureError> {
    match bring_to_front(cap, &last.window, &[], sleeper).await {
        Ok(()) => {
            cap.send_undo()?;
            Ok(UndoOutcome::Undone)
        }
        Err(e)
            if e.is_refusal()
                || matches!(e, CaptureError::WindowGone | CaptureError::FocusChanged) =>
        {
            tracing::info!(kind = e.kind(), "undo_to_clipboard");
            cap.set_clipboard_text(&last.original)?;
            Ok(UndoOutcome::OnClipboard)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{FakeClock, FakeSleeper};
    use crate::platform::{CaptureCall, FakeCapture, Foreground};

    fn target() -> Foreground {
        Foreground {
            window: WindowRef { id: 7, pid: 70 },
            process: "notepad.exe".into(),
            ..Foreground::default()
        }
    }

    fn popup() -> Foreground {
        Foreground {
            window: WindowRef { id: 9, pid: 90 },
            process: "redpen-app.exe".into(),
            ..Foreground::default()
        }
    }

    /// The target behind Redpen's popup, as at Accept.
    fn desktop() -> FakeCapture {
        let fake = FakeCapture::with_foreground(target());
        fake.set_foreground(popup());
        fake
    }

    #[tokio::test]
    async fn deliver_focuses_waits_checks_and_pastes() {
        let fake = desktop();
        let sleeper = FakeSleeper::new();
        let mut s = Settings::default();
        s.capture.paste_restore_delay_ms = 300;
        deliver(&fake, &target().window, "world", &s, &sleeper)
            .await
            .unwrap();
        assert_eq!(
            fake.call_names(),
            [
                "inspect",
                "foreground",
                "focus",
                "modifiers_down",
                "foreground",
                "paste"
            ]
        );
        assert_eq!(
            fake.calls()[5],
            CaptureCall::Paste {
                text: "world".into(),
                restore_delay: Duration::from_millis(300)
            }
        );
        assert_eq!(sleeper.sleeps(), vec![FOCUS_SETTLE]);
        assert_eq!(fake.typed(), vec![(target().window, "world".to_string())]);
    }

    #[tokio::test]
    async fn a_target_already_in_front_is_not_focused_again() {
        let fake = FakeCapture::with_foreground(target());
        deliver(
            &fake,
            &target().window,
            "x",
            &Settings::default(),
            &FakeSleeper::new(),
        )
        .await
        .unwrap();
        assert_eq!(fake.count("focus"), 0);
        assert_eq!(fake.count("paste"), 1);
    }

    #[tokio::test]
    async fn deliver_refuses_without_pasting() {
        let s = Settings::default();
        let sleeper = FakeSleeper::new();

        let gone = desktop();
        gone.close_window(&target().window);
        assert_eq!(
            deliver(&gone, &target().window, "x", &s, &sleeper).await,
            Err(CaptureError::WindowGone)
        );

        let stuck = desktop();
        stuck.set_focusable(&target().window, false);
        assert_eq!(
            deliver(&stuck, &target().window, "x", &s, &sleeper).await,
            Err(CaptureError::FocusChanged)
        );

        let password = desktop();
        password.update_window(&target().window, |w| w.is_password = true);
        assert_eq!(
            deliver(&password, &target().window, "x", &s, &sleeper).await,
            Err(CaptureError::PasswordField)
        );

        // Focus moved away again during the 50 ms wait.
        let moved = desktop();
        moved.script_foreground(Ok(popup()));
        moved.script_foreground(Ok(popup()));
        assert_eq!(
            deliver(&moved, &target().window, "x", &s, &sleeper).await,
            Err(CaptureError::FocusChanged)
        );

        // The password field shows only once the window is in front.
        let late = desktop();
        late.script_foreground(Ok(popup()));
        late.script_foreground(Ok(Foreground {
            is_password: true,
            ..target()
        }));
        assert_eq!(
            deliver(&late, &target().window, "x", &s, &sleeper).await,
            Err(CaptureError::PasswordField)
        );

        let held = desktop();
        held.hold_modifiers(200);
        assert_eq!(
            deliver(&held, &target().window, "x", &s, &sleeper).await,
            Err(CaptureError::KeysHeld)
        );

        for fake in [gone, stuck, password, moved, late, held] {
            assert_eq!(fake.count("paste"), 0);
        }
    }

    #[tokio::test]
    async fn undo_sends_one_ctrl_z_within_two_minutes() {
        let fake = FakeCapture::with_foreground(target());
        let clock = FakeClock::at(10_000);
        let undo = UndoBuffer::new();
        undo.remember(target().window, "hello wrold", &clock);
        clock.advance(UNDO_LIFETIME);
        assert!(undo.can_undo(&clock));
        let outcome = undo.undo(&fake, &clock, &FakeSleeper::new()).await;
        assert_eq!(outcome, Ok(UndoOutcome::Undone));
        assert_eq!(fake.count("send_undo"), 1);
        assert_eq!(fake.undone(), vec![target().window]);
        // Once only.
        assert_eq!(
            undo.undo(&fake, &clock, &FakeSleeper::new()).await,
            Err(CaptureError::NothingToUndo)
        );
        assert_eq!(fake.count("send_undo"), 1);
    }

    #[tokio::test]
    async fn undo_puts_the_original_on_the_clipboard_when_it_cannot_type() {
        let clock = FakeClock::at(0);

        let gone = desktop();
        let undo = UndoBuffer::new();
        undo.remember(target().window, "hello wrold", &clock);
        gone.close_window(&target().window);
        assert_eq!(
            undo.undo(&gone, &clock, &FakeSleeper::new()).await,
            Ok(UndoOutcome::OnClipboard)
        );
        assert_eq!(gone.clipboard().as_deref(), Some("hello wrold"));
        assert_eq!(gone.count("send_undo"), 0);

        let stuck = desktop();
        stuck.set_focusable(&target().window, false);
        undo.remember(target().window, "first", &clock);
        assert_eq!(
            undo.undo(&stuck, &clock, &FakeSleeper::new()).await,
            Ok(UndoOutcome::OnClipboard)
        );
        assert_eq!(stuck.clipboard().as_deref(), Some("first"));
        assert_eq!(stuck.count("send_undo"), 0);

        // No Ctrl+Z into a password field: the original goes on the clipboard instead.
        let password = FakeCapture::with_foreground(Foreground {
            is_password: true,
            ..target()
        });
        undo.remember(target().window, "second", &clock);
        assert_eq!(
            undo.undo(&password, &clock, &FakeSleeper::new()).await,
            Ok(UndoOutcome::OnClipboard)
        );
        assert_eq!(password.clipboard().as_deref(), Some("second"));
        assert_eq!(password.count("send_undo"), 0);
    }

    #[tokio::test]
    async fn undo_after_two_minutes_has_nothing_to_undo() {
        let fake = FakeCapture::with_foreground(target());
        let clock = FakeClock::at(0);
        let undo = UndoBuffer::new();
        assert_eq!(
            undo.undo(&fake, &clock, &FakeSleeper::new()).await,
            Err(CaptureError::NothingToUndo)
        );
        undo.remember(target().window, "hello", &clock);
        clock.advance(UNDO_LIFETIME + Duration::from_millis(1));
        assert!(!undo.can_undo(&clock));
        assert_eq!(
            undo.undo(&fake, &clock, &FakeSleeper::new()).await,
            Err(CaptureError::NothingToUndo)
        );
        assert!(fake.calls().is_empty());
    }

    #[tokio::test]
    async fn an_undo_that_fails_can_be_tried_again() {
        let fake = FakeCapture::with_foreground(target());
        let clock = FakeClock::at(0);
        let undo = UndoBuffer::new();
        undo.remember(target().window, "hello", &clock);
        fake.hold_modifiers(101);
        assert_eq!(
            undo.undo(&fake, &clock, &FakeSleeper::new()).await,
            Err(CaptureError::KeysHeld)
        );
        assert!(undo.can_undo(&clock));
        assert_eq!(
            undo.undo(&fake, &clock, &FakeSleeper::new()).await,
            Ok(UndoOutcome::Undone)
        );
        undo.remember(target().window, "again", &clock);
        undo.clear();
        assert!(!undo.can_undo(&clock));
    }

    #[test]
    fn outcomes_serialise_as_the_command_result() {
        assert_eq!(
            serde_json::to_value(UndoOutcome::Undone).unwrap(),
            serde_json::json!("undone")
        );
        assert_eq!(
            serde_json::to_value(UndoOutcome::OnClipboard).unwrap(),
            serde_json::json!("on_clipboard")
        );
    }
}
