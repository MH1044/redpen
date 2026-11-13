//! Module 03 tests in the app crate (docs/modules/03-publik.md, Acceptance tests): the publik API
//! module `publik.rs` and the Tauri-free parts around it, the resolver, the status sink, the
//! commands, the tray item, the self-test and the proof. A test named `cNN_…` covers contract test
//! NN (C1 to C19 in the spec), `tNN_…` Redpen's own bullet NN, and `e_…` an error path or a Scope
//! bullet the lists leave implicit. The core's half (the client's publik API errors, the
//! idempotency key, the tier rule, the copy rule) is in crates/redpen-core/tests/03_publik.rs.
//!
//! Expected values come from the spec (Interfaces, the `AiError` table, the Design notes'
//! `DISCLOSURE` and `balance_line`, acceptance tests C1 to C19 and 1 to 11),
//! docs/PUBLIK-CONTRACT.md §3 and §5 to §8, docs/PUBLIK-INTEGRATION.md §2 to §4 and §9,
//! docs/ARCHITECTURE.md §4 and §5, and the hand-written responses in
//! tests/cases/03-publik/*.fake.json, whose invented values are true by construction
//! (tests/cases/03-publik/REFERENCE.md). Nothing here is taken from Redpen's own output.
//!
//! The credential lives where the contract puts it, under %LOCALAPPDATA% on Windows and
//! $XDG_CONFIG_HOME on Linux, so every test that can reach it runs in a `Sandbox`: one at a time,
//! with those variables pointed at a temporary folder (checked through `credential_path()` before
//! anything is written) and the PUBLIK_* variables cleared, everything restored afterwards. The
//! fake app token is built at run time, so the committed token file stays the only place a
//! `pat_` token can be. While that file holds no token, the tests that need the real one say so
//! and pass; the two that start the built binaries never run with a real token, so no test can
//! reach publik API.
//!
//! Transport: `Script` answers each `METHOD path` from its own queue, in order, and keeps every
//! request; a request with no answer left fails as unrecorded, so an extra request or a retry
//! shows. It matches on the path rather than the body's hash because provisioning sends a fresh
//! random install_id; an answer may be a function of the request, to echo that install_id.
//!
//! The spec's Interfaces name `publik.rs`'s public surface, `startup_self_test`,
//! `ConnectOutcome`, `RedpenResolver` and `PublikStatusSink`. The Scope also describes behaviour
//! without naming its function: the app token's file and override, the state the commands work
//! on, the start-up wallet read, the tray item, the proof's `run`. Those calls are gathered in
//! `mod api`, so a different name changes one line there and no expected value.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use redpen_core::clock::{FakeClock, FakeSleeper};
use redpen_core::net::{
    Body, HeaderValue, HttpRequest, HttpResponse, HttpTransport, Method, TransportError,
    canonical_json, request_hash, request_path, url_host,
};
use redpen_core::provider::{
    AiClient, AiError, CachePolicy, ChatCall, ExposeSecret, Feature, Message, ProviderKind,
    ResolvedProvider, SecretString, StatusSink,
};
use redpen_core::store::settings::{ChatChoice, RewriteQuality};
use redpen_core::store::{Db, Secrets, Settings};
use serde_json::{Value, json};

use api::{ClaimState, ProviderResolver};

// ---------------------------------------------------------------------------------------------
// The calls the Scope describes without naming them, and the module paths the spec leaves open.

mod api {
    use std::path::Path;
    use std::sync::{Arc, RwLock};

    use redpen_app::publik::{
        LinkKind, ProvidersPatch, PublikOptions, PublikService, PublikStatusSink, StatusStore,
    };
    use redpen_core::clock::{Clock, FakeClock};
    use redpen_core::net::HttpTransport;
    use redpen_core::provider::search::{SearchError, SearchOutcome};
    use redpen_core::provider::{AiError, SecretString, StatusSink};
    use redpen_core::store::Settings;
    use serde_json::Value;

    pub use redpen_app::publik::{
        APP_SLUG, APP_VERSION, ConnectOutcome, Credential, DISCLOSURE, Resolution, balance_line,
        credential_path, map_publik_error, provision, resolve_provider, search, self_test,
        top_up_url_from_402,
    };
    pub use redpen_app::startup_self_test;
    pub use redpen_core::provider::ProviderResolver;
    pub use redpen_core::provider::tier::ClaimState;

    /// The user's own key as `resolve_provider` takes it: address, key and chat model.
    pub fn own_key(base_url: &str, key: &str, chat_model: &str) -> redpen_app::publik::OwnKey {
        redpen_app::publik::OwnKey {
            base_url: base_url.to_string(),
            key: SecretString::from(key.to_string()),
            chat_model: chat_model.to_string(),
        }
    }

    /// Which of the four answers of `resolve_provider` this is.
    pub fn resolution_kind(r: &Resolution) -> &'static str {
        match r {
            Resolution::OwnKey { .. } => "own_key",
            Resolution::Env { .. } => "env",
            Resolution::File { .. } => "file",
            Resolution::NeedsConsent => "needs_consent",
        }
    }

    /// The app token this build uses: `PUBLIK_APP_TOKEN` in the environment, else the one
    /// embedded at build time (the build environment's, else the file's); `None` without a
    /// `pat_` token.
    pub fn app_token() -> Option<String> {
        redpen_app::publik::app_token()
    }

    /// The token in the text of a `publik-app-token.txt`, if it holds one.
    pub fn token_in(file_text: &str) -> Option<String> {
        redpen_app::publik::parse_app_token(file_text)
    }

    /// A credential with the contract's fields (§3), as `credential_path()` holds it.
    pub fn credential(fields: Value) -> Credential {
        serde_json::from_value(fields).expect("a credential reads from the contract's fields")
    }

    /// The clock of the status the app keeps: the fixed time of these tests.
    fn clock() -> Arc<dyn Clock> {
        Arc::new(FakeClock::at(super::T0))
    }

    /// Settings as the app holds them after start, read from `settings.json`.
    fn settings(config_dir: &Path) -> Arc<RwLock<Settings>> {
        let loaded = Settings::load(config_dir).expect("settings.json loads");
        Arc::new(RwLock::new(loaded))
    }

    /// `RedpenResolver` as the app builds it at start: Settings from the config folder, the
    /// environment and credential as they are now, the status kept in the data folder.
    pub fn resolver(config_dir: &Path, data_dir: &Path) -> redpen_app::resolver::RedpenResolver {
        redpen_app::resolver::RedpenResolver::new(
            settings(config_dir),
            config_dir.to_path_buf(),
            PublikOptions::current(),
            Arc::new(StatusStore::load(data_dir, clock())),
        )
    }

    /// What the publik API commands work on, built as the app builds it at start: the
    /// transport, Settings from the config folder, the status kept in the data folder.
    pub type State = PublikService;

    pub fn state(t: Arc<dyn HttpTransport>, config_dir: &Path, data_dir: &Path) -> State {
        PublikService::new(
            t,
            PublikOptions::current(),
            config_dir.to_path_buf(),
            settings(config_dir),
            Arc::new(StatusStore::load(data_dir, clock())),
        )
    }

    /// The status sink the app hands to the chat client.
    pub fn state_sink(s: &State) -> Arc<dyn StatusSink> {
        Arc::new(PublikStatusSink::new(s.status().clone()))
    }

    /// The `PublikStatus` the app holds, as the interface receives it (null before there is one).
    pub fn state_status(s: &State) -> Value {
        serde_json::to_value(s.status().current()).expect("PublikStatus serialises")
    }

    /// What the app does at start: `GET /wallet` only while publik API is chosen and a
    /// credential exists.
    pub async fn start(s: &State) {
        let _ = s.refresh_at_start().await;
    }

    /// `consent_info()`, serialised as the interface receives it.
    pub fn consent_info() -> Value {
        serde_json::to_value(redpen_app::commands::publik::consent_info())
            .expect("consent info serialises")
    }

    /// `publik_connect`.
    pub async fn connect(s: &State) -> Result<ConnectOutcome, AiError> {
        s.connect().await
    }

    /// `publik_disconnect`.
    pub async fn disconnect(s: &State) {
        s.disconnect().await;
    }

    /// `publik_refresh`, its `PublikStatus` serialised.
    pub async fn refresh(s: &State) -> Result<Value, AiError> {
        s.refresh()
            .await
            .map(|status| serde_json::to_value(status).expect("PublikStatus serialises"))
    }

    /// `provider_status`, serialised.
    pub fn provider_status(s: &State) -> Value {
        serde_json::to_value(s.provider_status()).expect("ProviderStatus serialises")
    }

    /// The argument of `open_account_link`, as the interface sends it.
    pub fn link_kind(argument: Value) -> Result<LinkKind, String> {
        serde_json::from_value(argument).map_err(|e| e.to_string())
    }

    /// The address `open_account_link(kind)` hands to the system browser, or its error.
    pub fn account_link(s: &State, kind: &str) -> Result<String, String> {
        let kind = link_kind(Value::from(kind))?;
        s.account_link(kind).map_err(|e| format!("{e:?}"))
    }

    /// `set_providers` with the arguments the interface sends, `{chat?, rewrite_quality?,
    /// search?}`; an argument the command cannot take, or a refusal, is the error.
    pub fn set_providers(s: &State, arguments: Value) -> Result<Settings, String> {
        let patch: ProvidersPatch = serde_json::from_value(arguments).map_err(|e| e.to_string())?;
        s.set_providers(patch).map_err(|e| e.to_string())
    }

    /// The tray's disabled balance item, if it has one.
    pub fn tray_balance_item(s: &State) -> Option<String> {
        redpen_app::tray::balance_label(s.status().current().as_ref())
    }

    /// The first-run card's balance line.
    pub fn card_balance_line(o: &ConnectOutcome) -> String {
        o.balance_line.clone()
    }

    pub fn search_charge(o: &SearchOutcome) -> Option<u64> {
        o.charge_micros
    }

    /// The link of a search 402, if this is one.
    pub fn search_top_up_url(e: &SearchError) -> Option<String> {
        match e {
            SearchError::NeedsCredit { top_up_url, .. } => top_up_url.clone(),
            _ => None,
        }
    }

    pub fn is_provider_failed(e: &SearchError) -> bool {
        matches!(e, SearchError::ProviderFailed)
    }

    /// The proof's `run` with these arguments (no program name), answering its questions from
    /// `stdin`: its exit code and everything it printed.
    pub async fn proof(args: &[&str], t: &dyn HttpTransport, stdin: &str) -> (i32, String) {
        let options = redpen_app::proof::parse_args(args.iter().map(|a| a.to_string()))
            .expect("arguments the proof knows");
        let mut input = std::io::Cursor::new(stdin.as_bytes().to_vec());
        let mut out = Vec::new();
        let publik = PublikOptions::current();
        let code = redpen_app::proof::run(t, &options, &publik, &mut input, &mut out).await;
        (code, String::from_utf8_lossy(&out).into_owned())
    }
}

// ---------------------------------------------------------------------------------------------
// Expected values.

/// The fake publik API key of contract test 16, and a second one for the environment.
const KEY: &str = "pk_live_a";
const ENV_KEY: &str = "pk_live_b";
const BASE: &str = "https://publikhq.com/api/v1";
/// `PUBLIK_API_BASE_URL` in the tests; `.invalid` never resolves.
const ENV_BASE: &str = "https://publik.invalid/api/v1";
/// The own key's address and model, from the settings example of docs/ARCHITECTURE.md §4.
const OWN_BASE: &str = "https://api.example.com/v1";
const OWN_KEY: &str = "sk-test-a";
const OWN_MODEL: &str = "example-chat";
const LOCAL_BASE: &str = "http://localhost:11434/v1";
const CHAT: &str = "POST /chat/completions";
const INSTALLS: &str = "POST /installs";
const WALLET: &str = "GET /wallet";
const REVOKE: &str = "POST /installs/revoke";
const SEARCH: &str = "POST /search";
/// The install of the spec's acceptance test 11 and of the fake 201s.
const INSTALL_ID: &str = "5f0c8a52-6e0b-4c55-9a3e-2a3f1d9b7c11";
/// An install_id already on disk, kept after a Disconnect.
const KEPT_ID: &str = "0b7e6f2c-3d4a-4e5f-8a9b-1c2d3e4f5a6b";
const CLAIM_URL: &str = "https://publikhq.com/claim/fake-claim-token";
const ADD_CREDIT_URL: &str = "https://publikhq.com/account/add-credit";
const COST_SENTENCE: &str =
    "A typical rewrite costs less than one cent, paid from this computer's publik balance.";
/// The workspace version (0.1.0 when these tests were written).
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// A generic label, never the hostname (A2).
const DEVICE_NAME: &str = if cfg!(windows) {
    "Windows PC"
} else {
    "Linux PC"
};
/// The search of C19 and of `publik-proof --search`.
const QUERY: &str = "\"largest coral reef system\"";
/// 2026-09-21T14:13:20Z, local day 2026-09-21 (tests/cases/02-provider-layer/REFERENCE.md).
const T0: i64 = 1_790_000_000_000;
const T0_DAY: &str = "2026-09-21";
/// The only model values a Redpen chat request may carry (docs/PUBLIK-INTEGRATION.md §1).
const TIERS: [&str; 3] = ["publik-fast", "publik-balanced", "publik-smart"];
const FOUR_MB: usize = 4_000_000;
/// The eight headers of docs/PUBLIK-CONTRACT.md §7.
const STATUS_HEADERS: [&str; 8] = [
    "x-publik-balance",
    "x-publik-charge-micros",
    "x-publik-request-id",
    "x-publik-claim-state",
    "x-publik-starter-remaining",
    "x-publik-week-used",
    "x-publik-week-budget",
    "x-publik-week-resets-at",
];

/// docs/PUBLIK-INTEGRATION.md §4.
const TITLE: &str = "Choose how Redpen uses AI";
const DATA_PATH: &str = "Checking is done on this computer and needs no AI. Only the text you select and the action you choose go through publik's servers, and only when you press a rewrite button.";
/// The spec's Design notes.
const DISCLOSURE_TEXT: &str = "Requests go through publik's servers to a shared AI model account. publik does not train on them and keeps no request bodies beyond a 24-hour fingerprint that stops a request being charged twice. The AI provider's own abuse monitoring applies.";
const ZERO_ANONYMOUS: &str = "$0.00 · link this computer for $0.05 of free use";
const ZERO_CLAIMED: &str = "$0.00 of publik balance";

// The spec's AiError table and docs/PUBLIK-CONTRACT.md §5.
const MSG_DISCONNECTED: &str = "publik API disconnected this computer.";
const MSG_BUSY: &str = "publik API is busy; try again in a moment";
const MSG_500: &str = "publik API 500: The model returned an error.";
const MSG_NO_APP_TOKEN: &str = "This build has no publik API app token. Use your own key for now.";
const MSG_PROVISION_500: &str =
    "publik API could not set up this computer (500). Use your own key for now.";
const MSG_UNREACHABLE: &str = "publik API is unreachable right now. Use your own key for now.";
/// The spec's Design notes (Provisioning): a key was minted but the credential cannot be written.
const MSG_NOT_SAVED: &str =
    "Redpen could not save the publik API setup on this computer. Use your own key for now.";
/// Contract §11, test 3, and the spec's C3.
const TOKEN_SKIP: &str = "Publik app token not yet added (docs/PUBLIK-INTEGRATION.md §2)";

/// A fake app token of the contract's shape, `pat_<slug>_…`, put together at run time.
fn fake_token() -> String {
    ["pat", "redpen", "fake"].join("_")
}

// ---------------------------------------------------------------------------------------------
// The app token committed with the repository.

fn token_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("publik-app-token.txt")
}

/// The `pat_` line of the committed token file, if there is one.
fn file_token() -> Option<String> {
    fs::read_to_string(token_file())
        .ok()?
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("pat_"))
        .map(str::to_string)
}

/// The token this build embeds: `PUBLIK_APP_TOKEN` at build time, else the file's.
fn committed_token() -> Option<String> {
    option_env!("PUBLIK_APP_TOKEN")
        .map(str::to_string)
        .filter(|t| t.starts_with("pat_"))
        .or_else(file_token)
}

/// True, after saying why, when a real token is embedded and the test must not run.
fn real_token_embedded() -> bool {
    let embedded = committed_token().is_some();
    if embedded {
        println!("skipped: this build embeds a real app token, and the test must reach no network");
    }
    embedded
}

// ---------------------------------------------------------------------------------------------
// The sandbox.

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// The variables a sandbox sets or clears, all restored when it ends.
const SANDBOX_VARS: [&str; 8] = [
    "LOCALAPPDATA",
    "APPDATA",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "PUBLIK_API_KEY",
    "PUBLIK_API_BASE_URL",
    "PUBLIK_APP_TOKEN",
    "PUBLIK_SELF_TEST",
];

fn set_var(name: &str, value: impl AsRef<OsStr>) {
    // SAFETY: every test that reads or changes these variables holds ENV_LOCK, through its
    // Sandbox, and nothing else in this process changes the environment.
    unsafe { std::env::set_var(name, value) }
}

fn remove_var(name: &str) {
    // SAFETY: as in `set_var`.
    unsafe { std::env::remove_var(name) }
}

/// One test's private credential location, config and data folders, and environment.
struct Sandbox {
    saved: Vec<(&'static str, Option<OsString>)>,
    dir: tempfile::TempDir,
    _lock: MutexGuard<'static, ()>,
}

impl Sandbox {
    fn new() -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = tempfile::tempdir().expect("a temporary folder");
        let saved = SANDBOX_VARS
            .iter()
            .map(|&name| (name, std::env::var_os(name)))
            .collect();
        let sandbox = Self {
            saved,
            dir,
            _lock: lock,
        };
        for name in [
            "PUBLIK_API_KEY",
            "PUBLIK_API_BASE_URL",
            "PUBLIK_APP_TOKEN",
            "PUBLIK_SELF_TEST",
        ] {
            remove_var(name);
        }
        set_var("LOCALAPPDATA", sandbox.local());
        set_var("XDG_CONFIG_HOME", sandbox.local());
        set_var("APPDATA", sandbox.dir.path().join("roaming"));
        set_var("XDG_DATA_HOME", sandbox.dir.path().join("share"));
        assert_eq!(
            api::credential_path(),
            sandbox.credential_file(),
            "credential_path() must be %LOCALAPPDATA%\\publik\\apps\\redpen.json on Windows and \
             $XDG_CONFIG_HOME/publik/apps/redpen.json on Linux (docs/PUBLIK-CONTRACT.md §3); \
             nothing was written"
        );
        sandbox
    }

    /// With the fake app token in `PUBLIK_APP_TOKEN`.
    fn with_token(self) -> Self {
        set_var("PUBLIK_APP_TOKEN", fake_token());
        self
    }

    fn set(&self, name: &'static str, value: &str) {
        assert!(SANDBOX_VARS.contains(&name), "{name} is not restored");
        set_var(name, value);
    }

    fn unset(&self, name: &'static str) {
        assert!(SANDBOX_VARS.contains(&name), "{name} is not restored");
        remove_var(name);
    }

    fn local(&self) -> PathBuf {
        self.dir.path().join("local")
    }

    fn credential_file(&self) -> PathBuf {
        self.local().join("publik").join("apps").join("redpen.json")
    }

    fn config(&self) -> PathBuf {
        self.dir.path().join("config")
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn write_credential(&self, fields: &Value) {
        let path = self.credential_file();
        fs::create_dir_all(path.parent().expect("a parent folder")).expect("the folder");
        fs::write(path, fields.to_string()).expect("the credential is written");
    }

    /// The credential file's fields, if it exists.
    fn credential(&self) -> Option<Value> {
        let text = fs::read_to_string(self.credential_file()).ok()?;
        Some(serde_json::from_str(&text).expect("the credential file is JSON"))
    }

    fn remove_credential(&self) {
        fs::remove_file(self.credential_file()).expect("the credential is removed");
    }

    /// `status.json` in the data folder.
    fn status_file(&self) -> Value {
        let text = fs::read_to_string(self.data().join("status.json")).expect("status.json");
        serde_json::from_str(&text).expect("status.json is JSON")
    }

    /// Settings → Providers → AI provider, saved in `settings.json`, with the own key's address
    /// and model filled in.
    fn choose(&self, chat: Option<ChatChoice>) {
        let mut settings = Settings::default();
        settings.providers.chat = chat;
        settings.providers.own_key.base_url = OWN_BASE.to_string();
        settings.providers.own_key.chat_model = OWN_MODEL.to_string();
        settings
            .save(&self.config())
            .expect("settings.json is saved");
    }

    /// The own key, saved in `secrets.json`.
    fn save_own_key(&self) {
        Secrets {
            own_key: Some(SecretString::from(OWN_KEY.to_string())),
            search_key: None,
        }
        .save(&self.config())
        .expect("secrets.json is saved");
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        for (name, value) in &self.saved {
            match value {
                Some(v) => set_var(name, v),
                None => remove_var(name),
            }
        }
    }
}

/// The credential file of an install made with the fake 201: key, address, links and state.
fn credential(key: &str) -> Value {
    json!({
        "key": key,
        "base_url": BASE,
        "install_id": INSTALL_ID,
        "claim_url": CLAIM_URL,
        "claim_state": "anonymous",
        "add_credit_url": ADD_CREDIT_URL,
        "cost_sentence": COST_SENTENCE,
        "balance_micros": 0
    })
}

/// The credential after a Disconnect: no key, the install_id kept.
fn keyless(install_id: &str) -> Value {
    let mut fields = credential(KEY);
    let map = fields.as_object_mut().expect("an object");
    map.remove("key");
    map.insert("install_id".to_string(), json!(install_id));
    fields
}

// ---------------------------------------------------------------------------------------------
// Fake responses.

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cases/03-publik")
}

/// A `*.fake.json` file: `{"about", "status", "headers", "body"}`.
fn fake_file(name: &str) -> Value {
    let path = cases_dir().join(format!("{name}.fake.json"));
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} could not be read: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}.fake.json is not JSON: {e}"))
}

fn fake_status(name: &str) -> u16 {
    fake_file(name)["status"]
        .as_u64()
        .and_then(|s| u16::try_from(s).ok())
        .unwrap_or_else(|| panic!("{name}.fake.json has no status"))
}

/// The headers of a fake file, names lower-cased as every transport gives them.
fn fake_headers(name: &str) -> Vec<(String, String)> {
    fake_file(name)["headers"]
        .as_object()
        .map(|map| {
            map.iter()
                .map(|(k, v)| {
                    let value = v.as_str().unwrap_or_else(|| panic!("{name}: header {k}"));
                    (k.to_ascii_lowercase(), value.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn fake_body(name: &str) -> Value {
    fake_file(name)["body"].clone()
}

fn fake(name: &str) -> Reply {
    response(fake_status(name), fake_headers(name), &fake_body(name))
}

/// A fake response whose body `edit` changes first.
fn fake_with(name: &str, edit: impl FnOnce(&mut Value)) -> Reply {
    let mut body = fake_body(name);
    edit(&mut body);
    response(fake_status(name), fake_headers(name), &body)
}

fn response(status: u16, headers: Vec<(String, String)>, body: &Value) -> Reply {
    Ok(HttpResponse {
        status,
        headers,
        body: serde_json::to_vec(body).expect("a JSON value serialises"),
    })
}

/// A 200 chat answer carrying `content`, with no headers.
fn answer(content: &str) -> Reply {
    let body = json!({"choices": [{"message": {"role": "assistant", "content": content}}]});
    response(200, Vec::new(), &body)
}

/// A 200 chat answer charged `micros`.
fn charged(content: &str, micros: u64) -> Reply {
    let body = json!({"choices": [{"message": {"role": "assistant", "content": content}}]});
    let headers = vec![("x-publik-charge-micros".to_string(), micros.to_string())];
    response(200, headers, &body)
}

fn error_message(name: &str) -> String {
    fake_body(name)["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}.fake.json has no error.message"))
        .to_string()
}

fn fake_top_up_url(name: &str) -> String {
    fake_body(name)["error"]["top_up_url"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}.fake.json has no error.top_up_url"))
        .to_string()
}

/// The body of a request, as the transport would send it.
fn body_bytes(name: &str) -> Vec<u8> {
    serde_json::to_vec(&fake_body(name)).expect("a JSON value serialises")
}

/// A fake 201 (or 200) to `POST /installs` that gives back the install_id it was sent.
fn echoing(name: &'static str) -> impl Fn(&HttpRequest) -> Reply + Send + Sync + 'static {
    move |req| {
        let sent = body(req)["install_id"].clone();
        fake_with(name, |b| b["install_id"] = sent)
    }
}

// ---------------------------------------------------------------------------------------------
// A scripted transport.

type Reply = Result<HttpResponse, TransportError>;
type Answer = Box<dyn Fn(&HttpRequest) -> Reply + Send + Sync>;

/// Answers each `METHOD path` from its own queue, in order, and keeps every request. A request
/// with no answer left fails as unrecorded.
#[derive(Default)]
struct Script {
    answers: Mutex<HashMap<String, VecDeque<Answer>>>,
    seen: Mutex<Vec<HttpRequest>>,
}

impl Script {
    fn on(self, route: &str, reply: Reply) -> Self {
        self.on_fn(route, move |_| reply.clone())
    }

    fn on_fn(
        self,
        route: &str,
        answer: impl Fn(&HttpRequest) -> Reply + Send + Sync + 'static,
    ) -> Self {
        self.answers
            .lock()
            .expect("answers are not poisoned")
            .entry(route.to_string())
            .or_default()
            .push_back(Box::new(answer));
        self
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.seen.lock().expect("log is not poisoned").clone()
    }

    fn routes(&self) -> Vec<String> {
        self.requests().iter().map(route).collect()
    }
}

#[async_trait::async_trait]
impl HttpTransport for Script {
    async fn send(&self, req: HttpRequest) -> Reply {
        self.seen
            .lock()
            .expect("log is not poisoned")
            .push(req.clone());
        let next = self
            .answers
            .lock()
            .expect("answers are not poisoned")
            .get_mut(&route(&req))
            .and_then(VecDeque::pop_front);
        match next {
            Some(reply_to) => reply_to(&req),
            None => Err(TransportError::Unrecorded {
                hash: request_hash(&req),
                path: request_path(&req.url),
            }),
        }
    }
}

fn route(req: &HttpRequest) -> String {
    format!("{} {}", req.method, request_path(&req.url))
}

/// Runs a future on a runtime of its own (the tests themselves are plain, so
/// `startup_self_test` can make its own runtime).
fn run<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(future)
}

// ---------------------------------------------------------------------------------------------
// What requests carry.

fn header<'a>(req: &'a HttpRequest, name: &str) -> Option<&'a HeaderValue> {
    req.headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

fn body(req: &HttpRequest) -> Value {
    match &req.body {
        Body::Json(v) => v.clone(),
        Body::Empty => Value::Null,
    }
}

/// The key travels only as `Authorization: Bearer <key>`, a secret header value: never in the
/// URL, the body, another header or `x-api-key` (contract §6, test 4).
fn key_only_as_bearer(req: &HttpRequest, key: &str) {
    let auth = header(req, "authorization")
        .unwrap_or_else(|| panic!("{} has no Authorization header", route(req)));
    assert_eq!(auth.expose(), format!("Bearer {key}"), "{}", route(req));
    assert!(
        auth.is_secret(),
        "{}: the header is a secret value",
        route(req)
    );
    assert!(header(req, "x-api-key").is_none(), "{}", route(req));
    assert!(
        !req.url.contains(key),
        "{}: the key is in the URL",
        route(req)
    );
    assert!(
        !canonical_json(&body(req)).contains(key),
        "{}: the key is in the body",
        route(req)
    );
    for (name, value) in &req.headers {
        if !name.eq_ignore_ascii_case("authorization") {
            assert!(!value.expose().contains(key), "the key is in {name}");
        }
    }
}

/// Contract tests 4 and 15 over every request a test made: each body under 4 MB, none with an
/// `image_url` part, every publik API chat request naming a tier.
fn check_requests(reqs: &[HttpRequest]) {
    for req in reqs {
        let text = canonical_json(&body(req));
        assert!(text.len() < FOUR_MB, "a {} byte request", text.len());
        assert!(!text.contains("image_url"), "Redpen sends no images");
        if request_path(&req.url) == "/chat/completions"
            && url_host(&req.url).as_deref() == Some("publikhq.com")
        {
            let model = body(req)["model"].as_str().unwrap_or_default().to_string();
            assert!(TIERS.contains(&model.as_str()), "model {model:?}");
        }
    }
}

/// A version 4 UUID in lowercase hex (RFC 9562 §5.4).
fn is_v4(id: &str) -> bool {
    let groups: Vec<&str> = id.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        && groups[2].starts_with('4')
        && groups[3].starts_with(['8', '9', 'a', 'b'])
}

/// This computer's names, lower-cased, as far as the environment and `hostname` tell, leaving
/// out any name the expected body contains anyway (a computer named after the slug).
fn hostnames(expected_text: &str) -> Vec<String> {
    let mut names: Vec<String> = ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .collect();
    if let Ok(out) = Command::new("hostname").output() {
        names.push(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let expected = expected_text.to_lowercase();
    names
        .into_iter()
        .map(|n| n.trim().to_lowercase())
        .filter(|n| n.len() >= 3 && !expected.contains(n.as_str()))
        .collect()
}

/// Every distinct `http://` or `https://` address in a text.
fn urls_in(text: &str) -> BTreeSet<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '<' | '>'))
        .filter(|w| w.starts_with("http://") || w.starts_with("https://"))
        .map(|w| w.trim_end_matches(['.', ',', ';', ':', ')']).to_string())
        .collect()
}

fn publik() -> ResolvedProvider {
    ResolvedProvider {
        kind: ProviderKind::Publik,
        base_url: BASE.to_string(),
        key: Some(SecretString::from(KEY.to_string())),
        chat_model: "publik-fast".to_string(),
    }
}

/// A Fix grammar call on `text`.
fn call(text: &str) -> ChatCall {
    ChatCall {
        feature: Feature::FixGrammar,
        prompt_id: "fix-grammar",
        prompt_version: 1,
        messages: vec![
            Message::system("You correct grammar and spelling."),
            Message::user(text),
        ],
        max_tokens: 256,
        temperature: 0.0,
        timeout: Duration::from_secs(30),
        cache_input: json!({"text": text, "dialect": "british"}),
        cache: CachePolicy::Disk,
        validation_retry: 0,
    }
}

/// A chat client over `t` with the publik API status sink, an in-memory database and a fake
/// clock at `T0`.
fn chat_client(t: Arc<Script>, sink: Arc<dyn StatusSink>) -> (AiClient, Db) {
    let db = Db::open_in_memory().expect("an in-memory redpen.db");
    let client = AiClient::new(
        t,
        db.clone(),
        Arc::new(FakeClock::at(T0)),
        Arc::new(FakeSleeper::new()),
        sink,
    );
    (client, db)
}

// ---------------------------------------------------------------------------------------------
// C1: choosing the provider.

// C1: the own key beats PUBLIK_API_KEY, which beats the credential file; nothing set gives
// NeedsConsent.
#[test]
fn c01_own_key_beats_the_environment_which_beats_the_file() {
    let sb = Sandbox::new();
    let own = api::own_key(OWN_BASE, OWN_KEY, OWN_MODEL);
    let kind = |user_key| api::resolution_kind(&api::resolve_provider(user_key));
    assert_eq!(kind(None), "needs_consent");
    assert_eq!(kind(Some(&own)), "own_key");
    sb.write_credential(&credential(KEY));
    assert_eq!(kind(None), "file");
    sb.set("PUBLIK_API_KEY", ENV_KEY);
    sb.set("PUBLIK_API_BASE_URL", ENV_BASE);
    assert_eq!(kind(None), "env");
    assert_eq!(kind(Some(&own)), "own_key");
    sb.remove_credential();
    assert_eq!(kind(None), "env");
    sb.unset("PUBLIK_API_KEY");
    sb.unset("PUBLIK_API_BASE_URL");
    assert_eq!(kind(None), "needs_consent");
}

/// The provider `RedpenResolver` gives for the sandbox's settings, environment and credential,
/// built afresh as the app builds it at start.
fn resolved(sb: &Sandbox) -> Result<ResolvedProvider, AiError> {
    api::resolver(&sb.config(), &sb.data()).chat()
}

// C1 and Scope (docs/PUBLIK-INTEGRATION.md §3): `RedpenResolver` keys on Settings → AI provider:
// own key when chosen with a saved key; otherwise the environment, then the credential file,
// then NeedsConsent; local never consults publik API.
#[test]
fn c01_the_resolver_follows_the_settings_ai_provider() {
    let sb = Sandbox::new();
    sb.write_credential(&credential(KEY));
    sb.save_own_key();

    sb.choose(Some(ChatChoice::OwnKey));
    let p = resolved(&sb).expect("the own key");
    assert_eq!(
        (p.kind, p.base_url.as_str(), p.chat_model.as_str()),
        (ProviderKind::OwnKey, OWN_BASE, OWN_MODEL)
    );
    assert_eq!(p.key.as_ref().map(|k| k.expose_secret()), Some(OWN_KEY));

    for chat in [Some(ChatChoice::Publik), None] {
        sb.choose(chat);
        let p = resolved(&sb).expect("the credential file");
        assert_eq!((p.kind, p.base_url.as_str()), (ProviderKind::Publik, BASE));
        assert_eq!(p.key.as_ref().map(|k| k.expose_secret()), Some(KEY));
        sb.set("PUBLIK_API_KEY", ENV_KEY);
        sb.set("PUBLIK_API_BASE_URL", ENV_BASE);
        let p = resolved(&sb).expect("the environment");
        assert_eq!(
            (p.kind, p.base_url.as_str()),
            (ProviderKind::Publik, ENV_BASE)
        );
        assert_eq!(p.key.as_ref().map(|k| k.expose_secret()), Some(ENV_KEY));
        sb.unset("PUBLIK_API_KEY");
        sb.unset("PUBLIK_API_BASE_URL");
    }

    sb.choose(Some(ChatChoice::Local));
    sb.set("PUBLIK_API_KEY", ENV_KEY);
    let p = resolved(&sb).expect("local");
    assert_eq!(
        (p.kind, p.base_url.as_str()),
        (ProviderKind::Local, LOCAL_BASE)
    );
    assert!(p.key.is_none());
    sb.unset("PUBLIK_API_KEY");

    sb.remove_credential();
    for chat in [Some(ChatChoice::Publik), None] {
        sb.choose(chat);
        assert_eq!(resolved(&sb).unwrap_err(), AiError::NeedsConsent);
    }
}

// ---------------------------------------------------------------------------------------------
// C2: provisioning.

// C2: `provision()` sends exactly the body of contract §5, with no auth header, the generic
// device label and no hostname anywhere.
#[test]
fn c02_provision_sends_exactly_the_contract_body_and_no_hostname() {
    let _sb = Sandbox::new().with_token();
    let t = Script::default().on_fn(INSTALLS, echoing("installs-201"));
    run(api::provision(&t)).expect("provisioned");
    let reqs = t.requests();
    assert_eq!(t.routes(), [INSTALLS]);
    let req = &reqs[0];
    assert_eq!(req.method, Method::Post);
    assert_eq!(req.url, "https://publikhq.com/api/v1/installs");
    assert!(header(req, "authorization").is_none(), "no auth header");
    assert!(header(req, "x-api-key").is_none());
    let sent = body(req);
    let id = sent["install_id"].as_str().unwrap_or_default().to_string();
    assert!(is_v4(&id), "a fresh v4 install_id, not {id:?}");
    let mut expected = json!({
        "app_token": fake_token(),
        "app_slug": "redpen",
        "app_version": VERSION,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "device_name": DEVICE_NAME,
        "disclosure_version": 1,
        "dialects": ["chat_completions"]
    });
    let fixed_text = canonical_json(&expected);
    expected["install_id"] = json!(id);
    assert_eq!(sent, expected);
    let everything =
        format!("{} {:?} {}", req.url, req.headers, canonical_json(&sent)).to_lowercase();
    for name in hostnames(&fixed_text) {
        assert!(
            !everything.contains(&name),
            "the hostname {name:?} was sent"
        );
    }
    check_requests(&reqs);
}

// C2: a 201 writes the credential, with the contract's fields from the body, before
// `provision()` returns; mode 0600 on Unix.
#[test]
fn c02_a_201_writes_the_credential_before_returning() {
    let sb = Sandbox::new().with_token();
    let t = Script::default().on_fn(INSTALLS, echoing("installs-201"));
    run(api::provision(&t)).expect("provisioned");
    let sent_id = body(&t.requests()[0])["install_id"].clone();
    let file = sb
        .credential()
        .expect("the credential exists when provision() returns");
    assert_eq!(file["key"], KEY);
    assert_eq!(file["base_url"], BASE);
    assert_eq!(file["install_id"], sent_id);
    assert_eq!(file["claim_url"], CLAIM_URL);
    assert_eq!(file["claim_state"], "anonymous");
    assert_eq!(file["add_credit_url"], ADD_CREDIT_URL);
    assert_eq!(file["cost_sentence"], COST_SENTENCE);
    assert_eq!(file["balance_micros"], 0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(sb.credential_file())
            .expect("the credential file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

// C2: a 200 with `key: null` replays the install_id on disk and keeps the key there.
#[test]
fn c02_a_200_with_a_null_key_keeps_the_key_on_disk() {
    let sb = Sandbox::new().with_token();
    let mut on_disk = credential(KEY);
    on_disk["install_id"] = json!(KEPT_ID);
    sb.write_credential(&on_disk);
    let t = Script::default().on_fn(INSTALLS, echoing("installs-200-replay"));
    run(api::provision(&t)).expect("provisioned");
    let reqs = t.requests();
    assert_eq!(reqs.len(), 1, "one request");
    assert_eq!(body(&reqs[0])["install_id"], KEPT_ID, "the kept install_id");
    let file = sb.credential().expect("the credential");
    assert_eq!(file["key"], KEY);
    assert_eq!(file["install_id"], KEPT_ID);
}

// C2: a 200 with no key on disk mints once more, under a fresh install_id.
#[test]
fn c02_a_200_with_no_key_on_disk_mints_once_more_under_a_fresh_id() {
    let sb = Sandbox::new().with_token();
    sb.write_credential(&keyless(KEPT_ID));
    let t = Script::default()
        .on_fn(INSTALLS, echoing("installs-200-replay"))
        .on_fn(INSTALLS, echoing("installs-201"));
    run(api::provision(&t)).expect("provisioned");
    let reqs = t.requests();
    assert_eq!(t.routes(), [INSTALLS, INSTALLS], "exactly once more");
    assert_eq!(body(&reqs[0])["install_id"], KEPT_ID);
    let fresh = body(&reqs[1])["install_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        is_v4(&fresh) && fresh != KEPT_ID,
        "a fresh id, not {fresh:?}"
    );
    let file = sb.credential().expect("the credential");
    assert_eq!(file["key"], KEY);
    assert_eq!(file["install_id"], fresh.as_str());
}

// C2: 403 install_revoked mints once more, under a fresh install_id.
#[test]
fn c02_install_revoked_mints_once_more() {
    let sb = Sandbox::new().with_token();
    sb.write_credential(&keyless(KEPT_ID));
    let t = Script::default()
        .on(INSTALLS, fake("installs-403-install-revoked"))
        .on_fn(INSTALLS, echoing("installs-201"));
    run(api::provision(&t)).expect("provisioned");
    let reqs = t.requests();
    assert_eq!(reqs.len(), 2, "exactly once more");
    let fresh = body(&reqs[1])["install_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        is_v4(&fresh) && fresh != KEPT_ID,
        "a fresh id, not {fresh:?}"
    );
    assert_eq!(sb.credential().expect("the credential")["key"], KEY);
}

// C2: a second failure surfaces its error.message, after exactly two requests.
#[test]
fn c02_a_second_failure_surfaces_the_error_message() {
    let sb = Sandbox::new().with_token();
    let t = Script::default()
        .on(INSTALLS, fake("installs-403-install-revoked"))
        .on(INSTALLS, fake("installs-403-install-revoked-again"))
        .on(INSTALLS, fake("installs-201"));
    let err = run(api::provision(&t)).expect_err("a second failure");
    assert_eq!(
        err.to_string(),
        error_message("installs-403-install-revoked-again")
    );
    assert_eq!(t.requests().len(), 2);
    let key = sb.credential().and_then(|f| f.get("key").cloned());
    assert!(key.is_none_or(|k| k.is_null()), "no key was written");
}

// Error paths of contract §5: 429 shows error.message; another code shows error.message, or
// the code when there is none; no connection says publik API is unreachable. One request each.
#[test]
fn e_provision_errors_have_the_contract_messages() {
    let cases = [
        (fake("installs-429"), error_message("installs-429")),
        (
            fake("installs-503-with-message"),
            error_message("installs-503-with-message"),
        ),
        (fake("installs-500"), MSG_PROVISION_500.to_string()),
        (Err(TransportError::Offline), MSG_UNREACHABLE.to_string()),
    ];
    for (reply, message) in cases {
        let sb = Sandbox::new().with_token();
        let t = Script::default().on(INSTALLS, reply);
        let err = run(api::provision(&t)).expect_err("a failure");
        assert_eq!(err.to_string(), message);
        assert_eq!(t.requests().len(), 1, "{message}: one request");
        assert!(sb.credential().is_none(), "{message}: nothing written");
    }
}

// Error path: `publik_connect` gives the same failure as `ProvisionFailed`.
#[test]
fn e_publik_connect_reports_a_failed_setup() {
    let sb = Sandbox::new().with_token();
    let t = Arc::new(Script::default().on(INSTALLS, fake("installs-500")));
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    let err = run(api::connect(&state)).expect_err("a failure");
    assert_eq!(
        err,
        AiError::ProvisionFailed {
            message: MSG_PROVISION_500.to_string()
        }
    );
    assert_eq!(t.routes(), [INSTALLS]);
}

// Error path of the Design notes (Provisioning): publik API minted a key but the credential
// cannot be written, so provisioning, `publik_connect` and the self-test each fail with the
// "could not save" message, after one request, and nothing is written.
#[test]
fn e_a_credential_that_cannot_be_written_fails_the_setup() {
    let sb = Sandbox::new().with_token();
    // The credential's folder is a file, so nothing can be written under it.
    let file = sb.credential_file();
    let folder = file.parent().expect("a parent folder");
    fs::create_dir_all(folder.parent().expect("a grandparent folder")).expect("the folder");
    fs::write(folder, "x").expect("a file where the folder would be");

    let t = Script::default().on(INSTALLS, fake("installs-201"));
    let err = run(api::provision(&t)).expect_err("the credential cannot be written");
    assert_eq!(err.to_string(), MSG_NOT_SAVED);
    assert_eq!(t.routes(), [INSTALLS], "one request, never a second");
    assert!(sb.credential().is_none(), "nothing written");

    let t = Arc::new(Script::default().on(INSTALLS, fake("installs-201")));
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    let err = run(api::connect(&state)).expect_err("the credential cannot be written");
    assert_eq!(
        err,
        AiError::ProvisionFailed {
            message: MSG_NOT_SAVED.to_string()
        }
    );
    assert_eq!(err.to_string(), MSG_NOT_SAVED);
    assert_eq!(t.routes(), [INSTALLS], "one request, never a second");

    let t = Script::default()
        .on(INSTALLS, fake("installs-201"))
        .on(WALLET, fake("wallet-200"));
    let line = api::startup_self_test(Some(OsString::from("1")), &t);
    assert_eq!(
        line,
        Some(format!("publik self-test: failed: {MSG_NOT_SAVED}"))
    );
    assert_eq!(
        t.routes(),
        [INSTALLS],
        "no wallet read without a credential"
    );
    assert!(sb.credential().is_none(), "nothing written");
}

// ---------------------------------------------------------------------------------------------
// C3: the app token.

// C3: the token is read from the file's `pat_` line, and `PUBLIK_APP_TOKEN` overrides it.
#[test]
fn c03_the_token_comes_from_the_file_unless_publik_app_token_overrides_it() {
    let sb = Sandbox::new();
    let token = fake_token();
    assert_eq!(api::token_in(""), None);
    assert_eq!(
        api::token_in("# The maintainer adds the token here.\n"),
        None
    );
    assert_eq!(
        api::token_in(&format!("# The app token\n{token}\n")),
        Some(token.clone())
    );
    assert_eq!(
        api::token_in(&format!("  {token}  \r\n")),
        Some(token.clone())
    );
    assert_eq!(api::app_token(), committed_token());
    sb.set("PUBLIK_APP_TOKEN", &token);
    assert_eq!(api::app_token(), Some(token));
}

// C3 and Scope: until the maintainer's step the committed file holds one comment line and no
// token.
#[test]
fn c03_the_committed_token_file_holds_one_comment_line_until_the_token_is_added() {
    let text = fs::read_to_string(token_file()).expect("app/src-tauri/publik-app-token.txt");
    if file_token().is_some() {
        println!("skipped: the token has been added");
        return;
    }
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "{text:?}");
    assert!(!text.contains("pat_"));
    assert_eq!(api::token_in(&text), None);
}

// C3: the placeholder gives NoAppToken with its message, and nothing is sent.
#[test]
fn c03_the_placeholder_gives_no_app_token_and_sends_nothing() {
    if real_token_embedded() {
        return;
    }
    let sb = Sandbox::new();
    let t = Arc::new(Script::default().on(INSTALLS, fake("installs-201")));
    let err = run(api::provision(&*t)).expect_err("no token");
    assert_eq!(err.to_string(), MSG_NO_APP_TOKEN);
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    let err = run(api::connect(&state)).expect_err("no token");
    assert_eq!(err, AiError::NoAppToken);
    assert_eq!(err.to_string(), MSG_NO_APP_TOKEN);
    assert!(t.requests().is_empty(), "nothing was sent");
    assert!(sb.credential().is_none());
}

// C3: the real-token test, once the maintainer has added the token: provisioning sends the
// committed `pat_redpen_…` token.
#[test]
fn c03_the_real_token_is_what_provisioning_sends() {
    let Some(token) = committed_token() else {
        println!("skipped: {TOKEN_SKIP}");
        return;
    };
    assert!(token.starts_with("pat_redpen_"), "a token for the slug");
    let _sb = Sandbox::new();
    assert_eq!(api::app_token().as_deref(), Some(token.as_str()));
    let t = Script::default().on_fn(INSTALLS, echoing("installs-201"));
    run(api::provision(&t)).expect("provisioned against the fake server");
    assert_eq!(body(&t.requests()[0])["app_token"], token.as_str());
}

// ---------------------------------------------------------------------------------------------
// C4 and C15: what every publik API request carries.

// C4 and C15: `GET /wallet`, `POST /installs/revoke` and `POST /search` carry the key only as a
// bearer header, `POST /installs` carries none, and every request stays under 4 MB with no image.
#[test]
fn c04_c15_every_publik_request_carries_the_key_only_as_bearer() {
    let sb = Sandbox::new().with_token();
    sb.choose(Some(ChatChoice::Publik));
    let t = Arc::new(
        Script::default()
            .on_fn(INSTALLS, echoing("installs-201"))
            .on(WALLET, fake("wallet-200"))
            .on(SEARCH, fake("search-200"))
            .on(REVOKE, fake("revoke-200")),
    );
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    run(api::connect(&state)).expect("connected");
    run(api::refresh(&state)).expect("refreshed");
    let cred = api::credential(sb.credential().expect("the credential"));
    run(api::search(&*t, &cred, QUERY, 5)).expect("searched");
    run(api::disconnect(&state));
    assert_eq!(t.routes(), [INSTALLS, WALLET, SEARCH, REVOKE]);
    let reqs = t.requests();
    assert!(header(&reqs[0], "authorization").is_none());
    assert!(!canonical_json(&body(&reqs[0])).contains(KEY));
    for req in &reqs[1..] {
        key_only_as_bearer(req, KEY);
    }
    check_requests(&reqs);
}

// ---------------------------------------------------------------------------------------------
// C5 to C9: status and errors.

/// The commands' state after Connect; `t` answers that `POST /installs` first.
fn connected_state(sb: &Sandbox, t: &Arc<Script>) -> api::State {
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    run(api::connect(&state)).expect("connected");
    state
}

/// How many requests went to `to` (`METHOD path`).
fn sent(t: &Script, to: &str) -> usize {
    t.routes().iter().filter(|r| *r == to).count()
}

// C5: once connected, the status sink parses all eight headers of contract §7 into
// `PublikStatus`, returns the charge, and writes `status.json` without the key.
#[test]
fn c05_every_status_header_is_parsed_and_written_without_the_key() {
    let sb = Sandbox::new().with_token();
    let t = Arc::new(Script::default().on_fn(INSTALLS, echoing("installs-201")));
    let state = connected_state(&sb, &t);
    let charge = api::state_sink(&state).on_response(
        ProviderKind::Publik,
        Feature::FixGrammar,
        200,
        &fake_headers("chat-200-status"),
    );
    assert_eq!(charge, Some(210));
    assert_eq!(t.routes(), [INSTALLS]);
    for status in [api::state_status(&state), sb.status_file()] {
        assert_eq!(status["balance_micros"], 3_120_000);
        assert_eq!(status["last_charge_micros"], 210);
        assert_eq!(status["last_request_id"], "req_fake_0001");
        assert_eq!(status["claim_state"], "claimed");
        assert_eq!(status["starter_remaining_micros"], 50_000);
        assert_eq!(status["week_used_micros"], 1_234_000);
        assert_eq!(status["week_budget_micros"], 5_000_000);
        assert_eq!(status["week_resets_at"], "2026-11-16T00:00:00Z");
        assert_eq!(status["needs_credit"], false);
        assert!(status.get("key").is_none());
        assert!(!status.to_string().contains(KEY));
    }
}

// C5: `publik_refresh` reads `GET /wallet` with the key and returns the balance it gives.
#[test]
fn c05_publik_refresh_reads_get_wallet() {
    let sb = Sandbox::new();
    sb.write_credential(&credential(KEY));
    sb.choose(Some(ChatChoice::Publik));
    let t = Arc::new(Script::default().on(WALLET, fake("wallet-200")));
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    let status = run(api::refresh(&state)).expect("refreshed");
    assert_eq!(t.routes(), [WALLET]);
    let reqs = t.requests();
    assert_eq!(reqs[0].url, "https://publikhq.com/api/v1/wallet");
    key_only_as_bearer(&reqs[0], KEY);
    assert_eq!(status["balance_micros"], 3_120_000);
    assert_eq!(status["claim_state"], "claimed");
    assert!(!status.to_string().contains(KEY));
}

// Error path: a refresh refused with 401 is Disconnected.
#[test]
fn e_a_refused_refresh_is_disconnected() {
    let sb = Sandbox::new();
    sb.write_credential(&credential(KEY));
    sb.choose(Some(ChatChoice::Publik));
    let t = Arc::new(Script::default().on(WALLET, fake("chat-401-key-revoked")));
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    let err = run(api::refresh(&state)).expect_err("refused");
    assert_eq!(err, AiError::Disconnected);
    assert_eq!(t.requests().len(), 1);
}

// C6 and C7, anonymous and claimed: a 402 through the client is NeedsCredit with exactly one
// link, one request, nothing cached, and `needs_credit` set in the status and `status.json`; the
// next success clears it.
#[test]
fn c06_c07_a_402_sets_needs_credit_and_the_next_success_clears_it() {
    for name in ["chat-402-anonymous", "chat-402-claimed"] {
        let sb = Sandbox::new().with_token();
        let t = Arc::new(
            Script::default()
                .on_fn(INSTALLS, echoing("installs-201"))
                .on(CHAT, fake(name))
                .on(CHAT, fake("chat-200-status")),
        );
        let state = connected_state(&sb, &t);
        let (client, db) = chat_client(t.clone(), api::state_sink(&state));
        let err = run(client.chat(&publik(), call("First text."))).expect_err("a 402");
        let AiError::NeedsCredit {
            message,
            top_up_url: link,
        } = &err
        else {
            panic!("{name}: expected NeedsCredit, got {err:?}");
        };
        assert_eq!(message, &error_message(name));
        assert_eq!(link.as_deref(), Some(fake_top_up_url(name).as_str()));
        let shown = format!("{err} {}", err.to_json());
        assert_eq!(urls_in(&shown), BTreeSet::from([fake_top_up_url(name)]));
        assert_eq!(sent(&t, CHAT), 1, "{name}: one request");
        assert_eq!(
            db.cache_len().expect("ai_cache"),
            0,
            "{name}: nothing cached"
        );
        assert_eq!(api::state_status(&state)["needs_credit"], true, "{name}");
        assert_eq!(sb.status_file()["needs_credit"], true, "{name}");

        run(client.chat(&publik(), call("Second text."))).expect("a success");
        assert_eq!(api::state_status(&state)["needs_credit"], false, "{name}");
        assert_eq!(sb.status_file()["needs_credit"], false, "{name}");
        check_requests(&t.requests());
    }
}

// Error path: a 402 from the own key is not publik API's and sets nothing.
#[test]
fn e_an_own_key_402_does_not_set_needs_credit() {
    let sb = Sandbox::new().with_token();
    let t = Arc::new(Script::default().on_fn(INSTALLS, echoing("installs-201")));
    let state = connected_state(&sb, &t);
    api::state_sink(&state).on_response(
        ProviderKind::OwnKey,
        Feature::Clearer,
        402,
        &fake_headers("chat-402-anonymous"),
    );
    assert_eq!(api::state_status(&state)["needs_credit"], false);
}

// C6: `map_publik_error` and `top_up_url_from_402` on both 402 bodies: the message and the one
// link, never the other links in the body.
#[test]
fn c06_a_402_body_maps_to_needs_credit_with_its_top_up_url() {
    for name in ["chat-402-anonymous", "chat-402-claimed"] {
        let err = api::map_publik_error(402, &body_bytes(name));
        assert_eq!(
            err,
            AiError::NeedsCredit {
                message: error_message(name),
                top_up_url: Some(fake_top_up_url(name)),
            }
        );
        assert_eq!(
            api::top_up_url_from_402(&body_bytes(name)),
            Some(fake_top_up_url(name))
        );
    }
    assert_eq!(api::top_up_url_from_402(b"not json"), None);
    assert_eq!(
        api::top_up_url_from_402(br#"{"error": {"message": "No link."}}"#),
        None
    );
}

// C8: 401 invalid_api_key, 401 key_revoked and 403 install_revoked map to Disconnected.
#[test]
fn c08_401_and_403_install_revoked_map_to_disconnected() {
    for name in [
        "chat-401-invalid-api-key",
        "chat-401-key-revoked",
        "chat-403-install-revoked",
    ] {
        let err = api::map_publik_error(fake_status(name), &body_bytes(name));
        assert_eq!(err, AiError::Disconnected, "{name}");
        assert_eq!(err.to_string(), MSG_DISCONNECTED);
    }
}

// C9: 429, 503, 500 and another code map to their messages.
#[test]
fn c09_429_503_500_and_400_map_to_their_messages() {
    let cases = [
        ("chat-429", error_message("chat-429")),
        ("chat-503", MSG_BUSY.to_string()),
        ("chat-500", MSG_500.to_string()),
        (
            "chat-400",
            format!("publik API 400: {}", error_message("chat-400")),
        ),
    ];
    for (name, message) in cases {
        let err = api::map_publik_error(fake_status(name), &body_bytes(name));
        assert_eq!(err.to_string(), message, "{name}");
    }
    assert_eq!(
        api::map_publik_error(429, &body_bytes("chat-429")),
        AiError::PublikRateLimited {
            message: error_message("chat-429")
        }
    );
    assert_eq!(
        api::map_publik_error(503, &body_bytes("chat-503")),
        AiError::PublikBusy
    );
    assert_eq!(
        api::map_publik_error(500, &body_bytes("chat-500")),
        AiError::PublikHttp {
            code: 500,
            message: "The model returned an error.".to_string()
        }
    );
}

// ---------------------------------------------------------------------------------------------
// C12: local never reaches publik API.

/// Ollama's `/api/tags` with one 14B chat model.
fn tags() -> Reply {
    let body = json!({"models": [{"name": "qwen2.5:14b", "model": "qwen2.5:14b",
        "details": {"format": "gguf", "parameter_size": "14.8B"}}]});
    response(200, Vec::new(), &body)
}

// C12 (and invariant 12): with a credential file present and the AI provider set to local,
// start-up, Settings opening, the status and a rewrite all leave publik API alone; every request
// goes to localhost.
#[test]
fn c12_local_with_a_credential_on_disk_never_contacts_publik() {
    let sb = Sandbox::new();
    sb.write_credential(&credential(KEY));
    sb.choose(Some(ChatChoice::Local));
    let t = Arc::new(
        Script::default()
            .on("GET /api/tags", tags())
            .on(CHAT, answer("Fixed.")),
    );
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    run(api::start(&state));
    // Whatever Settings opening answers here, it sends nothing.
    let _ = run(api::refresh(&state));
    assert!(!api::provider_status(&state).is_null());
    assert!(t.requests().is_empty(), "nothing at start or on refresh");
    let p = resolved(&sb).expect("local");
    assert_eq!(p.kind, ProviderKind::Local);
    let (client, _db) = chat_client(t.clone(), api::state_sink(&state));
    run(client.chat(&p, call("Local text."))).expect("answered locally");
    assert_eq!(t.routes(), ["GET /api/tags", CHAT]);
    for req in t.requests() {
        assert_eq!(
            url_host(&req.url).as_deref(),
            Some("localhost"),
            "{}",
            req.url
        );
        assert!(!req.url.contains("publikhq.com"));
    }
}

// ---------------------------------------------------------------------------------------------
// C16: the key never shows.

/// A `tracing` subscriber that writes the fields of every span and event, as ` name=value`, one
/// line each, into a shared text.
#[derive(Clone, Default)]
struct Captured {
    text: Arc<Mutex<String>>,
    spans: Arc<AtomicU64>,
}

impl Captured {
    fn text(&self) -> String {
        self.text.lock().expect("log is not poisoned").clone()
    }

    fn line(&self, fields: Fields) {
        let mut text = self.text.lock().expect("log is not poisoned");
        text.push_str(&fields.0);
        text.push('\n');
    }
}

/// The fields of one span or event.
struct Fields(String);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!(" {}={value:?}", field.name()));
    }
}

impl tracing::Subscriber for Captured {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let mut fields = Fields(span.metadata().name().to_string());
        span.record(&mut fields);
        self.line(fields);
        tracing::span::Id::from_u64(self.spans.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        let mut fields = Fields(String::new());
        values.record(&mut fields);
        self.line(fields);
    }

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = Fields(event.metadata().target().to_string());
        event.record(&mut fields);
        self.line(fields);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

// C16 (invariant 6): over Connect, a response's status headers, a refresh, the status, a search
// and its 402, the error mapping, Disconnect and the self-test, the key `pk_live_a` appears in no
// log line, no command result, no error, no `PublikStatus`, no `status.json` and no credential's
// `Debug`.
#[test]
fn c16_the_key_never_appears_in_logs_results_or_the_status() {
    let sb = Sandbox::new().with_token();
    sb.choose(Some(ChatChoice::Publik));
    let t = Arc::new(
        Script::default()
            .on_fn(INSTALLS, echoing("installs-201"))
            .on(WALLET, fake("wallet-200"))
            .on(SEARCH, fake("search-200"))
            .on(SEARCH, fake("search-402"))
            .on(REVOKE, fake("revoke-200")),
    );
    let log = Captured::default();
    let guard = tracing::subscriber::set_default(log.clone());
    // Call sites the tests running beside this one registered first are switched on for it too.
    tracing::callsite::rebuild_interest_cache();
    let mut seen = Vec::new();

    let state = api::state(t.clone(), &sb.config(), &sb.data());
    let outcome = run(api::connect(&state)).expect("connected");
    seen.push(format!("{outcome:?}"));
    seen.push(serde_json::to_string(&outcome).expect("ConnectOutcome serialises"));
    api::state_sink(&state).on_response(
        ProviderKind::Publik,
        Feature::Clearer,
        200,
        &fake_headers("chat-200-status"),
    );
    seen.push(api::state_status(&state).to_string());
    seen.push(api::provider_status(&state).to_string());
    seen.push(run(api::refresh(&state)).expect("refreshed").to_string());
    seen.push(api::consent_info().to_string());
    seen.push(format!("{:?}", api::tray_balance_item(&state)));
    let cred = api::credential(sb.credential().expect("the credential"));
    seen.push(format!("{cred:?}"));
    let found = run(api::search(&*t, &cred, QUERY, 5)).expect("searched");
    seen.push(format!("{found:?}"));
    let refused = run(api::search(&*t, &cred, QUERY, 5)).expect_err("a 402");
    seen.push(format!("{refused:?} {refused}"));
    let mapped = api::map_publik_error(402, &body_bytes("chat-402-anonymous"));
    seen.push(format!("{mapped:?} {mapped} {}", mapped.to_json()));
    seen.push(sb.status_file().to_string());
    run(api::disconnect(&state));
    seen.push(api::provider_status(&state).to_string());
    let again = Script::default()
        .on(INSTALLS, fake("installs-201"))
        .on(WALLET, fake("wallet-200"));
    seen.push(run(api::self_test(&again, VERSION)));
    drop(guard);

    let text = log.text();
    assert!(!text.contains(KEY), "the key is in the log:\n{text}");
    for s in &seen {
        assert!(!s.contains(KEY), "the key is in {s}");
    }
}

// ---------------------------------------------------------------------------------------------
// C19: web search at the transport level.

fn search_credential() -> api::Credential {
    api::credential(credential(KEY))
}

// C19: `search(…, "\"largest coral reef system\"", 5)` sends exactly `POST /search` with
// `{"q", "count"}` and the key only as a bearer header, and no idempotency key; the 200 gives two
// results and a charge of one cent.
#[test]
fn c19_search_sends_q_and_count_and_parses_two_results_and_the_charge() {
    let t = Script::default().on(SEARCH, fake("search-200"));
    let outcome = run(api::search(&t, &search_credential(), QUERY, 5)).expect("searched");
    let reqs = t.requests();
    assert_eq!(t.routes(), [SEARCH]);
    let req = &reqs[0];
    assert_eq!(req.method, Method::Post);
    assert_eq!(req.url, "https://publikhq.com/api/v1/search");
    assert_eq!(
        canonical_json(&body(req)),
        r#"{"count":5,"q":"\"largest coral reef system\""}"#
    );
    key_only_as_bearer(req, KEY);
    assert!(header(req, "x-publik-idempotency-key").is_none());
    let expected = fake_body("search-200")["results"].clone();
    assert_eq!(outcome.results.len(), 2);
    for (got, want) in outcome
        .results
        .iter()
        .zip(expected.as_array().expect("results"))
    {
        assert_eq!(got.url, want["url"].as_str().unwrap_or_default());
        assert_eq!(got.title, want["title"].as_str().unwrap_or_default());
        assert_eq!(got.snippet, want["snippet"].as_str().unwrap_or_default());
        assert_eq!(got.site, want["site"].as_str().unwrap_or_default());
    }
    assert_eq!(api::search_charge(&outcome), Some(10_000));
    check_requests(&reqs);
}

// Scope: a body without `charge_micros` takes the charge from `x-publik-charge-micros`.
#[test]
fn e_the_search_charge_falls_back_to_the_header() {
    let t = Script::default().on(SEARCH, fake("search-200-header-charge"));
    let outcome = run(api::search(&t, &search_credential(), QUERY, 5)).expect("searched");
    assert_eq!(outcome.results.len(), 1);
    assert_eq!(api::search_charge(&outcome), Some(10_000));
}

// C19: a search 402 is NeedsCredit with exactly one link, after one request.
#[test]
fn c19_a_search_402_is_needs_credit_with_one_link() {
    let t = Script::default().on(SEARCH, fake("search-402"));
    let err = run(api::search(&t, &search_credential(), QUERY, 5)).expect_err("a 402");
    assert_eq!(
        api::search_top_up_url(&err),
        Some(fake_top_up_url("search-402"))
    );
    let shown = format!("{err:?} {err}");
    assert_eq!(
        urls_in(&shown),
        BTreeSet::from([fake_top_up_url("search-402")])
    );
    assert!(
        err.to_string().contains(&error_message("search-402")),
        "{err}"
    );
    assert_eq!(t.requests().len(), 1);
}

// C19: 502 and 504 are ProviderFailed, nothing charged, one request each and no retry.
#[test]
fn c19_search_502_and_504_are_provider_failed_with_nothing_charged() {
    for name in ["search-502", "search-504"] {
        let t = Script::default()
            .on(SEARCH, fake(name))
            .on(SEARCH, fake(name));
        let err = run(api::search(&t, &search_credential(), QUERY, 5)).expect_err("failed");
        assert!(api::is_provider_failed(&err), "{name}: {err:?}");
        assert!(
            err.to_string().contains("nothing was charged"),
            "{name}: {err}"
        );
        assert_eq!(t.requests().len(), 1, "{name}: never retried");
    }
}

// Scope: the other search errors are the chat ones, each after one request.
#[test]
fn e_other_search_errors_are_as_for_chat_and_never_retried() {
    let cases = [
        (fake("chat-401-key-revoked"), MSG_DISCONNECTED.to_string()),
        (fake("chat-429"), error_message("chat-429")),
        (fake("chat-503"), MSG_BUSY.to_string()),
        (fake("chat-500"), MSG_500.to_string()),
    ];
    for (reply, message) in cases {
        let t = Script::default()
            .on(SEARCH, reply.clone())
            .on(SEARCH, reply);
        let err = run(api::search(&t, &search_credential(), QUERY, 5)).expect_err("an error");
        assert_eq!(err.to_string(), message);
        assert_eq!(t.requests().len(), 1, "{message}: one request");
    }
    let t = Script::default()
        .on(SEARCH, Err(TransportError::Timeout))
        .on(SEARCH, Err(TransportError::Timeout));
    let err = run(api::search(&t, &search_credential(), QUERY, 5));
    assert!(err.is_err());
    assert_eq!(t.requests().len(), 1, "a timeout is not retried");
}

// ---------------------------------------------------------------------------------------------
// Redpen's own, bullets 1 to 7.

// Bullet 1 (R11): two answers charged 210 and 180 for Fix grammar add up to 390 in that day's
// `usage_daily`, through the publik API status sink of a connected computer.
#[test]
fn t01_r11_charges_add_up_in_usage_daily() {
    let sb = Sandbox::new().with_token();
    let t = Arc::new(
        Script::default()
            .on_fn(INSTALLS, echoing("installs-201"))
            .on(CHAT, charged("First.", 210))
            .on(CHAT, charged("Second.", 180)),
    );
    let state = connected_state(&sb, &t);
    let (client, db) = chat_client(t.clone(), api::state_sink(&state));
    run(client.chat(&publik(), call("First text."))).expect("answered");
    run(client.chat(&publik(), call("Second text."))).expect("answered");
    let usage = db
        .usage(T0_DAY, "publik", "fix-grammar")
        .expect("usage_daily")
        .expect("a row for the day");
    assert_eq!(usage.charge_micros, 390);
    assert_eq!(usage.calls, 2);
}

// Bullet 2 (R12): at start, no credential means no request; a credential with publik API chosen
// means exactly one, `GET /wallet` with the key; a credential with the own key, local or nothing
// chosen means none.
#[test]
fn t02_r12_start_reads_the_wallet_only_with_a_credential_and_publik_chosen() {
    let cases = [
        (false, None, false),
        (false, Some(ChatChoice::Publik), false),
        (true, Some(ChatChoice::Publik), true),
        (true, Some(ChatChoice::OwnKey), false),
        (true, Some(ChatChoice::Local), false),
        (true, None, false),
    ];
    for (has_credential, chat, reads_wallet) in cases {
        let sb = Sandbox::new();
        if has_credential {
            sb.write_credential(&credential(KEY));
        }
        sb.choose(chat);
        let t = Arc::new(Script::default().on(WALLET, fake("wallet-200")));
        let state = api::state(t.clone(), &sb.config(), &sb.data());
        run(api::start(&state));
        let case = format!("credential {has_credential}, AI provider {chat:?}");
        if reads_wallet {
            assert_eq!(t.routes(), [WALLET], "{case}");
            key_only_as_bearer(&t.requests()[0], KEY);
        } else {
            assert!(t.requests().is_empty(), "{case}: {:?}", t.routes());
        }
    }
}

// Bullet 2 (R12): before Connect, nothing sends `POST /installs`, not start-up, Settings opening,
// the status, the consent text or a provider lookup; `publik_connect` sends it, once.
#[test]
fn t02_r12_publik_connect_is_the_only_path_to_post_installs() {
    let sb = Sandbox::new().with_token();
    sb.choose(Some(ChatChoice::Publik));
    let t = Arc::new(Script::default().on_fn(INSTALLS, echoing("installs-201")));
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    run(api::start(&state));
    // Without a credential Settings opening has nothing to read; whatever it answers, it sends
    // nothing.
    let _ = run(api::refresh(&state));
    assert!(!api::provider_status(&state).is_null());
    assert!(!api::consent_info().is_null());
    assert_eq!(resolved(&sb).unwrap_err(), AiError::NeedsConsent);
    assert!(t.requests().is_empty(), "{:?}", t.routes());
    run(api::connect(&state)).expect("connected");
    assert_eq!(t.routes(), [INSTALLS]);
}

/// Connects with the fake 201 `name` and returns the outcome.
fn connected(name: &'static str) -> api::ConnectOutcome {
    let sb = Sandbox::new().with_token();
    let t = Arc::new(Script::default().on_fn(INSTALLS, echoing(name)));
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    let outcome = run(api::connect(&state)).expect("connected");
    assert_eq!(t.routes(), [INSTALLS]);
    outcome
}

// Bullet 3 (R13): a 201 with `balance_micros: 0` holds the pending action and fills the
// first-run card: the $0.00 balance line, the body's cost sentence verbatim and its claim link.
#[test]
fn t03_r13_a_zero_balance_shows_the_first_run_card_and_holds_the_action() {
    assert_eq!(fake_body("installs-201")["cost_sentence"], COST_SENTENCE);
    let outcome = connected("installs-201");
    assert!(!outcome.run_pending);
    assert_eq!(
        api::card_balance_line(&outcome),
        api::balance_line(0, ClaimState::Anonymous)
    );
    assert_eq!(api::card_balance_line(&outcome), ZERO_ANONYMOUS);
    assert_eq!(outcome.cost_sentence, COST_SENTENCE);
    assert_eq!(outcome.claim_url, CLAIM_URL);
    let status = serde_json::to_value(&outcome.status).expect("PublikStatus serialises");
    assert_eq!(status["balance_micros"], 0);
    assert_eq!(status["claim_state"], "anonymous");
}

// Bullet 3 (R13): with `balance_micros: 50000` the pending action runs at once.
#[test]
fn t03_r13_a_funded_install_runs_the_pending_action() {
    let outcome = connected("installs-201-funded");
    assert!(outcome.run_pending);
    assert_eq!(api::card_balance_line(&outcome), "$0.05 of publik balance");
}

/// Disconnects with the revoke answered by `reply`, and checks what remains.
fn disconnect_with(reply: Reply) {
    let sb = Sandbox::new();
    sb.write_credential(&credential(KEY));
    sb.choose(Some(ChatChoice::Publik));
    let t = Arc::new(Script::default().on(REVOKE, reply));
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    run(api::disconnect(&state));
    assert_eq!(t.routes(), [REVOKE]);
    let reqs = t.requests();
    assert_eq!(reqs[0].url, "https://publikhq.com/api/v1/installs/revoke");
    key_only_as_bearer(&reqs[0], KEY);
    let file = sb
        .credential()
        .expect("the file stays, holding the install_id");
    assert!(file.get("key").is_none_or(Value::is_null), "{file}");
    assert_eq!(file["install_id"], INSTALL_ID);
    assert_eq!(
        api::resolution_kind(&api::resolve_provider(None)),
        "needs_consent"
    );
}

// Bullet 4: `publik_disconnect` sends `POST /installs/revoke` with the bearer key, forgets the
// key and keeps the install_id.
#[test]
fn t04_disconnect_revokes_forgets_the_key_and_keeps_the_install_id() {
    disconnect_with(fake("revoke-200"));
}

// Bullet 4: a failed revoke, refused or unreachable, still forgets the key.
#[test]
fn t04_a_failed_revoke_still_forgets_the_key() {
    disconnect_with(fake("chat-500"));
    disconnect_with(Err(TransportError::Offline));
}

// Bullet 5: `open_account_link("top_up")` with no pending 402 is NoLink; the claim and
// add-credit links come from the credential; the interface can only name a kind, never a URL;
// nothing is sent.
#[test]
fn t05_open_account_link_takes_a_kind_and_has_no_top_up_link_without_a_402() {
    let sb = Sandbox::new();
    sb.write_credential(&credential(KEY));
    let t = Arc::new(Script::default());
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    let top_up = api::account_link(&state, "top_up").expect_err("no pending 402");
    assert!(top_up.contains("NoLink"), "{top_up}");
    assert_eq!(api::account_link(&state, "claim").as_deref(), Ok(CLAIM_URL));
    assert_eq!(
        api::account_link(&state, "add_credit").as_deref(),
        Ok(ADD_CREDIT_URL)
    );
    for kind in ["claim", "add_credit", "top_up"] {
        assert!(api::link_kind(json!(kind)).is_ok(), "{kind}");
    }
    for argument in [
        json!("https://publikhq.com/claim/fake-claim-token"),
        json!("http://localhost/"),
        json!({"url": "https://publikhq.com/account/add-credit"}),
        json!("topup"),
        json!(""),
    ] {
        assert!(api::link_kind(argument.clone()).is_err(), "{argument}");
    }
    assert!(t.requests().is_empty(), "opening a link sends nothing");
}

// Bullet 6: the tray has no balance item before a credential exists, and shows `balance_line`
// after a response with `x-publik-balance`.
#[test]
fn t06_the_tray_shows_the_balance_line_once_a_credential_exists() {
    let sb = Sandbox::new().with_token();
    let t = Arc::new(Script::default().on_fn(INSTALLS, echoing("installs-201")));
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    assert_eq!(api::tray_balance_item(&state), None);
    run(api::connect(&state)).expect("connected");
    api::state_sink(&state).on_response(
        ProviderKind::Publik,
        Feature::Clearer,
        200,
        &fake_headers("chat-200-status"),
    );
    assert_eq!(
        api::tray_balance_item(&state).as_deref(),
        Some("$3.12 of publik balance")
    );
}

// Bullet 7: `consent_info()` returns the title, the data-path sentence of
// docs/PUBLIK-INTEGRATION.md §4 character for character, and `DISCLOSURE` unchanged.
#[test]
fn t07_consent_info_carries_the_data_path_sentence_and_the_disclosure() {
    let info = api::consent_info();
    assert_eq!(info["title"], TITLE);
    assert_eq!(info["data_path"], DATA_PATH);
    assert_eq!(info["disclosure"], api::DISCLOSURE);
    assert_eq!(info["disclosure"], DISCLOSURE_TEXT);
    assert_eq!(info["disclosure_version"], 1);
    assert_eq!(api::DISCLOSURE, DISCLOSURE_TEXT);
}

// ---------------------------------------------------------------------------------------------
// The publik API module's constants and money lines (Scope; the spec's Design notes).

// Scope: the contract's public names with Redpen's slug and the workspace version.
#[test]
fn e_the_slug_and_version_are_redpens() {
    assert_eq!(api::APP_SLUG, "redpen");
    assert_eq!(api::APP_VERSION, VERSION);
}

// Design notes: above 0 the amount, four decimals under one cent; at 0 the claim invitation while
// anonymous and the plain line once claimed; a negative balance reads as 0.
#[test]
fn e_balance_line_follows_the_design_notes() {
    let cases = [
        (3_120_000, ClaimState::Claimed, "$3.12 of publik balance"),
        (3_120_000, ClaimState::Anonymous, "$3.12 of publik balance"),
        (1_000_000, ClaimState::Claimed, "$1.00 of publik balance"),
        (50_000, ClaimState::Anonymous, "$0.05 of publik balance"),
        (10_000, ClaimState::Claimed, "$0.01 of publik balance"),
        (2_100, ClaimState::Claimed, "$0.0021 of publik balance"),
        (100, ClaimState::Anonymous, "$0.0001 of publik balance"),
        (0, ClaimState::Anonymous, ZERO_ANONYMOUS),
        (0, ClaimState::Claimed, ZERO_CLAIMED),
        (-5, ClaimState::Anonymous, ZERO_ANONYMOUS),
        (-5, ClaimState::Claimed, ZERO_CLAIMED),
    ];
    for (micros, claim_state, line) in cases {
        assert_eq!(api::balance_line(micros, claim_state), line, "{micros}");
    }
}

// Scope: `set_providers` saves the choices in `settings.json`, leaves out what it was not given,
// and cannot take a provider that does not exist; nothing is sent.
#[test]
fn e_set_providers_saves_the_choice_and_refuses_an_unknown_one() {
    let sb = Sandbox::new();
    let t = Arc::new(Script::default());
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    let saved = api::set_providers(&state, json!({"chat": "local", "rewrite_quality": "best"}))
        .expect("saved");
    assert_eq!(saved.providers.chat, Some(ChatChoice::Local));
    assert_eq!(saved.providers.rewrite_quality, RewriteQuality::Best);
    assert_eq!(saved.providers.search, None);
    let on_disk = Settings::load(&sb.config()).expect("settings.json");
    assert_eq!(on_disk.providers.chat, Some(ChatChoice::Local));
    assert_eq!(on_disk.providers.rewrite_quality, RewriteQuality::Best);
    let saved = api::set_providers(&state, json!({"rewrite_quality": "standard"})).expect("saved");
    assert_eq!(
        saved.providers.chat,
        Some(ChatChoice::Local),
        "chat is kept"
    );
    assert_eq!(saved.providers.rewrite_quality, RewriteQuality::Standard);
    let refused = api::set_providers(&state, json!({"chat": "someone-else"}));
    assert!(refused.is_err(), "{refused:?}");
    let on_disk = Settings::load(&sb.config()).expect("settings.json");
    assert_eq!(on_disk.providers.chat, Some(ChatChoice::Local));
    assert!(t.requests().is_empty());
}

// Scope: `provider_status` has no publik API status before Connect, and the keyless status after.
#[test]
fn e_provider_status_carries_the_publik_status_without_the_key() {
    let sb = Sandbox::new().with_token();
    sb.choose(Some(ChatChoice::Publik));
    let t = Arc::new(Script::default().on_fn(INSTALLS, echoing("installs-201")));
    let state = api::state(t.clone(), &sb.config(), &sb.data());
    let before = api::provider_status(&state);
    assert!(before.get("publik").is_none_or(Value::is_null), "{before}");
    run(api::connect(&state)).expect("connected");
    let after = api::provider_status(&state);
    assert_eq!(after["chat"], "publik");
    assert_eq!(after["publik"]["balance_micros"], 0);
    assert_eq!(after["publik"]["claim_state"], "anonymous");
    assert!(!after.to_string().contains(KEY));
}

// ---------------------------------------------------------------------------------------------
// Bullets 8 and 9: the proof.

// Bullet 8: without a token the proof prints the NoAppToken message, exits 2 and sends nothing.
#[test]
fn t08_the_proof_without_a_token_prints_no_app_token_and_sends_nothing() {
    if real_token_embedded() {
        return;
    }
    let _sb = Sandbox::new();
    let t = Script::default().on_fn(INSTALLS, echoing("installs-201"));
    let (code, out) = run(api::proof(&[], &t, "y\n"));
    assert_eq!(code, 2);
    assert!(out.contains(MSG_NO_APP_TOKEN), "{out}");
    assert!(t.requests().is_empty());
}

/// Runs a built binary with `args` and `env`, stdin closed, for at most `limit`: its exit code
/// (`None` if it had to be stopped), standard output, standard error and how long it took.
fn run_binary(
    path: &str,
    args: &[&str],
    env: &[(&str, &str)],
    limit: Duration,
) -> (Option<i32>, String, String, Duration) {
    let mut command = Command::new(path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in ["PUBLIK_API_KEY", "PUBLIK_API_BASE_URL", "PUBLIK_APP_TOKEN"] {
        command.env_remove(name);
    }
    for (name, value) in env {
        command.env(name, value);
    }
    let started = Instant::now();
    let mut child = command.spawn().expect("the binary starts");
    let mut stopped = false;
    while child
        .try_wait()
        .expect("the binary can be waited on")
        .is_none()
    {
        if started.elapsed() > limit {
            let _ = child.kill();
            stopped = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let output = child.wait_with_output().expect("the output");
    let took = started.elapsed();
    let code = if stopped { None } else { output.status.code() };
    (
        code,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        took,
    )
}

// Bullet 8: the built `publik-proof` without a token prints the NoAppToken message and exits 2.
#[test]
fn t08_the_built_proof_without_a_token_exits_2() {
    if real_token_embedded() {
        return;
    }
    let _sb = Sandbox::new();
    let (code, out, err, _) = run_binary(
        env!("CARGO_BIN_EXE_publik-proof"),
        &[],
        &[],
        Duration::from_secs(30),
    );
    assert_eq!(code, Some(2), "stdout:\n{out}\nstderr:\n{err}");
    assert!(
        out.contains(MSG_NO_APP_TOKEN) || err.contains(MSG_NO_APP_TOKEN),
        "stdout:\n{out}\nstderr:\n{err}"
    );
}

// Scope: with no credential the proof shows DISCLOSURE and asks y/N before provisioning; "n"
// sends nothing.
#[test]
fn t09_the_proof_asks_before_provisioning_and_no_sends_nothing() {
    let _sb = Sandbox::new().with_token();
    let t = Script::default().on_fn(INSTALLS, echoing("installs-201"));
    let (_, out) = run(api::proof(&[], &t, "n\n"));
    assert!(out.contains(DISCLOSURE_TEXT), "{out}");
    assert!(t.requests().is_empty(), "{:?}", t.routes());
}

/// The proof's transport for an install that already has its credential: one chat answer.
fn proof_sandbox() -> Sandbox {
    let sb = Sandbox::new().with_token();
    sb.write_credential(&credential(KEY));
    sb
}

// Bullet 9: the proof's `run` makes one publik-fast call ("Reply with the word ok.") and prints
// the reply, the charge line and the names of the `x-publik-*` headers received, never the key or
// the balance.
#[test]
fn t09_the_proof_prints_the_reply_the_charge_and_the_header_names() {
    let _sb = proof_sandbox();
    let t = Script::default()
        .on(CHAT, fake("chat-200-status"))
        .on_fn(INSTALLS, echoing("installs-201"));
    let (code, out) = run(api::proof(&[], &t, "y\n"));
    assert_eq!(code, 0, "{out}");
    let chats: Vec<HttpRequest> = t
        .requests()
        .into_iter()
        .filter(|r| route(r) == CHAT)
        .collect();
    assert_eq!(chats.len(), 1, "one call");
    let sent = body(&chats[0]);
    assert_eq!(sent["model"], "publik-fast");
    assert!(
        canonical_json(&sent).contains("Reply with the word ok."),
        "{sent}"
    );
    key_only_as_bearer(&chats[0], KEY);
    assert!(
        out.lines().any(|l| l.trim_end().ends_with("ok")),
        "the reply: {out}"
    );
    assert!(
        out.lines().any(|l| l.to_lowercase().contains("charge")),
        "the charge line: {out}"
    );
    for name in STATUS_HEADERS {
        assert!(out.contains(name), "{name} in {out}");
    }
    for secret in [KEY, "3120000", "3.12"] {
        assert!(!out.contains(secret), "{secret} in {out}");
    }
    check_requests(&t.requests());
}

// Bullet 9: `--models` prints the ids of the `/models` body.
#[test]
fn t09_the_proof_lists_the_models() {
    let _sb = proof_sandbox();
    let t = Script::default().on("GET /models", fake("models-200"));
    let (code, out) = run(api::proof(&["--models"], &t, "y\n"));
    assert_eq!(code, 0, "{out}");
    for id in TIERS {
        assert!(out.contains(id), "{id} in {out}");
    }
    key_only_as_bearer(&t.requests()[0], KEY);
    assert!(!out.contains(KEY));
}

// Bullet 9: `--probe-json` asks for `response_format: {"type": "json_object"}` and prints
// "honoured" for a valid JSON answer, "not honoured" otherwise.
#[test]
fn t09_the_proof_probes_json_mode() {
    for (content, honoured) in [("{\"ok\": true}", true), ("ok", false)] {
        let _sb = proof_sandbox();
        let t = Script::default().on(CHAT, answer(content));
        let (_, out) = run(api::proof(&["--probe-json"], &t, "y\n"));
        let reqs = t.requests();
        assert_eq!(reqs.len(), 1, "{out}");
        assert_eq!(
            body(&reqs[0])["response_format"],
            json!({"type": "json_object"})
        );
        assert_eq!(out.contains("not honoured"), !honoured, "{content}: {out}");
        assert!(out.contains("honoured"), "{content}: {out}");
    }
}

// Scope: `--search` makes one web search after its own y/N and prints the first result's title
// and site and the charge line; "n" sends nothing.
#[test]
fn t09_the_proof_searches_once_after_its_own_question() {
    let _sb = proof_sandbox();
    let t = Script::default().on(SEARCH, fake("search-200"));
    let (code, out) = run(api::proof(&["--search"], &t, "y\ny\n"));
    assert_eq!(code, 0, "{out}");
    assert_eq!(t.routes(), [SEARCH]);
    assert_eq!(body(&t.requests()[0])["q"], QUERY);
    let first = &fake_body("search-200")["results"][0];
    for field in ["title", "site"] {
        let value = first[field].as_str().unwrap_or_default();
        assert!(out.contains(value), "{field} {value:?} in {out}");
    }
    assert!(
        out.lines().any(|l| l.to_lowercase().contains("charge")),
        "{out}"
    );
    assert!(!out.contains(KEY));
    drop(_sb);

    let _sb = proof_sandbox();
    let t = Script::default().on(SEARCH, fake("search-200"));
    run(api::proof(&["--search"], &t, "n\nn\n"));
    assert!(t.requests().is_empty(), "{:?}", t.routes());
}

// ---------------------------------------------------------------------------------------------
// Bullet 11 (C18): the built-app self-test.

fn self_test_line_ok() -> String {
    format!("publik self-test: ok redpen {VERSION} (install 5f0c8a52)")
}

// Bullet 11: with the flag set, the fake token and a fake server answering `POST /installs` with
// 201 and then `GET /wallet` with 200, the start-up hook returns exactly the ok line and the
// credential is written.
#[test]
fn t11_c18_the_startup_self_test_provisions_reads_the_wallet_and_says_ok() {
    let sb = Sandbox::new().with_token();
    let t = Script::default()
        .on(INSTALLS, fake("installs-201"))
        .on(WALLET, fake("wallet-200"));
    let line = api::startup_self_test(Some(OsString::from("1")), &t);
    assert_eq!(line, Some(self_test_line_ok()));
    assert_eq!(t.routes(), [INSTALLS, WALLET]);
    let reqs = t.requests();
    assert!(header(&reqs[0], "authorization").is_none());
    assert_eq!(reqs[1].url, "https://publikhq.com/api/v1/wallet");
    key_only_as_bearer(&reqs[1], KEY);
    let file = sb
        .credential()
        .expect("the self-test writes the credential");
    assert_eq!(file["install_id"], INSTALL_ID);
    assert_eq!(file["key"], KEY);
}

// Bullet 11: with the fake answering 500 the hook returns the failed line with the contract's
// message, after one request.
#[test]
fn t11_c18_a_failing_server_gives_the_failed_line() {
    let _sb = Sandbox::new().with_token();
    let t = Script::default().on(INSTALLS, fake("installs-500"));
    let line = api::startup_self_test(Some(OsString::from("1")), &t);
    assert_eq!(
        line.as_deref(),
        Some(
            "publik self-test: failed: publik API could not set up this computer (500). Use your own key for now."
        )
    );
    assert_eq!(t.routes(), [INSTALLS]);
}

// Bullet 11: without the flag the hook returns None and sends nothing.
#[test]
fn t11_c18_without_the_flag_the_self_test_never_runs() {
    let _sb = Sandbox::new().with_token();
    let t = Script::default()
        .on(INSTALLS, fake("installs-201"))
        .on(WALLET, fake("wallet-200"));
    assert_eq!(api::startup_self_test(None, &t), None);
    assert!(t.requests().is_empty());
}

// Error path of bullet 11: publik API counts the line only after a real wallet answer, so a
// failed `GET /wallet` is a failed self-test.
#[test]
fn t11_c18_a_failed_wallet_read_is_a_failed_self_test() {
    let _sb = Sandbox::new().with_token();
    let t = Script::default()
        .on(INSTALLS, fake("installs-201"))
        .on(WALLET, fake("chat-500"));
    let line = api::startup_self_test(Some(OsString::from("1")), &t).expect("a line");
    assert!(line.starts_with("publik self-test: failed: "), "{line}");
    assert!(!line.contains(KEY));
    assert_eq!(t.routes(), [INSTALLS, WALLET]);
}

// Bullet 11: `self_test` names the version it is given.
#[test]
fn t11_c18_self_test_names_the_version_it_is_given() {
    let _sb = Sandbox::new().with_token();
    let t = Script::default()
        .on(INSTALLS, fake("installs-201"))
        .on(WALLET, fake("wallet-200"));
    assert_eq!(
        run(api::self_test(&t, "9.8.7")),
        "publik self-test: ok redpen 9.8.7 (install 5f0c8a52)"
    );
}

// Bullet 11: with the placeholder token the hook fails with NoAppToken and sends nothing.
#[test]
fn t11_c18_the_self_test_without_a_token_fails_with_no_app_token() {
    if real_token_embedded() {
        return;
    }
    let _sb = Sandbox::new();
    let t = Script::default().on(INSTALLS, fake("installs-201"));
    let line = api::startup_self_test(Some(OsString::from("1")), &t);
    assert_eq!(
        line.as_deref(),
        Some(
            "publik self-test: failed: This build has no publik API app token. Use your own key for now."
        )
    );
    assert!(t.requests().is_empty());
}

// Bullet 11: the built `redpen-app` with `PUBLIK_SELF_TEST=1` and the placeholder token prints
// exactly one line, the NoAppToken failure, and exits within 5 s, before any window, tray icon
// or hotkey: started with `--smoke`, the interface's ready event never fires.
#[test]
fn t11_c18_the_built_app_prints_one_line_and_exits_before_any_window() {
    if real_token_embedded() {
        return;
    }
    let sb = Sandbox::new();
    let local = sb.local().to_string_lossy().into_owned();
    let (code, out, err, took) = run_binary(
        env!("CARGO_BIN_EXE_redpen-app"),
        &["--smoke"],
        &[
            ("PUBLIK_SELF_TEST", "1"),
            ("LOCALAPPDATA", local.as_str()),
            ("XDG_CONFIG_HOME", local.as_str()),
        ],
        Duration::from_secs(5),
    );
    assert!(
        code.is_some(),
        "it exits within 5 s\nstdout:\n{out}\nstderr:\n{err}"
    );
    assert!(took < Duration::from_secs(5), "{took:?}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        [
            "publik self-test: failed: This build has no publik API app token. Use your own key for now."
        ],
        "stderr:\n{err}"
    );
    assert!(
        !out.contains("ready") && !err.contains("ready"),
        "{out}{err}"
    );
    assert!(sb.credential().is_none());
}
