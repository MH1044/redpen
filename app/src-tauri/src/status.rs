//! The publik status (docs/PUBLIK-CONTRACT.md §7, rule A5 of docs/PUBLIK-INTEGRATION.md §2):
//! the balance, this week's usage and budget, the claim state and the last call's charge, read
//! from the `x-publik-*` headers of every publik API response and from `GET /wallet`, kept in
//! memory and in `<data dir>/status.json`.
//!
//! The status never holds the key (invariant 6): it is built from response headers and bodies
//! only, and `status.json` is the [`PublikStatus`] as it is. Writing it never fails the user's
//! action. A 402 sets `needs_credit` and keeps the response's one link for
//! `open_account_link("top_up")`; the next success clears both. A `Disconnected` error sets
//! `disconnected`; the next success clears it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use redpen_core::clock::Clock;
use redpen_core::net::HttpResponse;
use redpen_core::provider::{AiError, ClaimState, Feature, ProviderKind, SearchError, StatusSink};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The status file's name in the data directory.
pub const STATUS_FILE: &str = "status.json";

/// The eight status headers of contract §7, in its order.
pub const STATUS_HEADERS: [&str; 8] = [
    "x-publik-balance",
    "x-publik-charge-micros",
    "x-publik-request-id",
    "x-publik-claim-state",
    "x-publik-starter-remaining",
    "x-publik-week-used",
    "x-publik-week-budget",
    "x-publik-week-resets-at",
];

/// What the interface and the tray know about publik API. No key, ever.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublikStatus {
    /// Always `publik`.
    pub provider: String,
    pub claim_state: ClaimState,
    /// `x-publik-balance`: the balance after the last call, in micro-dollars.
    pub balance_micros: i64,
    /// `x-publik-charge-micros`: what the last call cost.
    pub last_charge_micros: Option<u64>,
    /// `x-publik-request-id`.
    pub last_request_id: Option<String>,
    /// `x-publik-starter-remaining`.
    pub starter_remaining_micros: i64,
    /// `x-publik-week-used`.
    pub week_used_micros: i64,
    /// `x-publik-week-budget`, when there is a weekly budget.
    pub week_budget_micros: Option<i64>,
    /// `x-publik-week-resets-at`, as publik writes it.
    pub week_resets_at: Option<String>,
    /// A 402 was the last answer; cleared by the next success.
    pub needs_credit: bool,
    /// publik API disconnected this computer (401, or 403 `install_revoked`).
    pub disconnected: bool,
    pub updated_at_ms: i64,
}

impl PublikStatus {
    /// A status with this claim state and balance and nothing else known yet.
    pub fn new(claim_state: ClaimState, balance_micros: i64, now_ms: i64) -> Self {
        Self {
            provider: "publik".into(),
            claim_state,
            balance_micros,
            last_charge_micros: None,
            last_request_id: None,
            starter_remaining_micros: 0,
            week_used_micros: 0,
            week_budget_micros: None,
            week_resets_at: None,
            needs_credit: false,
            disconnected: false,
            updated_at_ms: now_ms,
        }
    }

    /// Reads every status header present (names compared without regard to case) into the
    /// status, and returns the call's charge (`x-publik-charge-micros`). A header that cannot
    /// be read leaves its field as it was.
    pub fn apply_headers(&mut self, headers: &[(String, String)]) -> Option<u64> {
        let get = |name: &str| {
            headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.trim())
                .filter(|v| !v.is_empty())
        };
        if let Some(v) = get("x-publik-balance").and_then(parse_micros) {
            self.balance_micros = v;
        }
        let charge = get("x-publik-charge-micros").and_then(|v| v.parse::<u64>().ok());
        if charge.is_some() {
            self.last_charge_micros = charge;
        }
        if let Some(v) = get("x-publik-request-id") {
            self.last_request_id = Some(v.to_string());
        }
        if let Some(v) = get("x-publik-claim-state").and_then(ClaimState::parse) {
            self.claim_state = v;
        }
        if let Some(v) = get("x-publik-starter-remaining").and_then(parse_micros) {
            self.starter_remaining_micros = v;
        }
        if let Some(v) = get("x-publik-week-used").and_then(parse_micros) {
            self.week_used_micros = v;
        }
        if let Some(v) = get("x-publik-week-budget").and_then(parse_micros) {
            self.week_budget_micros = Some(v);
        }
        if let Some(v) = get("x-publik-week-resets-at") {
            self.week_resets_at = Some(v.to_string());
        }
        charge
    }

    /// Reads a `GET /wallet` body: the fields of the status it carries, at the top level or
    /// under `wallet`. Fields it does not carry stay as the headers left them.
    pub fn apply_wallet(&mut self, body: &Value) {
        let wallet = body.get("wallet").filter(|w| w.is_object()).unwrap_or(body);
        let micros = |name: &str| wallet.get(name).and_then(json_micros);
        if let Some(v) = micros("balance_micros") {
            self.balance_micros = v;
        }
        if let Some(v) = wallet
            .get("claim_state")
            .and_then(Value::as_str)
            .and_then(ClaimState::parse)
        {
            self.claim_state = v;
        }
        if let Some(v) = micros("starter_remaining_micros") {
            self.starter_remaining_micros = v;
        }
        if let Some(v) = micros("week_used_micros") {
            self.week_used_micros = v;
        }
        if let Some(v) = micros("week_budget_micros") {
            self.week_budget_micros = Some(v);
        }
        if let Some(v) = wallet.get("week_resets_at").and_then(Value::as_str) {
            self.week_resets_at = Some(v.to_string());
        }
    }
}

/// A money header in micro-dollars. A whole number is micro-dollars (`3120000`); a number with
/// a decimal point is read as dollars (`3.12` is 3,120,000), in integer arithmetic, to the
/// micro-dollar, in case the balance header turns out to be in dollars (to be checked on the
/// maintainer's first real run).
pub fn parse_micros(value: &str) -> Option<i64> {
    let value = value.trim();
    let (negative, digits) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let magnitude = match digits.split_once('.') {
        None => digits.parse::<i64>().ok().filter(|_| is_digits(digits))?,
        Some((whole, fraction)) => {
            let whole_ok = whole.is_empty() || is_digits(whole);
            if !whole_ok || !is_digits(fraction) {
                return None;
            }
            let whole: i64 = if whole.is_empty() {
                0
            } else {
                whole.parse().ok()?
            };
            let mut micros_part = String::from(&fraction[..fraction.len().min(6)]);
            while micros_part.len() < 6 {
                micros_part.push('0');
            }
            whole
                .checked_mul(1_000_000)?
                .checked_add(micros_part.parse::<i64>().ok()?)?
        }
    };
    Some(if negative { -magnitude } else { magnitude })
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// A wallet body's money field: a JSON integer, or a string as [`parse_micros`] reads it.
fn json_micros(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str().and_then(parse_micros))
}

/// Called with the new status (or `None` once it is cleared) whenever it changes.
pub type StatusListener = Box<dyn Fn(Option<&PublikStatus>) + Send + Sync>;

/// The status in memory and in `status.json`, the pending 402 link, and who to tell when they
/// change. Share it behind an `Arc`.
pub struct StatusStore {
    path: PathBuf,
    clock: Arc<dyn Clock>,
    current: Mutex<Option<PublikStatus>>,
    top_up: Mutex<Option<String>>,
    listeners: Mutex<Vec<StatusListener>>,
}

impl std::fmt::Debug for StatusStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatusStore")
            .field("current", &self.current())
            .finish_non_exhaustive()
    }
}

impl StatusStore {
    /// An empty store writing `<data_dir>/status.json`.
    pub fn new(data_dir: &Path, clock: Arc<dyn Clock>) -> Self {
        Self {
            path: data_dir.join(STATUS_FILE),
            clock,
            current: Mutex::new(None),
            top_up: Mutex::new(None),
            listeners: Mutex::new(Vec::new()),
        }
    }

    /// A store starting from `<data_dir>/status.json` as the last run left it, if it can be
    /// read, so the tray shows the balance before `GET /wallet` answers.
    pub fn load(data_dir: &Path, clock: Arc<dyn Clock>) -> Self {
        let store = Self::new(data_dir, clock);
        if let Some(status) = std::fs::read_to_string(&store.path)
            .ok()
            .and_then(|text| serde_json::from_str::<PublikStatus>(&text).ok())
        {
            *lock(&store.current) = Some(status);
        }
        store
    }

    /// Where the status is written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The status, if publik API has answered or been set up.
    pub fn current(&self) -> Option<PublikStatus> {
        lock(&self.current).clone()
    }

    /// The link of the last 402 not yet followed by a success.
    pub fn pending_top_up(&self) -> Option<String> {
        lock(&self.top_up).clone()
    }

    /// Calls `listener` on every change from now on.
    pub fn subscribe(&self, listener: StatusListener) {
        lock(&self.listeners).push(listener);
    }

    /// Replaces the status (after provisioning), writes it and tells the listeners.
    pub fn replace(&self, mut status: PublikStatus) {
        status.updated_at_ms = self.clock.now_ms();
        *lock(&self.current) = Some(status.clone());
        self.saved(Some(&status));
    }

    /// Forgets the status (on Disconnect): removes `status.json` and the pending link.
    pub fn clear(&self) {
        *lock(&self.current) = None;
        *lock(&self.top_up) = None;
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => tracing::warn!("status.json could not be removed"),
        }
        self.notify(None);
    }

    /// Reads one publik API response: its status headers, and its status code (a 2xx clears
    /// `needs_credit`, `disconnected` and the pending link; a 402 sets `needs_credit`). Returns
    /// the call's charge for `usage_daily`.
    pub fn on_response(&self, code: u16, headers: &[(String, String)]) -> Option<u64> {
        let (status, charge) = self.update(|s| {
            let charge = s.apply_headers(headers);
            match code {
                200..=299 => {
                    s.needs_credit = false;
                    s.disconnected = false;
                }
                402 => s.needs_credit = true,
                _ => {}
            }
            charge
        });
        if (200..=299).contains(&code) {
            *lock(&self.top_up) = None;
        }
        self.saved(Some(&status));
        charge
    }

    /// Reads a `GET /wallet` answer: its headers, then its body.
    pub fn on_wallet(&self, resp: &HttpResponse) -> PublikStatus {
        let body = resp.json();
        let (status, _) = self.update(|s| {
            s.apply_headers(&resp.headers);
            if let Some(body) = &body {
                s.apply_wallet(body);
            }
            if (200..=299).contains(&resp.status) {
                s.disconnected = false;
            }
        });
        self.saved(Some(&status));
        status
    }

    /// Keeps what a failed chat call means for the status: a 402's link, or a disconnection.
    pub fn note_error(&self, e: &AiError) {
        match e {
            AiError::NeedsCredit { top_up_url, .. } => self.needs_credit(top_up_url.clone()),
            AiError::Disconnected => self.disconnected(),
            _ => {}
        }
    }

    /// As [`StatusStore::note_error`], for a web search.
    pub fn note_search_error(&self, e: &SearchError) {
        match e {
            SearchError::NeedsCredit { top_up_url, .. } => self.needs_credit(top_up_url.clone()),
            SearchError::Disconnected => self.disconnected(),
            _ => {}
        }
    }

    fn needs_credit(&self, top_up_url: Option<String>) {
        *lock(&self.top_up) = top_up_url;
        let (status, _) = self.update(|s| s.needs_credit = true);
        self.saved(Some(&status));
    }

    fn disconnected(&self) {
        let (status, _) = self.update(|s| s.disconnected = true);
        self.saved(Some(&status));
    }

    /// Applies `change` to the status (a new one if there is none), stamps the time, and
    /// returns the result.
    fn update<T>(&self, change: impl FnOnce(&mut PublikStatus) -> T) -> (PublikStatus, T) {
        let now = self.clock.now_ms();
        let mut current = lock(&self.current);
        let status =
            current.get_or_insert_with(|| PublikStatus::new(ClaimState::Anonymous, 0, now));
        let out = change(status);
        status.updated_at_ms = now;
        (status.clone(), out)
    }

    /// Writes `status.json` (never failing the caller) and tells the listeners.
    fn saved(&self, status: Option<&PublikStatus>) {
        if let Some(status) = status {
            let mut text = serde_json::to_string_pretty(status).expect("the status serialises");
            text.push('\n');
            if redpen_core::store::write_atomic(&self.path, text.as_bytes(), false).is_err() {
                tracing::warn!("status.json could not be written");
            }
        }
        self.notify(status);
    }

    fn notify(&self, status: Option<&PublikStatus>) {
        for listener in lock(&self.listeners).iter() {
            listener(status);
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The [`StatusSink`] for publik API: every publik response's headers go into the
/// [`StatusStore`], and its charge into `usage_daily`. Other providers' responses are ignored.
#[derive(Debug, Clone)]
pub struct PublikStatusSink {
    store: Arc<StatusStore>,
}

impl PublikStatusSink {
    pub fn new(store: Arc<StatusStore>) -> Self {
        Self { store }
    }

    /// The store this sink writes to.
    pub fn store(&self) -> &Arc<StatusStore> {
        &self.store
    }

    /// The current status.
    pub fn status(&self) -> Option<PublikStatus> {
        self.store.current()
    }
}

impl StatusSink for PublikStatusSink {
    fn on_response(
        &self,
        kind: ProviderKind,
        _feature: Feature,
        status: u16,
        headers: &[(String, String)],
    ) -> Option<u64> {
        if kind != ProviderKind::Publik {
            return None;
        }
        self.store.on_response(status, headers)
    }

    fn on_error(&self, kind: ProviderKind, error: &AiError) {
        if kind == ProviderKind::Publik {
            self.store.note_error(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redpen_core::clock::FakeClock;
    use serde_json::json;

    const NOW: i64 = 1_790_000_000_000;

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    }

    fn all_eight() -> Vec<(String, String)> {
        headers(&[
            ("x-publik-balance", "3120000"),
            ("x-publik-charge-micros", "210"),
            ("x-publik-request-id", "req_abc"),
            ("x-publik-claim-state", "claimed"),
            ("x-publik-starter-remaining", "40000"),
            ("x-publik-week-used", "4200"),
            ("x-publik-week-budget", "2000000"),
            ("x-publik-week-resets-at", "2026-11-16T00:00:00Z"),
        ])
    }

    fn store() -> (tempfile::TempDir, Arc<StatusStore>) {
        let dir = tempfile::tempdir().unwrap();
        let s = Arc::new(StatusStore::new(dir.path(), Arc::new(FakeClock::at(NOW))));
        (dir, s)
    }

    #[test]
    fn every_status_header_is_read() {
        let mut s = PublikStatus::new(ClaimState::Anonymous, 0, NOW);
        assert_eq!(s.apply_headers(&all_eight()), Some(210));
        assert_eq!(
            s,
            PublikStatus {
                provider: "publik".into(),
                claim_state: ClaimState::Claimed,
                balance_micros: 3_120_000,
                last_charge_micros: Some(210),
                last_request_id: Some("req_abc".into()),
                starter_remaining_micros: 40_000,
                week_used_micros: 4_200,
                week_budget_micros: Some(2_000_000),
                week_resets_at: Some("2026-11-16T00:00:00Z".into()),
                needs_credit: false,
                disconnected: false,
                updated_at_ms: NOW,
            }
        );
        // Names are compared without regard to case, and unknown values change nothing.
        let mut s = PublikStatus::new(ClaimState::Anonymous, 5, NOW);
        s.apply_headers(&headers(&[
            ("X-Publik-Balance", "7"),
            ("x-publik-claim-state", "linked"),
        ]));
        assert_eq!(
            (s.balance_micros, s.claim_state),
            (7, ClaimState::Anonymous)
        );
        assert_eq!(STATUS_HEADERS.len(), 8);
    }

    #[test]
    fn money_reads_as_micros_or_dollars() {
        assert_eq!(parse_micros("3120000"), Some(3_120_000));
        assert_eq!(parse_micros("3.12"), Some(3_120_000));
        assert_eq!(parse_micros("0.0021"), Some(2_100));
        assert_eq!(parse_micros(".5"), Some(500_000));
        assert_eq!(parse_micros("-0.01"), Some(-10_000));
        assert_eq!(parse_micros("1.2345678"), Some(1_234_567));
        assert_eq!(parse_micros("0"), Some(0));
        for bad in ["", "abc", "3.", "1e6", "3,12", "--1", "1.2.3"] {
            assert_eq!(parse_micros(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_402_sets_needs_credit_and_a_success_clears_it() {
        let (_dir, s) = store();
        s.on_response(402, &[]);
        s.note_error(&AiError::NeedsCredit {
            message: "m".into(),
            top_up_url: Some("https://publikhq.com/claim/x".into()),
        });
        assert!(s.current().unwrap().needs_credit);
        assert_eq!(
            s.pending_top_up().as_deref(),
            Some("https://publikhq.com/claim/x")
        );
        s.on_response(200, &headers(&[("x-publik-charge-micros", "210")]));
        assert!(!s.current().unwrap().needs_credit);
        assert_eq!(s.pending_top_up(), None);
    }

    #[test]
    fn a_disconnection_is_kept_until_a_success() {
        let (_dir, s) = store();
        s.note_error(&AiError::Disconnected);
        assert!(s.current().unwrap().disconnected);
        s.note_search_error(&SearchError::Busy);
        assert!(s.current().unwrap().disconnected);
        s.on_response(200, &[]);
        assert!(!s.current().unwrap().disconnected);
        s.note_search_error(&SearchError::Disconnected);
        assert!(s.current().unwrap().disconnected);
    }

    #[test]
    fn status_json_is_written_after_every_response_and_has_no_key() {
        let (dir, s) = store();
        s.on_response(200, &all_eight());
        let text = std::fs::read_to_string(dir.path().join("status.json")).unwrap();
        let written: PublikStatus = serde_json::from_str(&text).unwrap();
        assert_eq!(Some(written), s.current());
        let value: Value = serde_json::from_str(&text).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        // docs/ARCHITECTURE.md §4, status.json.
        assert_eq!(
            keys,
            [
                "balance_micros",
                "claim_state",
                "disconnected",
                "last_charge_micros",
                "last_request_id",
                "needs_credit",
                "provider",
                "starter_remaining_micros",
                "updated_at_ms",
                "week_budget_micros",
                "week_resets_at",
                "week_used_micros"
            ]
        );
        let again = StatusStore::load(dir.path(), Arc::new(FakeClock::at(NOW)));
        assert_eq!(again.current(), s.current());
        s.clear();
        assert!(s.current().is_none());
        assert!(!dir.path().join("status.json").exists());
    }

    #[test]
    fn a_status_file_that_cannot_be_written_never_fails_the_call() {
        let dir = tempfile::tempdir().unwrap();
        // The data directory is a file, so nothing can be written under it.
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, "x").unwrap();
        let s = StatusStore::new(&blocked, Arc::new(FakeClock::at(NOW)));
        assert_eq!(s.on_response(200, &all_eight()), Some(210));
        assert_eq!(s.current().unwrap().balance_micros, 3_120_000);
    }

    #[test]
    fn the_wallet_body_and_headers_both_count() {
        let (_dir, s) = store();
        let resp = HttpResponse {
            status: 200,
            headers: headers(&[("x-publik-week-used", "4200")]),
            body: serde_json::to_vec(&json!({"wallet": {"balance_micros": 50000,
                "claim_state": "claimed", "week_budget_micros": 2000000,
                "week_resets_at": "2026-11-16T00:00:00Z"}}))
            .unwrap(),
        };
        let status = s.on_wallet(&resp);
        assert_eq!(status.balance_micros, 50_000);
        assert_eq!(status.claim_state, ClaimState::Claimed);
        assert_eq!(status.week_used_micros, 4_200);
        assert_eq!(status.week_budget_micros, Some(2_000_000));
        let flat = HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: br#"{"balance_micros": "3.12"}"#.to_vec(),
        };
        assert_eq!(s.on_wallet(&flat).balance_micros, 3_120_000);
    }

    #[test]
    fn listeners_hear_every_change() {
        let (_dir, s) = store();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = heard.clone();
        s.subscribe(Box::new(move |status| {
            sink.lock()
                .unwrap()
                .push(status.map(|st| st.balance_micros))
        }));
        s.on_response(200, &headers(&[("x-publik-balance", "100")]));
        s.replace(PublikStatus::new(ClaimState::Claimed, 7, 0));
        s.clear();
        assert_eq!(*heard.lock().unwrap(), vec![Some(100), Some(7), None]);
    }

    #[test]
    fn the_sink_reads_publik_responses_only() {
        let (_dir, s) = store();
        let sink = PublikStatusSink::new(s.clone());
        assert_eq!(
            sink.on_response(ProviderKind::OwnKey, Feature::FixGrammar, 200, &all_eight()),
            None
        );
        assert!(sink.status().is_none());
        assert_eq!(
            sink.on_response(ProviderKind::Publik, Feature::FixGrammar, 200, &all_eight()),
            Some(210)
        );
        assert_eq!(sink.status().unwrap().balance_micros, 3_120_000);
        sink.on_error(ProviderKind::Local, &AiError::Disconnected);
        assert!(!sink.status().unwrap().disconnected);
        sink.on_error(ProviderKind::Publik, &AiError::Disconnected);
        assert!(sink.status().unwrap().disconnected);
        assert!(Arc::ptr_eq(sink.store(), &s));
    }
}
