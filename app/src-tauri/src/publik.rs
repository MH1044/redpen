//! publik API for Redpen, written from docs/PUBLIK-CONTRACT.md (ADR-0010): the contract's public
//! names, `APP_SLUG = "redpen"`, and rules A1 to A8 of docs/PUBLIK-INTEGRATION.md §2. There is
//! no sign-in gate and no usage telemetry: publik API is set up only when the user chooses it on
//! the consent view at the first AI action, and nothing is sent at start except `GET /wallet`
//! while publik API is the chosen AI provider and a credential exists.
//!
//! Where each rule lives:
//!
//! - **A1** the key never reaches the web view: [`Credential`] and [`OwnKey`] hold it as a
//!   `SecretString` whose `Debug` is `***`; every command returns keyless types ([`ConnectOutcome`],
//!   [`PublikStatus`], [`ProviderStatus`], [`ConsentInfo`]).
//! - **A2** `device_name` is [`device_name`]: `Windows PC` or `Linux PC`, never the hostname.
//! - **A3** every request goes over `redpen_core::net::HttpTransport` (reqwest in production,
//!   the fake transport in tests); the key travels only in an `Authorization` header marked
//!   secret.
//! - **A4** chat goes through `redpen_core::provider::AiClient`: [`Credential::provider`] hands it
//!   `{base_url, key, tier}`.
//! - **A5** the status (`status.rs`) is refreshed from `GET /wallet` at start and when Settings
//!   opens, only while publik API is chosen and a credential exists ([`PublikService::refresh`]).
//! - **A6** the tier rule is `redpen_core::provider::tier`; `resolver.rs` feeds it.
//! - **A7** `dialects: ["chat_completions"]` in [`installs_body`].
//! - **A8** [`search`] goes over the transport too and returns `SearchOutcome`; 502 and 504 are
//!   `SearchError::ProviderFailed`.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use redpen_core::net::{
    Body, HeaderValue, HttpRequest, HttpResponse, HttpTransport, Method, TransportError,
};
use redpen_core::provider::publik_errors::{
    self, INSTALL_REVOKED, PROVISION_BUSY_FALLBACK, UNREACHABLE, is_https_link, parse_error_body,
    provision_failed_message,
};
use redpen_core::provider::search::{
    SEARCH_PATH, SEARCH_TIMEOUT_SECONDS, parse_search_answer, search_body, search_error,
    search_transport_error,
};
use redpen_core::provider::{
    AiError, ClaimState, ProviderKind, ResolvedProvider, SearchError, SearchOutcome,
};
use redpen_core::store::settings::{ChatChoice, RewriteQuality, SearchChoice};
use redpen_core::store::{Settings, SettingsError};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use crate::status::{PublikStatus, PublikStatusSink, StatusStore};
pub use redpen_core::provider::publik_errors::{map_publik_error, top_up_url_from_402};

/// This app's slug on publik.
pub const APP_SLUG: &str = "redpen";
/// The version sent in `POST /installs`.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// publik API's base URL (contract §1).
pub const DEFAULT_BASE_URL: &str = "https://publikhq.com/api/v1";
/// The version of [`DISCLOSURE`] sent as `disclosure_version`.
pub const DISCLOSURE_VERSION: u32 = 1;
/// Shown verbatim under the data-path sentence on the consent view, and accepted, before
/// [`provision`] (contract §3 and §1, Privacy). Redpen's own text (ADR-0010).
pub const DISCLOSURE: &str = "Requests go through publik's servers to a shared AI model account. publik does not train on them and keeps no request bodies beyond a 24-hour fingerprint that stops a request being charged twice. The AI provider's own abuse monitoring applies.";
/// The consent view's title (docs/PUBLIK-INTEGRATION.md §4).
pub const CONSENT_TITLE: &str = "Choose how Redpen uses AI";
/// Redpen's data-path sentence (docs/PUBLIK-INTEGRATION.md §4), character for character.
pub const DATA_PATH: &str = "Checking is done on this computer and needs no AI. Only the text you select and the action you choose go through publik's servers, and only when you press a rewrite button.";
/// When the credential could not be written after a key was minted.
pub const CREDENTIAL_NOT_SAVED: &str =
    "Redpen could not save the publik API setup on this computer. Use your own key for now.";

/// The environment variable that overrides the app token (at build time, and at run time).
pub const ENV_APP_TOKEN: &str = "PUBLIK_APP_TOKEN";
/// The environment variables of contract §4, step 2.
pub const ENV_API_KEY: &str = "PUBLIK_API_KEY";
pub const ENV_BASE_URL: &str = "PUBLIK_API_BASE_URL";

/// `POST /installs`, `POST /installs/revoke` and `GET /wallet` under the base URL.
pub const INSTALLS_PATH: &str = "/installs";
pub const REVOKE_PATH: &str = "/installs/revoke";
pub const WALLET_PATH: &str = "/wallet";
/// How long a housekeeping request may take (connect 5 s).
pub const HOUSEKEEPING_TIMEOUT: Duration = Duration::from_secs(30);

/// The token compiled in by `build.rs`: `publik-app-token.txt`, or `PUBLIK_APP_TOKEN` at build
/// time; empty without one.
const BUILD_APP_TOKEN: &str = env!("REDPEN_PUBLIK_APP_TOKEN");

// ---------------------------------------------------------------------------------------------
// The credential.

/// The per-install credential (contract §3), stored at [`credential_path`] with mode 0600.
/// After Disconnect the key is gone and `install_id` stays, so reconnecting does not create a
/// second install.
#[derive(Clone, Serialize, Deserialize)]
pub struct Credential {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "secret_key")]
    pub key: Option<SecretString>,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub install_id: String,
    #[serde(default)]
    pub claim_url: String,
    #[serde(default)]
    pub claim_state: ClaimState,
    #[serde(default)]
    pub add_credit_url: String,
    #[serde(default)]
    pub cost_sentence: String,
    #[serde(default)]
    pub balance_micros: i64,
}

fn default_base_url() -> String {
    DEFAULT_BASE_URL.to_string()
}

/// No key, publik's base URL, nothing else known.
impl Default for Credential {
    fn default() -> Self {
        Self {
            key: None,
            base_url: default_base_url(),
            install_id: String::new(),
            claim_url: String::new(),
            claim_state: ClaimState::default(),
            add_credit_url: String::new(),
            cost_sentence: String::new(),
            balance_micros: 0,
        }
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("key", &self.key.as_ref().map(|_| "***"))
            .field("base_url", &self.base_url)
            .field("install_id", &self.install_id)
            .field("claim_state", &self.claim_state)
            .field("balance_micros", &self.balance_micros)
            .finish_non_exhaustive()
    }
}

impl Credential {
    /// Whether the credential holds a usable key.
    pub fn has_key(&self) -> bool {
        self.key
            .as_ref()
            .is_some_and(|k| !k.expose_secret().trim().is_empty())
    }

    /// The provider for a chat call on this credential with `tier` as the model.
    pub fn provider(&self, tier: &str) -> ResolvedProvider {
        ResolvedProvider {
            kind: ProviderKind::Publik,
            base_url: self.base_url.clone(),
            key: self.key.clone(),
            chat_model: tier.to_string(),
        }
    }

    /// `path` under this credential's base URL.
    pub fn url(&self, path: &str) -> String {
        join_url(&self.base_url, path)
    }

    /// Reads the credential at `path`; `None` when there is none or it cannot be read.
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Writes the credential at `path` atomically, mode 0600 on Unix.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut text = serde_json::to_string_pretty(self).expect("the credential serialises");
        text.push('\n');
        redpen_core::store::write_atomic(path, text.as_bytes(), true)
    }
}

/// `key` as a plain string in the file, and a `SecretString` in memory.
mod secret_key {
    use secrecy::{ExposeSecret, SecretString};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(key: &Option<SecretString>, s: S) -> Result<S::Ok, S::Error> {
        match key {
            Some(k) => s.serialize_str(k.expose_secret()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SecretString>, D::Error> {
        let key = Option::<String>::deserialize(d)?;
        Ok(key.filter(|k| !k.trim().is_empty()).map(SecretString::from))
    }
}

/// Where the credential lives (contract §3): `%LOCALAPPDATA%\publik\apps\redpen.json` on Windows,
/// `$XDG_CONFIG_HOME/publik/apps/redpen.json` (or `~/.config/...`) on Linux.
pub fn credential_path() -> PathBuf {
    credential_path_from(|name| std::env::var_os(name))
}

/// [`credential_path`] with the environment read through `var`.
pub fn credential_path_from(var: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    let set = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let file = format!("{APP_SLUG}.json");
    let base = if cfg!(windows) {
        set("LOCALAPPDATA")
            .or_else(|| set("USERPROFILE").map(|home| home.join("AppData").join("Local")))
            .unwrap_or_else(|| PathBuf::from("."))
    } else if cfg!(target_os = "macos") {
        set("HOME")
            .map(|home| home.join("Library").join("Application Support"))
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        set("XDG_CONFIG_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| set("HOME").map(|home| home.join(".config")))
            .unwrap_or_else(|| PathBuf::from("."))
    };
    base.join("publik").join("apps").join(file)
}

// ---------------------------------------------------------------------------------------------
// Choosing the provider (contract §4).

/// The user's own key: any OpenAI-compatible address, key and chat model.
#[derive(Clone)]
pub struct OwnKey {
    pub base_url: String,
    pub key: SecretString,
    pub chat_model: String,
}

impl fmt::Debug for OwnKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnKey")
            .field("base_url", &self.base_url)
            .field("key", &"***")
            .field("chat_model", &self.chat_model)
            .finish()
    }
}

impl OwnKey {
    /// The own key as `AiClient` calls it.
    pub fn provider(&self) -> ResolvedProvider {
        ResolvedProvider::own_key(
            self.base_url.clone(),
            self.key.clone(),
            self.chat_model.clone(),
        )
    }
}

/// Where chat calls go, in the contract's order.
#[derive(Debug, Clone)]
pub enum Resolution {
    /// The user's own key.
    OwnKey(OwnKey),
    /// `PUBLIK_API_KEY` (and `PUBLIK_API_BASE_URL`) in the environment.
    Env(Credential),
    /// The credential file.
    File(Credential),
    /// Nothing is set up: show the consent view.
    NeedsConsent,
}

impl Resolution {
    /// The publik credential, for `Env` and `File`.
    pub fn credential(&self) -> Option<&Credential> {
        match self {
            Resolution::Env(c) | Resolution::File(c) => Some(c),
            Resolution::OwnKey(_) | Resolution::NeedsConsent => None,
        }
    }
}

/// The provider in the contract's order: the user's own key if given; else `PUBLIK_API_KEY`
/// with `PUBLIK_API_BASE_URL` (or publik's base URL); else the credential file, if it holds a
/// key; else `NeedsConsent`.
pub fn resolve_provider(user_key: Option<&OwnKey>) -> Resolution {
    resolve_provider_with(user_key, &env_var, &credential_path())
}

/// [`resolve_provider`] with the environment read through `env` and the credential at `path`.
pub fn resolve_provider_with(
    user_key: Option<&OwnKey>,
    env: &dyn Fn(&str) -> Option<String>,
    path: &Path,
) -> Resolution {
    if let Some(own) = user_key {
        return Resolution::OwnKey(own.clone());
    }
    if let Some(key) = env(ENV_API_KEY).filter(|k| !k.trim().is_empty()) {
        return Resolution::Env(Credential {
            key: Some(SecretString::from(key.trim().to_string())),
            base_url: env(ENV_BASE_URL)
                .map(|b| b.trim().to_string())
                .filter(|b| !b.is_empty())
                .unwrap_or_else(default_base_url),
            ..Credential::default()
        });
    }
    match Credential::load(path) {
        Some(c) if c.has_key() => Resolution::File(c),
        _ => Resolution::NeedsConsent,
    }
}

/// An environment variable, if set to valid Unicode.
fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

// ---------------------------------------------------------------------------------------------
// The app token (contract §2).

/// The `pat_` token in a token file's text: its first line that is neither empty nor a `#`
/// comment, if that line is a `pat_` token. The placeholder file gives `None`.
pub fn parse_app_token(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))?;
    let valid = line.len() > "pat_".len()
        && line.starts_with("pat_")
        && line
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    valid.then(|| line.to_string())
}

/// The app token: `PUBLIK_APP_TOKEN` at run time, else the one compiled in from
/// `publik-app-token.txt` (or `PUBLIK_APP_TOKEN` at build time). `None` for a build without one.
pub fn app_token() -> Option<String> {
    app_token_from(env_var(ENV_APP_TOKEN).as_deref(), BUILD_APP_TOKEN)
}

/// [`app_token`] from a run-time value and the compiled-in text.
pub fn app_token_from(run_time: Option<&str>, built_in: &str) -> Option<String> {
    run_time
        .and_then(parse_app_token)
        .or_else(|| parse_app_token(built_in))
}

/// Whether this build carries an app token, without the run-time override: what the
/// maintainer's setup step adds.
pub fn build_has_app_token() -> bool {
    parse_app_token(BUILD_APP_TOKEN).is_some()
}

// ---------------------------------------------------------------------------------------------
// Money.

/// The balance line (contract §9; Redpen's wording, ADR-0010): above 0, the amount and " of
/// publik balance" (`$3.12 of publik balance`, `$0.0021 of publik balance` under one cent); at 0
/// while anonymous `$0.00 · link this computer for $0.05 of free use`; at 0 once claimed
/// `$0.00 of publik balance`. A negative balance reads as 0.
pub fn balance_line(micros: i64, claim_state: ClaimState) -> String {
    if micros <= 0 {
        return match claim_state {
            ClaimState::Anonymous => "$0.00 · link this computer for $0.05 of free use".into(),
            ClaimState::Claimed => "$0.00 of publik balance".into(),
        };
    }
    format!("{} of publik balance", dollars(micros))
}

/// An amount of micro-dollars in dollars, rounded down so a balance is never overstated: whole
/// cents from one cent up (`$3.12`, `$1,250.00`), four decimals below (`$0.0021`). Integer
/// arithmetic only.
pub fn dollars(micros: i64) -> String {
    let micros = micros.max(0);
    if micros < 10_000 {
        return format!("$0.{:04}", micros / 100);
    }
    let cents = micros / 10_000;
    format!("${}.{:02}", group_thousands(cents / 100), cents % 100)
}

fn group_thousands(n: i64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Provisioning (contract §5).

/// Why provisioning failed. `Display` is the message the interface shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisionError {
    /// This build has no app token.
    NoAppToken,
    /// publik's `error.message`, or the contract's fixed text.
    Failed { message: String },
}

impl fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProvisionError::NoAppToken => f.write_str(publik_errors::NO_APP_TOKEN),
            ProvisionError::Failed { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for ProvisionError {}

impl From<ProvisionError> for AiError {
    fn from(e: ProvisionError) -> Self {
        match e {
            ProvisionError::NoAppToken => AiError::NoAppToken,
            ProvisionError::Failed { message } => AiError::ProvisionFailed { message },
        }
    }
}

/// What provisioning and the provider choice read from the computer, gathered once so tests can
/// give their own.
#[derive(Debug, Clone)]
pub struct PublikOptions {
    /// Where the credential is kept.
    pub credential_path: PathBuf,
    /// The `pat_` app token, if this build (or the environment) has one.
    pub app_token: Option<String>,
    /// The base URL `POST /installs` goes to.
    pub base_url: String,
    /// `app_version` in `POST /installs`.
    pub app_version: String,
    /// A fresh install id when there is none to keep.
    pub new_install_id: fn() -> String,
    /// Reads an environment variable (`PUBLIK_API_KEY`, `PUBLIK_API_BASE_URL`).
    pub env: fn(&str) -> Option<String>,
}

impl PublikOptions {
    /// The options of this computer and build: [`credential_path`], [`app_token`],
    /// `PUBLIK_API_BASE_URL` or publik's base URL, [`APP_VERSION`], a random version 4 id.
    pub fn current() -> Self {
        Self {
            credential_path: credential_path(),
            app_token: app_token(),
            base_url: env_var(ENV_BASE_URL)
                .map(|b| b.trim().to_string())
                .filter(|b| !b.is_empty())
                .unwrap_or_else(default_base_url),
            app_version: APP_VERSION.to_string(),
            new_install_id,
            env: env_var,
        }
    }

    /// The provider choice with these options.
    pub fn resolve(&self, user_key: Option<&OwnKey>) -> Resolution {
        resolve_provider_with(user_key, &self.env, &self.credential_path)
    }
}

/// A random version 4 UUID, lower case with hyphens.
pub fn new_install_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// `windows` or `linux` (`std::env::consts::OS`).
pub fn os_name() -> &'static str {
    std::env::consts::OS
}

/// `x86_64` or `aarch64` (`std::env::consts::ARCH`).
pub fn arch_name() -> &'static str {
    std::env::consts::ARCH
}

/// The generic label sent as `device_name` (rule A2): never the hostname.
pub fn device_name() -> &'static str {
    if cfg!(windows) {
        "Windows PC"
    } else if cfg!(target_os = "macos") {
        "Mac"
    } else {
        "Linux PC"
    }
}

/// The body of `POST /installs`, exactly as contract §5 gives it.
pub fn installs_body(app_token: &str, app_version: &str, install_id: &str) -> Value {
    json!({
        "app_token": app_token,
        "app_slug": APP_SLUG,
        "app_version": app_version,
        "os": os_name(),
        "arch": arch_name(),
        "device_name": device_name(),
        "install_id": install_id,
        "disclosure_version": DISCLOSURE_VERSION,
        "dialects": ["chat_completions"],
    })
}

/// Sets up this computer: `POST /installs` with the body of contract §5 (no auth header), and
/// the credential written with mode 0600 before it returns. Uses [`PublikOptions::current`].
pub async fn provision(t: &dyn HttpTransport) -> Result<Credential, ProvisionError> {
    provision_with(t, &PublikOptions::current()).await
}

/// [`provision`] with these options.
///
/// - The `install_id` kept in the credential is reused (Reconnect never creates a second
///   install); otherwise a fresh one.
/// - **201**, or **200** with a key: the new credential.
/// - **200** with `key: null`: the key already on disk is kept; with no key on disk, one more
///   attempt under a fresh `install_id`.
/// - **403** `install_revoked`: one more attempt under a fresh `install_id`.
/// - **429**: `error.message`; anything else: `error.message` or "publik API could not set up
///   this computer (code). Use your own key for now."; no answer: "publik API is unreachable
///   right now. Use your own key for now." A second failure gives its own message.
pub async fn provision_with(
    t: &dyn HttpTransport,
    o: &PublikOptions,
) -> Result<Credential, ProvisionError> {
    let token = o.app_token.clone().ok_or(ProvisionError::NoAppToken)?;
    let existing = Credential::load(&o.credential_path);
    let mut install_id = existing
        .as_ref()
        .map(|c| c.install_id.trim().to_string())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(o.new_install_id);
    let mut minted_again = false;
    loop {
        let req = HttpRequest::post_json(
            join_url(&o.base_url, INSTALLS_PATH),
            installs_body(&token, &o.app_version, &install_id),
        )
        .header("Content-Type", HeaderValue::plain("application/json"))
        .timeout(HOUSEKEEPING_TIMEOUT);
        let resp = t.send(req).await.map_err(provision_transport_error)?;
        tracing::info!(status = resp.status, "publik API answered POST /installs");
        let body = resp.json().unwrap_or(Value::Null);
        let new_key = body
            .get("key")
            .and_then(Value::as_str)
            .filter(|k| !k.trim().is_empty());
        let failed = |fallback: String| ProvisionError::Failed {
            message: parse_error_body(&resp.body).message.unwrap_or(fallback),
        };
        match resp.status {
            200 | 201 if new_key.is_some() => {
                let key = SecretString::from(new_key.unwrap_or_default().trim().to_string());
                return save_new(o, credential_from(&body, key, &o.base_url, &install_id));
            }
            200 => {
                let kept = existing
                    .as_ref()
                    .filter(|c| c.has_key() && c.install_id.trim() == install_id)
                    .and_then(|c| c.key.clone());
                if let Some(key) = kept {
                    let base = existing
                        .as_ref()
                        .map(|c| c.base_url.clone())
                        .unwrap_or_else(|| o.base_url.clone());
                    return save_new(o, credential_from(&body, key, &base, &install_id));
                }
                if !minted_again {
                    minted_again = true;
                    install_id = (o.new_install_id)();
                    continue;
                }
                return Err(failed(provision_failed_message(200)));
            }
            403 if parse_error_body(&resp.body).code.as_deref() == Some(INSTALL_REVOKED)
                && !minted_again =>
            {
                minted_again = true;
                install_id = (o.new_install_id)();
                continue;
            }
            429 => return Err(failed(PROVISION_BUSY_FALLBACK.to_string())),
            code => return Err(failed(provision_failed_message(code))),
        }
    }
}

/// Writes the new credential before handing it back.
fn save_new(o: &PublikOptions, c: Credential) -> Result<Credential, ProvisionError> {
    match c.save(&o.credential_path) {
        Ok(()) => Ok(c),
        Err(_) => {
            tracing::warn!("the publik credential could not be written");
            Err(ProvisionError::Failed {
                message: CREDENTIAL_NOT_SAVED.to_string(),
            })
        }
    }
}

/// The credential from a `POST /installs` answer (contract §5): `key`, `base_url`,
/// `install_id`, `claim_url`, `claim_state`, `balance_micros`, `wallet {balance_micros,
/// add_credit_url, claim_state}` and `disclosure {cost}` as `cost_sentence`.
fn credential_from(
    body: &Value,
    key: SecretString,
    base_url: &str,
    install_id: &str,
) -> Credential {
    let wallet = body.get("wallet").filter(|w| w.is_object());
    let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    let either =
        |name: &str| text(body.get(name)).or_else(|| text(wallet.and_then(|w| w.get(name))));
    Credential {
        key: Some(key),
        base_url: text(body.get("base_url"))
            .filter(|b| b.starts_with("https://") || b.starts_with("http://"))
            .unwrap_or_else(|| base_url.to_string()),
        install_id: text(body.get("install_id"))
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| install_id.to_string()),
        claim_url: either("claim_url").unwrap_or_default(),
        claim_state: either("claim_state")
            .and_then(|s| ClaimState::parse(&s))
            .unwrap_or_default(),
        add_credit_url: either("add_credit_url").unwrap_or_default(),
        cost_sentence: text(body.get("cost_sentence"))
            .or_else(|| text(body.get("disclosure").and_then(|d| d.get("cost"))))
            .unwrap_or_default(),
        balance_micros: body
            .get("balance_micros")
            .and_then(Value::as_i64)
            .or_else(|| {
                wallet
                    .and_then(|w| w.get("balance_micros"))
                    .and_then(Value::as_i64)
            })
            .unwrap_or(0),
    }
}

/// No answer to `POST /installs`.
fn provision_transport_error(e: TransportError) -> ProvisionError {
    let message = match e {
        TransportError::Unrecorded { .. } => e.to_string(),
        TransportError::Timeout | TransportError::Offline | TransportError::Tls(_) => {
            UNREACHABLE.to_string()
        }
    };
    tracing::warn!("POST /installs got no answer");
    ProvisionError::Failed { message }
}

/// `{base}{path}` without a doubled slash.
fn join_url(base: &str, path: &str) -> String {
    format!("{}{path}", base.trim_end_matches('/'))
}

/// A request with the key as `Authorization: Bearer`, marked secret (rule A3).
fn with_bearer(req: HttpRequest, key: &SecretString) -> HttpRequest {
    req.header(
        "Authorization",
        HeaderValue::secret(format!("Bearer {}", key.expose_secret())),
    )
}

// ---------------------------------------------------------------------------------------------
// Disconnect, the wallet, the self-test.

/// Disconnect (contract §5, observed): `POST {base_url}/installs/revoke` with the key, best
/// effort, then the key is forgotten and `install_id` kept.
pub async fn disconnect(t: &dyn HttpTransport) {
    disconnect_at(t, &credential_path()).await
}

/// [`disconnect`] for the credential at `path`.
pub async fn disconnect_at(t: &dyn HttpTransport, path: &Path) {
    let Some(mut cred) = Credential::load(path) else {
        return;
    };
    if let Some(key) = cred.key.clone().filter(|_| cred.has_key()) {
        let req = with_bearer(
            HttpRequest {
                method: Method::Post,
                url: cred.url(REVOKE_PATH),
                headers: Vec::new(),
                body: Body::Empty,
                timeout: HOUSEKEEPING_TIMEOUT,
            },
            &key,
        );
        match t.send(req).await {
            Ok(resp) => tracing::info!(status = resp.status, "publik API answered the revoke"),
            Err(_) => tracing::warn!("the revoke got no answer; the key is forgotten anyway"),
        }
    }
    cred.key = None;
    if cred.save(path).is_err() {
        tracing::warn!("the publik credential could not be rewritten without its key");
    }
}

/// `GET {base_url}/wallet` with the key. The answer, or the error for a failed one.
pub async fn wallet(t: &dyn HttpTransport, cred: &Credential) -> Result<HttpResponse, AiError> {
    let key = cred
        .key
        .clone()
        .filter(|_| cred.has_key())
        .ok_or(AiError::NeedsConsent)?;
    let req = with_bearer(
        HttpRequest::get(cred.url(WALLET_PATH)).timeout(HOUSEKEEPING_TIMEOUT),
        &key,
    );
    let resp = t.send(req).await.map_err(|e| match e {
        TransportError::Timeout => AiError::PublikTimeout {
            seconds: HOUSEKEEPING_TIMEOUT.as_secs(),
        },
        TransportError::Offline | TransportError::Tls(_) => AiError::Offline,
        TransportError::Unrecorded { hash, path } => AiError::Unrecorded { hash, path },
    })?;
    tracing::info!(status = resp.status, "publik API answered GET /wallet");
    if (200..=299).contains(&resp.status) {
        Ok(resp)
    } else {
        Err(map_publik_error(resp.status, &resp.body))
    }
}

/// The built-app self-test (contract §3, test 18): provisions for real, then reads `GET
/// /wallet` with the new key, and gives exactly one line, `publik self-test: ok redpen <version>
/// (install <first 8 characters of install_id>)` or `publik self-test: failed: <message>`.
pub async fn self_test(t: &dyn HttpTransport, app_version: &str) -> String {
    let mut o = PublikOptions::current();
    o.app_version = app_version.to_string();
    self_test_with(t, &o).await
}

/// [`self_test`] with these options.
pub async fn self_test_with(t: &dyn HttpTransport, o: &PublikOptions) -> String {
    let cred = match provision_with(t, o).await {
        Ok(c) => c,
        Err(e) => return format!("publik self-test: failed: {e}"),
    };
    match wallet(t, &cred).await {
        Ok(_) => format!(
            "publik self-test: ok {APP_SLUG} {} (install {})",
            o.app_version,
            cred.install_id.chars().take(8).collect::<String>()
        ),
        Err(e) => format!("publik self-test: failed: {e}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Web search (contract §6a, rule A8).

/// One web search: `POST {base_url}/search` with exactly `{"q", "count"}` and the key only in
/// the `Authorization` header, never retried. 402 → `NeedsCredit`; 502 and 504 →
/// `ProviderFailed` (nothing was charged); the rest as for chat.
pub async fn search(
    t: &dyn HttpTransport,
    cred: &Credential,
    q: &str,
    count: u8,
) -> Result<SearchOutcome, SearchError> {
    search_tracked(t, cred, q, count, None).await
}

/// [`search`], with the response's status headers read into `status` like any other.
pub async fn search_tracked(
    t: &dyn HttpTransport,
    cred: &Credential,
    q: &str,
    count: u8,
    status: Option<&StatusStore>,
) -> Result<SearchOutcome, SearchError> {
    let key = cred
        .key
        .clone()
        .filter(|_| cred.has_key())
        .ok_or(SearchError::Disconnected)?;
    let req = with_bearer(
        HttpRequest::post_json(cred.url(SEARCH_PATH), search_body(q, count))
            .header("Content-Type", HeaderValue::plain("application/json"))
            .timeout(Duration::from_secs(SEARCH_TIMEOUT_SECONDS)),
        &key,
    );
    let resp = match t.send(req).await {
        Ok(resp) => resp,
        Err(e) => return Err(search_transport_error(e)),
    };
    tracing::info!(
        status = resp.status,
        bytes = resp.body.len(),
        "publik API answered a search"
    );
    if let Some(store) = status {
        store.on_response(resp.status, &resp.headers);
    }
    let result = if (200..=299).contains(&resp.status) {
        parse_search_answer(&resp.body, resp.header("x-publik-charge-micros"))
    } else {
        Err(search_error(resp.status, &resp.body))
    };
    if let (Err(e), Some(store)) = (&result, status) {
        store.note_search_error(e);
    }
    result
}

// ---------------------------------------------------------------------------------------------
// What the commands return.

/// The consent view's text (`consent_info`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConsentInfo {
    pub title: String,
    pub data_path: String,
    pub disclosure: String,
    pub disclosure_version: u32,
}

/// The consent view's title, data-path sentence and [`DISCLOSURE`].
pub fn consent_info() -> ConsentInfo {
    ConsentInfo {
        title: CONSENT_TITLE.to_string(),
        data_path: DATA_PATH.to_string(),
        disclosure: DISCLOSURE.to_string(),
        disclosure_version: DISCLOSURE_VERSION,
    }
}

/// What `publik_connect` returns: the status, whether the pending action runs now
/// (`balance_micros > 0`), and the first-run card (docs/PUBLIK-INTEGRATION.md §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectOutcome {
    pub status: PublikStatus,
    pub run_pending: bool,
    /// `balance_line(balance_micros, claim_state)`.
    pub balance_line: String,
    /// The `/installs` answer's cost sentence, verbatim.
    pub cost_sentence: String,
    /// Where **Sign in with publik** goes; opened through `open_account_link("claim")`.
    pub claim_url: String,
    pub claim_state: ClaimState,
}

impl ConnectOutcome {
    /// The outcome of provisioning `cred`, with `status` built from it.
    pub fn new(cred: &Credential, status: PublikStatus) -> Self {
        Self {
            run_pending: cred.balance_micros > 0,
            balance_line: balance_line(cred.balance_micros, cred.claim_state),
            cost_sentence: cred.cost_sentence.clone(),
            claim_url: cred.claim_url.clone(),
            claim_state: cred.claim_state,
            status,
        }
    }
}

/// What `provider_status` returns: the chosen AI provider and, while publik API is set up, its
/// status and balance line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderStatus {
    pub chat: Option<ChatChoice>,
    pub publik: Option<PublikStatus>,
    pub balance_line: Option<String>,
}

/// The links `open_account_link` can open: never a URL from the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// The credential's `claim_url` (Sign in with publik).
    Claim,
    /// The credential's `add_credit_url` (Add a plan or pack).
    AddCredit,
    /// The `top_up_url` of the last 402.
    TopUp,
}

/// Why no link was opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// There is no such link now.
    NoLink,
    /// The system browser could not be started.
    CouldNotOpen,
}

impl LinkError {
    pub fn kind(&self) -> &'static str {
        match self {
            LinkError::NoLink => "NoLink",
            LinkError::CouldNotOpen => "CouldNotOpen",
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LinkError::NoLink => "There is no publik link to open right now.",
            LinkError::CouldNotOpen => "Redpen could not open your browser.",
        })
    }
}

impl Serialize for LinkError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        json!({"kind": self.kind(), "message": self.to_string()}).serialize(s)
    }
}

/// `set_providers`: each field given replaces the setting; a field left out stays.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ProvidersPatch {
    pub chat: Option<ChatChoice>,
    pub rewrite_quality: Option<RewriteQuality>,
    pub search: Option<SearchChoice>,
}

/// Settings shared by the commands and the resolver.
pub type SharedSettings = Arc<RwLock<Settings>>;

// ---------------------------------------------------------------------------------------------
// The service behind the commands.

/// Everything the publik commands do, without Tauri, so tests drive it with a fake transport.
pub struct PublikService {
    transport: Arc<dyn HttpTransport>,
    options: PublikOptions,
    config_dir: PathBuf,
    settings: SharedSettings,
    status: Arc<StatusStore>,
}

impl fmt::Debug for PublikService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PublikService")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl PublikService {
    /// A service over this transport and these options; Settings are saved under `config_dir`.
    pub fn new(
        transport: Arc<dyn HttpTransport>,
        options: PublikOptions,
        config_dir: PathBuf,
        settings: SharedSettings,
        status: Arc<StatusStore>,
    ) -> Self {
        Self {
            transport,
            options,
            config_dir,
            settings,
            status,
        }
    }

    /// The options in use.
    pub fn options(&self) -> &PublikOptions {
        &self.options
    }

    /// The status store.
    pub fn status(&self) -> &Arc<StatusStore> {
        &self.status
    }

    /// The settings.
    pub fn settings(&self) -> Settings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The credential file, if there is one.
    pub fn credential(&self) -> Option<Credential> {
        Credential::load(&self.options.credential_path)
    }

    /// The publik credential chat would use now (environment, then file), if any.
    pub fn active_credential(&self) -> Option<Credential> {
        self.options.resolve(None).credential().cloned()
    }

    /// `publik_connect`, after the user chose **Continue with publik API**: provisions, writes
    /// the status, makes publik API the AI provider, and returns the first-run card.
    pub async fn connect(&self) -> Result<ConnectOutcome, AiError> {
        let cred = provision_with(self.transport.as_ref(), &self.options).await?;
        let status = PublikStatus::new(cred.claim_state, cred.balance_micros, 0);
        self.status.replace(status);
        let status = self.status.current().expect("the status was just set");
        let patch = ProvidersPatch {
            chat: Some(ChatChoice::Publik),
            ..ProvidersPatch::default()
        };
        if self.set_providers(patch).is_err() {
            tracing::warn!("publik API could not be saved as the AI provider");
        }
        Ok(ConnectOutcome::new(&cred, status))
    }

    /// `publik_disconnect`: revoke (best effort), forget the key, keep `install_id`, forget the
    /// status.
    pub async fn disconnect(&self) {
        disconnect_at(self.transport.as_ref(), &self.options.credential_path).await;
        self.status.clear();
    }

    /// `publik_refresh` (Settings opening): `GET /wallet`, only while publik API is the chosen
    /// AI provider and a credential exists. `NotConfigured` while another provider is chosen,
    /// `NeedsConsent` with no credential; neither sends anything.
    pub async fn refresh(&self) -> Result<PublikStatus, AiError> {
        if self.settings().providers.chat != Some(ChatChoice::Publik) {
            return Err(AiError::NotConfigured);
        }
        let cred = self.active_credential().ok_or(AiError::NeedsConsent)?;
        match wallet(self.transport.as_ref(), &cred).await {
            Ok(resp) => Ok(self.status.on_wallet(&resp)),
            Err(e) => {
                self.status.note_error(&e);
                Err(e)
            }
        }
    }

    /// The refresh at start: `GET /wallet` exactly when [`PublikService::refresh`] would send
    /// it, otherwise nothing (`None`).
    pub async fn refresh_at_start(&self) -> Option<Result<PublikStatus, AiError>> {
        let chosen = self.settings().providers.chat == Some(ChatChoice::Publik);
        if !chosen || self.active_credential().is_none() {
            return None;
        }
        Some(self.refresh().await)
    }

    /// `provider_status`.
    pub fn provider_status(&self) -> ProviderStatus {
        let publik = self.active_credential().and_then(|_| self.status.current());
        ProviderStatus {
            chat: self.settings().providers.chat,
            balance_line: publik
                .as_ref()
                .map(|s| balance_line(s.balance_micros, s.claim_state)),
            publik,
        }
    }

    /// The URL `open_account_link(kind)` opens: the credential's claim or add-credit link, or
    /// the last 402's top-up link; only an `https` address.
    pub fn account_link(&self, kind: LinkKind) -> Result<String, LinkError> {
        let url = match kind {
            LinkKind::Claim => self.credential().map(|c| c.claim_url),
            LinkKind::AddCredit => self.credential().map(|c| c.add_credit_url),
            LinkKind::TopUp => self.status.pending_top_up(),
        };
        url.filter(|u| is_https_link(u))
            .map(|u| u.trim().to_string())
            .ok_or(LinkError::NoLink)
    }

    /// `set_providers`: applies the patch, saves `settings.json`, and returns the settings.
    /// Nothing changes when the file cannot be written.
    pub fn set_providers(&self, patch: ProvidersPatch) -> Result<Settings, SettingsError> {
        let mut settings = self.settings.write().unwrap_or_else(|e| e.into_inner());
        let mut next = settings.clone();
        if let Some(chat) = patch.chat {
            next.providers.chat = Some(chat);
        }
        if let Some(quality) = patch.rewrite_quality {
            next.providers.rewrite_quality = quality;
        }
        if let Some(search) = patch.search {
            next.providers.search = Some(search);
        }
        next.save(&self.config_dir)?;
        *settings = next.clone();
        Ok(next)
    }

    /// One web search on the active credential, its headers read into the status.
    pub async fn search(&self, q: &str, count: u8) -> Result<SearchOutcome, SearchError> {
        let cred = self.active_credential().ok_or(SearchError::Disconnected)?;
        search_tracked(self.transport.as_ref(), &cred, q, count, Some(&self.status)).await
    }
}

#[cfg(test)]
mod tests;
