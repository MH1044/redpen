//! The coordinating layer (docs/ARCHITECTURE.md §3), written against traits so tests run it end
//! to end with fakes. Module 05 adds the capture half of the rewrite flow
//! ([`rewrite::capture_session`]) and the replace path Accept uses ([`deliver()`]), with the undo
//! buffer ([`UndoBuffer`]). Each refuses before any synthetic key (invariants 3 and 4). Module 06
//! adds [`session`]: the popup's sessions, which join the capture, Harper, the rewrite engine,
//! Accept, Copy and Undo.

pub mod deliver;
pub mod rewrite;
pub mod session;

use std::time::Duration;

use crate::clock::Sleeper;
use crate::platform::{Capture, CaptureError};

pub use deliver::{FOCUS_SETTLE, UNDO_LIFETIME, UndoBuffer, UndoOutcome, deliver};
pub use rewrite::{Captured, capture_session};
pub use session::{RewriteResult, Session, SessionError, Sessions, provider_line};

/// How often the key wait asks whether a modifier key is still down.
pub const KEY_POLL: Duration = Duration::from_millis(10);

/// How many times the key wait asks again after the first check: 100 polls of 10 ms, 1 s.
pub const KEY_POLLS: u32 = 100;

/// Waits until Ctrl, Alt, Shift and the Windows keys are all up, so a synthetic Ctrl+C, Ctrl+V or
/// Ctrl+Z is not mixed with keys the user still holds (the shortcut that started the capture,
/// or Ctrl+Enter in the popup). Asks once, then up to [`KEY_POLLS`] more times, sleeping
/// [`KEY_POLL`] through `sleeper` before each; it counts polls and never reads the clock.
/// Returns how many polls it waited, or [`CaptureError::KeysHeld`] when a key is still down
/// after the last one.
pub async fn wait_for_keys_up(
    cap: &dyn Capture,
    sleeper: &dyn Sleeper,
) -> Result<u32, CaptureError> {
    let mut polls = 0;
    while cap.modifiers_down() {
        if polls == KEY_POLLS {
            return Err(CaptureError::KeysHeld);
        }
        sleeper.sleep(KEY_POLL).await;
        polls += 1;
    }
    Ok(polls)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeSleeper;
    use crate::platform::FakeCapture;

    #[tokio::test]
    async fn keys_up_at_once_need_no_sleep() {
        let fake = FakeCapture::new();
        let sleeper = FakeSleeper::new();
        assert_eq!(wait_for_keys_up(&fake, &sleeper).await, Ok(0));
        assert!(sleeper.sleeps().is_empty());
        assert_eq!(fake.count("modifiers_down"), 1);
    }

    #[tokio::test]
    async fn keys_released_within_a_second_are_waited_for() {
        let fake = FakeCapture::new();
        fake.hold_modifiers(30);
        let sleeper = FakeSleeper::new();
        assert_eq!(wait_for_keys_up(&fake, &sleeper).await, Ok(30));
        assert_eq!(sleeper.sleeps(), vec![KEY_POLL; 30]);
    }

    #[tokio::test]
    async fn the_last_poll_is_at_one_second() {
        // Down for the first check and 99 polls, up at the 100th: still in time.
        let fake = FakeCapture::new();
        fake.hold_modifiers(100);
        let sleeper = FakeSleeper::new();
        assert_eq!(wait_for_keys_up(&fake, &sleeper).await, Ok(100));
        assert_eq!(
            sleeper.sleeps().iter().sum::<Duration>(),
            Duration::from_secs(1)
        );

        // Down for all 101 checks: the keys are held.
        let fake = FakeCapture::new();
        fake.hold_modifiers(101);
        let sleeper = FakeSleeper::new();
        assert_eq!(
            wait_for_keys_up(&fake, &sleeper).await,
            Err(CaptureError::KeysHeld)
        );
        assert_eq!(fake.count("modifiers_down"), 101);
        assert_eq!(
            sleeper.sleeps().iter().sum::<Duration>(),
            Duration::from_secs(1)
        );
    }
}
