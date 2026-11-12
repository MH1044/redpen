//! `redpen.db` (docs/ARCHITECTURE.md §4): the answer cache `ai_cache`, the daily usage counts
//! `usage_daily` and the opt-in `sent_log`, in SQLite. The caller names the data directory, or
//! asks for an in-memory database (tests, and the command line's `chat`).
//!
//! Nothing here is the source of truth for anything: the cache can be emptied at any time, and
//! usage and the sent log are disposable history. A failed write never fails a user's action;
//! the provider layer logs it and goes on.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

/// One day in milliseconds.
pub const DAY_MS: i64 = 86_400_000;

/// How long the sent log keeps a row.
pub const SENT_LOG_DAYS: u32 = 7;

/// The schema version in `PRAGMA user_version`.
pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS ai_cache (
  key            TEXT PRIMARY KEY,
  provider       TEXT NOT NULL,
  model          TEXT NOT NULL,
  prompt_id      TEXT NOT NULL,
  prompt_version INTEGER NOT NULL,
  response_json  TEXT NOT NULL,
  charge_micros  INTEGER,
  created_at_ms  INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS ai_cache_created ON ai_cache(created_at_ms);

CREATE TABLE IF NOT EXISTS usage_daily (
  day           TEXT NOT NULL,
  provider      TEXT NOT NULL,
  feature       TEXT NOT NULL,
  calls         INTEGER NOT NULL DEFAULT 0,
  charge_micros INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, provider, feature)
) STRICT;

CREATE TABLE IF NOT EXISTS sent_log (
  id        INTEGER PRIMARY KEY,
  at_ms     INTEGER NOT NULL,
  provider  TEXT NOT NULL,
  endpoint  TEXT NOT NULL,
  feature   TEXT NOT NULL,
  model     TEXT NOT NULL,
  sent_text TEXT NOT NULL,
  bytes     INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS sent_log_at ON sent_log(at_ms);
";

/// A database failure. The message names the operation, never a cached answer or sent text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("redpen.db: {0}")]
pub struct DbError(pub String);

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        // SQLite's own messages name the failing statement or constraint, not bound values.
        DbError(e.to_string())
    }
}

/// One `ai_cache` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheRow {
    pub key: String,
    pub provider: String,
    pub model: String,
    pub prompt_id: String,
    pub prompt_version: u32,
    pub response_json: String,
    pub charge_micros: Option<u64>,
    pub created_at_ms: i64,
}

/// One `usage_daily` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub day: String,
    pub provider: String,
    pub feature: String,
    pub calls: u64,
    pub charge_micros: u64,
}

/// One `sent_log` row, as Settings → Privacy lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SentEntry {
    pub id: i64,
    pub at_ms: i64,
    pub provider: String,
    pub endpoint: String,
    pub feature: String,
    pub model: String,
    pub sent_text: String,
    pub bytes: u64,
}

/// How many rows a sweep deleted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Swept {
    pub ai_cache: usize,
    pub sent_log: usize,
}

/// `redpen.db`. Cloning shares the one connection.
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db").finish_non_exhaustive()
    }
}

impl Db {
    /// The file's name in the data directory.
    pub const FILE: &'static str = "redpen.db";

    /// Opens (or creates) `<data_dir>/redpen.db` in WAL mode, creating `data_dir` if needed.
    pub fn open(data_dir: &Path) -> Result<Self, DbError> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| DbError(format!("the data folder could not be created: {e}")))?;
        let conn = Connection::open(data_dir.join(Self::FILE))?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })?;
        Self::init(conn)
    }

    /// A database that lives as long as the process: tests, and `redpen chat`.
    pub fn open_in_memory() -> Result<Self, DbError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, DbError> {
        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(DbError(format!(
                "schema version {version} was written by a newer Redpen"
            )));
        }
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// The connection, for queries this type has no method for. Held only briefly, and never
    /// across an `await`.
    pub fn connection(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves SQLite consistent (every write is one
        // statement), so a poisoned lock is still usable.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ai_cache

    /// The row for `key` created at or after `not_before_ms`.
    pub fn cache_get(&self, key: &str, not_before_ms: i64) -> Result<Option<CacheRow>, DbError> {
        let conn = self.connection();
        let row = conn
            .query_row(
                "SELECT key, provider, model, prompt_id, prompt_version, response_json,
                        charge_micros, created_at_ms
                 FROM ai_cache WHERE key = ?1 AND created_at_ms >= ?2",
                params![key, not_before_ms],
                |r| {
                    Ok(CacheRow {
                        key: r.get(0)?,
                        provider: r.get(1)?,
                        model: r.get(2)?,
                        prompt_id: r.get(3)?,
                        prompt_version: u32::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
                        response_json: r.get(5)?,
                        charge_micros: r
                            .get::<_, Option<i64>>(6)?
                            .and_then(|c| u64::try_from(c).ok()),
                        created_at_ms: r.get(7)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Stores a row, replacing any row with the same key.
    pub fn cache_put(&self, row: &CacheRow) -> Result<(), DbError> {
        self.connection().execute(
            "INSERT OR REPLACE INTO ai_cache
               (key, provider, model, prompt_id, prompt_version, response_json, charge_micros,
                created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                row.key,
                row.provider,
                row.model,
                row.prompt_id,
                i64::from(row.prompt_version),
                row.response_json,
                row.charge_micros.map(saturating_i64),
                row.created_at_ms,
            ],
        )?;
        Ok(())
    }

    /// How many answers are cached on disk.
    pub fn cache_len(&self) -> Result<usize, DbError> {
        count(&self.connection(), "ai_cache")
    }

    /// Empties `ai_cache` (Settings → Privacy → Clear now).
    pub fn cache_clear(&self) -> Result<(), DbError> {
        self.connection().execute("DELETE FROM ai_cache", [])?;
        Ok(())
    }

    // usage_daily

    /// Adds `calls` and `charge_micros` to the row for (day, provider, feature), creating it.
    pub fn usage_add(
        &self,
        day: &str,
        provider: &str,
        feature: &str,
        calls: u64,
        charge_micros: u64,
    ) -> Result<(), DbError> {
        self.connection().execute(
            "INSERT INTO usage_daily (day, provider, feature, calls, charge_micros)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (day, provider, feature) DO UPDATE SET
               calls = calls + excluded.calls,
               charge_micros = charge_micros + excluded.charge_micros",
            params![
                day,
                provider,
                feature,
                saturating_i64(calls),
                saturating_i64(charge_micros)
            ],
        )?;
        Ok(())
    }

    /// The row for (day, provider, feature), if any call was counted there.
    pub fn usage(
        &self,
        day: &str,
        provider: &str,
        feature: &str,
    ) -> Result<Option<Usage>, DbError> {
        let conn = self.connection();
        let row = conn
            .query_row(
                "SELECT day, provider, feature, calls, charge_micros FROM usage_daily
                 WHERE day = ?1 AND provider = ?2 AND feature = ?3",
                params![day, provider, feature],
                usage_row,
            )
            .optional()?;
        Ok(row)
    }

    /// Every usage row, by day, provider and feature.
    pub fn usage_rows(&self) -> Result<Vec<Usage>, DbError> {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "SELECT day, provider, feature, calls, charge_micros FROM usage_daily
             ORDER BY day, provider, feature",
        )?;
        let rows = stmt
            .query_map([], usage_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // sent_log

    /// Adds a row (the caller checks that the log is on).
    #[allow(clippy::too_many_arguments)]
    pub fn sent_log_add(
        &self,
        at_ms: i64,
        provider: &str,
        endpoint: &str,
        feature: &str,
        model: &str,
        sent_text: &str,
        bytes: u64,
    ) -> Result<(), DbError> {
        self.connection().execute(
            "INSERT INTO sent_log (at_ms, provider, endpoint, feature, model, sent_text, bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                at_ms,
                provider,
                endpoint,
                feature,
                model,
                sent_text,
                saturating_i64(bytes)
            ],
        )?;
        Ok(())
    }

    /// Every row, newest first.
    pub fn sent_log_list(&self) -> Result<Vec<SentEntry>, DbError> {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "SELECT id, at_ms, provider, endpoint, feature, model, sent_text, bytes
             FROM sent_log ORDER BY at_ms DESC, id DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SentEntry {
                    id: r.get(0)?,
                    at_ms: r.get(1)?,
                    provider: r.get(2)?,
                    endpoint: r.get(3)?,
                    feature: r.get(4)?,
                    model: r.get(5)?,
                    sent_text: r.get(6)?,
                    bytes: u64::try_from(r.get::<_, i64>(7)?).unwrap_or(0),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Empties `sent_log` (Settings → Privacy → Clear).
    pub fn sent_log_clear(&self) -> Result<(), DbError> {
        self.connection().execute("DELETE FROM sent_log", [])?;
        Ok(())
    }

    // Sweeps

    /// Deletes `ai_cache` rows older than `remember_answers_days` (all of them for 0) and
    /// `sent_log` rows older than 7 days. Run at start and every hour.
    pub fn sweep(&self, now_ms: i64, remember_answers_days: u32) -> Result<Swept, DbError> {
        let conn = self.connection();
        let cache_cutoff = now_ms - i64::from(remember_answers_days) * DAY_MS;
        let log_cutoff = now_ms - i64::from(SENT_LOG_DAYS) * DAY_MS;
        let ai_cache = conn.execute(
            "DELETE FROM ai_cache WHERE created_at_ms < ?1",
            [cache_cutoff],
        )?;
        let sent_log = conn.execute("DELETE FROM sent_log WHERE at_ms < ?1", [log_cutoff])?;
        Ok(Swept { ai_cache, sent_log })
    }
}

fn usage_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Usage> {
    Ok(Usage {
        day: r.get(0)?,
        provider: r.get(1)?,
        feature: r.get(2)?,
        calls: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(0),
        charge_micros: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
    })
}

fn count(conn: &Connection, table: &str) -> Result<usize, DbError> {
    let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
    Ok(usize::try_from(n).unwrap_or(0))
}

fn saturating_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// The local calendar date, `YYYY-MM-DD`, of an instant given in UTC milliseconds and the
/// time zone's offset from UTC in minutes. Integer arithmetic only (the days-to-civil algorithm
/// of the proleptic Gregorian calendar), so it gives the same answer on every platform.
pub fn local_day(now_ms: i64, utc_offset_minutes: i32) -> String {
    let local_ms = now_ms + i64::from(utc_offset_minutes) * 60_000;
    let days = local_ms.div_euclid(DAY_MS);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Year, month and day of the day `days` after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, created_at_ms: i64) -> CacheRow {
        CacheRow {
            key: key.into(),
            provider: "own_key".into(),
            model: "example-chat".into(),
            prompt_id: "fix-grammar".into(),
            prompt_version: 1,
            response_json: r#"{"content":"hello"}"#.into(),
            charge_micros: None,
            created_at_ms,
        }
    }

    #[test]
    fn opens_with_the_schema_and_its_version() {
        let db = Db::open_in_memory().unwrap();
        let conn = db.connection();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);
        for table in ["ai_cache", "usage_daily", "sent_log"] {
            assert_eq!(count(&conn, table).unwrap(), 0, "{table}");
        }
    }

    #[test]
    fn a_file_database_uses_wal_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        {
            let db = Db::open(&data).unwrap();
            db.cache_put(&row("k", 5)).unwrap();
            let mode: String = db
                .connection()
                .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .unwrap();
            assert_eq!(mode.to_ascii_lowercase(), "wal");
        }
        let db = Db::open(&data).unwrap();
        assert_eq!(db.cache_len().unwrap(), 1);
        assert!(data.join("redpen.db").is_file());
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = Connection::open(dir.path().join("redpen.db")).unwrap();
            conn.execute_batch("PRAGMA user_version = 2").unwrap();
        }
        let err = Db::open(dir.path()).unwrap_err();
        assert!(err.to_string().contains("newer Redpen"), "{err}");
    }

    #[test]
    fn cache_rows_round_trip_and_respect_their_age() {
        let db = Db::open_in_memory().unwrap();
        let mut r = row("k", 1_000);
        r.charge_micros = Some(210);
        db.cache_put(&r).unwrap();
        assert_eq!(db.cache_get("k", 0).unwrap(), Some(r.clone()));
        assert_eq!(db.cache_get("k", 1_000).unwrap(), Some(r));
        assert_eq!(db.cache_get("k", 1_001).unwrap(), None);
        assert_eq!(db.cache_get("other", 0).unwrap(), None);
        db.cache_put(&row("k", 2_000)).unwrap();
        assert_eq!(db.cache_len().unwrap(), 1);
        db.cache_clear().unwrap();
        assert_eq!(db.cache_len().unwrap(), 0);
    }

    #[test]
    fn usage_adds_up_per_day_provider_and_feature() {
        let db = Db::open_in_memory().unwrap();
        db.usage_add("2026-11-12", "publik", "fix-grammar", 1, 210)
            .unwrap();
        db.usage_add("2026-11-12", "publik", "fix-grammar", 1, 180)
            .unwrap();
        db.usage_add("2026-11-12", "publik", "clearer", 1, 0)
            .unwrap();
        db.usage_add("2026-11-13", "publik", "fix-grammar", 1, 0)
            .unwrap();
        let u = db
            .usage("2026-11-12", "publik", "fix-grammar")
            .unwrap()
            .unwrap();
        assert_eq!((u.calls, u.charge_micros), (2, 390));
        assert_eq!(
            db.usage("2026-11-12", "local", "fix-grammar").unwrap(),
            None
        );
        assert_eq!(db.usage_rows().unwrap().len(), 3);
    }

    #[test]
    fn sweep_deletes_old_rows_only() {
        let db = Db::open_in_memory().unwrap();
        let now = 100 * DAY_MS;
        db.cache_put(&row("old", now - 8 * DAY_MS)).unwrap();
        db.cache_put(&row("edge", now - 7 * DAY_MS)).unwrap();
        db.cache_put(&row("new", now - 6 * DAY_MS)).unwrap();
        db.sent_log_add(
            now - 8 * DAY_MS,
            "own_key",
            "/chat/completions",
            "clearer",
            "m",
            "a",
            1,
        )
        .unwrap();
        db.sent_log_add(
            now - DAY_MS,
            "own_key",
            "/chat/completions",
            "clearer",
            "m",
            "b",
            1,
        )
        .unwrap();
        let swept = db.sweep(now, 7).unwrap();
        assert_eq!(
            swept,
            Swept {
                ai_cache: 1,
                sent_log: 1
            }
        );
        assert!(db.cache_get("old", 0).unwrap().is_none());
        assert!(db.cache_get("edge", 0).unwrap().is_some());
        assert!(db.cache_get("new", 0).unwrap().is_some());
        let log = db.sent_log_list().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].sent_text, "b");
        // 0 days keeps nothing on disk.
        assert_eq!(db.sweep(now, 0).unwrap().ai_cache, 2);
    }

    #[test]
    fn the_sent_log_lists_newest_first_and_clears() {
        let db = Db::open_in_memory().unwrap();
        db.sent_log_add(1, "local", "/chat/completions", "shorter", "m", "first", 5)
            .unwrap();
        db.sent_log_add(2, "local", "/chat/completions", "shorter", "m", "second", 6)
            .unwrap();
        let log = db.sent_log_list().unwrap();
        assert_eq!(
            log.iter().map(|e| e.sent_text.as_str()).collect::<Vec<_>>(),
            ["second", "first"]
        );
        assert_eq!(log[0].bytes, 6);
        db.sent_log_clear().unwrap();
        assert!(db.sent_log_list().unwrap().is_empty());
    }

    #[test]
    fn local_days_follow_the_offset() {
        // Reference day numbers from Python's `datetime.date` arithmetic: 2026-11-12 is 20,769
        // days after 1970-01-01, 2024-02-29 is 19,782 and 2000-03-01 is 11,017.
        let midnight = 20_769 * DAY_MS;
        assert_eq!(local_day(midnight, 0), "2026-11-12");
        assert_eq!(local_day(midnight - 1, 0), "2026-11-11");
        assert_eq!(local_day(midnight - 1, 60), "2026-11-12");
        assert_eq!(local_day(midnight, -300), "2026-11-11");
        assert_eq!(local_day(0, 0), "1970-01-01");
        assert_eq!(local_day(19_782 * DAY_MS, 0), "2024-02-29");
        assert_eq!(local_day(11_017 * DAY_MS, 0), "2000-03-01");
        assert_eq!(local_day(-DAY_MS, 0), "1969-12-31");
    }
}
