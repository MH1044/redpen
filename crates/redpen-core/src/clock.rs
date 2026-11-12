//! Time, injected (docs/ARCHITECTURE.md §7, invariant 10). Core code asks a [`Clock`] for the time
//! and a [`Sleeper`] to wait, so tests run with a [`FakeClock`] they set and a [`FakeSleeper`]
//! that returns at once and records what was asked.

use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The current time in milliseconds since the Unix epoch, and the local time zone's offset.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;

    /// Minutes to add to UTC to get local time now (60 for UTC+1, -300 for UTC-5). The local
    /// day of `usage_daily` comes from it (docs/modules/02-provider-layer.md).
    fn utc_offset_minutes(&self) -> i32;
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

    /// The operating system's offset for the current instant, daylight saving included. SQLite's
    /// `localtime` modifier asks the operating system (`localtime_s` on Windows, `localtime_r`
    /// on Linux), so no time zone database is shipped. 0 if the question cannot be answered.
    fn utc_offset_minutes(&self) -> i32 {
        let now_s = self.now_ms().div_euclid(1000);
        let offset_s = rusqlite::Connection::open_in_memory().and_then(|db| {
            db.query_row(
                "SELECT CAST(strftime('%s', ?1, 'unixepoch', 'localtime') AS INTEGER) - ?1",
                [now_s],
                |row| row.get::<_, i64>(0),
            )
        });
        offset_s
            .ok()
            .and_then(|s| i32::try_from(s.div_euclid(60)).ok())
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

/// A clock that returns the time it was given and moves only when told to. Its offset from UTC
/// is 0 unless set, so tests see UTC dates.
#[derive(Debug, Default)]
pub struct FakeClock {
    now_ms: AtomicI64,
    offset_minutes: AtomicI32,
}

impl FakeClock {
    /// A fake clock reading `now_ms`, in UTC.
    pub fn at(now_ms: i64) -> Self {
        Self {
            now_ms: AtomicI64::new(now_ms),
            offset_minutes: AtomicI32::new(0),
        }
    }

    /// Sets the local time zone's offset from UTC, in minutes.
    pub fn set_utc_offset_minutes(&self, minutes: i32) {
        self.offset_minutes.store(minutes, Ordering::SeqCst);
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

    fn utc_offset_minutes(&self) -> i32 {
        self.offset_minutes.load(Ordering::SeqCst)
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
    fn system_offset_is_a_real_time_zone() {
        // Every time zone in use lies between UTC-12 and UTC+14, in whole quarter hours.
        let offset = SystemClock.utc_offset_minutes();
        assert!((-12 * 60..=14 * 60).contains(&offset), "{offset}");
        assert_eq!(offset % 15, 0, "{offset}");
    }

    #[test]
    fn fake_clock_offset_is_utc_unless_set() {
        let clock = FakeClock::at(0);
        assert_eq!(clock.utc_offset_minutes(), 0);
        clock.set_utc_offset_minutes(-300);
        assert_eq!(clock.utc_offset_minutes(), -300);
        assert_eq!(FakeClock::default().utc_offset_minutes(), 0);
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
