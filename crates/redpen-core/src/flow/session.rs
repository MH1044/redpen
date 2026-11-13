//! The popup's sessions (docs/modules/06-popup-and-settings-ui.md): one per press of the check
//! shortcut, holding the captured text in memory by id until the popup closes.
//!
//! [`Sessions`] runs the whole rewrite flow of docs/ARCHITECTURE.md §3 against traits, so tests
//! drive it end to end with [`crate::platform::FakeCapture`] and [`crate::net::FakeTransport`]:
//!
//! - [`Sessions::open`]: the capture ([`super::capture_session`]), then Harper on this computer.
//!   Nothing is sent anywhere.
//! - [`Sessions::apply_fix`]: one suggestion applied, then the text checked again.
//! - [`Sessions::rewrite`]: one of the six actions through the [`AiClient`] on the provider the
//!   [`ProviderResolver`] gives now, with redaction as Settings → Privacy says, and the session's
//!   own count of answers that failed validation ([`ValidationRetries`]), so a Try again after an
//!   unusable answer is a new request and any other Try again repeats the same one.
//! - [`Sessions::accept`]: the result (or the fixed text) typed into the window the selection came
//!   from ([`super::deliver()`]), remembered for Undo.
//! - [`Sessions::copy_result`], [`Sessions::undo`].
//! - [`Sessions::close`]: the text is dropped from memory.
//!
//! Each call that reads a setting takes the settings in force from the caller (the app keeps one
//! copy that Settings changes), so a change applies from the next action on.
//!
//! Ids are counters (`s1`, `r1`), so nothing here reads randomness, and no log line carries the
//! text, a result, a key or the process name (docs/ARCHITECTURE.md §8).

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;

use super::deliver::{UndoBuffer, UndoOutcome, deliver};
use super::rewrite::{Captured, capture_session};
use crate::check::{CheckError, CheckOptions, Checker, Dialect, Finding};
use crate::clock::{Clock, Sleeper};
use crate::diff::DiffOp;
use crate::platform::{Capture, CaptureError, Point, Rect, WindowRef};
use crate::provider::{AiClient, AiError, ProviderKind, ProviderResolver, ResolvedProvider};
use crate::rewrite::{
    Action, RewriteError, RewriteRequest, Style, ValidationRetries, Warning, provider_for,
};
use crate::store::Settings;

/// What the popup shows for a session: the text as it is now (after any fixes), Harper's
/// findings for it, and the dialect they were checked in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Session {
    pub id: String,
    pub text: String,
    pub findings: Vec<Finding>,
    pub dialect: Dialect,
    /// False when the text is reliably in another language ([`crate::lang::detect`]).
    pub english: bool,
    /// What the loading view names for each action ("Rewriting… (publik-balanced)"): the tier on
    /// publik API, "Your own key · <model>", "Local · <model>". Empty while no provider is set
    /// up. Never a key.
    pub provider_lines: BTreeMap<Action, String>,
    /// Whether the last Accept can still be undone.
    pub can_undo: bool,
}

/// A rewrite's result, as the popup shows it: [`crate::rewrite::RewriteOutcome`] with the id
/// Accept and Copy name it by, and the provider that answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RewriteResult {
    pub result_id: String,
    pub text: String,
    pub diff: Vec<DiffOp>,
    pub warnings: Vec<Warning>,
    pub accept_default: bool,
    pub unchanged: bool,
    pub from_cache: bool,
    pub provider: ProviderKind,
    /// The footer's first part: the tier on publik API, "Your own key · <model>" or
    /// "Local · <model>".
    pub provider_line: String,
    /// What publik API charged, in micro-dollars; `None` from the cache and off publik API.
    pub charge_micros: Option<u64>,
}

/// Why a session command stopped. `Display` is the message the interface shows; the interface
/// receives `{"kind", "message"}` plus the error's fields, the wrapped errors exactly as their own
/// types serialise.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// The session was closed (or never opened): its text is gone from memory.
    #[error("This text is no longer open in Redpen. Select it and press the shortcut again.")]
    SessionGone,
    /// Accept or Copy named a result this session does not have.
    #[error("That result is no longer available. Run the action again.")]
    ResultGone,
    #[error(transparent)]
    Check(#[from] CheckError),
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error(transparent)]
    Rewrite(#[from] RewriteError),
}

impl From<AiError> for SessionError {
    fn from(e: AiError) -> Self {
        SessionError::Rewrite(RewriteError::Ai(e))
    }
}

impl SessionError {
    /// The error's name: `SessionGone`, `ResultGone`, or the wrapped error's kind.
    pub fn kind(&self) -> &'static str {
        match self {
            SessionError::SessionGone => "SessionGone",
            SessionError::ResultGone => "ResultGone",
            SessionError::Check(e) => e.kind(),
            SessionError::Capture(e) => e.kind(),
            SessionError::Rewrite(e) => e.kind(),
        }
    }

    /// `{"kind", "message"}` plus the error's fields.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("an error serialises")
    }
}

impl Serialize for SessionError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            SessionError::Check(e) => e.serialize(serializer),
            SessionError::Capture(e) => e.serialize(serializer),
            SessionError::Rewrite(e) => e.serialize(serializer),
            SessionError::SessionGone | SessionError::ResultGone => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", self.kind())?;
                map.serialize_entry("message", &self.to_string())?;
                map.end()
            }
        }
    }
}

/// The footer's (and the loading view's) name for a provider: the model on publik API (a tier)
/// and the fake provider, "Your own key · <model>", "Local · <model>" (or "Local" while Ollama
/// picks the model at the time of the call).
pub fn provider_line(p: &ResolvedProvider) -> String {
    let model = p.chat_model.trim();
    match p.kind {
        ProviderKind::Publik | ProviderKind::Fake => model.to_string(),
        ProviderKind::OwnKey => format!("Your own key · {model}"),
        ProviderKind::Local if model.is_empty() => "Local".to_string(),
        ProviderKind::Local => format!("Local · {model}"),
    }
}

/// One open session.
struct Entry {
    text: String,
    findings: Vec<Finding>,
    dialect: Dialect,
    english: bool,
    window: WindowRef,
    /// The selection as captured, which Undo puts back.
    original: String,
    caret: Option<Rect>,
    cursor: Point,
    /// Every result this session got, by id.
    results: Vec<(String, String)>,
    retries: Arc<ValidationRetries>,
}

/// The open sessions, the checker and the undo buffer.
pub struct Sessions {
    capture: Arc<dyn Capture>,
    ai: Arc<AiClient>,
    resolver: Arc<dyn ProviderResolver>,
    clock: Arc<dyn Clock>,
    sleeper: Arc<dyn Sleeper>,
    checker: Mutex<Option<Arc<Checker>>>,
    entries: Mutex<HashMap<String, Entry>>,
    next_session: AtomicU64,
    next_result: AtomicU64,
    undo: UndoBuffer,
}

impl std::fmt::Debug for Sessions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sessions")
            .field("open", &self.lock().len())
            .finish_non_exhaustive()
    }
}

impl Sessions {
    /// Sessions over these parts, with no session open.
    pub fn new(
        capture: Arc<dyn Capture>,
        ai: Arc<AiClient>,
        resolver: Arc<dyn ProviderResolver>,
        clock: Arc<dyn Clock>,
        sleeper: Arc<dyn Sleeper>,
    ) -> Self {
        Self {
            capture,
            ai,
            resolver,
            clock,
            sleeper,
            checker: Mutex::new(None),
            entries: Mutex::new(HashMap::new()),
            next_session: AtomicU64::new(1),
            next_result: AtomicU64::new(1),
            undo: UndoBuffer::new(),
        }
    }

    /// The chat client the rewrites go through.
    pub fn ai(&self) -> &Arc<AiClient> {
        &self.ai
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Harper for the dialect and categories of `s`, built once and kept while they stay the
    /// same.
    fn checker(&self, s: &Settings) -> Arc<Checker> {
        let opts = CheckOptions {
            dialect: s.check.dialect,
            disabled: s.check.disabled_categories.clone(),
        };
        let mut slot = self.checker.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_ref() {
            Some(c) if *c.options() == opts => c.clone(),
            _ => {
                let c = Arc::new(Checker::new(opts));
                *slot = Some(c.clone());
                c
            }
        }
    }

    /// Builds the checker for `settings` ahead of the next shortcut press, so the first popup
    /// after a start or a change of dialect or categories is as quick as the others.
    pub fn warm(&self, settings: &Settings) {
        let _ = self.checker(settings);
    }

    /// The shortcut: reads the selection in the window in front (with the refusals, the key wait
    /// and the copy timeout of `settings`), checks it with Harper in the dialect and categories
    /// of `settings`, and opens a session. Nothing leaves the computer.
    pub async fn open(&self, settings: &Settings) -> Result<Session, SessionError> {
        let captured = capture_session(
            self.capture.as_ref(),
            settings,
            self.clock.as_ref(),
            self.sleeper.as_ref(),
        )
        .await?;
        self.open_captured(captured, settings)
    }

    /// Opens a session on a selection that was already captured: Harper's check, then the
    /// session is kept by its new id.
    pub fn open_captured(&self, c: Captured, settings: &Settings) -> Result<Session, SessionError> {
        let s = settings;
        let checker = self.checker(s);
        let findings = checker.check(&c.text)?;
        let english = crate::lang::detect(&c.text).is_none_or(|lang| lang == "English");
        let id = format!("s{}", self.next_session.fetch_add(1, Ordering::SeqCst));
        tracing::info!(
            bytes = c.text.len(),
            findings = findings.len(),
            english,
            "session_opened"
        );
        let entry = Entry {
            text: c.text.clone(),
            findings,
            dialect: s.check.dialect,
            english,
            window: c.window,
            original: c.text,
            caret: c.caret,
            cursor: c.cursor,
            results: Vec::new(),
            retries: Arc::new(ValidationRetries::new()),
        };
        let session = self.view(&id, &entry);
        self.lock().insert(id, entry);
        Ok(session)
    }

    /// The session as the popup shows it now.
    pub fn get(&self, id: &str) -> Result<Session, SessionError> {
        let entries = self.lock();
        let entry = entries.get(id).ok_or(SessionError::SessionGone)?;
        Ok(self.view(id, entry))
    }

    /// Where the selection was: the caret, when the app showed one, and the mouse.
    pub fn anchor(&self, id: &str) -> Option<(Option<Rect>, Point)> {
        self.lock().get(id).map(|e| (e.caret, e.cursor))
    }

    /// Whether `id` is open.
    pub fn is_open(&self, id: &str) -> bool {
        self.lock().contains_key(id)
    }

    /// How many sessions are open.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether no session is open.
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Closes `id`: its text, results and retry counts are dropped from memory. True when it was
    /// open.
    pub fn close(&self, id: &str) -> bool {
        let closed = self.lock().remove(id).is_some();
        if closed {
            tracing::info!("session_closed");
        }
        closed
    }

    /// Closes every session.
    pub fn close_all(&self) {
        self.lock().clear();
    }

    fn view(&self, id: &str, e: &Entry) -> Session {
        Session {
            id: id.to_string(),
            text: e.text.clone(),
            findings: e.findings.clone(),
            dialect: e.dialect,
            english: e.english,
            provider_lines: self.provider_lines(),
            can_undo: self.undo.can_undo(self.clock.as_ref()),
        }
    }

    /// The loading line of each action on the provider the resolver gives now; empty while none
    /// is set up.
    fn provider_lines(&self) -> BTreeMap<Action, String> {
        let Ok(p) = self.resolver.chat() else {
            return BTreeMap::new();
        };
        let ctx = self.resolver.tier_context();
        Action::ALL
            .iter()
            .map(|&a| (a, provider_line(&provider_for(&p, &ctx, a))))
            .collect()
    }

    /// Applies suggestion `suggestion_index` of finding `finding_id` and checks the text again,
    /// in the dialect and categories of `settings`. A finding the session no longer has is
    /// [`CheckError::StaleFinding`]; a refused fix leaves the text as it was.
    pub fn apply_fix(
        &self,
        id: &str,
        finding_id: &str,
        suggestion_index: usize,
        settings: &Settings,
    ) -> Result<Session, SessionError> {
        let checker = self.checker(settings);
        let mut entries = self.lock();
        let entry = entries.get_mut(id).ok_or(SessionError::SessionGone)?;
        let finding = entry
            .findings
            .iter()
            .find(|f| f.id == finding_id)
            .ok_or(CheckError::StaleFinding)?;
        let text = checker.apply(&entry.text, finding, suggestion_index)?;
        let findings = checker.check(&text)?;
        tracing::info!(findings = findings.len(), "fix_applied");
        entry.text = text;
        entry.findings = findings;
        entry.dialect = checker.options().dialect;
        let entry = &*entry;
        Ok(self.view(id, entry))
    }

    /// Runs `action` on the session's text (after any fixes), in the dialect the session was
    /// checked in, with redaction as `settings` say. `variant` counts presses of Another
    /// version; a Try again sends the same arguments again and the session sets the validation
    /// retry. The author's own style until style presets arrive (module 08).
    pub async fn rewrite(
        &self,
        id: &str,
        action: Action,
        instruction: Option<String>,
        variant: u32,
        settings: &Settings,
    ) -> Result<RewriteResult, SessionError> {
        let (text, dialect, retries) = {
            let entries = self.lock();
            let entry = entries.get(id).ok_or(SessionError::SessionGone)?;
            (entry.text.clone(), entry.dialect, entry.retries.clone())
        };
        let provider = self.resolver.chat()?;
        let ctx = self.resolver.tier_context();
        let request = RewriteRequest {
            action,
            text,
            instruction,
            dialect,
            style: Style::default(),
            redact: settings.privacy.redaction,
            variant,
            validation_retry: 0,
        };
        let outcome = retries
            .rewrite(self.ai.as_ref(), &provider, &ctx, request)
            .await?;
        let result_id = format!("r{}", self.next_result.fetch_add(1, Ordering::SeqCst));
        let mut entries = self.lock();
        // Closed while the call ran: the answer is dropped with the session.
        let entry = entries.get_mut(id).ok_or(SessionError::SessionGone)?;
        entry
            .results
            .push((result_id.clone(), outcome.text.clone()));
        Ok(RewriteResult {
            result_id,
            text: outcome.text,
            diff: outcome.diff,
            warnings: outcome.warnings,
            accept_default: outcome.accept_default,
            unchanged: outcome.unchanged,
            from_cache: outcome.from_cache,
            provider: provider.kind,
            provider_line: provider_line(&provider_for(&provider, &ctx, action)),
            charge_micros: outcome.charge_micros,
        })
    }

    /// The text Accept or Copy uses: result `result_id`, or the session's text (after any fixes)
    /// when no result is named.
    fn chosen(
        &self,
        id: &str,
        result_id: Option<&str>,
    ) -> Result<(WindowRef, String, String), SessionError> {
        let entries = self.lock();
        let entry = entries.get(id).ok_or(SessionError::SessionGone)?;
        let text = match result_id {
            None => entry.text.clone(),
            Some(r) => entry
                .results
                .iter()
                .find(|(rid, _)| rid == r)
                .map(|(_, t)| t.clone())
                .ok_or(SessionError::ResultGone)?,
        };
        Ok((entry.window, entry.original.clone(), text))
    }

    /// Accept: types the chosen text into the window the selection came from, after the checks
    /// of [`deliver()`] (with the blocklist and paste delay of `settings`), and remembers the
    /// replace for Undo. The session stays open; the caller closes it.
    pub async fn accept(
        &self,
        id: &str,
        result_id: Option<&str>,
        settings: &Settings,
    ) -> Result<(), SessionError> {
        let (window, original, text) = self.chosen(id, result_id)?;
        deliver(
            self.capture.as_ref(),
            &window,
            &text,
            settings,
            self.sleeper.as_ref(),
        )
        .await?;
        self.undo.remember(window, original, self.clock.as_ref());
        Ok(())
    }

    /// Copy: puts the chosen text on the clipboard as an ordinary entry.
    pub async fn copy_result(&self, id: &str, result_id: Option<&str>) -> Result<(), SessionError> {
        let (_, _, text) = self.chosen(id, result_id)?;
        self.capture.set_clipboard_text(&text)?;
        tracing::info!(bytes = text.len(), "copied");
        Ok(())
    }

    /// Undo: takes back the last Accept ([`UndoBuffer::undo`]).
    pub async fn undo(&self) -> Result<UndoOutcome, SessionError> {
        Ok(self
            .undo
            .undo(
                self.capture.as_ref(),
                self.clock.as_ref(),
                self.sleeper.as_ref(),
            )
            .await?)
    }

    /// Whether the last Accept can still be undone.
    pub fn can_undo(&self) -> bool {
        self.undo.can_undo(self.clock.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{FakeClock, FakeSleeper};
    use crate::net::{FakeEntry, FakeTransport, HttpRequest};
    use crate::platform::{FakeCapture, Foreground};
    use crate::provider::{FixedResolver, NoopStatusSink};
    use crate::store::Db;
    use serde_json::json;

    struct Fixture {
        fake: Arc<FakeCapture>,
        transport: Arc<FakeTransport>,
        sessions: Sessions,
    }

    fn fixture(selection: &str, entries: Vec<FakeEntry>, resolver: FixedResolver) -> Fixture {
        let fake = Arc::new(FakeCapture::new().with_selection(selection));
        let transport = Arc::new(FakeTransport::scripted(entries));
        let clock: Arc<dyn Clock> = Arc::new(FakeClock::at(1_000));
        let sleeper: Arc<dyn Sleeper> = Arc::new(FakeSleeper::new());
        let ai = Arc::new(AiClient::new(
            transport.clone(),
            Db::open_in_memory().unwrap(),
            clock.clone(),
            sleeper.clone(),
            Arc::new(NoopStatusSink),
        ));
        let sessions = Sessions::new(fake.clone(), ai, Arc::new(resolver), clock, sleeper);
        Fixture {
            fake,
            transport,
            sessions,
        }
    }

    fn fake_provider() -> FixedResolver {
        FixedResolver::new(ResolvedProvider::fake())
    }

    /// The fake provider's answer to the request a Clearer rewrite of `text` sends.
    fn clearer_answer(text: &str, answer: &str) -> FakeEntry {
        let body = crate::rewrite::request_body(
            &ResolvedProvider::fake(),
            &Default::default(),
            &RewriteRequest::new(Action::Clearer, text),
        )
        .unwrap();
        let req = HttpRequest::post_json("http://fake.invalid/v1/chat/completions", body);
        let content = json!({ "text": answer }).to_string();
        FakeEntry::reply(
            &req,
            200,
            json!({"choices": [{"message": {"role": "assistant", "content": content}}]}),
        )
    }

    #[tokio::test]
    async fn open_checks_and_get_returns_the_session() {
        let f = fixture("I saw teh cat.", Vec::new(), fake_provider());
        let s = f.sessions.open(&Settings::default()).await.unwrap();
        assert_eq!(s.id, "s1");
        assert_eq!(s.text, "I saw teh cat.");
        assert_eq!(s.findings.len(), 1);
        assert!(s.english);
        assert!(!s.can_undo);
        assert_eq!(s.provider_lines[&Action::FixGrammar], "publik-fast");
        assert_eq!(s.provider_lines[&Action::Clearer], "publik-balanced");
        assert_eq!(f.sessions.get("s1").unwrap(), s);
        assert!(f.transport.requests_seen().is_empty());
    }

    #[tokio::test]
    async fn a_closed_session_is_gone() {
        let f = fixture("Hello there.", Vec::new(), fake_provider());
        let s = f.sessions.open(&Settings::default()).await.unwrap();
        assert!(f.sessions.close(&s.id));
        assert!(!f.sessions.close(&s.id));
        let err = f.sessions.get(&s.id).unwrap_err();
        assert_eq!(err, SessionError::SessionGone);
        assert_eq!(
            err.to_json(),
            json!({"kind": "SessionGone",
                   "message": "This text is no longer open in Redpen. Select it and press the shortcut again."})
        );
        assert_eq!(
            f.sessions
                .apply_fix(&s.id, "x", 0, &Settings::default())
                .unwrap_err(),
            SessionError::SessionGone
        );
        assert!(f.sessions.is_empty());
    }

    #[tokio::test]
    async fn a_fix_then_accept_pastes_the_fixed_text_and_sends_nothing() {
        let f = fixture("I saw teh cat.", Vec::new(), fake_provider());
        let s = f.sessions.open(&Settings::default()).await.unwrap();
        let fixed = f
            .sessions
            .apply_fix(&s.id, &s.findings[0].id, 0, &Settings::default())
            .unwrap();
        assert_eq!(fixed.text, "I saw the cat.");
        assert!(fixed.findings.is_empty());
        f.sessions
            .accept(&s.id, None, &Settings::default())
            .await
            .unwrap();
        assert_eq!(
            f.fake.typed(),
            vec![(
                FakeCapture::default_window().window,
                "I saw the cat.".to_string()
            )]
        );
        assert!(f.transport.requests_seen().is_empty());
        assert!(f.sessions.can_undo());
        assert!(f.sessions.get(&s.id).unwrap().can_undo);
    }

    #[tokio::test]
    async fn a_stale_finding_is_refused() {
        let f = fixture("I saw teh cat.", Vec::new(), fake_provider());
        let s = f.sessions.open(&Settings::default()).await.unwrap();
        let first = s.findings[0].id.clone();
        f.sessions
            .apply_fix(&s.id, &first, 0, &Settings::default())
            .unwrap();
        assert_eq!(
            f.sessions
                .apply_fix(&s.id, &first, 0, &Settings::default())
                .unwrap_err(),
            SessionError::Check(CheckError::StaleFinding)
        );
        let s = f.sessions.open(&Settings::default()).await.unwrap();
        assert_eq!(
            f.sessions
                .apply_fix(&s.id, &s.findings[0].id, 99, &Settings::default())
                .unwrap_err(),
            SessionError::Check(CheckError::NoSuchSuggestion)
        );
    }

    #[tokio::test]
    async fn a_rewrite_result_is_accepted_and_copied_by_id() {
        let text = "The invoice should of been paid.";
        let f = fixture(
            text,
            vec![clearer_answer(text, "Please pay the invoice.")],
            fake_provider(),
        );
        let s = f.sessions.open(&Settings::default()).await.unwrap();
        let r = f
            .sessions
            .rewrite(&s.id, Action::Clearer, None, 0, &Settings::default())
            .await
            .unwrap();
        assert_eq!(r.result_id, "r1");
        assert_eq!(r.text, "Please pay the invoice.");
        assert_eq!(r.provider, ProviderKind::Fake);
        assert_eq!(r.provider_line, "publik-balanced");
        assert!(!r.from_cache);
        f.sessions.copy_result(&s.id, Some("r1")).await.unwrap();
        assert_eq!(
            f.fake.clipboard().as_deref(),
            Some("Please pay the invoice.")
        );
        assert_eq!(
            f.sessions.copy_result(&s.id, Some("r9")).await.unwrap_err(),
            SessionError::ResultGone
        );
        f.sessions
            .accept(&s.id, Some("r1"), &Settings::default())
            .await
            .unwrap();
        assert_eq!(f.fake.typed()[0].1, "Please pay the invoice.");
        assert_eq!(f.transport.requests_seen().len(), 1);
        // Undo sends one Ctrl+Z to the window.
        assert_eq!(f.sessions.undo().await.unwrap(), UndoOutcome::Undone);
    }

    #[tokio::test]
    async fn no_provider_means_needs_consent_and_nothing_sent() {
        let f = fixture(
            "Hello there.",
            Vec::new(),
            FixedResolver::failing(AiError::NeedsConsent),
        );
        let s = f.sessions.open(&Settings::default()).await.unwrap();
        assert!(s.provider_lines.is_empty());
        let err = f
            .sessions
            .rewrite(&s.id, Action::Clearer, None, 0, &Settings::default())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "NeedsConsent");
        assert!(f.transport.requests_seen().is_empty());
    }

    #[tokio::test]
    async fn a_refused_capture_opens_no_session() {
        let fake = Arc::new(
            FakeCapture::with_foreground(Foreground {
                is_password: true,
                ..FakeCapture::default_window()
            })
            .with_selection("secret"),
        );
        let f = fixture("x", Vec::new(), fake_provider());
        let sessions = Sessions::new(
            fake.clone(),
            f.sessions.ai().clone(),
            Arc::new(fake_provider()),
            Arc::new(FakeClock::at(0)),
            Arc::new(FakeSleeper::new()),
        );
        let err = sessions.open(&Settings::default()).await.unwrap_err();
        assert_eq!(err, SessionError::Capture(CaptureError::PasswordField));
        assert_eq!(err.kind(), "PasswordField");
        assert!(sessions.is_empty());
        assert_eq!(fake.count("read_selection"), 0);
    }

    #[test]
    fn provider_lines_name_the_tier_or_the_model_never_a_key() {
        assert_eq!(
            provider_line(&ResolvedProvider::own_key(
                "https://api.example.com/v1",
                "sk-test-a",
                "example-chat"
            )),
            "Your own key · example-chat"
        );
        assert_eq!(
            provider_line(&ResolvedProvider::local(
                "http://localhost:11434/v1",
                "qwen2.5:14b"
            )),
            "Local · qwen2.5:14b"
        );
        assert_eq!(
            provider_line(&ResolvedProvider::local("http://localhost:11434/v1", "")),
            "Local"
        );
        let mut publik = ResolvedProvider::fake();
        publik.kind = ProviderKind::Publik;
        publik.chat_model = "publik-fast".into();
        assert_eq!(provider_line(&publik), "publik-fast");
    }

    #[test]
    fn errors_serialise_as_the_wrapped_error() {
        assert_eq!(
            SessionError::from(AiError::Offline).to_json()["kind"],
            "Offline"
        );
        assert_eq!(
            SessionError::ResultGone.to_json(),
            json!({"kind": "ResultGone",
                   "message": "That result is no longer available. Run the action again."})
        );
        assert_eq!(
            SessionError::Capture(CaptureError::FocusChanged).kind(),
            "FocusChanged"
        );
    }
}
