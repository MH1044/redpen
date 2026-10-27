//! Time, injected (docs/ARCHITECTURE.md §7, invariant 10). Core code asks a [`Clock`] for the time
//! and a [`Sleeper`] to wait, so tests run with a [`FakeClock`] they set and a [`FakeSleeper`]
//! that returns at once and records what was asked.

use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The current time in milliseconds since the Unix epoch.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// Waits for a duration. Retries, timeouts and the hourly cache sweep go through it.
#[async_trait::async_trait]
pub trait Sleeper: Send + Sync {
    async fn sleep(&self, d: Duration);
}

/// The operating system's clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        // A system clock set before 1970 is treated as the epoch rather than a panic.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

/// Sleeps on the Tokio runtime.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokioSleeper;

#[async_trait::async_trait]
impl Sleeper for TokioSleeper {
    async fn sleep(&self, d: Duration) {
        tokio::time::sleep(d).await;
    }
}

/// A clock that returns the time it was given and moves only when told to.
#[derive(Debug, Default)]
pub struct FakeClock {
    now_ms: AtomicI64,
}

impl FakeClock {
    /// A fake clock reading `now_ms`.
    pub fn at(now_ms: i64) -> Self {
        Self {
            now_ms: AtomicI64::new(now_ms),
        }
    }

    /// Sets the time.
    pub fn set(&self, now_ms: i64) {
        self.now_ms.store(now_ms, Ordering::SeqCst);
    }

    /// Moves the time forward by `d`.
    pub fn advance(&self, d: Duration) {
        let ms = i64::try_from(d.as_millis()).unwrap_or(i64::MAX);
        self.now_ms.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

/// A sleeper that never waits: it records every requested duration, in order, and returns.
#[derive(Debug, Default)]
pub struct FakeSleeper {
    sleeps: Mutex<Vec<Duration>>,
}

impl FakeSleeper {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every duration that was asked for, in order.
    pub fn sleeps(&self) -> Vec<Duration> {
        self.sleeps
            .lock()
            .expect("sleep log is not poisoned")
            .clone()
    }
}

#[async_trait::async_trait]
impl Sleeper for FakeSleeper {
    async fn sleep(&self, d: Duration) {
        self.sleeps
            .lock()
            .expect("sleep log is not poisoned")
            .push(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_after_2026() {
        // 2026-01-01T00:00:00Z in milliseconds.
        assert!(SystemClock.now_ms() > 1_767_225_600_000);
    }

    #[test]
    fn fake_clock_sets_and_advances() {
        let clock = FakeClock::at(1_000);
        clock.advance(Duration::from_millis(500));
        assert_eq!(clock.now_ms(), 1_500);
        clock.set(7);
        assert_eq!(clock.now_ms(), 7);
    }

    #[tokio::test]
    async fn fake_sleeper_records_without_waiting() {
        let sleeper = FakeSleeper::new();
        sleeper.sleep(Duration::from_millis(500)).await;
        sleeper.sleep(Duration::from_secs(1)).await;
        assert_eq!(
            sleeper.sleeps(),
            vec![Duration::from_millis(500), Duration::from_secs(1)]
        );
    }
}
