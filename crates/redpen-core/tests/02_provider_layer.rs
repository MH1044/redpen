//! Module 02 tests (docs/modules/02-provider-layer.md, Acceptance tests). A test named `tNN_…`
//! covers bullet NN; a test named `e_…` covers an error path or a Scope bullet the numbered list
//! leaves implicit. Bullet 19 is the golden cases `tests/cases/02-provider-layer/fake-chat` and
//! `fake-chat-unrecorded`.
//!
//! Expected values come from the spec, docs/ARCHITECTURE.md §4 (the database schema, the
//! settings file and its defaults), docs/PUBLIK-INTEGRATION.md §3 and §6 (the local model choice,
//! the timeouts and the retry delays) and SHA-256 (FIPS 180-4) computed with `sha256sum` over the
//! canonical strings in `tests/cases/02-provider-layer/REFERENCE.md`, which also derives the
//! dates. Nothing here is taken from Redpen's own output.
//!
//! Transports: `FakeTransport::scripted` answers by request hash, with the same entry every time,
//! and has no way to fail with a transport error, so the tests of retries, timeouts, a refused
//! connection and TLS use `Script` (below): one queue of answers per path, and a log of every
//! request. A request with no answer left fails as unrecorded, so an extra retry shows.
//!
//! Databases: in memory, except where a test reads rows back; then `Db::open` gets a temporary
//! data directory and the test reads `redpen.db` through a second SQLite connection.
//!
//! The spec's Interfaces name `AiClient`, `ResolvedProvider`, `ChatCall`, `ChatReply`,
//! `StatusSink`, `pick_local_model` and `AiError`. The Scope also describes behaviour without
//! naming its function: opening the database, applying Settings → Privacy, the sweep, emptying
//! the tables, the own key's Test, the settings and secrets files, and a local provider whose
//! model is left to `pick_local_model`. Those calls are gathered in `mod api`, so a different name
//! changes one line there and no expected value.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use redpen_core::clock::{Clock, FakeClock, FakeSleeper, Sleeper};
use redpen_core::net::fake::unrecorded_path;
use redpen_core::net::{
    Body, FakeEntry, FakeTransport, HeaderValue, HttpRequest, HttpResponse, HttpTransport, Method,
    ReqwestTransport, TransportError, canonical_json, request_hash, request_path,
};
use redpen_core::provider::{
    AiClient, AiError, CachePolicy, ChatCall, ChatReply, Feature, ProviderKind, ResolvedProvider,
    StatusSink, pick_local_model,
};
use redpen_core::store::Db;
use rusqlite::{Connection, OptionalExtension, params};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

/// The fake own key of contract test 16.
const KEY: &str = "sk-test-a";
/// The own key's base URL and model, from the settings example of docs/ARCHITECTURE.md §4.
const OWN_BASE: &str = "https://api.example.com/v1";
const OWN_MODEL: &str = "example-chat";
const LOCAL_BASE: &str = "http://localhost:11434/v1";
/// The fake provider of the Design notes.
const FAKE_BASE: &str = "http://fake.invalid/v1";
const PUBLIK_BASE: &str = "https://publikhq.com/api/v1";
const CHAT: &str = "/chat/completions";
const TAGS: &str = "/api/tags";
const MODELS: &str = "/models";
const SYSTEM: &str = "You correct grammar and spelling.";
/// Every user text in this file contains `MARK`, so a log line carrying document text shows.
const TEXT: &str = "Their going to the libary.";
const MARK: &str = "libary";
/// 2026-09-21T14:13:20Z (REFERENCE.md).
const T0: i64 = 1_790_000_000_000;
const T0_DAY: &str = "2026-09-21";
const DAY_MS: i64 = 86_400_000;
/// Two more attempts after 0.5 s and 1 s (docs/PUBLIK-INTEGRATION.md §6).
const RETRY_SLEEPS: [Duration; 2] = [Duration::from_millis(500), Duration::from_millis(1_000)];

const MSG_NOT_CONFIGURED: &str = "Choose an AI provider in Settings → Providers.";
const MSG_REFUSED_401: &str = "Your key was refused (401). Check it in Settings → Providers.";
const MSG_MODEL_UNKNOWN: &str =
    "Your provider does not know the model “example-chat” (404). Check the model name in Settings.";
const MSG_RATE_LIMITED: &str =
    "Your provider says you are sending too many requests (429). Wait a moment and try again.";
const MSG_PROVIDER_503: &str = "Your provider had a problem (503). Try again later.";
const MSG_OWN_TIMEOUT: &str = "No answer from your own key's server after 30 seconds. Try again.";
const MSG_LOCAL_TIMEOUT: &str = "No answer from the local model after 120 seconds. Try again.";
const MSG_OFFLINE: &str =
    "No connection. Rewrites need a connection or a local model; checking still works.";
const MSG_NOT_RUNNING: &str = "Ollama isn't running. Start it, or choose another provider.";
const MSG_NO_MODEL: &str =
    "Ollama has no chat model installed. Install one in Ollama, or choose another provider.";
const MSG_NOT_LOCAL: &str = "A local model must run on this computer (localhost).";
const MSG_INVALID: &str = "The AI answer could not be used.";
const MSG_NEWER_SETTINGS: &str = "Settings were written by a newer Redpen.";

// ---------------------------------------------------------------------------------------------
// The calls the Scope describes without naming them. Everything else in this file uses only the
// names of the spec's Interfaces and of module 00.

mod api {
    use std::path::Path;

    use redpen_core::provider::{AiClient, AiError, Message, ProviderKind, ResolvedProvider};
    use redpen_core::store::{Db, Secrets, Settings};
    use secrecy::{ExposeSecret, SecretString};

    /// The files the stores keep in the directories they are given (docs/ARCHITECTURE.md §4).
    pub const DB_FILE: &str = "redpen.db";
    pub const SETTINGS_FILE: &str = "settings.json";
    pub const SECRETS_FILE: &str = "secrets.json";

    /// One chat message, `{"role", "content"}`.
    pub fn message(role: &str, content: &str) -> Message {
        Message {
            role: role.to_string(),
            content: content.to_string(),
        }
    }

    /// The database `redpen chat` and the tests use.
    pub fn memory_db() -> Db {
        Db::open_in_memory().expect("an in-memory database opens")
    }

    /// `redpen.db` in the data directory the caller passes.
    pub fn file_db(data_dir: &Path) -> Db {
        Db::open(data_dir).expect("redpen.db opens in the data directory")
    }

    /// Settings → Privacy as the client applies it: `remember_answers_days` and "Keep a log".
    pub fn set_privacy(client: &AiClient, remember_answers_days: u32, sent_log: bool) {
        client.set_remember_answers_days(remember_answers_days);
        client.set_sent_log(sent_log);
    }

    /// The sweep that runs at start and every hour: `ai_cache` rows older than
    /// `remember_answers_days`, `sent_log` rows older than 7 days.
    pub fn sweep(db: &Db, now_ms: i64, remember_answers_days: u32) {
        db.sweep(now_ms, remember_answers_days)
            .expect("the sweep runs");
    }

    /// The `cache_clear` command.
    pub fn cache_clear(db: &Db) {
        db.cache_clear().expect("cache_clear runs");
    }

    /// The `sent_log_clear` command.
    pub fn sent_log_clear(db: &Db) {
        db.sent_log_clear().expect("sent_log_clear runs");
    }

    /// The hourly sweep the app runs for as long as it lives; it never returns.
    pub async fn keep_sweeping(client: &AiClient) {
        client.keep_sweeping().await;
    }

    /// The own key as Settings → Providers keeps it: base URL and chat model in
    /// `settings.json`, the key in `secrets.json`.
    pub fn own_key_from_files(config_dir: &Path) -> Result<ResolvedProvider, AiError> {
        let settings = Settings::load(config_dir).expect("settings.json loads");
        let secrets = Secrets::load(config_dir).expect("secrets.json loads");
        ResolvedProvider::from_own_key_settings(&settings.providers.own_key, &secrets)
    }

    /// `provider_test("own_key")`: `{ok, message}` (docs/ARCHITECTURE.md §5).
    pub async fn provider_test(client: &AiClient, p: &ResolvedProvider) -> (bool, String) {
        let outcome = client.provider_test(p).await;
        (outcome.ok, outcome.message)
    }

    /// A local provider whose model Settings leave to `pick_local_model`
    /// (`providers.local.chat_model: null`).
    pub fn local_auto(base_url: &str) -> ResolvedProvider {
        ResolvedProvider {
            kind: ProviderKind::Local,
            base_url: base_url.to_string(),
            key: None,
            chat_model: String::new(),
        }
    }

    /// `settings.json` from the config directory, or the defaults when there is none.
    pub fn settings_load(config_dir: &Path) -> Settings {
        Settings::load(config_dir).expect("settings.json loads")
    }

    /// Whether the loaded settings came from a newer `version`.
    pub fn settings_read_only(settings: &Settings) -> bool {
        settings.read_only()
    }

    /// Writes `settings.json`; the error's `Display` is the user's message.
    pub fn settings_save(config_dir: &Path, settings: &Settings) -> Result<(), String> {
        settings.save(config_dir).map_err(|e| e.to_string())
    }

    /// Saves the own key in `secrets.json`, keeping whatever else the file holds.
    pub fn own_key_save(config_dir: &Path, key: &str) {
        let mut secrets = Secrets::load(config_dir).expect("secrets.json loads");
        secrets.own_key = Some(SecretString::from(key.to_string()));
        secrets.save(config_dir).expect("secrets.json saves");
    }

    /// The own key from `secrets.json`, if there is one.
    pub fn own_key_load(config_dir: &Path) -> Option<String> {
        Secrets::load(config_dir)
            .expect("secrets.json loads")
            .own_key
            .map(|k| k.expose_secret().to_string())
    }

    /// The `Debug` output of the loaded secrets.
    pub fn secrets_debug(config_dir: &Path) -> String {
        format!(
            "{:?}",
            Secrets::load(config_dir).expect("secrets.json loads")
        )
    }

    /// The errors of the two files (Design notes, `settings.json` and `secrets.json`).
    pub use redpen_core::store::{SecretsError, SettingsError};

    /// Loads `settings.json`, or says why it cannot be loaded.
    pub fn settings_try_load(config_dir: &Path) -> Result<Settings, SettingsError> {
        Settings::load(config_dir)
    }

    /// Writes `settings.json`, or says why it cannot be written.
    pub fn settings_try_save(config_dir: &Path, settings: &Settings) -> Result<(), SettingsError> {
        settings.save(config_dir)
    }

    /// Loads `secrets.json`, or says why it cannot be loaded.
    pub fn secrets_try_load(config_dir: &Path) -> Result<Option<String>, SecretsError> {
        Secrets::load(config_dir).map(|s| s.own_key.map(|k| k.expose_secret().to_string()))
    }

    /// Writes `secrets.json` holding the own key, or says why it cannot be written.
    pub fn own_key_try_save(config_dir: &Path, key: &str) -> Result<(), SecretsError> {
        Secrets {
            own_key: Some(SecretString::from(key.to_string())),
            search_key: None,
        }
        .save(config_dir)
    }

    /// Drops tables of `redpen.db` behind the client's back, so every use of them fails.
    pub fn drop_tables(db: &Db, tables: &[&str]) {
        for table in tables {
            db.connection()
                .execute_batch(&format!("DROP TABLE {table}"))
                .unwrap_or_else(|e| panic!("{table} could not be dropped: {e}"));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// A scripted transport for sequences and transport errors.

type Reply = Result<HttpResponse, TransportError>;

/// Answers each path (module 00's `request_path`) from its own queue, in order, and keeps every
/// request. A request with no answer left fails as unrecorded.
#[derive(Default)]
struct Script {
    answers: Mutex<HashMap<String, VecDeque<Reply>>>,
    seen: Mutex<Vec<HttpRequest>>,
}

impl Script {
    fn new() -> Self {
        Self::default()
    }

    fn on(self, path: &str, answers: Vec<Reply>) -> Self {
        self.answers
            .lock()
            .expect("answers are not poisoned")
            .entry(path.to_string())
            .or_default()
            .extend(answers);
        self
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.seen.lock().expect("log is not poisoned").clone()
    }

    fn sent_to(&self, path: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| request_path(&r.url) == path)
            .count()
    }
}

#[async_trait::async_trait]
impl HttpTransport for Script {
    async fn send(&self, req: HttpRequest) -> Reply {
        self.seen
            .lock()
            .expect("log is not poisoned")
            .push(req.clone());
        let path = request_path(&req.url);
        let next = self
            .answers
            .lock()
            .expect("answers are not poisoned")
            .get_mut(&path)
            .and_then(VecDeque::pop_front);
        next.unwrap_or_else(|| {
            Err(TransportError::Unrecorded {
                hash: request_hash(&req),
                path,
            })
        })
    }
}

fn respond(status: u16, body: &[u8]) -> Reply {
    Ok(HttpResponse {
        status,
        headers: Vec::new(),
        body: body.to_vec(),
    })
}

fn respond_json(status: u16, body: &Value) -> Reply {
    respond(status, body.to_string().as_bytes())
}

/// An OpenAI-compatible answer carrying `content`.
fn answer(content: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": content}}]})
}

fn ok(content: &str) -> Reply {
    respond_json(200, &answer(content))
}

/// An Ollama `/api/tags` body listing `(name, details.parameter_size)`.
fn tags(models: &[(&str, &str)]) -> Value {
    let models: Vec<Value> = models
        .iter()
        .map(|(name, size)| {
            json!({
                "name": name,
                "model": name,
                "size": 4_700_000_000_u64,
                "details": {"format": "gguf", "family": "example", "parameter_size": size,
                            "quantization_level": "Q4_K_M"}
            })
        })
        .collect();
    json!({ "models": models })
}

/// The five models of bullet 11.
fn five_tags() -> Value {
    tags(&[
        ("llama3.2:3b", "3.2B"),
        ("qwen2.5:14b", "14.8B"),
        ("example-16b", "16.0B"),
        ("nomic-embed-text", "137M"),
        ("mixtral:8x22b", "141B"),
    ])
}

// ---------------------------------------------------------------------------------------------
// A status sink, a log capture and the test environment.

/// A status sink that returns a fixed charge and keeps the status of every response it saw.
struct Sink {
    charge: Option<u64>,
    statuses: Mutex<Vec<u16>>,
}

impl Sink {
    fn statuses(&self) -> Vec<u16> {
        self.statuses.lock().expect("log is not poisoned").clone()
    }
}

impl StatusSink for Sink {
    fn on_response(
        &self,
        _kind: ProviderKind,
        _feature: Feature,
        status: u16,
        _headers: &[(String, String)],
    ) -> Option<u64> {
        self.statuses
            .lock()
            .expect("log is not poisoned")
            .push(status);
        self.charge
    }
}

/// What a capturing `tracing` subscriber wrote.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("log is not poisoned")).into_owned()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("log is not poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A client over a transport, a fake clock at `T0`, a fake sleeper, a status sink and a
/// database, in memory or in a temporary data directory.
struct Env<T> {
    client: AiClient,
    db: Db,
    t: Arc<T>,
    clock: Arc<FakeClock>,
    sleeper: Arc<FakeSleeper>,
    sink: Arc<Sink>,
    /// Last, so the database is closed before its folder is removed.
    dir: Option<tempfile::TempDir>,
}

impl<T: HttpTransport + 'static> Env<T> {
    fn memory(t: T) -> Self {
        Self::build(t, false, None)
    }

    fn on_disk(t: T) -> Self {
        Self::build(t, true, None)
    }

    /// On disk, with a status sink that reports `charge` for every response.
    fn charging(t: T, charge: u64) -> Self {
        Self::build(t, true, Some(charge))
    }

    fn build(t: T, on_disk: bool, charge: Option<u64>) -> Self {
        let dir = on_disk.then(|| tempfile::tempdir().expect("a temporary data directory"));
        let db = match &dir {
            Some(dir) => api::file_db(dir.path()),
            None => api::memory_db(),
        };
        let t = Arc::new(t);
        let clock = Arc::new(FakeClock::at(T0));
        let sleeper = Arc::new(FakeSleeper::new());
        let sink = Arc::new(Sink {
            charge,
            statuses: Mutex::new(Vec::new()),
        });
        let client = AiClient::new(
            t.clone(),
            db.clone(),
            clock.clone(),
            sleeper.clone(),
            sink.clone(),
        );
        Self {
            client,
            db,
            t,
            clock,
            sleeper,
            sink,
            dir,
        }
    }

    /// A second client over the same database, transport, clock and sink, as after a restart.
    fn restarted(&self) -> AiClient {
        let client = AiClient::new(
            self.t.clone(),
            self.db.clone(),
            self.clock.clone(),
            self.sleeper.clone(),
            self.sink.clone(),
        );
        api::set_privacy(&client, 7, false);
        client
    }

    /// A second connection to `redpen.db`, to read rows back.
    fn sqlite(&self) -> Connection {
        let dir = self
            .dir
            .as_ref()
            .expect("this test keeps its database on disk");
        Connection::open(dir.path().join(api::DB_FILE)).expect("redpen.db opens")
    }
}

// ---------------------------------------------------------------------------------------------
// Providers, calls and expected requests.

fn secret(key: &str) -> SecretString {
    SecretString::from(key.to_string())
}

fn own_key() -> ResolvedProvider {
    ResolvedProvider {
        kind: ProviderKind::OwnKey,
        base_url: OWN_BASE.to_string(),
        key: Some(secret(KEY)),
        chat_model: OWN_MODEL.to_string(),
    }
}

fn fake(model: &str) -> ResolvedProvider {
    ResolvedProvider {
        kind: ProviderKind::Fake,
        base_url: FAKE_BASE.to_string(),
        key: None,
        chat_model: model.to_string(),
    }
}

fn local(base_url: &str, model: &str) -> ResolvedProvider {
    ResolvedProvider {
        kind: ProviderKind::Local,
        base_url: base_url.to_string(),
        key: None,
        chat_model: model.to_string(),
    }
}

/// A Publik provider as module 03 resolves it (the key is the same fake key, so test 14 covers
/// it too).
fn publik() -> ResolvedProvider {
    ResolvedProvider {
        kind: ProviderKind::Publik,
        base_url: PUBLIK_BASE.to_string(),
        key: Some(secret(KEY)),
        chat_model: "publik-fast".to_string(),
    }
}

/// A user text unique to this run of a scenario. The scenarios also run inside test 14, in the
/// same process, so no two runs share a cache key even if the memory cache outlived a client.
fn text(tag: &str) -> String {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let run = RUNS.fetch_add(1, Ordering::SeqCst);
    format!("{tag} #{run}: {TEXT}")
}

/// A Fix grammar call as module 04 makes one: a system and a user message, the user's text and
/// dialect as the cache input, 30 seconds (own key).
fn call(user_text: &str, cache: CachePolicy) -> ChatCall {
    ChatCall {
        feature: Feature::FixGrammar,
        prompt_id: "fix-grammar",
        prompt_version: 1,
        messages: vec![
            api::message("system", SYSTEM),
            api::message("user", user_text),
        ],
        max_tokens: 300,
        temperature: 0.4,
        timeout: Duration::from_secs(30),
        cache_input: json!({"text": user_text, "dialect": "british"}),
        cache,
        validation_retry: 0,
    }
}

/// The same call to a local model: 120 seconds (docs/PUBLIK-INTEGRATION.md §6).
fn local_call(user_text: &str) -> ChatCall {
    let mut c = call(user_text, CachePolicy::Off);
    c.timeout = Duration::from_secs(120);
    c
}

/// The body `call(user_text, …)` must send (Design notes: exactly these four keys, `messages`
/// as `[{"role", "content"}]`, numbers in their shortest form).
fn body(model: &str, user_text: &str) -> Value {
    json!({
        "model": model,
        "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": user_text}],
        "max_tokens": 300,
        "temperature": 0.4
    })
}

/// A hand-written entry answering `POST {base}/chat/completions` with `body`.
fn entry(base: &str, body: &Value, answer_body: Value) -> FakeEntry {
    let req = HttpRequest::post_json(format!("{base}{CHAT}"), body.clone());
    FakeEntry {
        request_sha256: request_hash(&req),
        source: "hand-written".to_string(),
        request: body.clone(),
        status: 200,
        headers: Vec::new(),
        body: answer_body,
    }
}

/// Own-key entries answering `hello` to each text.
fn own_entries<S: AsRef<str>>(texts: &[S]) -> Vec<FakeEntry> {
    texts
        .iter()
        .map(|t| entry(OWN_BASE, &body(OWN_MODEL, t.as_ref()), answer("hello")))
        .collect()
}

fn json_body(req: &HttpRequest) -> &Value {
    match &req.body {
        Body::Json(v) => v,
        Body::Empty => panic!("{} {} has no body", req.method, req.url),
    }
}

/// The key rides only in `Authorization: Bearer`, as a secret header value, and nowhere else.
fn assert_bearer_only(req: &HttpRequest, key: &str) {
    let auth: Vec<_> = req
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .collect();
    assert_eq!(auth.len(), 1, "one Authorization header on {}", req.url);
    assert_eq!(auth[0].1.expose(), format!("Bearer {key}"));
    assert!(auth[0].1.is_secret(), "the key is a secret header value");
    for (name, value) in &req.headers {
        if !name.eq_ignore_ascii_case("authorization") {
            assert!(!value.expose().contains(key), "{name} carries the key");
        }
    }
    assert!(!req.url.contains(key), "the URL carries the key");
    if let Body::Json(v) = &req.body {
        assert!(!v.to_string().contains(key), "the body carries the key");
    }
}

/// Design notes: `AiError` serialises as `{"kind", "message"}` plus the variant's fields, and
/// its `Display` is the message. Returns the serialised form for the field checks.
fn check_error(e: &AiError, kind: &str, message: &str) -> Value {
    assert_eq!(e.to_string(), message, "{e:?}");
    let v = serde_json::to_value(e).expect("an AiError serialises");
    assert_eq!(v["kind"], kind, "{v}");
    assert_eq!(v["message"], message, "{v}");
    v
}

fn failed(result: Result<ChatReply, AiError>) -> AiError {
    match result {
        Ok(reply) => panic!("expected an error, got the reply {:?}", reply.content),
        Err(e) => e,
    }
}

// ---------------------------------------------------------------------------------------------
// Rows of redpen.db (docs/ARCHITECTURE.md §4).

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap_or_else(|e| panic!("{table} could not be counted: {e}"))
}

struct CacheRow {
    key: String,
    provider: String,
    model: String,
    prompt_id: String,
    prompt_version: i64,
    charge_micros: Option<i64>,
    created_at_ms: i64,
}

fn cache_rows(conn: &Connection) -> Vec<CacheRow> {
    let mut stmt = conn
        .prepare(
            "SELECT key, provider, model, prompt_id, prompt_version, charge_micros, created_at_ms \
             FROM ai_cache ORDER BY created_at_ms",
        )
        .expect("ai_cache has the columns of docs/ARCHITECTURE.md §4");
    let rows: Vec<CacheRow> = stmt
        .query_map([], |r| {
            Ok(CacheRow {
                key: r.get(0)?,
                provider: r.get(1)?,
                model: r.get(2)?,
                prompt_id: r.get(3)?,
                prompt_version: r.get(4)?,
                charge_micros: r.get(5)?,
                created_at_ms: r.get(6)?,
            })
        })
        .expect("ai_cache reads")
        .map(|r| r.expect("an ai_cache row reads"))
        .collect();
    rows
}

struct SentRow {
    at_ms: i64,
    provider: String,
    endpoint: String,
    feature: String,
    model: String,
    sent_text: String,
    bytes: i64,
}

fn sent_rows(conn: &Connection) -> Vec<SentRow> {
    let mut stmt = conn
        .prepare(
            "SELECT at_ms, provider, endpoint, feature, model, sent_text, bytes \
             FROM sent_log ORDER BY at_ms, id",
        )
        .expect("sent_log has the columns of docs/ARCHITECTURE.md §4");
    let rows: Vec<SentRow> = stmt
        .query_map([], |r| {
            Ok(SentRow {
                at_ms: r.get(0)?,
                provider: r.get(1)?,
                endpoint: r.get(2)?,
                feature: r.get(3)?,
                model: r.get(4)?,
                sent_text: r.get(5)?,
                bytes: r.get(6)?,
            })
        })
        .expect("sent_log reads")
        .map(|r| r.expect("a sent_log row reads"))
        .collect();
    rows
}

/// `(calls, charge_micros)` of one `usage_daily` row.
fn usage(conn: &Connection, day: &str, provider: &str, feature: &str) -> Option<(i64, i64)> {
    conn.query_row(
        "SELECT calls, charge_micros FROM usage_daily WHERE day = ?1 AND provider = ?2 AND feature = ?3",
        params![day, provider, feature],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .expect("usage_daily reads")
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
        .expect("pragma_table_info works");
    let names: Vec<String> = stmt
        .query_map([table], |r| r.get(0))
        .expect("table_info reads")
        .map(|r| r.expect("a column name"))
        .collect();
    names
}

// ---------------------------------------------------------------------------------------------
// Scenarios. Each runs on its own and again inside test 14, and returns every error it saw.

/// Bullet 1.
async fn b01() -> Vec<AiError> {
    let first = text("b01");
    let whole = text("b01 whole temperature");
    let mut whole_body = body(OWN_MODEL, &whole);
    whole_body["temperature"] = json!(0);
    let e = Env::memory(FakeTransport::scripted(vec![
        entry(
            OWN_BASE,
            &body(OWN_MODEL, &first),
            json!({"choices": [{"message": {"content": "hello"}}]}),
        ),
        entry(OWN_BASE, &whole_body, answer("hello")),
    ]));

    let result = e
        .client
        .chat(&own_key(), call(&first, CachePolicy::Off))
        .await;
    let seen = e.t.requests_seen();
    assert_eq!(seen.len(), 1, "one request");
    let req = &seen[0];
    assert_eq!(req.method, Method::Post);
    assert_eq!(req.url, "https://api.example.com/v1/chat/completions");
    assert_eq!(req.timeout, Duration::from_secs(30), "the call's timeout");
    assert_eq!(json_body(req), &body(OWN_MODEL, &first), "the exact body");
    assert!(
        canonical_json(json_body(req)).ends_with(r#""model":"example-chat","temperature":0.4}"#),
        "0.4 is written in its shortest form: {}",
        canonical_json(json_body(req))
    );
    assert_bearer_only(req, KEY);
    let reply = result.expect("a 200 with a content string is a reply");
    assert_eq!(reply.content, "hello");
    assert!(!reply.from_cache);

    let mut whole_call = call(&whole, CachePolicy::Off);
    whole_call.temperature = 0.0;
    let reply = e
        .client
        .chat(&own_key(), whole_call)
        .await
        .expect("the call with temperature 0 is answered");
    assert_eq!(reply.content, "hello");
    let seen = e.t.requests_seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(json_body(&seen[1]), &whole_body);
    assert!(
        canonical_json(json_body(&seen[1])).ends_with(r#""temperature":0}"#),
        "a whole temperature is an integer: {}",
        canonical_json(json_body(&seen[1]))
    );
    Vec::new()
}

/// Bullet 2.
async fn b02() -> Vec<AiError> {
    let t = text("b02");
    // The fake provider gets the same model, so its request body is the same and the one entry
    // answers both. The cache key counts the provider's kind and never its address, so the
    // second provider misses only because the caches are per provider.
    let e = Env::memory(FakeTransport::scripted(own_entries(&[&t])));

    let first = e
        .client
        .chat(&own_key(), call(&t, CachePolicy::Disk))
        .await
        .expect("the first call is answered");
    assert!(!first.from_cache);
    let second = e
        .client
        .chat(&own_key(), call(&t, CachePolicy::Disk))
        .await
        .expect("the second call is answered");
    assert_eq!(second.content, "hello");
    assert!(
        second.from_cache,
        "an identical call is answered from the cache"
    );
    assert_eq!(
        e.t.requests_seen().len(),
        1,
        "identical input never sends twice"
    );

    let other = e
        .client
        .chat(&fake(OWN_MODEL), call(&t, CachePolicy::Disk))
        .await
        .expect("the other provider is answered");
    assert!(!other.from_cache, "caches are per provider");
    let seen = e.t.requests_seen();
    assert_eq!(seen.len(), 2, "caches are per provider");
    assert!(seen[1].url.starts_with(FAKE_BASE), "{}", seen[1].url);

    let mut bumped = call(&t, CachePolicy::Disk);
    bumped.prompt_version = 2;
    let reply = e
        .client
        .chat(&own_key(), bumped)
        .await
        .expect("version 2 is answered");
    assert!(!reply.from_cache, "a prompt version bump misses");
    assert_eq!(e.t.requests_seen().len(), 3, "a prompt version bump misses");
    Vec::new()
}

/// Bullet 3.
async fn b03() -> Vec<AiError> {
    let memory = text("b03 memory");
    let off = text("b03 off");
    let zero = text("b03 zero days");
    let disk = text("b03 disk");
    let e = Env::on_disk(FakeTransport::scripted(own_entries(&[
        &memory, &off, &zero, &disk,
    ])));
    api::set_privacy(&e.client, 7, false);

    for (i, expect_cached) in [false, true].into_iter().enumerate() {
        let reply = e
            .client
            .chat(&own_key(), call(&memory, CachePolicy::Memory))
            .await
            .expect("Memory calls are answered");
        assert_eq!(reply.from_cache, expect_cached, "Memory call {i}");
    }
    assert_eq!(
        e.t.requests_seen().len(),
        1,
        "Memory answers the second call"
    );
    assert_eq!(
        count(&e.sqlite(), "ai_cache"),
        0,
        "Memory writes no ai_cache row"
    );

    for _ in 0..2 {
        let reply = e
            .client
            .chat(&own_key(), call(&off, CachePolicy::Off))
            .await
            .expect("Off calls are answered");
        assert!(!reply.from_cache, "Off never answers from a cache");
    }
    assert_eq!(
        e.t.requests_seen().len(),
        3,
        "Off sends a request every time"
    );
    assert_eq!(
        count(&e.sqlite(), "ai_cache"),
        0,
        "Off writes no ai_cache row"
    );

    api::set_privacy(&e.client, 0, false);
    for (i, expect_cached) in [false, true].into_iter().enumerate() {
        let reply = e
            .client
            .chat(&own_key(), call(&zero, CachePolicy::Disk))
            .await
            .expect("Disk calls are answered");
        assert_eq!(reply.from_cache, expect_cached, "0 days: Disk call {i}");
    }
    assert_eq!(
        e.t.requests_seen().len(),
        4,
        "with 0 days Disk behaves as Memory"
    );
    assert_eq!(
        count(&e.sqlite(), "ai_cache"),
        0,
        "remember_answers_days: 0 writes no row"
    );

    api::set_privacy(&e.client, 7, false);
    let reply = e
        .client
        .chat(&own_key(), call(&disk, CachePolicy::Disk))
        .await
        .expect("the Disk call is answered");
    assert!(!reply.from_cache);
    assert_eq!(e.t.requests_seen().len(), 5);
    let rows = cache_rows(&e.sqlite());
    assert_eq!(rows.len(), 1, "Disk writes one ai_cache row");
    let row = &rows[0];
    assert_eq!(
        row.key,
        AiClient::cache_key(&own_key(), &call(&disk, CachePolicy::Disk))
    );
    assert_eq!(row.key.len(), 64);
    assert_eq!(row.provider, "own_key");
    assert_eq!(row.model, OWN_MODEL);
    assert_eq!(row.prompt_id, "fix-grammar");
    assert_eq!(row.prompt_version, 1);
    assert_eq!(row.charge_micros, None, "no charge off Publik");
    assert_eq!(row.created_at_ms, T0, "created at the clock's time");

    let restarted = e.restarted();
    let reply = restarted
        .chat(&own_key(), call(&disk, CachePolicy::Disk))
        .await
        .expect("the Disk answer survives a restart");
    assert!(reply.from_cache);
    assert_eq!(reply.content, "hello");
    assert_eq!(e.t.requests_seen().len(), 5, "answered from redpen.db");
    Vec::new()
}

/// Bullet 4.
async fn b04() -> Vec<AiError> {
    let old = text("b04 eight days");
    let recent = text("b04 six days");
    let e = Env::on_disk(FakeTransport::scripted(own_entries(&[&old, &recent])));
    api::set_privacy(&e.client, 7, false);

    e.clock.set(T0 - 8 * DAY_MS);
    e.client
        .chat(&own_key(), call(&old, CachePolicy::Disk))
        .await
        .expect("the old call is answered");
    e.clock.set(T0 - 6 * DAY_MS);
    e.client
        .chat(&own_key(), call(&recent, CachePolicy::Disk))
        .await
        .expect("the recent call is answered");
    assert_eq!(count(&e.sqlite(), "ai_cache"), 2);

    e.clock.set(T0);
    api::sweep(&e.db, T0, 7);
    let rows = cache_rows(&e.sqlite());
    assert_eq!(
        rows.len(),
        1,
        "the 8-day-old row is deleted, the 6-day-old row kept"
    );
    assert_eq!(rows[0].created_at_ms, T0 - 6 * DAY_MS);
    assert_eq!(
        rows[0].key,
        AiClient::cache_key(&own_key(), &call(&recent, CachePolicy::Disk))
    );

    let restarted = e.restarted();
    let kept = restarted
        .chat(&own_key(), call(&recent, CachePolicy::Disk))
        .await
        .expect("the kept answer is served");
    assert!(kept.from_cache, "the 6-day-old answer is kept");
    assert_eq!(e.t.requests_seen().len(), 2);
    let gone = restarted
        .chat(&own_key(), call(&old, CachePolicy::Disk))
        .await
        .expect("the old call is sent again");
    assert!(!gone.from_cache, "the 8-day-old answer is gone");
    assert_eq!(e.t.requests_seen().len(), 3);
    Vec::new()
}

/// Bullet 5.
async fn b05() -> Vec<AiError> {
    let t = text("b05");
    let e = Env::memory(Script::new().on(
        CHAT,
        vec![
            Err(TransportError::Timeout),
            Err(TransportError::Timeout),
            ok("hello"),
        ],
    ));
    let reply = e
        .client
        .chat(&own_key(), call(&t, CachePolicy::Off))
        .await
        .expect("the third attempt is answered");
    assert_eq!(reply.content, "hello");
    assert_eq!(e.t.sent_to(CHAT), 3);
    assert_eq!(
        e.sleeper.sleeps(),
        RETRY_SLEEPS,
        "exactly 500 ms, then 1,000 ms"
    );

    let e = Env::memory(Script::new().on(CHAT, vec![Err(TransportError::Timeout); 3]));
    let err = failed(e.client.chat(&own_key(), call(&t, CachePolicy::Off)).await);
    let v = check_error(&err, "Timeout", MSG_OWN_TIMEOUT);
    assert_eq!(v["seconds"], 30, "{v}");
    assert_eq!(e.t.sent_to(CHAT), 3, "three requests, no fourth");
    assert_eq!(e.sleeper.sleeps(), RETRY_SLEEPS);
    vec![err]
}

/// Bullet 6. The error bodies echo the key, as some providers do, so test 14 sees whether any
/// error repeats them.
async fn b06() -> Vec<AiError> {
    let t = text("b06");
    let rate_limited = json!({"error": {"message": format!("Rate limit reached for {KEY}."),
                                        "type": "requests"}});
    let e =
        Env::memory(Script::new().on(CHAT, vec![respond_json(429, &rate_limited), ok("hello")]));
    let reply = e
        .client
        .chat(&own_key(), call(&t, CachePolicy::Off))
        .await
        .expect("the retry is answered");
    assert_eq!(reply.content, "hello");
    assert_eq!(e.t.sent_to(CHAT), 2, "one retry");
    assert_eq!(e.sleeper.sleeps(), [Duration::from_millis(500)]);

    let unavailable = json!({"error": {"message": format!("Service unavailable for {KEY}.")}});
    let e = Env::memory(Script::new().on(CHAT, vec![respond_json(503, &unavailable); 3]));
    let err = failed(e.client.chat(&own_key(), call(&t, CachePolicy::Off)).await);
    let v = check_error(&err, "ProviderError", MSG_PROVIDER_503);
    assert_eq!(v["code"], 503, "{v}");
    assert!(
        matches!(err, AiError::ProviderError { code: 503 }),
        "{err:?}"
    );
    assert_eq!(e.t.sent_to(CHAT), 3);
    assert_eq!(e.sleeper.sleeps(), RETRY_SLEEPS);
    vec![err]
}

/// Bullet 7.
async fn b07() -> Vec<AiError> {
    let t = text("b07");
    let refused = json!({"error": {"message": format!("Incorrect API key provided: {KEY}."),
                                   "code": "invalid_api_key"}});
    let e = Env::memory(Script::new().on(CHAT, vec![respond_json(401, &refused); 3]));
    let err_401 = failed(e.client.chat(&own_key(), call(&t, CachePolicy::Off)).await);
    let v = check_error(&err_401, "OwnKeyRefused", MSG_REFUSED_401);
    assert_eq!(v["code"], 401, "{v}");
    assert!(
        matches!(err_401, AiError::OwnKeyRefused { code: 401 }),
        "{err_401:?}"
    );
    assert_eq!(e.t.sent_to(CHAT), 1, "a 401 is never retried");
    assert!(e.sleeper.sleeps().is_empty());

    let unknown = json!({"error": {"message": "The model example-chat does not exist.",
                                   "code": "model_not_found"}});
    let e = Env::memory(Script::new().on(CHAT, vec![respond_json(404, &unknown); 3]));
    let err_404 = failed(e.client.chat(&own_key(), call(&t, CachePolicy::Off)).await);
    let v = check_error(&err_404, "ModelUnknown", MSG_MODEL_UNKNOWN);
    assert_eq!(v["model"], OWN_MODEL, "{v}");
    assert_eq!(e.t.sent_to(CHAT), 1, "a 404 is never retried");
    assert!(e.sleeper.sleeps().is_empty());
    vec![err_401, err_404]
}

/// Bullet 8.
async fn b08() -> Vec<AiError> {
    let t = text("b08");
    let e = Env::on_disk(Script::new().on(
        CHAT,
        vec![
            respond(200, b"not json"),
            respond(200, b"not json"),
            respond_json(
                200,
                &json!({"id": "chatcmpl-1", "object": "chat.completion"}),
            ),
            respond_json(200, &json!({"choices": []})),
            respond_json(200, &json!({"choices": [{"message": {"content": 42}}]})),
        ],
    ));
    api::set_privacy(&e.client, 7, false);
    let mut errors = Vec::new();
    for (i, what) in [
        "not json",
        "not json, sent again because nothing was cached",
        "no choices",
        "an empty choices array",
        "a content that is not a string",
    ]
    .into_iter()
    .enumerate()
    {
        let err = failed(e.client.chat(&own_key(), call(&t, CachePolicy::Disk)).await);
        check_error(&err, "InvalidAnswer", MSG_INVALID);
        assert_eq!(e.t.sent_to(CHAT), i + 1, "{what}: one request each");
        errors.push(err);
    }
    assert!(e.sleeper.sleeps().is_empty(), "a 200 is never retried");
    assert_eq!(count(&e.sqlite(), "ai_cache"), 0, "nothing cached");
    errors
}

/// Bullet 9, and the Design notes' mapping of a TLS failure to `Offline`.
async fn b09() -> Vec<AiError> {
    let t = text("b09");
    let mut errors = Vec::new();
    for failure in [
        TransportError::Offline,
        TransportError::Tls("invalid peer certificate".to_string()),
    ] {
        let e = Env::memory(Script::new().on(CHAT, vec![Err(failure.clone()); 3]));
        let err = failed(e.client.chat(&own_key(), call(&t, CachePolicy::Off)).await);
        check_error(&err, "Offline", MSG_OFFLINE);
        assert!(matches!(err, AiError::Offline), "{failure:?} → {err:?}");
        assert_eq!(e.t.sent_to(CHAT), 1, "{failure:?} is not retried");
        assert!(e.sleeper.sleeps().is_empty());
        errors.push(err);
    }
    errors
}

/// Bullet 10, and the Scope's rule that a Publik provider can never retry, on 429 and 5xx too.
async fn b10() -> Vec<AiError> {
    let t = text("b10");
    let mut errors = Vec::new();
    let answers: [(&str, Reply); 3] = [
        ("a timeout", Err(TransportError::Timeout)),
        (
            "a 503",
            respond_json(503, &json!({"error": {"message": "busy"}})),
        ),
        (
            "a 429",
            respond_json(429, &json!({"error": {"message": "slow down"}})),
        ),
    ];
    for (what, failure) in answers {
        let e = Env::memory(Script::new().on(CHAT, vec![failure; 3]));
        let err = failed(e.client.chat(&publik(), call(&t, CachePolicy::Off)).await);
        assert_eq!(
            e.t.sent_to(CHAT),
            1,
            "Publik after {what}: one request only"
        );
        assert!(
            e.sleeper.sleeps().is_empty(),
            "Publik after {what}: no sleep"
        );
        errors.push(err);
    }
    errors
}

/// Bullet 12.
async fn b12() -> Vec<AiError> {
    let t = text("b12");

    let e = Env::memory(Script::new().on(TAGS, vec![Err(TransportError::Offline)]));
    let not_running = failed(
        e.client
            .chat(&api::local_auto(LOCAL_BASE), local_call(&t))
            .await,
    );
    check_error(&not_running, "LocalNotRunning", MSG_NOT_RUNNING);
    let seen = e.t.requests();
    assert_eq!(seen.len(), 1, "only /api/tags was asked");
    assert_eq!(seen[0].method, Method::Get);
    assert_eq!(seen[0].url, "http://localhost:11434/api/tags");

    let e = Env::memory(Script::new().on(TAGS, vec![respond_json(200, &json!({"models": []}))]));
    let no_model = failed(
        e.client
            .chat(&api::local_auto(LOCAL_BASE), local_call(&t))
            .await,
    );
    check_error(&no_model, "LocalNoModel", MSG_NO_MODEL);
    assert_eq!(e.t.sent_to(CHAT), 0, "no chat call without a model");

    let only_embeddings = tags(&[("nomic-embed-text", "137M")]);
    let e = Env::memory(Script::new().on(TAGS, vec![respond_json(200, &only_embeddings)]));
    let embed_only = failed(
        e.client
            .chat(&api::local_auto(LOCAL_BASE), local_call(&t))
            .await,
    );
    check_error(&embed_only, "LocalNoModel", MSG_NO_MODEL);
    assert_eq!(e.t.sent_to(CHAT), 0);

    let mut errors = vec![not_running, no_model, embed_only];
    // The first is the spec's; the others are not localhost either (RFC 3986: the host of
    // `localhost@192.168.1.5` is `192.168.1.5`).
    for base in [
        "http://192.168.1.5:11434/v1",
        "http://localhost.example.com:11434/v1",
        "http://localhost@192.168.1.5:11434/v1",
    ] {
        for p in [api::local_auto(base), local(base, "llama3.2:3b")] {
            let e = Env::memory(Script::new());
            let err = failed(e.client.chat(&p, local_call(&t)).await);
            check_error(&err, "LocalNotLocal", MSG_NOT_LOCAL);
            assert!(
                e.t.requests().is_empty(),
                "no request at all for {base}: {:?}",
                e.t.requests()
                    .iter()
                    .map(|r| r.url.clone())
                    .collect::<Vec<_>>()
            );
            errors.push(err);
        }
    }
    errors
}

/// Bullet 13.
async fn b13() -> Vec<AiError> {
    let data: Vec<Value> = (1..=14)
        .map(|i| json!({"id": format!("example-model-{i:02}"), "object": "model", "owned_by": "example"}))
        .collect();
    let list = json!({"object": "list", "data": data});
    let e = Env::memory(Script::new().on(MODELS, vec![respond_json(200, &list); 2]));
    let n = e
        .client
        .list_models(&own_key())
        .await
        .expect("a 14-entry list is read");
    assert_eq!(n, 14);
    let seen = e.t.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, Method::Get);
    assert_eq!(seen[0].url, "https://api.example.com/v1/models");
    assert_bearer_only(&seen[0], KEY);
    let (works, message) = api::provider_test(&e.client, &own_key()).await;
    assert!(works);
    assert_eq!(message, "Key works: 14 models available.");

    let refused = json!({"error": {"message": format!("Incorrect API key provided: {KEY}.")}});
    let e = Env::memory(Script::new().on(MODELS, vec![respond_json(401, &refused); 3]));
    let err = match e.client.list_models(&own_key()).await {
        Ok(n) => panic!("a refused key listed {n} models"),
        Err(err) => err,
    };
    check_error(&err, "OwnKeyRefused", MSG_REFUSED_401);
    assert_eq!(e.t.sent_to(MODELS), 1, "a 401 is never retried");
    vec![err]
}

/// Error path: 403 is refused like 401, with its own code (Design notes, error mapping).
async fn err_403() -> Vec<AiError> {
    let t = text("e403");
    let forbidden = json!({"error": {"message": format!("Key {KEY} has no access.")}});
    let e = Env::memory(Script::new().on(CHAT, vec![respond_json(403, &forbidden); 3]));
    let err = failed(e.client.chat(&own_key(), call(&t, CachePolicy::Off)).await);
    let v = check_error(
        &err,
        "OwnKeyRefused",
        "Your key was refused (403). Check it in Settings → Providers.",
    );
    assert_eq!(v["code"], 403, "{v}");
    assert_eq!(e.t.sent_to(CHAT), 1, "a 403 is never retried");
    assert!(e.sleeper.sleeps().is_empty());
    vec![err]
}

/// Error path: any other 4xx is a `ProviderError` and is never retried.
async fn err_400() -> Vec<AiError> {
    let t = text("e400");
    let bad = json!({"error": {"message": "Invalid request."}});
    let e = Env::memory(Script::new().on(CHAT, vec![respond_json(400, &bad); 3]));
    let err = failed(e.client.chat(&own_key(), call(&t, CachePolicy::Off)).await);
    let v = check_error(
        &err,
        "ProviderError",
        "Your provider had a problem (400). Try again later.",
    );
    assert_eq!(v["code"], 400, "{v}");
    assert_eq!(e.t.sent_to(CHAT), 1, "other 4xx are never retried");
    assert!(e.sleeper.sleeps().is_empty());
    vec![err]
}

/// Error path: 429 three times gives `RateLimited` with the own key's message.
async fn err_429() -> Vec<AiError> {
    let t = text("e429");
    let slow = json!({"error": {"message": format!("Rate limit reached for {KEY}.")}});
    let e = Env::memory(Script::new().on(CHAT, vec![respond_json(429, &slow); 3]));
    let err = failed(e.client.chat(&own_key(), call(&t, CachePolicy::Off)).await);
    check_error(&err, "RateLimited", MSG_RATE_LIMITED);
    assert_eq!(e.t.sent_to(CHAT), 3);
    assert_eq!(e.sleeper.sleeps(), RETRY_SLEEPS);
    vec![err]
}

/// Error path: the local model's timeout names it and its 120 seconds, after two retries.
async fn err_local_timeout() -> Vec<AiError> {
    let t = text("elocal");
    let e = Env::memory(
        Script::new()
            .on(TAGS, vec![respond_json(200, &five_tags()); 3])
            .on(CHAT, vec![Err(TransportError::Timeout); 3]),
    );
    let err = failed(
        e.client
            .chat(&local(LOCAL_BASE, "llama3.2:3b"), local_call(&t))
            .await,
    );
    let v = check_error(&err, "Timeout", MSG_LOCAL_TIMEOUT);
    assert_eq!(v["seconds"], 120, "{v}");
    assert_eq!(e.t.sent_to(CHAT), 3);
    assert_eq!(e.sleeper.sleeps(), RETRY_SLEEPS);
    for req in e.t.requests() {
        assert!(
            req.url.starts_with("http://localhost:11434/"),
            "{}",
            req.url
        );
        if request_path(&req.url) == CHAT {
            assert_eq!(req.timeout, Duration::from_secs(120));
        }
    }
    vec![err]
}

/// Error path: an unrecorded request is `Unrecorded` with the fake transport's message, and
/// is not retried.
async fn err_unrecorded() -> Vec<AiError> {
    let t = text("eunrecorded");
    let e = Env::memory(FakeTransport::scripted(Vec::new()));
    let err = failed(e.client.chat(&own_key(), call(&t, CachePolicy::Off)).await);
    let expected = HttpRequest::post_json(format!("{OWN_BASE}{CHAT}"), body(OWN_MODEL, &t));
    let hash = request_hash(&expected);
    let v = check_error(
        &err,
        "Unrecorded",
        &format!(
            "Unrecorded request {hash} to /chat/completions; body written to target/unrecorded/{hash}.json. Add a hand-written entry, or ask the maintainer to record it."
        ),
    );
    assert_eq!(v["hash"], hash.as_str(), "{v}");
    assert_eq!(v["path"], CHAT, "{v}");
    assert_eq!(e.t.requests_seen().len(), 1);
    let _ = fs::remove_file(unrecorded_path(&hash));
    vec![err]
}

/// Error path: retries are for own key and local only. The fake provider never retries; the
/// local one does.
async fn err_retry_kinds() -> Vec<AiError> {
    let t = text("eretry");
    let e = Env::memory(Script::new().on(CHAT, vec![Err(TransportError::Timeout); 3]));
    let err = failed(
        e.client
            .chat(&fake(OWN_MODEL), call(&t, CachePolicy::Off))
            .await,
    );
    assert_eq!(e.t.sent_to(CHAT), 1, "the fake provider never retries");
    assert!(e.sleeper.sleeps().is_empty());

    let e = Env::memory(
        Script::new()
            .on(TAGS, vec![respond_json(200, &five_tags()); 2])
            .on(
                CHAT,
                vec![
                    respond_json(429, &json!({"error": {"message": "slow down"}})),
                    ok("hello"),
                ],
            ),
    );
    let reply = e
        .client
        .chat(&local(LOCAL_BASE, "llama3.2:3b"), local_call(&t))
        .await
        .expect("the local retry is answered");
    assert_eq!(reply.content, "hello");
    assert_eq!(e.t.sent_to(CHAT), 2, "the local provider retries a 429");
    assert_eq!(e.sleeper.sleeps(), [Duration::from_millis(500)]);
    vec![err]
}

/// Bullet 15's store, run again inside test 14.
fn secrets_round_trip() {
    let dir = tempfile::tempdir().expect("a temporary config directory");
    assert_eq!(
        api::own_key_load(dir.path()),
        None,
        "no secrets.json, no key"
    );
    api::own_key_save(dir.path(), KEY);
    let file = dir.path().join(api::SECRETS_FILE);
    let written = read_json(&file);
    assert_eq!(written["version"], 1, "{written}");
    assert_eq!(written["own_key"], KEY);
    assert_eq!(api::own_key_load(dir.path()).as_deref(), Some(KEY));
    assert!(
        !api::secrets_debug(dir.path()).contains(KEY),
        "the loaded secrets print no key"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = file
            .metadata()
            .expect("secrets.json exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "secrets.json has mode {mode:o}");
    }
}

// ---------------------------------------------------------------------------------------------
// Bullets 1 to 13.

// Bullet 1: own key 200 → the content; Authorization: Bearer with the key, the model from
// settings, the exact max_tokens and temperature (0.4 as 0.4, 0 as 0).
#[tokio::test]
async fn t01_own_key_200_gives_the_content_and_sends_the_exact_request() {
    b01().await;
}

// Bullet 2: a second identical call comes from the cache; another provider kind and prompt
// version 2 each send a request.
#[tokio::test]
async fn t02_identical_calls_hit_the_cache_per_provider_and_prompt_version() {
    b02().await;
}

// Bullet 2 and the Design notes' cache key K, against sha256sum (REFERENCE.md): only the
// provider, model, prompt id and version and the cache input count.
#[test]
fn t02_cache_key_is_the_sha256_of_the_canonical_input() {
    const K_OWN: &str = "f17781d369d7294097135b31fafd42208a55e1aff0edafc7c9e4eec21156741c";
    const K_FAKE: &str = "0d969e53c78f813b9eccb1c6874c40bb9a1be3022f766f23ecd537de2f0c6719";
    const K_OWN_V2: &str = "a2adb751c5e411101085b9018c2650b0317d1eea860c306edbee29d7ea425198";
    let c = call(TEXT, CachePolicy::Disk);
    assert_eq!(AiClient::cache_key(&own_key(), &c), K_OWN);
    assert_eq!(AiClient::cache_key(&fake(OWN_MODEL), &c), K_FAKE);
    let mut v2 = call(TEXT, CachePolicy::Disk);
    v2.prompt_version = 2;
    assert_eq!(AiClient::cache_key(&own_key(), &v2), K_OWN_V2);

    let mut other = call(TEXT, CachePolicy::Off);
    other.messages = vec![api::message("user", "something else entirely")];
    other.max_tokens = 10;
    other.temperature = 0.0;
    other.timeout = Duration::from_secs(120);
    other.validation_retry = 2;
    assert_eq!(AiClient::cache_key(&own_key(), &other), K_OWN);
    let mut other_key = own_key();
    other_key.key = Some(secret("sk-test-b"));
    other_key.base_url = "https://api.example.org/v1".to_string();
    assert_eq!(AiClient::cache_key(&other_key, &c), K_OWN);
}

// Bullet 3: Memory writes no row, Off sends every time, 0 days makes Disk behave as Memory;
// Disk writes one row of docs/ARCHITECTURE.md §4 that a restarted client answers from.
#[tokio::test]
async fn t03_cache_policies_memory_off_and_zero_days() {
    b03().await;
}

// Bullet 4: the start-up sweep with 7 days deletes the 8-day-old row and keeps the 6-day-old one.
#[tokio::test]
async fn t04_sweep_deletes_rows_older_than_remember_answers_days() {
    b04().await;
}

// Bullet 5: timeout, timeout, 200 → success after sleeps of 500 ms and 1,000 ms; three timeouts
// → Timeout with its message after three requests.
#[tokio::test]
async fn t05_own_key_timeouts_retry_after_half_a_second_and_a_second() {
    b05().await;
}

// Bullet 6: 429 then 200 → success after one retry; 503 three times → ProviderError 503.
#[tokio::test]
async fn t06_own_key_429_retries_and_503_three_times_is_a_provider_error() {
    b06().await;
}

// Bullet 7: 401 → OwnKeyRefused, one request; 404 → ModelUnknown, one request.
#[tokio::test]
async fn t07_own_key_401_and_404_are_never_retried() {
    b07().await;
}

// Bullet 8: a 200 that is not JSON or has no usable content → InvalidAnswer, nothing cached.
#[tokio::test]
async fn t08_unusable_answers_are_invalid_and_never_cached() {
    b08().await;
}

// Bullet 9: Offline (and TLS) → Offline with its message, not retried.
#[tokio::test]
async fn t09_offline_gives_the_offline_message() {
    b09().await;
}

// Bullet 10: a Publik provider that times out (or answers 503 or 429) → one request, no sleep.
#[tokio::test]
async fn t10_publik_is_never_retried() {
    b10().await;
}

// Bullet 11: pick_local_model, docs/PUBLIK-INTEGRATION.md §3 (REFERENCE.md works the example).
#[test]
fn t11_pick_local_model_takes_the_largest_chat_model_at_or_under_15b() {
    assert_eq!(
        pick_local_model(&five_tags()),
        Some("qwen2.5:14b".to_string())
    );
    assert_eq!(
        pick_local_model(&tags(&[("nomic-embed-text", "137M")])),
        None,
        "only an embedding model"
    );
    assert_eq!(
        pick_local_model(&tags(&[
            ("beta-chat:7b", "7.6B"),
            ("alpha-chat:7b", "7.6B")
        ])),
        Some("alpha-chat:7b".to_string()),
        "two models of 7.6B: the first by name, whatever the listing order"
    );
    // The listing order never matters.
    assert_eq!(
        pick_local_model(&tags(&[
            ("mixtral:8x22b", "141B"),
            ("nomic-embed-text", "137M"),
            ("example-16b", "16.0B"),
            ("qwen2.5:14b", "14.8B"),
            ("llama3.2:3b", "3.2B"),
        ])),
        Some("qwen2.5:14b".to_string())
    );
    // "At or under 15B": 15.0B is chosen, 15.1B is not.
    assert_eq!(
        pick_local_model(&tags(&[
            ("small:1b", "1.2B"),
            ("over:15b", "15.1B"),
            ("edge:15b", "15.0B"),
        ])),
        Some("edge:15b".to_string())
    );
    // A name containing `embed` is dropped whatever its size.
    assert_eq!(
        pick_local_model(&tags(&[
            ("llama3.2:3b", "3.2B"),
            ("example-embed-xl", "7.0B"),
        ])),
        Some("llama3.2:3b".to_string())
    );
    assert_eq!(pick_local_model(&json!({"models": []})), None);
}

// Bullet 12: /api/tags refused → LocalNotRunning; no chat model → LocalNoModel; a base URL off
// this computer → LocalNotLocal and no request at all.
#[tokio::test]
async fn t12_local_provider_errors() {
    b12().await;
}

// Bullet 12 (Scope, Ollama detection): /api/tags is asked on the base URL's scheme, host and
// port, and the chat goes to the model pick_local_model chose.
#[tokio::test]
async fn t12_local_detection_uses_the_base_urls_host_and_port() {
    let t = text("b12 detection");
    let e = Env::memory(
        Script::new()
            .on(TAGS, vec![respond_json(200, &five_tags())])
            .on(CHAT, vec![ok("hello")]),
    );
    let reply = e
        .client
        .chat(
            &api::local_auto("http://127.0.0.1:11500/v1"),
            local_call(&t),
        )
        .await
        .expect("the detected model answers");
    assert_eq!(reply.content, "hello");
    let seen = e.t.requests();
    assert_eq!(
        seen.len(),
        2,
        "{:?}",
        seen.iter().map(|r| &r.url).collect::<Vec<_>>()
    );
    assert_eq!(seen[0].method, Method::Get);
    assert_eq!(seen[0].url, "http://127.0.0.1:11500/api/tags");
    assert_eq!(seen[1].method, Method::Post);
    assert_eq!(seen[1].url, "http://127.0.0.1:11500/v1/chat/completions");
    assert_eq!(json_body(&seen[1]), &body("qwen2.5:14b", &t));
}

// Bullet 13: list_models over a 14-entry `data` array → 14 and "Key works: 14 models
// available."; a refused key → OwnKeyRefused.
#[tokio::test]
async fn t13_list_models_counts_the_models() {
    b13().await;
}

// ---------------------------------------------------------------------------------------------
// Bullet 14.

// Bullet 14 (contract test 16, first half): the fake key never appears in the Display, Debug or
// serialisation of any AiError, in ResolvedProvider's Debug, or in anything a capturing
// `tracing` subscriber writes over tests 1 to 10 (and the other provider scenarios here). The
// log also never carries the user's text (docs/ARCHITECTURE.md §8).
#[test]
fn t14_the_key_never_appears_in_errors_debug_output_or_logs() {
    let log = Captured::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    let errors = tracing::subscriber::with_default(subscriber, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime");
        runtime.block_on(async {
            let mut all = b01().await;
            all.extend(b02().await);
            all.extend(b03().await);
            all.extend(b04().await);
            all.extend(b05().await);
            all.extend(b06().await);
            all.extend(b07().await);
            all.extend(b08().await);
            all.extend(b09().await);
            all.extend(b10().await);
            all.extend(b12().await);
            all.extend(b13().await);
            all.extend(err_403().await);
            all.extend(err_400().await);
            all.extend(err_429().await);
            all.extend(err_local_timeout().await);
            all.extend(err_unrecorded().await);
            all.extend(err_retry_kinds().await);
            secrets_round_trip();
            all
        })
    });

    assert!(
        errors.len() >= 30,
        "the scenarios produced {} errors",
        errors.len()
    );
    for e in &errors {
        let shown = e.to_string();
        let debug = format!("{e:?}");
        let serialised = serde_json::to_string(e).expect("an AiError serialises");
        assert!(!shown.contains(KEY), "Display carries the key: {shown}");
        assert!(!debug.contains(KEY), "Debug carries the key: {debug}");
        assert!(
            !serialised.contains(KEY),
            "the serialised error carries the key: {serialised}"
        );
    }
    for p in [own_key(), publik()] {
        let debug = format!("{p:?}");
        assert!(
            !debug.contains(KEY),
            "ResolvedProvider's Debug carries the key: {debug}"
        );
    }

    let written = log.text();
    assert!(
        !written.is_empty(),
        "the provider layer writes the log events of docs/ARCHITECTURE.md §8"
    );
    if let Some(line) = written.lines().find(|l| l.contains(KEY)) {
        panic!("a log line carries the key: {line}");
    }
    if let Some(line) = written.lines().find(|l| l.contains(MARK)) {
        panic!("a log line carries the user's text: {line}");
    }
}

// ---------------------------------------------------------------------------------------------
// Bullets 15 to 18.

// Bullet 15: secrets.json is created with mode 0600 on Unix; saving then loading returns the
// key; the file is `{"version": 1, "own_key": …}` (docs/ARCHITECTURE.md §4).
#[test]
fn t15_secrets_json_keeps_the_own_key() {
    secrets_round_trip();
}

/// The settings example of docs/ARCHITECTURE.md §4, with values that differ from the defaults
/// where the schema allows, so a save that wrote defaults would show.
fn example_settings() -> Value {
    json!({
        "version": 1,
        "hotkeys": {"check": "Ctrl+Alt+R"},
        "check": {"dialect": "british", "disabled_categories": ["style"]},
        "providers": {"chat": "own_key", "rewrite_quality": "standard", "search": null,
            "own_key": {"base_url": "https://api.example.com/v1", "chat_model": "example-chat"},
            "local": {"chat_base_url": "http://localhost:11434/v1", "chat_model": null}},
        "privacy": {"redaction": true, "remember_answers_days": 3, "sent_log": true,
                    "blocklist": ["1password.exe", "bitwarden.exe", "keepass.exe", "keepassxc.exe",
                                  "dashlane.exe", "enpass.exe", "nordpass.exe", "proton pass.exe"]},
        "capture": {"copy_timeout_ms": 400, "paste_restore_delay_ms": 250},
        "plagiarism": {"citation_style": "apa7"},
        "ui": {"start_with_os": false, "update_check": true, "paused": false, "welcomed": false}
    })
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{} could not be read: {e}", path.display()))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&read_text(path))
        .unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("the directory reads")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

// Bullet 16: a round trip keeps the unknown field `"future": {"x": 1}` and every known value;
// the write leaves no temporary file behind.
#[test]
fn t16_settings_round_trip_keeps_unknown_fields() {
    let dir = tempfile::tempdir().expect("a temporary config directory");
    let file = dir.path().join(api::SETTINGS_FILE);
    let mut original = example_settings();
    original["future"] = json!({"x": 1});
    fs::write(
        &file,
        serde_json::to_string_pretty(&original).expect("JSON serialises"),
    )
    .expect("settings.json is written");

    let settings = api::settings_load(dir.path());
    assert!(!api::settings_read_only(&settings), "version 1 is writable");
    api::settings_save(dir.path(), &settings).expect("version 1 saves");

    let saved = read_json(&file);
    assert_eq!(
        saved["future"],
        json!({"x": 1}),
        "the unknown field is kept"
    );
    assert_eq!(saved, original, "every value survives the round trip");
    assert_eq!(
        file_names(dir.path()),
        [api::SETTINGS_FILE],
        "the temporary file was renamed over settings.json"
    );
}

// Bullet 16: `"version": 2` loads read-only, saving is refused with "Settings were written by a
// newer Redpen.", and the file is left as it was.
#[test]
fn t16_newer_settings_open_read_only_and_refuse_to_save() {
    let dir = tempfile::tempdir().expect("a temporary config directory");
    let file = dir.path().join(api::SETTINGS_FILE);
    let mut newer = example_settings();
    newer["version"] = json!(2);
    newer["future"] = json!({"x": 1});
    let text = serde_json::to_string_pretty(&newer).expect("JSON serialises");
    fs::write(&file, &text).expect("settings.json is written");

    let settings = api::settings_load(dir.path());
    assert!(
        api::settings_read_only(&settings),
        "a newer version is read-only"
    );
    let refused = api::settings_save(dir.path(), &settings);
    assert_eq!(refused, Err(MSG_NEWER_SETTINGS.to_string()));
    assert_eq!(read_text(&file), text, "the newer file is untouched");
    assert_eq!(file_names(dir.path()), [api::SETTINGS_FILE]);
}

// Scope (settings.json, the full schema with defaults): with no file, the settings are the
// defaults of docs/ARCHITECTURE.md §4, and saving writes every field. `check.dialect` defaults
// from the operating system's locale, so it is checked to be one of the five dialects only.
#[test]
fn e_settings_defaults_are_the_full_schema_of_architecture_4() {
    let dir = tempfile::tempdir().expect("a temporary config directory");
    let settings = api::settings_load(dir.path());
    assert!(!api::settings_read_only(&settings));
    api::settings_save(dir.path(), &settings).expect("the defaults save");
    let mut saved = read_json(&dir.path().join(api::SETTINGS_FILE));

    let dialect = saved["check"]["dialect"].clone();
    assert!(
        ["american", "british", "australian", "canadian", "indian"]
            .iter()
            .any(|d| dialect == *d),
        "check.dialect is {dialect}"
    );
    saved["check"]
        .as_object_mut()
        .expect("check is an object")
        .remove("dialect");

    let expected = json!({
        "version": 1,
        "hotkeys": {"check": "Ctrl+Alt+R"},
        "check": {"disabled_categories": []},
        "providers": {"chat": null, "rewrite_quality": "standard", "search": null,
            "own_key": {"base_url": "", "chat_model": ""},
            "local": {"chat_base_url": "http://localhost:11434/v1", "chat_model": null}},
        "privacy": {"redaction": true, "remember_answers_days": 7, "sent_log": false,
                    "blocklist": ["1password.exe", "bitwarden.exe", "keepass.exe", "keepassxc.exe",
                                  "dashlane.exe", "enpass.exe", "nordpass.exe", "proton pass.exe"]},
        "capture": {"copy_timeout_ms": 400, "paste_restore_delay_ms": 250},
        "plagiarism": {"citation_style": "apa7"},
        "ui": {"start_with_os": false, "update_check": true, "paused": false, "welcomed": false}
    });
    assert_eq!(saved, expected);
}

// Bullet 17: three sent calls and one cache hit for fix-grammar → calls == 3 for that day,
// provider and feature; the charges the status sink reports are added; another feature gets its
// own row, named in kebab-case.
#[tokio::test]
async fn t17_usage_daily_counts_sent_calls_not_cache_hits() {
    let texts = [text("b17 one"), text("b17 two"), text("b17 three")];
    let test_text = text("b17 test");
    let e = Env::charging(
        FakeTransport::scripted(own_entries(&[&texts[0], &texts[1], &texts[2], &test_text])),
        210,
    );
    api::set_privacy(&e.client, 7, false);
    for (i, t) in texts.iter().enumerate() {
        let reply = e
            .client
            .chat(&own_key(), call(t, CachePolicy::Disk))
            .await
            .expect("a sent call is answered");
        assert!(!reply.from_cache, "call {i} is sent");
        assert_eq!(reply.charge_micros, Some(210), "the sink's charge");
    }
    let hit = e
        .client
        .chat(&own_key(), call(&texts[0], CachePolicy::Disk))
        .await
        .expect("the cache hit is answered");
    assert!(hit.from_cache);
    assert_eq!(e.t.requests_seen().len(), 3);
    assert_eq!(
        e.sink.statuses(),
        [200, 200, 200],
        "one status per response"
    );

    let conn = e.sqlite();
    assert_eq!(
        usage(&conn, T0_DAY, "own_key", "fix-grammar"),
        Some((3, 630)),
        "calls 3 and 3 × 210 micros"
    );
    assert_eq!(
        count(&conn, "usage_daily"),
        1,
        "one row per (day, provider, feature)"
    );

    let mut test_call = call(&test_text, CachePolicy::Off);
    test_call.feature = Feature::Test;
    e.client
        .chat(&own_key(), test_call)
        .await
        .expect("the Test call is answered");
    assert_eq!(usage(&conn, T0_DAY, "own_key", "test"), Some((1, 210)));
    assert_eq!(
        usage(&conn, T0_DAY, "own_key", "fix-grammar"),
        Some((3, 630))
    );
    assert_eq!(count(&conn, "usage_daily"), 2);
}

// Bullet 17 and the Design notes' local day: the day comes from the clock's UTC offset
// (REFERENCE.md: 23:30Z at UTC+1 is the next day; 02:00Z at UTC-5 is the day before).
#[tokio::test]
async fn t17_usage_day_is_the_local_day() {
    let late = text("b17 late");
    let early = text("b17 early");
    let e = Env::on_disk(FakeTransport::scripted(own_entries(&[&late, &early])));
    api::set_privacy(&e.client, 7, false);

    e.clock.set(1_790_033_400_000);
    e.clock.set_utc_offset_minutes(60);
    e.client
        .chat(&own_key(), call(&late, CachePolicy::Off))
        .await
        .expect("the late call is answered");
    e.clock.set(1_790_042_400_000);
    e.clock.set_utc_offset_minutes(-300);
    e.client
        .chat(&own_key(), call(&early, CachePolicy::Off))
        .await
        .expect("the early call is answered");

    let conn = e.sqlite();
    assert_eq!(
        usage(&conn, "2026-09-22", "own_key", "fix-grammar"),
        Some((1, 0)),
        "2026-09-21T23:30Z at UTC+1"
    );
    assert_eq!(
        usage(&conn, "2026-09-21", "own_key", "fix-grammar"),
        Some((1, 0)),
        "2026-09-22T02:00Z at UTC-5"
    );
}

// Bullet 18: no sent_log rows while the log is off (also by default); with it on, one row per
// sent call with the user content (not the system prompt, not for a cache hit); rows older than
// 7 days are deleted whatever remember_answers_days says; sent_log_clear empties the table.
#[tokio::test]
async fn t18_sent_log_only_while_on_and_for_seven_days() {
    let by_default = text("b18 default");
    let off = text("b18 off");
    let old = text("b18 eight days");
    let recent = text("b18 six days");
    let e = Env::on_disk(FakeTransport::scripted(own_entries(&[
        &by_default,
        &off,
        &old,
        &recent,
    ])));

    e.client
        .chat(&own_key(), call(&by_default, CachePolicy::Off))
        .await
        .expect("the call is answered");
    assert_eq!(
        count(&e.sqlite(), "sent_log"),
        0,
        "the log is off by default"
    );
    api::set_privacy(&e.client, 7, false);
    e.client
        .chat(&own_key(), call(&off, CachePolicy::Off))
        .await
        .expect("the call is answered");
    assert_eq!(
        count(&e.sqlite(), "sent_log"),
        0,
        "no rows while the log is off"
    );

    api::set_privacy(&e.client, 7, true);
    e.clock.set(T0 - 8 * DAY_MS);
    e.client
        .chat(&own_key(), call(&old, CachePolicy::Disk))
        .await
        .expect("the old call is answered");
    e.clock.set(T0 - 6 * DAY_MS);
    for _ in 0..2 {
        e.client
            .chat(&own_key(), call(&recent, CachePolicy::Disk))
            .await
            .expect("the recent call is answered");
    }
    assert_eq!(
        e.t.requests_seen().len(),
        4,
        "the second recent call is a cache hit"
    );

    let rows = sent_rows(&e.sqlite());
    assert_eq!(
        rows.len(),
        2,
        "one row per sent call, none for the cache hit"
    );
    for (row, (at_ms, sent)) in rows
        .iter()
        .zip([(T0 - 8 * DAY_MS, &old), (T0 - 6 * DAY_MS, &recent)])
    {
        assert_eq!(row.at_ms, at_ms);
        assert_eq!(row.provider, "own_key");
        assert_eq!(row.endpoint, "/chat/completions");
        assert_eq!(row.feature, "fix-grammar");
        assert_eq!(row.model, OWN_MODEL);
        assert_eq!(&row.sent_text, sent, "exactly the user content sent");
        assert!(
            row.bytes >= i64::try_from(sent.len()).expect("a small length"),
            "bytes {} for a {}-byte text",
            row.bytes,
            sent.len()
        );
    }

    e.clock.set(T0);
    api::sweep(&e.db, T0, 30);
    let rows = sent_rows(&e.sqlite());
    assert_eq!(rows.len(), 1, "the 8-day-old row is deleted after 7 days");
    assert_eq!(rows[0].sent_text, recent);

    api::sent_log_clear(&e.db);
    assert_eq!(
        count(&e.sqlite(), "sent_log"),
        0,
        "sent_log_clear empties it"
    );
}

// ---------------------------------------------------------------------------------------------
// Error paths and the rest of the Scope.

// The messages of the variants without fields, and their serialisation (spec, `AiError` table).
#[test]
fn e_messages_of_the_variants_without_fields() {
    for (e, kind, message) in [
        (
            AiError::NotConfigured,
            "NotConfigured",
            "Choose an AI provider in Settings → Providers.",
        ),
        (AiError::Offline, "Offline", MSG_OFFLINE),
        (AiError::LocalNotRunning, "LocalNotRunning", MSG_NOT_RUNNING),
        (AiError::LocalNoModel, "LocalNoModel", MSG_NO_MODEL),
        (AiError::LocalNotLocal, "LocalNotLocal", MSG_NOT_LOCAL),
        (AiError::InvalidAnswer, "InvalidAnswer", MSG_INVALID),
    ] {
        let v = check_error(&e, kind, message);
        assert_eq!(
            v.as_object().map(|o| o.len()),
            Some(2),
            "{kind} serialises as kind and message only: {v}"
        );
    }
}

// Design notes (error mapping): 403 → OwnKeyRefused {code: 403}, never retried.
#[tokio::test]
async fn e_own_key_403_is_refused_with_its_code() {
    err_403().await;
}

// Design notes (error mapping): any other 4xx → ProviderError {code}, never retried.
#[tokio::test]
async fn e_other_4xx_is_a_provider_error_and_never_retried() {
    err_400().await;
}

// Spec, `RateLimited`: 429 on all three attempts → the own key's message.
#[tokio::test]
async fn e_own_key_429_three_times_is_rate_limited() {
    err_429().await;
}

// Spec, `Timeout`: the local model's message names it and its 120 seconds.
#[tokio::test]
async fn e_local_timeout_names_the_local_model() {
    err_local_timeout().await;
}

// Design notes (error mapping): an unrecorded request → Unrecorded with the fake transport's
// message (docs/TESTING.md §1).
#[tokio::test]
async fn e_unrecorded_request_gives_the_fake_transports_message() {
    err_unrecorded().await;
}

// Scope (retries): own key and local retry; the fake provider, like Publik, does not.
#[tokio::test]
async fn e_retries_are_for_own_key_and_local_only() {
    err_retry_kinds().await;
}

// Scope (local): localhost, 127.0.0.1 and ::1 are local; the chat goes to that host.
#[tokio::test]
async fn e_local_hosts_are_localhost_127_0_0_1_and_ipv6_loopback() {
    for (base, chat_url) in [
        (
            "http://localhost:11434/v1",
            "http://localhost:11434/v1/chat/completions",
        ),
        (
            "http://127.0.0.1:11434/v1",
            "http://127.0.0.1:11434/v1/chat/completions",
        ),
        (
            "http://[::1]:11434/v1",
            "http://[::1]:11434/v1/chat/completions",
        ),
    ] {
        let t = text("elocalhosts");
        let e = Env::memory(
            Script::new()
                .on(TAGS, vec![respond_json(200, &five_tags())])
                .on(CHAT, vec![ok("hello")]),
        );
        let reply = e
            .client
            .chat(&local(base, "llama3.2:3b"), local_call(&t))
            .await
            .unwrap_or_else(|err| panic!("{base} is local: {err:?}"));
        assert_eq!(reply.content, "hello");
        let chats: Vec<HttpRequest> =
            e.t.requests()
                .into_iter()
                .filter(|r| request_path(&r.url) == CHAT)
                .collect();
        assert_eq!(chats.len(), 1, "{base}");
        assert_eq!(chats[0].url, chat_url);
        assert_eq!(json_body(&chats[0]), &body("llama3.2:3b", &t));
    }
}

// Scope (answer cache): cache_clear empties ai_cache, and the next identical call is sent.
#[tokio::test]
async fn e_cache_clear_empties_ai_cache() {
    let t = text("ecacheclear");
    let e = Env::on_disk(FakeTransport::scripted(own_entries(&[&t])));
    api::set_privacy(&e.client, 7, false);
    e.client
        .chat(&own_key(), call(&t, CachePolicy::Disk))
        .await
        .expect("the call is answered");
    assert_eq!(count(&e.sqlite(), "ai_cache"), 1);
    api::cache_clear(&e.db);
    assert_eq!(count(&e.sqlite(), "ai_cache"), 0, "cache_clear empties it");
    let reply = e
        .client
        .chat(&own_key(), call(&t, CachePolicy::Disk))
        .await
        .expect("the call is sent again");
    assert!(!reply.from_cache, "a cleared answer is not served");
    assert_eq!(e.t.requests_seen().len(), 2);
}

// Scope (answer cache, usage, sent log): redpen.db has the schema of docs/ARCHITECTURE.md §4:
// user_version 1, WAL, three STRICT tables with these columns, and their two indexes.
#[test]
fn e_redpen_db_has_the_schema_of_architecture_4() {
    let dir = tempfile::tempdir().expect("a temporary data directory");
    let _db = api::file_db(dir.path());
    let conn = Connection::open(dir.path().join(api::DB_FILE)).expect("redpen.db opens");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("user_version reads");
    assert_eq!(version, 1);
    let journal: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .expect("journal_mode reads");
    assert_eq!(journal.to_ascii_lowercase(), "wal");

    for (table, expected) in [
        (
            "ai_cache",
            &[
                "key",
                "provider",
                "model",
                "prompt_id",
                "prompt_version",
                "response_json",
                "charge_micros",
                "created_at_ms",
            ][..],
        ),
        (
            "usage_daily",
            &["day", "provider", "feature", "calls", "charge_micros"][..],
        ),
        (
            "sent_log",
            &[
                "id",
                "at_ms",
                "provider",
                "endpoint",
                "feature",
                "model",
                "sent_text",
                "bytes",
            ][..],
        ),
    ] {
        assert_eq!(columns(&conn, table), expected, "{table}");
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("{table} exists: {e}"));
        assert!(
            sql.to_ascii_uppercase().contains("STRICT"),
            "{table}: {sql}"
        );
    }
    for index in ["ai_cache_created", "sent_log_at"] {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                [index],
                |r| r.get(0),
            )
            .expect("sqlite_master reads");
        assert_eq!(n, 1, "index {index}");
    }
}

// ---------------------------------------------------------------------------------------------
// The sweeps at start and every hour, an own key without its parts, the own key's two files and
// the local Test.

const HOUR: Duration = Duration::from_secs(3_600);
const HALF_HOUR_MS: i64 = 1_800_000;

/// A sleeper for the hourly sweep: it keeps every duration asked for and moves the fake clock on
/// by it. After `wakes` sleeps it never wakes again, so the sweeping loop parks there.
struct Hours {
    clock: Arc<FakeClock>,
    wakes: usize,
    asked: Mutex<Vec<Duration>>,
}

impl Hours {
    fn asked(&self) -> Vec<Duration> {
        self.asked.lock().expect("log is not poisoned").clone()
    }
}

#[async_trait::async_trait]
impl Sleeper for Hours {
    async fn sleep(&self, d: Duration) {
        let n = {
            let mut asked = self.asked.lock().expect("log is not poisoned");
            asked.push(d);
            asked.len()
        };
        if n > self.wakes {
            std::future::pending::<()>().await;
        }
        self.clock.advance(d);
    }
}

// Bullets 4 and 18, the start-up sweep itself: a client starting at T0 (answers remembered for 7
// days until Settings say otherwise, Design notes) deletes the cached answer and the log row made
// 8 days earlier, with no other call, and keeps the ones made 6 days earlier.
#[tokio::test]
async fn t04_the_start_up_sweep_runs_when_the_client_starts() {
    let old = text("b04 start-up eight days");
    let recent = text("b04 start-up six days");
    let e = Env::on_disk(FakeTransport::scripted(own_entries(&[&old, &recent])));
    api::set_privacy(&e.client, 7, true);
    e.clock.set(T0 - 8 * DAY_MS);
    e.client
        .chat(&own_key(), call(&old, CachePolicy::Disk))
        .await
        .expect("the old call is answered");
    e.clock.set(T0 - 6 * DAY_MS);
    e.client
        .chat(&own_key(), call(&recent, CachePolicy::Disk))
        .await
        .expect("the recent call is answered");
    assert_eq!(count(&e.sqlite(), "ai_cache"), 2);
    assert_eq!(count(&e.sqlite(), "sent_log"), 2);

    e.clock.set(T0);
    let _started = AiClient::new(
        e.t.clone(),
        e.db.clone(),
        e.clock.clone(),
        e.sleeper.clone(),
        e.sink.clone(),
    );
    let rows = cache_rows(&e.sqlite());
    assert_eq!(
        rows.len(),
        1,
        "the start-up sweep deletes the 8-day-old answer and keeps the 6-day-old one"
    );
    assert_eq!(rows[0].created_at_ms, T0 - 6 * DAY_MS);
    let sent = sent_rows(&e.sqlite());
    assert_eq!(
        sent.len(),
        1,
        "the start-up sweep deletes the 8-day-old log row"
    );
    assert_eq!(sent[0].sent_text, recent);
    assert!(
        e.sleeper.sleeps().is_empty(),
        "the start-up sweep waits for nothing"
    );
}

// Scope (answer cache, sent log): the sweep repeats every hour while the app runs
// (`keep_sweeping`, Design notes). An answer and a log row made 7 days less half an hour before
// T0 are not yet older than 7 days, so the start-up sweep at T0 keeps them; the sweep an hour
// later, when they are 7 days and half an hour old, deletes them.
#[tokio::test]
async fn e_the_sweep_repeats_every_hour() {
    let t = text("ehourly");
    let e = Env::on_disk(FakeTransport::scripted(own_entries(&[&t])));
    api::set_privacy(&e.client, 7, true);
    e.clock.set(T0 - 7 * DAY_MS + HALF_HOUR_MS);
    e.client
        .chat(&own_key(), call(&t, CachePolicy::Disk))
        .await
        .expect("the call is answered");

    e.clock.set(T0);
    let hours = Arc::new(Hours {
        clock: e.clock.clone(),
        wakes: 1,
        asked: Mutex::new(Vec::new()),
    });
    let client = AiClient::new(
        e.t.clone(),
        e.db.clone(),
        e.clock.clone(),
        hours.clone(),
        e.sink.clone(),
    );
    assert_eq!(
        count(&e.sqlite(), "ai_cache"),
        1,
        "at start the answer is under 7 days old"
    );
    assert_eq!(
        count(&e.sqlite(), "sent_log"),
        1,
        "at start the log row is under 7 days old"
    );

    let mut sweeping = std::pin::pin!(api::keep_sweeping(&client));
    let first = std::future::poll_fn(|cx| Poll::Ready(sweeping.as_mut().poll(cx))).await;
    assert!(
        first.is_pending(),
        "the sweep goes on for as long as the app runs"
    );
    assert_eq!(
        hours.asked(),
        [HOUR, HOUR],
        "an hour's wait before each sweep"
    );
    assert_eq!(e.clock.now_ms(), T0 + 3_600_000);
    assert_eq!(
        count(&e.sqlite(), "ai_cache"),
        0,
        "an hour later the answer is older than 7 days"
    );
    assert_eq!(
        count(&e.sqlite(), "sent_log"),
        0,
        "an hour later the log row is older than 7 days"
    );
}

// Design notes (an own key needs its parts): no key, an empty key, no chat model, or a base URL
// that is not http or https gives NotConfigured, for a chat and for the Test alike, and nothing
// is sent, counted, logged or cached.
#[tokio::test]
async fn e_an_own_key_without_its_parts_is_not_configured() {
    let t = text("enotconfigured");
    let e = Env::on_disk(Script::new());
    api::set_privacy(&e.client, 7, true);

    let mut no_key = own_key();
    no_key.key = None;
    let mut empty_key = own_key();
    empty_key.key = Some(secret(""));
    let mut no_model = own_key();
    no_model.chat_model = String::new();
    let mut no_url = own_key();
    no_url.base_url = String::new();
    let mut not_http = own_key();
    not_http.base_url = "ftp://api.example.com/v1".to_string();

    for p in [
        no_key.clone(),
        empty_key,
        no_model,
        no_url,
        not_http.clone(),
    ] {
        let err = failed(e.client.chat(&p, call(&t, CachePolicy::Disk)).await);
        check_error(&err, "NotConfigured", MSG_NOT_CONFIGURED);
    }
    for p in [no_key, not_http] {
        assert_eq!(
            api::provider_test(&e.client, &p).await,
            (false, MSG_NOT_CONFIGURED.to_string())
        );
    }

    assert!(
        e.t.requests().is_empty(),
        "nothing is sent: {:?}",
        e.t.requests()
            .iter()
            .map(|r| r.url.clone())
            .collect::<Vec<_>>()
    );
    let conn = e.sqlite();
    assert_eq!(
        count(&conn, "usage_daily"),
        0,
        "a call refused before sending is not counted"
    );
    assert_eq!(count(&conn, "sent_log"), 0, "nor logged");
    assert_eq!(count(&conn, "ai_cache"), 0, "nor cached");
}

// Scope (own key): the base URL and chat model come from settings.json and the key from
// secrets.json; while no key is saved the own key is not configured.
#[test]
fn e_the_own_key_comes_from_settings_json_and_secrets_json() {
    let dir = tempfile::tempdir().expect("a temporary config directory");
    fs::write(
        dir.path().join(api::SETTINGS_FILE),
        example_settings().to_string(),
    )
    .expect("settings.json is written");

    match api::own_key_from_files(dir.path()) {
        Ok(p) => panic!("an own key with no key saved resolved to {p:?}"),
        Err(err) => {
            check_error(&err, "NotConfigured", MSG_NOT_CONFIGURED);
        }
    }

    api::own_key_save(dir.path(), KEY);
    let p = api::own_key_from_files(dir.path()).expect("the own key resolves");
    assert_eq!(p.kind, ProviderKind::OwnKey);
    assert_eq!(p.base_url, OWN_BASE);
    assert_eq!(p.chat_model, OWN_MODEL);
    assert_eq!(p.key.as_ref().map(|k| k.expose_secret()), Some(KEY));
    let settings = read_text(&dir.path().join(api::SETTINGS_FILE));
    assert!(!settings.contains(KEY), "the key is never in settings.json");
}

// Design notes (the Test button): for a local provider the Test asks Ollama and names the model
// Redpen will use, the chosen one or Ollama's pick; it fails with the error's message when
// Ollama is not running or the address is not on this computer.
#[tokio::test]
async fn e_the_local_test_names_the_model_redpen_will_use() {
    let e = Env::memory(Script::new().on(TAGS, vec![respond_json(200, &five_tags()); 2]));
    assert_eq!(
        api::provider_test(&e.client, &api::local_auto(LOCAL_BASE)).await,
        (
            true,
            "Ollama is running. Redpen will use qwen2.5:14b.".to_string()
        )
    );
    assert_eq!(
        api::provider_test(&e.client, &local(LOCAL_BASE, "llama3.2:3b")).await,
        (
            true,
            "Ollama is running. Redpen will use llama3.2:3b.".to_string()
        )
    );
    let seen = e.t.requests();
    assert_eq!(seen.len(), 2, "each Test asks Ollama");
    for req in &seen {
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.url, "http://localhost:11434/api/tags");
    }

    let e = Env::memory(Script::new().on(TAGS, vec![Err(TransportError::Offline)]));
    assert_eq!(
        api::provider_test(&e.client, &api::local_auto(LOCAL_BASE)).await,
        (false, MSG_NOT_RUNNING.to_string())
    );

    let e = Env::memory(Script::new());
    assert_eq!(
        api::provider_test(&e.client, &api::local_auto("http://192.168.1.5:11434/v1")).await,
        (false, MSG_NOT_LOCAL.to_string())
    );
    assert!(e.t.requests().is_empty(), "no request leaves this computer");
}

// ---------------------------------------------------------------------------------------------
// The remaining error paths: the two files' errors, Ollama's other answers, a model list off
// this computer, a database failure during a call, and a header the production transport cannot
// send.

const SETTINGS_IO: &str = "settings.json could not be read or written: ";
const SECRETS_IO: &str = "secrets.json could not be read or written: ";
const MSG_SECRETS_DAMAGED: &str =
    "secrets.json is damaged. Save your key again in Settings → Providers.";

/// The `Display` of the error kinds an operating system gives for a file that is a folder, or a
/// file it may not replace. An I/O error's reason is one of these and nothing more.
fn io_kinds() -> Vec<String> {
    use std::io::ErrorKind;
    [
        ErrorKind::PermissionDenied,
        ErrorKind::IsADirectory,
        ErrorKind::NotADirectory,
        ErrorKind::AlreadyExists,
        ErrorKind::DirectoryNotEmpty,
        ErrorKind::InvalidInput,
        ErrorKind::Other,
    ]
    .iter()
    .map(ErrorKind::to_string)
    .collect()
}

/// An I/O error of one of the two files: the message is `prefix` and the error's kind, and
/// neither the message nor the `Debug` output holds the folder's path or any name in it (the
/// path would name the user's home folder).
fn check_io_error<E: std::fmt::Display + std::fmt::Debug>(err: &E, prefix: &str, dir: &Path) {
    let message = err.to_string();
    let reason = message
        .strip_prefix(prefix)
        .unwrap_or_else(|| panic!("{message:?} begins with {prefix:?}"));
    let kinds = io_kinds();
    assert!(
        kinds.iter().any(|k| k == reason),
        "{reason:?} is the error's kind and nothing more, one of {kinds:?}"
    );
    let shown = format!("{message} {err:?}");
    assert!(
        !shown.contains(&dir.display().to_string()),
        "{shown:?} holds the path"
    );
    for part in dir.components() {
        if let std::path::Component::Normal(name) = part {
            let name = name.to_string_lossy();
            // Short names ("tmp") could be part of an ordinary word; the folder's own name is
            // never short.
            if name.len() >= 4 {
                assert!(!shown.contains(name.as_ref()), "{shown:?} names {name:?}");
            }
        }
    }
    assert!(
        !shown.contains('/') && !shown.contains('\\'),
        "{shown:?} holds a path separator"
    );
}

// Design notes (`settings.json`): an I/O error is SettingsError::Io, "settings.json could not be
// read or written: " and the operating system's error kind, never a path. A folder where the file
// should be fails the read and the write alike, and the write leaves it as it was.
#[test]
fn e_settings_json_io_errors_name_the_kind_and_no_path() {
    let dir = tempfile::tempdir().expect("a temporary config directory");
    let file = dir.path().join(api::SETTINGS_FILE);
    fs::create_dir(&file).expect("a folder named settings.json");

    let err = api::settings_try_load(dir.path()).expect_err("a folder is not a settings file");
    assert!(matches!(err, api::SettingsError::Io(_)), "{err:?}");
    check_io_error(&err, SETTINGS_IO, dir.path());

    let err = api::settings_try_save(dir.path(), &redpen_core::store::Settings::default())
        .expect_err("settings.json cannot replace a folder");
    assert!(matches!(err, api::SettingsError::Io(_)), "{err:?}");
    check_io_error(&err, SETTINGS_IO, dir.path());
    assert!(file.is_dir(), "the folder is left as it was");
}

// Design notes (`settings.json`): a file that is not a JSON object, is not JSON, or has a known
// field of the wrong type in a version-1 file is SettingsError::Invalid, "settings.json is not
// valid: " and the reason.
#[test]
fn e_settings_json_that_is_not_valid_says_why() {
    let dir = tempfile::tempdir().expect("a temporary config directory");
    let file = dir.path().join(api::SETTINGS_FILE);

    fs::write(&file, "[1, 2]").expect("settings.json is written");
    let err = api::settings_try_load(dir.path()).expect_err("an array is not settings");
    assert_eq!(
        err,
        api::SettingsError::Invalid("the file is not a JSON object".to_string())
    );
    assert_eq!(
        err.to_string(),
        "settings.json is not valid: the file is not a JSON object"
    );

    for text in [
        "{not json",
        r#"{"version": 1, "privacy": {"remember_answers_days": "seven"}}"#,
    ] {
        fs::write(&file, text).expect("settings.json is written");
        let err = api::settings_try_load(dir.path()).expect_err(text);
        let api::SettingsError::Invalid(reason) = &err else {
            panic!("{text}: {err:?}");
        };
        assert!(!reason.is_empty(), "{text}: the reason is given");
        assert_eq!(
            err.to_string(),
            format!("settings.json is not valid: {reason}"),
            "{text}"
        );
    }
}

// Design notes (`secrets.json`): an I/O error is SecretsError::Io, "secrets.json could not be read
// or written: " and the error's kind only, never a path, and never the key being saved.
#[test]
fn e_secrets_json_io_errors_name_the_kind_and_no_path_or_key() {
    let dir = tempfile::tempdir().expect("a temporary config directory");
    let file = dir.path().join(api::SECRETS_FILE);
    fs::create_dir(&file).expect("a folder named secrets.json");

    let err = api::secrets_try_load(dir.path()).expect_err("a folder is not a secrets file");
    assert!(matches!(err, api::SecretsError::Io(_)), "{err:?}");
    check_io_error(&err, SECRETS_IO, dir.path());

    let err =
        api::own_key_try_save(dir.path(), KEY).expect_err("secrets.json cannot replace a folder");
    assert!(matches!(err, api::SecretsError::Io(_)), "{err:?}");
    check_io_error(&err, SECRETS_IO, dir.path());
    assert!(
        !format!("{err} {err:?}").contains(KEY),
        "the error carries the key"
    );
    assert!(file.is_dir(), "the folder is left as it was");
}

// Design notes (`secrets.json`): a file that does not parse, is not an object, or holds a key that
// is not a string is SecretsError::Invalid with one fixed message, which quotes nothing of the
// file.
#[test]
fn e_damaged_secrets_json_has_one_fixed_message() {
    let dir = tempfile::tempdir().expect("a temporary config directory");
    for text in [
        KEY.to_string(),
        format!(r#"{{"version": 1, "own_key": "{KEY}"#),
        format!(r#""{KEY}""#),
        format!(r#"{{"version": 1, "own_key": ["{KEY}"]}}"#),
    ] {
        fs::write(dir.path().join(api::SECRETS_FILE), &text).expect("secrets.json is written");
        let err = match api::secrets_try_load(dir.path()) {
            Ok(_) => panic!("a damaged secrets.json loaded"),
            Err(err) => err,
        };
        assert_eq!(err, api::SecretsError::Invalid);
        assert_eq!(err.to_string(), MSG_SECRETS_DAMAGED);
        assert!(!format!("{err:?}").contains(KEY), "Debug carries the key");
    }
}

// Design notes (the local provider, error mapping): Ollama's /api/tags answering a status other
// than 2xx or 404 is ProviderError {code}, retried on a 5xx as any local request is and not on
// another 4xx; a 2xx whose body is not JSON is InvalidAnswer. No chat follows either.
#[tokio::test]
async fn e_ollama_tags_other_statuses_and_a_body_that_is_not_json() {
    let t = text("etags");

    let e = Env::memory(Script::new().on(TAGS, vec![respond_json(503, &json!({})); 3]));
    let err = failed(
        e.client
            .chat(&api::local_auto(LOCAL_BASE), local_call(&t))
            .await,
    );
    let v = check_error(&err, "ProviderError", MSG_PROVIDER_503);
    assert_eq!(v["code"], 503, "{v}");
    assert_eq!(e.t.sent_to(TAGS), 3, "a 5xx is retried twice on local");
    assert_eq!(e.sleeper.sleeps(), RETRY_SLEEPS);
    assert_eq!(e.t.sent_to(CHAT), 0, "no chat without a model");

    let e = Env::memory(Script::new().on(TAGS, vec![respond_json(400, &json!({})); 3]));
    let err = failed(
        e.client
            .chat(&api::local_auto(LOCAL_BASE), local_call(&t))
            .await,
    );
    let v = check_error(
        &err,
        "ProviderError",
        "Your provider had a problem (400). Try again later.",
    );
    assert_eq!(v["code"], 400, "{v}");
    assert_eq!(e.t.sent_to(TAGS), 1, "another 4xx is never retried");
    assert!(e.sleeper.sleeps().is_empty());
    assert_eq!(e.t.sent_to(CHAT), 0);

    let e = Env::memory(Script::new().on(TAGS, vec![respond(200, b"not json"); 3]));
    let err = failed(
        e.client
            .chat(&api::local_auto(LOCAL_BASE), local_call(&t))
            .await,
    );
    check_error(&err, "InvalidAnswer", MSG_INVALID);
    assert_eq!(e.t.sent_to(TAGS), 1);
    assert_eq!(e.t.sent_to(CHAT), 0);
    assert_eq!(
        api::provider_test(&e.client, &api::local_auto(LOCAL_BASE)).await,
        (false, MSG_INVALID.to_string()),
        "the Test button gives the same error"
    );
}

// Design notes (the local provider): the localhost check comes before the model list too, so
// list_models on a local provider whose base URL is not this computer is LocalNotLocal and sends
// nothing.
#[tokio::test]
async fn e_list_models_off_this_computer_is_not_local_and_sends_nothing() {
    for base in [
        "http://192.168.1.5:11434/v1",
        "http://localhost.example.com:11434/v1",
        "http://localhost@192.168.1.5:11434/v1",
    ] {
        for p in [local(base, "llama3.2:3b"), api::local_auto(base)] {
            let e = Env::memory(Script::new());
            let err = match e.client.list_models(&p).await {
                Ok(n) => panic!("{base} listed {n} models"),
                Err(err) => err,
            };
            check_error(&err, "LocalNotLocal", MSG_NOT_LOCAL);
            assert!(e.t.requests().is_empty(), "no request at all for {base}");
        }
    }
}

/// Runs `scenario` under a capturing `tracing` subscriber, on a runtime of its own, and returns
/// what it logged.
fn logged<F: Future<Output = ()>>(scenario: impl FnOnce() -> F) -> String {
    let log = Captured::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime")
            .block_on(scenario());
    });
    log.text()
}

/// Two identical own-key calls, the answer cache on disk, the sent log on and a status sink that
/// charges 210 micros, after `tables` of redpen.db were dropped: both are answered, and the first
/// is sent and returns the sink's charge.
async fn calls_without(tables: &[&str]) {
    let t = text("edb");
    let e = Env::charging(FakeTransport::scripted(own_entries(&[&t])), 210);
    api::set_privacy(&e.client, 7, true);
    api::drop_tables(&e.db, tables);
    let first = e
        .client
        .chat(&own_key(), call(&t, CachePolicy::Disk))
        .await
        .unwrap_or_else(|err| panic!("without {tables:?} the call failed: {err:?}"));
    assert_eq!(first.content, "hello", "without {tables:?}");
    assert!(!first.from_cache, "without {tables:?}");
    assert_eq!(first.charge_micros, Some(210), "without {tables:?}");
    let second = e
        .client
        .chat(&own_key(), call(&t, CachePolicy::Disk))
        .await
        .unwrap_or_else(|err| panic!("without {tables:?} the second call failed: {err:?}"));
    assert_eq!(second.content, "hello", "without {tables:?}");
}

// Design notes (`redpen.db`): a database failure during a call is logged and never fails the call.
// With usage_daily, sent_log or ai_cache gone (each alone, then all three), counting the call,
// adding its charge, writing the sent log, looking the answer up or caching it fails; the call is
// still sent and answered with its charge, a warning is logged, and no log line carries the key
// or the text or says the chat failed.
#[test]
fn e_a_database_failure_is_logged_and_never_fails_the_call() {
    let healthy = logged(|| calls_without(&[]));
    assert!(
        !healthy.contains("WARN"),
        "a healthy database logs no warning: {healthy}"
    );
    for tables in [
        &["usage_daily"][..],
        &["sent_log"],
        &["ai_cache"],
        &["usage_daily", "sent_log", "ai_cache"],
    ] {
        let written = logged(|| calls_without(tables));
        assert!(
            written.lines().any(|l| l.contains("WARN")),
            "without {tables:?} the failure is logged: {written}"
        );
        assert!(!written.contains("chat failed"), "{written}");
        assert!(!written.contains(KEY), "a log line carries the key");
        assert!(!written.contains(MARK), "a log line carries the text");
    }
}

// Design notes (the production transport): a header value that cannot be sent, such as a key
// pasted with a line break, is TransportError::Offline, and nothing is sent. The request is
// refused while it is built, before any connection; the address is a port this test listens on,
// on this computer, so even a connection would not reach the network and would show.
#[tokio::test]
async fn e_the_production_transport_refuses_a_header_it_cannot_send() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port on this computer");
    listener
        .set_nonblocking(true)
        .expect("the listener does not block");
    let port = listener
        .local_addr()
        .expect("the listener's address")
        .port();
    let transport = ReqwestTransport::new().expect("the production transport builds");
    for key in [format!("{KEY}\n"), format!("{KEY}\r\nX-Injected: 1")] {
        let req = HttpRequest::get(format!("http://127.0.0.1:{port}{MODELS}"))
            .header(
                "Authorization",
                HeaderValue::secret(format!("Bearer {key}")),
            )
            .timeout(Duration::from_secs(2));
        let err = match transport.send(req).await {
            Ok(resp) => panic!("a header with a line break was sent: {}", resp.status),
            Err(err) => err,
        };
        assert_eq!(err, TransportError::Offline);
        assert!(
            !format!("{err} {err:?}").contains(KEY),
            "the error carries the key"
        );
    }
    match listener.accept() {
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
        Err(e) => panic!("the listener failed: {e}"),
        Ok((_, from)) => panic!("a connection was made from {from}"),
    }
}
