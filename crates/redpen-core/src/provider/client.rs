//! [`AiClient`]: the one chat client every provider goes through (docs/modules/02-provider-layer.md).
//!
//! A chat call, in order: the provider is checked (an own key needs its key, address and model;
//! a local address must be this computer, and with no model chosen Ollama is asked for one);
//! the cache is looked up by key K; on a miss the call is counted in `usage_daily`, written to
//! the sent log when that is on, and sent, with the own key's and local retries; every response
//! goes through the status sink; a 2xx answer must hold a `choices[0].message.content` string,
//! which is cached as the call's policy says and returned. Nothing that fails is cached.
//! [`AiClient::chat_validated`] adds the caller's check of that content (module 04's schema and
//! marker checks) before it is cached, so an answer the rewrite engine cannot use never is.
//!
//! On publik API (module 03) every chat request also carries `x-publik-idempotency-key`
//! ([`idempotency_key`] of K and the call's `validation_retry`), its errors map through
//! [`map_publik_error`] with their body (a 402 keeps its one link), a timeout is
//! `PublikTimeout`, and nothing is ever retried (invariant 9). A failed call is also reported to
//! the status sink's `on_error`.
//!
//! Log events carry the provider, the feature, the attempt, the HTTP status, sizes in bytes and
//! durations; never the text, the key, a model's answer or a URL (docs/ARCHITECTURE.md §8).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use secrecy::ExposeSecret;
use serde::Serialize;
use serde_json::{Value, json};

use super::cache::{self, CachedAnswer, MemoryCache, Placement, idempotency_key};
use super::errors::AiError;
use super::local::{self, LocalModel, is_local_url, pick_local_model, tags_url};
use super::publik_errors::map_publik_error;
use super::resolved::{ProviderKind, ResolvedProvider};
use super::retry::{is_retryable, retry_delays};
use super::{ChatCall, ChatReply, Feature, StatusSink};
use crate::clock::{Clock, Sleeper};
use crate::net::{
    HeaderValue, HttpRequest, HttpResponse, HttpTransport, TransportError, canonical_json, url_host,
};
use crate::store::db::{CacheRow, DAY_MS, Db, DbError, Swept, local_day};
use crate::store::settings::PrivacySettings;

/// The chat endpoint under a provider's base URL.
pub const CHAT_PATH: &str = "/chat/completions";
/// The model list under a provider's base URL (the own key's Test).
pub const MODELS_PATH: &str = "/models";
/// How long `GET /models` and Ollama's `GET /api/tags` may take.
pub const LISTING_TIMEOUT: Duration = Duration::from_secs(30);
/// How often old cache and sent-log rows are swept after the start-up sweep.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(3600);

/// The privacy settings the client follows (Settings → Privacy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivacyOptions {
    /// Days an answer stays in `ai_cache` (at most 7); 0 keeps answers in memory only.
    pub remember_answers_days: u32,
    /// "Keep a log": write every sent call to `sent_log`.
    pub sent_log: bool,
}

impl Default for PrivacyOptions {
    fn default() -> Self {
        Self {
            remember_answers_days: 7,
            sent_log: false,
        }
    }
}

impl From<&PrivacySettings> for PrivacyOptions {
    fn from(p: &PrivacySettings) -> Self {
        Self {
            remember_answers_days: p.remember_answers_days,
            sent_log: p.sent_log,
        }
    }
}

/// The result of a provider's Test button: `{ok, message}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderTest {
    pub ok: bool,
    pub message: String,
}

/// The chat client. Share it behind an `Arc`; every method takes `&self`.
pub struct AiClient {
    transport: Arc<dyn HttpTransport>,
    db: Db,
    clock: Arc<dyn Clock>,
    sleeper: Arc<dyn Sleeper>,
    sink: Arc<dyn StatusSink>,
    memory: MemoryCache,
    privacy: Mutex<PrivacyOptions>,
}

impl std::fmt::Debug for AiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiClient")
            .field("privacy", &self.privacy())
            .finish_non_exhaustive()
    }
}

impl AiClient {
    /// A client over these parts, with the default privacy settings (answers kept 7 days, no
    /// sent log). Runs the start-up sweep of `redpen.db`.
    pub fn new(
        t: Arc<dyn HttpTransport>,
        db: Db,
        clock: Arc<dyn Clock>,
        sleeper: Arc<dyn Sleeper>,
        sink: Arc<dyn StatusSink>,
    ) -> Self {
        let client = Self {
            transport: t,
            db,
            clock,
            sleeper,
            sink,
            memory: MemoryCache::default(),
            privacy: Mutex::new(PrivacyOptions::default()),
        };
        client.sweep();
        client
    }

    /// The database the client writes to.
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// The privacy settings in force.
    pub fn privacy(&self) -> PrivacyOptions {
        *self.privacy.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Follows new privacy settings, and sweeps at once so a shorter "remember answers"
    /// period takes effect now.
    pub fn set_privacy(&self, options: PrivacyOptions) {
        *self.privacy.lock().unwrap_or_else(|e| e.into_inner()) = options;
        self.sweep();
    }

    /// Sets Settings → Privacy → remember AI answers (days; 0 is Off).
    pub fn set_remember_answers_days(&self, days: u32) {
        let mut options = self.privacy();
        options.remember_answers_days = days;
        self.set_privacy(options);
    }

    /// Turns Settings → Privacy → "Keep a log" on or off.
    pub fn set_sent_log(&self, on: bool) {
        let mut options = self.privacy();
        options.sent_log = on;
        self.set_privacy(options);
    }

    /// The cache key K of a call (64 lowercase hex).
    pub fn cache_key(p: &ResolvedProvider, c: &ChatCall) -> String {
        cache::cache_key(p, c)
    }

    /// Sends a chat call and returns `choices[0].message.content`, or the answer cached for the
    /// same key.
    pub async fn chat(&self, p: &ResolvedProvider, c: ChatCall) -> Result<ChatReply, AiError> {
        self.chat_validated(p, c, |_| Ok::<(), AiError>(()))
            .await
            .map(|(reply, ())| reply)
    }

    /// [`AiClient::chat`] with a check of the answer (module 04: the schema, the markers, the
    /// cleaning). `check` runs on the content before it is cached or returned, so an answer that
    /// fails it is never cached: the call returns the check's error, and the next Try again goes
    /// to the provider. A cached answer is checked too; one that fails is passed over and the
    /// call is sent. Errors of the call itself reach the status sink as with `chat`; a failed
    /// check does not, since the provider answered.
    pub async fn chat_validated<T, E>(
        &self,
        p: &ResolvedProvider,
        c: ChatCall,
        check: impl Fn(&str) -> Result<T, E> + Send + Sync,
    ) -> Result<(ChatReply, T), E>
    where
        T: Send,
        E: From<AiError> + Send,
    {
        match self.chat_inner(p, &c, &check).await {
            Ok(done) => Ok(done),
            Err(Failed::Call(e)) => {
                tracing::info!(
                    provider = p.kind.as_str(),
                    feature = c.feature.as_str(),
                    error = e.kind(),
                    "chat failed"
                );
                self.sink.on_error(p.kind, &e);
                Err(E::from(e))
            }
            Err(Failed::Check(e)) => {
                tracing::info!(
                    provider = p.kind.as_str(),
                    feature = c.feature.as_str(),
                    "chat answer failed its check"
                );
                Err(e)
            }
        }
    }

    async fn chat_inner<T, E>(
        &self,
        p: &ResolvedProvider,
        c: &ChatCall,
        check: &(impl Fn(&str) -> Result<T, E> + Send + Sync),
    ) -> Result<(ChatReply, T), Failed<E>>
    where
        T: Send,
        E: Send,
    {
        let provider = self.prepare(p).await?;
        let key = cache::cache_key(&provider, c);
        let privacy = self.privacy();
        let now = self.clock.now_ms();
        let placement = cache::placement(c.cache, privacy.remember_answers_days);
        // A Try again after an invalid answer must reach the provider: the cached answer, if
        // any, is the one that failed.
        let cached = if c.validation_retry == 0 {
            self.cache_lookup(&key, placement, now)
        } else {
            None
        };
        if let Some(hit) = cached {
            if let Ok(checked) = check(&hit.content) {
                tracing::info!(
                    provider = provider.kind.as_str(),
                    feature = c.feature.as_str(),
                    "chat answered from the cache"
                );
                let reply = ChatReply {
                    content: hit.content,
                    from_cache: true,
                    charge_micros: None,
                    request_id: None,
                };
                return Ok((reply, checked));
            }
            tracing::info!(
                provider = provider.kind.as_str(),
                feature = c.feature.as_str(),
                "a cached answer failed its check and is passed over"
            );
        }

        let body = chat_body(&provider, c);
        let bytes = u64::try_from(canonical_json(&body).len()).unwrap_or(u64::MAX);
        let mut req = authorised(
            &provider,
            HttpRequest::post_json(provider.url(CHAT_PATH), body).timeout(c.timeout),
        );
        if provider.kind == ProviderKind::Publik {
            req = req.header(
                IDEMPOTENCY_HEADER,
                HeaderValue::plain(idempotency_key(&key, c.validation_retry)),
            );
        }
        let day = local_day(now, self.clock.utc_offset_minutes());
        self.count_call(&provider, c.feature, &day);
        if privacy.sent_log {
            self.log_sent(&provider, c, now, bytes);
        }

        let (outcome, charge) = self
            .send(&provider, CHAT_PATH, Some((c.feature, day.as_str())), req)
            .await;
        let resp = outcome.map_err(|e| transport_error(&provider, e, c.timeout))?;
        if !(200..=299).contains(&resp.status) {
            return Err(status_error(&provider, &resp, true).into());
        }
        let content = answer_content(&resp).ok_or(AiError::InvalidAnswer)?;
        // Checked before it is stored: an answer that fails is never cached (invariant 8).
        let checked = check(&content).map_err(Failed::Check)?;
        let answer = CachedAnswer { content };
        self.cache_store(key, &provider, c, &answer, charge, placement);
        let reply = ChatReply {
            content: answer.content,
            from_cache: false,
            charge_micros: charge,
            request_id: resp
                .header("x-publik-request-id")
                .or_else(|| resp.header("x-request-id"))
                .map(str::to_string),
        };
        Ok((reply, checked))
    }

    /// `GET {base_url}/models` with the key: how many models the provider lists.
    pub async fn list_models(&self, p: &ResolvedProvider) -> Result<usize, AiError> {
        match p.kind {
            ProviderKind::OwnKey if !has_key(p) || url_host(&p.base_url).is_none() => {
                return Err(AiError::NotConfigured);
            }
            ProviderKind::Local if !is_local_url(&p.base_url) => {
                return Err(AiError::LocalNotLocal);
            }
            _ => {}
        }
        let req = authorised(
            p,
            HttpRequest::get(p.url(MODELS_PATH)).timeout(LISTING_TIMEOUT),
        );
        let (outcome, _) = self.send(p, MODELS_PATH, None, req).await;
        let resp = outcome.map_err(|e| transport_error(p, e, LISTING_TIMEOUT))?;
        if !(200..=299).contains(&resp.status) {
            return Err(status_error(p, &resp, false));
        }
        resp.json()
            .and_then(|v| v.get("data").and_then(Value::as_array).map(Vec::len))
            .ok_or(AiError::InvalidAnswer)
    }

    /// The Test button: "Key works: 14 models available." for the own key; for a local
    /// provider, whether Ollama runs and which model Redpen will use. A failure carries the
    /// error's message.
    pub async fn provider_test(&self, p: &ResolvedProvider) -> ProviderTest {
        let outcome = match p.kind {
            ProviderKind::Local => self
                .local_model_for(p)
                .await
                .map(|model| format!("Ollama is running. Redpen will use {model}.")),
            _ => self.list_models(p).await.map(|n| match n {
                1 => "Key works: 1 model available.".to_string(),
                n => format!("Key works: {n} models available."),
            }),
        };
        match outcome {
            Ok(message) => ProviderTest { ok: true, message },
            Err(e) => ProviderTest {
                ok: false,
                message: e.to_string(),
            },
        }
    }

    /// Every model Ollama has at the local `base_url` (`GET /api/tags`).
    pub async fn local_models(&self, base_url: &str) -> Result<Vec<LocalModel>, AiError> {
        Ok(local::local_models(&self.tags(base_url).await?))
    }

    /// The model [`pick_local_model`] chooses from Ollama at the local `base_url`.
    pub async fn detect_local(&self, base_url: &str) -> Result<String, AiError> {
        pick_local_model(&self.tags(base_url).await?).ok_or(AiError::LocalNoModel)
    }

    /// Deletes cached answers older than `remember_answers_days` and sent-log rows older than
    /// 7 days. Runs at start (in [`AiClient::new`]), on a privacy change, and every hour in
    /// [`AiClient::keep_sweeping`].
    pub fn sweep(&self) -> Swept {
        let days = self
            .privacy()
            .remember_answers_days
            .min(cache::MAX_REMEMBER_DAYS);
        match self.db.sweep(self.clock.now_ms(), days) {
            Ok(swept) => {
                tracing::info!(
                    ai_cache = swept.ai_cache,
                    sent_log = swept.sent_log,
                    "redpen.db swept"
                );
                swept
            }
            Err(_) => {
                tracing::warn!("redpen.db could not be swept");
                Swept::default()
            }
        }
    }

    /// Sweeps every hour, for as long as the app runs. Spawn it once after start-up.
    pub async fn keep_sweeping(&self) {
        loop {
            self.sleeper.sleep(SWEEP_INTERVAL).await;
            self.sweep();
        }
    }

    /// Settings → Privacy → Clear now: empties `ai_cache` and the in-memory answers.
    pub fn cache_clear(&self) -> Result<(), DbError> {
        self.memory.clear();
        self.db.cache_clear()
    }

    /// Settings → Privacy → the sent log's Clear.
    pub fn sent_log_clear(&self) -> Result<(), DbError> {
        self.db.sent_log_clear()
    }

    // The parts of a call.

    /// The provider as it will be called, or why it cannot be.
    async fn prepare(&self, p: &ResolvedProvider) -> Result<ResolvedProvider, AiError> {
        match p.kind {
            ProviderKind::OwnKey => {
                if !has_key(p) || url_host(&p.base_url).is_none() || p.chat_model.trim().is_empty()
                {
                    return Err(AiError::NotConfigured);
                }
                Ok(p.clone())
            }
            ProviderKind::Local => {
                // Before any request, so a local call never reaches another host (invariant 12).
                if !is_local_url(&p.base_url) {
                    return Err(AiError::LocalNotLocal);
                }
                let mut p = p.clone();
                if p.chat_model.trim().is_empty() {
                    p.chat_model = self.detect_local(&p.base_url).await?;
                }
                Ok(p)
            }
            // publik API without a key: this computer is not set up, so the consent view.
            ProviderKind::Publik if !has_key(p) => Err(AiError::NeedsConsent),
            ProviderKind::Publik | ProviderKind::Fake => Ok(p.clone()),
        }
    }

    /// The model a local provider uses: its own, after checking Ollama answers, or the one
    /// Ollama's list gives.
    async fn local_model_for(&self, p: &ResolvedProvider) -> Result<String, AiError> {
        if !is_local_url(&p.base_url) {
            return Err(AiError::LocalNotLocal);
        }
        let tags = self.tags(&p.base_url).await?;
        if p.chat_model.trim().is_empty() {
            pick_local_model(&tags).ok_or(AiError::LocalNoModel)
        } else {
            Ok(p.chat_model.clone())
        }
    }

    /// Ollama's `GET /api/tags` on the local base URL's scheme, host and port.
    async fn tags(&self, base_url: &str) -> Result<Value, AiError> {
        let url = tags_url(base_url).ok_or(AiError::LocalNotLocal)?;
        let p = ResolvedProvider::local(base_url, "");
        let req = HttpRequest::get(url).timeout(LISTING_TIMEOUT);
        let (outcome, _) = self.send(&p, "/api/tags", None, req).await;
        let resp = outcome.map_err(|e| transport_error(&p, e, LISTING_TIMEOUT))?;
        match resp.status {
            200..=299 => resp.json().ok_or(AiError::InvalidAnswer),
            // Something answers, but it is not Ollama.
            404 => Err(AiError::LocalNotRunning),
            code => Err(AiError::ProviderError { code }),
        }
    }

    /// Sends `req`, retrying as the provider's kind allows. With `usage`, every response goes
    /// through the status sink and its charge is added to that feature's day; the charge of the
    /// last response is returned.
    async fn send(
        &self,
        p: &ResolvedProvider,
        endpoint: &'static str,
        usage: Option<(Feature, &str)>,
        req: HttpRequest,
    ) -> (Result<HttpResponse, TransportError>, Option<u64>) {
        let delays = retry_delays(p.kind);
        let mut attempt = 0;
        loop {
            let started = self.clock.now_ms();
            let outcome = self.transport.send(req.clone()).await;
            let ms = self.clock.now_ms() - started;
            let mut charge = None;
            match &outcome {
                Ok(resp) => {
                    tracing::info!(
                        provider = p.kind.as_str(),
                        endpoint,
                        attempt = attempt + 1,
                        status = resp.status,
                        bytes = resp.body.len(),
                        ms,
                        "provider answered"
                    );
                    if let Some((feature, day)) = usage {
                        charge = self
                            .sink
                            .on_response(p.kind, feature, resp.status, &resp.headers);
                        if let Some(micros) = charge.filter(|m| *m > 0) {
                            self.add_usage(p, feature, day, 0, micros);
                        }
                    }
                }
                Err(e) => tracing::warn!(
                    provider = p.kind.as_str(),
                    endpoint,
                    attempt = attempt + 1,
                    error = transport_kind(e),
                    ms,
                    "provider request failed"
                ),
            }
            if attempt < delays.len() && is_retryable(&outcome) {
                self.sleeper.sleep(delays[attempt]).await;
                attempt += 1;
                continue;
            }
            return (outcome, charge);
        }
    }

    fn count_call(&self, p: &ResolvedProvider, feature: Feature, day: &str) {
        self.add_usage(p, feature, day, 1, 0);
    }

    fn add_usage(
        &self,
        p: &ResolvedProvider,
        feature: Feature,
        day: &str,
        calls: u64,
        micros: u64,
    ) {
        if self
            .db
            .usage_add(day, p.kind.as_str(), feature.as_str(), calls, micros)
            .is_err()
        {
            tracing::warn!("usage could not be counted");
        }
    }

    /// One `sent_log` row: the user content exactly as sent (after redaction, which happens
    /// before the call is built) and the size of the request body in bytes.
    fn log_sent(&self, p: &ResolvedProvider, c: &ChatCall, now: i64, bytes: u64) {
        let sent_text = c
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let written = self.db.sent_log_add(
            now,
            p.kind.as_str(),
            CHAT_PATH,
            c.feature.as_str(),
            &p.chat_model,
            &sent_text,
            bytes,
        );
        if written.is_err() {
            tracing::warn!("the sent log could not be written");
        }
    }

    fn cache_lookup(&self, key: &str, placement: Placement, now: i64) -> Option<CachedAnswer> {
        match placement {
            Placement::Nowhere => None,
            Placement::Memory => self.memory.get(key),
            Placement::Disk { days } => {
                let not_before = now - i64::from(days) * DAY_MS;
                match self.db.cache_get(key, not_before) {
                    Ok(Some(row)) => CachedAnswer::from_json(&row.response_json),
                    Ok(None) => None,
                    Err(_) => {
                        tracing::warn!("the answer cache could not be read");
                        None
                    }
                }
                .or_else(|| self.memory.get(key))
            }
        }
    }

    fn cache_store(
        &self,
        key: String,
        p: &ResolvedProvider,
        c: &ChatCall,
        answer: &CachedAnswer,
        charge: Option<u64>,
        placement: Placement,
    ) {
        match placement {
            Placement::Nowhere => {}
            Placement::Memory => self.memory.put(key, answer.clone()),
            Placement::Disk { .. } => {
                let row = CacheRow {
                    key,
                    provider: p.kind.as_str().to_string(),
                    model: p.chat_model.clone(),
                    prompt_id: c.prompt_id.to_string(),
                    prompt_version: c.prompt_version,
                    response_json: answer.to_json(),
                    charge_micros: charge,
                    created_at_ms: self.clock.now_ms(),
                };
                if self.db.cache_put(&row).is_err() {
                    tracing::warn!("the answer could not be cached");
                }
            }
        }
    }
}

/// Why a checked chat call gave no answer: the call failed, or its answer failed the check.
enum Failed<E> {
    Call(AiError),
    Check(E),
}

impl<E> From<AiError> for Failed<E> {
    fn from(e: AiError) -> Self {
        Failed::Call(e)
    }
}

/// Whether the provider has a non-empty key.
fn has_key(p: &ResolvedProvider) -> bool {
    p.key
        .as_ref()
        .is_some_and(|k| !k.expose_secret().trim().is_empty())
}

/// The request with `Authorization: Bearer <key>` (as a secret header value) when the provider
/// has a key, and `Content-Type: application/json` on a body.
fn authorised(p: &ResolvedProvider, req: HttpRequest) -> HttpRequest {
    let mut req = req;
    if let Some(key) = &p.key {
        req = req.header(
            "Authorization",
            HeaderValue::secret(format!("Bearer {}", key.expose_secret())),
        );
    }
    if matches!(req.body, crate::net::Body::Json(_)) {
        req = req.header("Content-Type", HeaderValue::plain("application/json"));
    }
    req
}

/// `{"model", "messages", "max_tokens", "temperature"}`, exactly.
pub fn chat_body(p: &ResolvedProvider, c: &ChatCall) -> Value {
    let messages: Vec<Value> = c
        .messages
        .iter()
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();
    json!({
        "model": p.chat_model,
        "messages": messages,
        "max_tokens": c.max_tokens,
        "temperature": shortest_number(c.temperature),
    })
}

/// An `f32` as JSON in its shortest form: a whole number is an integer (`0`), and `0.4` is
/// `0.4`, not the `f64` widening `0.4000000059604645`, because recordings match on the body.
pub fn shortest_number(x: f32) -> Value {
    if !x.is_finite() {
        return Value::from(0);
    }
    if x.fract() == 0.0 && x.abs() < 1e15 {
        return Value::from(x as i64);
    }
    // `f32`'s `Display` is the shortest decimal that reads back as the same `f32`.
    x.to_string()
        .parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map(Value::Number)
        .unwrap_or(Value::from(0))
}

/// `choices[0].message.content`, if it is a string.
fn answer_content(resp: &HttpResponse) -> Option<String> {
    resp.json()?
        .pointer("/choices/0/message/content")?
        .as_str()
        .map(str::to_string)
}

/// The header that carries a publik API call's idempotency key.
pub const IDEMPOTENCY_HEADER: &str = "x-publik-idempotency-key";

/// The error for a response that is not a 2xx. publik API's errors are read from their body
/// (contract §8); for the others, a 404 names the model only on a chat call.
fn status_error(p: &ResolvedProvider, resp: &HttpResponse, chat: bool) -> AiError {
    if p.kind == ProviderKind::Publik {
        return map_publik_error(resp.status, &resp.body);
    }
    match resp.status {
        401 | 403 => AiError::OwnKeyRefused { code: resp.status },
        404 if chat => AiError::ModelUnknown {
            model: p.chat_model.clone(),
        },
        429 => AiError::rate_limited(),
        code => AiError::ProviderError { code },
    }
}

/// The error for a request that got no response. On a local provider, no connection means
/// nothing is listening at the address.
fn transport_error(p: &ResolvedProvider, e: TransportError, timeout: Duration) -> AiError {
    match e {
        TransportError::Timeout if p.kind == ProviderKind::Publik => AiError::PublikTimeout {
            seconds: timeout.as_secs(),
        },
        TransportError::Timeout => AiError::Timeout {
            who: p.kind,
            seconds: timeout.as_secs(),
        },
        TransportError::Offline | TransportError::Tls(_) if p.kind == ProviderKind::Local => {
            AiError::LocalNotRunning
        }
        TransportError::Offline | TransportError::Tls(_) => AiError::Offline,
        TransportError::Unrecorded { hash, path } => AiError::Unrecorded { hash, path },
    }
}

fn transport_kind(e: &TransportError) -> &'static str {
    match e {
        TransportError::Timeout => "timeout",
        TransportError::Offline => "offline",
        TransportError::Tls(_) => "tls",
        TransportError::Unrecorded { .. } => "unrecorded",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{FakeClock, FakeSleeper};
    use crate::net::{FakeEntry, FakeTransport, request_hash};
    use crate::provider::{CachePolicy, Message, NoopStatusSink};

    const OWN_BASE: &str = "https://api.example.com/v1";
    const NOW: i64 = 1_790_000_000_000;

    struct Harness {
        fake: Arc<FakeTransport>,
        clock: Arc<FakeClock>,
        sleeper: Arc<FakeSleeper>,
        db: Db,
        client: AiClient,
    }

    fn harness(entries: Vec<FakeEntry>) -> Harness {
        harness_with_sink(entries, Arc::new(NoopStatusSink))
    }

    fn harness_with_sink(entries: Vec<FakeEntry>, sink: Arc<dyn StatusSink>) -> Harness {
        let fake = Arc::new(FakeTransport::scripted(entries));
        let clock = Arc::new(FakeClock::at(NOW));
        let sleeper = Arc::new(FakeSleeper::new());
        let db = Db::open_in_memory().unwrap();
        let client = AiClient::new(
            fake.clone(),
            db.clone(),
            clock.clone(),
            sleeper.clone(),
            sink,
        );
        Harness {
            fake,
            clock,
            sleeper,
            db,
            client,
        }
    }

    fn own_key() -> ResolvedProvider {
        ResolvedProvider::own_key(OWN_BASE, "sk-test-a", "example-chat")
    }

    fn call(text: &str) -> ChatCall {
        ChatCall {
            feature: Feature::FixGrammar,
            prompt_id: "fix-grammar",
            prompt_version: 1,
            messages: vec![Message::system("Fix it."), Message::user(text)],
            max_tokens: 256,
            temperature: 0.0,
            timeout: Duration::from_secs(30),
            cache_input: json!({"text": text, "dialect": "british"}),
            cache: CachePolicy::Disk,
            validation_retry: 0,
        }
    }

    fn chat_request(p: &ResolvedProvider, c: &ChatCall) -> HttpRequest {
        HttpRequest::post_json(p.url(CHAT_PATH), chat_body(p, c))
    }

    fn answer(content: &str) -> Value {
        json!({"choices": [{"message": {"role": "assistant", "content": content}}]})
    }

    #[test]
    fn the_body_is_exactly_the_four_fields_in_shortest_form() {
        let mut c = call("hi");
        c.temperature = 0.4;
        let body = chat_body(&ResolvedProvider::fake(), &c);
        assert_eq!(
            canonical_json(&body),
            r#"{"max_tokens":256,"messages":[{"content":"Fix it.","role":"system"},{"content":"hi","role":"user"}],"model":"fake-chat","temperature":0.4}"#
        );
        assert_eq!(shortest_number(0.0), json!(0));
        assert_eq!(serde_json::to_string(&shortest_number(0.0)).unwrap(), "0");
        assert_eq!(serde_json::to_string(&shortest_number(0.4)).unwrap(), "0.4");
        assert_eq!(serde_json::to_string(&shortest_number(0.7)).unwrap(), "0.7");
        assert_eq!(serde_json::to_string(&shortest_number(1.0)).unwrap(), "1");
        assert_eq!(
            serde_json::to_string(&shortest_number(f32::NAN)).unwrap(),
            "0"
        );
    }

    #[test]
    fn the_cli_fake_chat_hashes_as_the_spec_body() {
        // docs/modules/02-provider-layer.md: the fake provider's `redpen chat --message "hi"`.
        let c = ChatCall {
            feature: Feature::Test,
            prompt_id: "chat",
            prompt_version: 1,
            messages: vec![Message::user("hi")],
            max_tokens: 256,
            temperature: 0.0,
            timeout: Duration::from_secs(30),
            cache_input: json!({}),
            cache: CachePolicy::Off,
            validation_retry: 0,
        };
        let req = chat_request(&ResolvedProvider::fake(), &c);
        let spec = HttpRequest::post_json(
            "http://fake.invalid/v1/chat/completions",
            json!({"model": "fake-chat", "messages": [{"role": "user", "content": "hi"}],
                   "max_tokens": 256, "temperature": 0}),
        );
        assert_eq!(request_hash(&req), request_hash(&spec));
    }

    #[tokio::test]
    async fn a_chat_sends_the_key_as_bearer_and_returns_the_content() {
        let p = own_key();
        let c = call("teh cat");
        let h = harness(vec![FakeEntry::reply(
            &chat_request(&p, &c),
            200,
            answer("hello"),
        )]);
        let reply = h.client.chat(&p, c).await.unwrap();
        assert_eq!(
            reply,
            ChatReply {
                content: "hello".into(),
                from_cache: false,
                charge_micros: None,
                request_id: None
            }
        );
        let seen = h.fake.requests_seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].url, "https://api.example.com/v1/chat/completions");
        let auth = seen[0]
            .headers
            .iter()
            .find(|(n, _)| n == "Authorization")
            .unwrap();
        assert!(auth.1.is_secret());
        assert_eq!(auth.1.expose(), "Bearer sk-test-a");
        assert_eq!(seen[0].timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn the_future_is_send() {
        fn assert_send<T: Send>(_: &T) {}
        let h = harness(Vec::new());
        let p = own_key();
        let fut = h.client.chat(&p, call("x"));
        assert_send(&fut);
        let _ = fut.await;
    }

    #[tokio::test]
    async fn caches_follow_the_policy() {
        let p = own_key();
        let c = call("hi");
        let h = harness(vec![FakeEntry::reply(
            &chat_request(&p, &c),
            200,
            answer("hello"),
        )]);
        assert!(!h.client.chat(&p, c.clone()).await.unwrap().from_cache);
        assert!(h.client.chat(&p, c.clone()).await.unwrap().from_cache);
        assert_eq!(h.fake.requests_seen().len(), 1);
        assert_eq!(h.db.cache_len().unwrap(), 1);

        let mut memory = c.clone();
        memory.cache = CachePolicy::Memory;
        memory.cache_input = json!({"text": "other"});
        let h = harness(vec![FakeEntry::reply(
            &chat_request(&p, &memory),
            200,
            answer("m"),
        )]);
        h.client.chat(&p, memory.clone()).await.unwrap();
        assert!(h.client.chat(&p, memory).await.unwrap().from_cache);
        assert_eq!(h.db.cache_len().unwrap(), 0);

        let mut off = c.clone();
        off.cache = CachePolicy::Off;
        let h = harness(vec![FakeEntry::reply(
            &chat_request(&p, &off),
            200,
            answer("o"),
        )]);
        h.client.chat(&p, off.clone()).await.unwrap();
        assert!(!h.client.chat(&p, off).await.unwrap().from_cache);
        assert_eq!(h.fake.requests_seen().len(), 2);
        assert_eq!(h.db.cache_len().unwrap(), 0);

        let h = harness(vec![FakeEntry::reply(
            &chat_request(&p, &c),
            200,
            answer("d"),
        )]);
        h.client.set_remember_answers_days(0);
        h.client.chat(&p, c.clone()).await.unwrap();
        assert!(h.client.chat(&p, c).await.unwrap().from_cache);
        assert_eq!(h.db.cache_len().unwrap(), 0);
    }

    #[tokio::test]
    async fn an_expired_row_is_not_served() {
        let p = own_key();
        let c = call("hi");
        let h = harness(vec![FakeEntry::reply(
            &chat_request(&p, &c),
            200,
            answer("hello"),
        )]);
        h.client.chat(&p, c.clone()).await.unwrap();
        h.clock.set(NOW + 7 * DAY_MS + 1);
        assert!(!h.client.chat(&p, c).await.unwrap().from_cache);
        assert_eq!(h.fake.requests_seen().len(), 2);
    }

    #[tokio::test]
    async fn own_key_retries_timeouts_429_and_5xx_with_the_two_waits() {
        let p = own_key();
        let c = call("hi");
        let req = chat_request(&p, &c);
        let h = harness(vec![
            FakeEntry::failing(&req, TransportError::Timeout),
            FakeEntry::reply(&req, 429, json!({})),
            FakeEntry::reply(&req, 200, answer("hello")),
        ]);
        assert_eq!(h.client.chat(&p, c).await.unwrap().content, "hello");
        assert_eq!(h.fake.requests_seen().len(), 3);
        assert_eq!(
            h.sleeper.sleeps(),
            [Duration::from_millis(500), Duration::from_millis(1000)]
        );

        let c = call("again");
        let req = chat_request(&p, &c);
        let h = harness(vec![FakeEntry::reply(&req, 503, json!({}))]);
        let err = h.client.chat(&p, c).await.unwrap_err();
        assert_eq!(err, AiError::ProviderError { code: 503 });
        assert_eq!(h.fake.requests_seen().len(), 3);
        assert_eq!(h.db.cache_len().unwrap(), 0);
    }

    #[tokio::test]
    async fn other_4xx_are_not_retried() {
        let p = own_key();
        for (code, expected) in [
            (401, AiError::OwnKeyRefused { code: 401 }),
            (403, AiError::OwnKeyRefused { code: 403 }),
            (
                404,
                AiError::ModelUnknown {
                    model: "example-chat".into(),
                },
            ),
            (400, AiError::ProviderError { code: 400 }),
        ] {
            let c = call("hi");
            let h = harness(vec![FakeEntry::reply(
                &chat_request(&p, &c),
                code,
                json!({}),
            )]);
            assert_eq!(h.client.chat(&p, c).await.unwrap_err(), expected);
            assert_eq!(h.fake.requests_seen().len(), 1, "{code}");
            assert!(h.sleeper.sleeps().is_empty());
        }
    }

    #[tokio::test]
    async fn publik_and_fake_never_retry() {
        for kind in [ProviderKind::Publik, ProviderKind::Fake] {
            let p = ResolvedProvider {
                kind,
                base_url: "https://publikhq.com/api/v1".into(),
                key: Some("pk_live_a".into()),
                chat_model: "publik-fast".into(),
            };
            let c = call("hi");
            let h = harness(vec![FakeEntry::failing(
                &chat_request(&p, &c),
                TransportError::Timeout,
            )]);
            let err = h.client.chat(&p, c).await.unwrap_err();
            let expected = match kind {
                ProviderKind::Publik => AiError::PublikTimeout { seconds: 30 },
                _ => AiError::Timeout {
                    who: kind,
                    seconds: 30,
                },
            };
            assert_eq!(err, expected);
            assert_eq!(h.fake.requests_seen().len(), 1);
            assert!(h.sleeper.sleeps().is_empty());
        }
    }

    fn publik(key: Option<&str>) -> ResolvedProvider {
        ResolvedProvider {
            kind: ProviderKind::Publik,
            base_url: "https://publikhq.com/api/v1".into(),
            key: key.map(Into::into),
            chat_model: "publik-fast".into(),
        }
    }

    fn idempotency_header(req: &HttpRequest) -> Option<String> {
        req.headers
            .iter()
            .find(|(n, _)| n == IDEMPOTENCY_HEADER)
            .map(|(_, v)| v.expose().to_string())
    }

    #[tokio::test]
    async fn publik_calls_carry_the_idempotency_key_of_k_and_n() {
        let p = publik(Some("pk_live_a"));
        let c = call("hi");
        let k = AiClient::cache_key(&p, &c);
        let req = chat_request(&p, &c);
        let h = harness(vec![
            FakeEntry::reply(&req, 200, json!({"choices": []})),
            FakeEntry::reply(&req, 200, answer("fixed")),
        ]);
        // A first call carries (K, 0); its unusable answer is not cached.
        assert_eq!(
            h.client.chat(&p, c.clone()).await.unwrap_err(),
            AiError::InvalidAnswer
        );
        // The Try again after it carries (K, 1); the valid answer is stored under K.
        let mut retry = c.clone();
        retry.validation_retry = 1;
        assert_eq!(h.client.chat(&p, retry).await.unwrap().content, "fixed");
        let seen = h.fake.requests_seen();
        assert_eq!(
            idempotency_header(&seen[0]).unwrap(),
            idempotency_key(&k, 0)
        );
        assert_eq!(
            idempotency_header(&seen[1]).unwrap(),
            idempotency_key(&k, 1)
        );
        assert!(h.db.cache_get(&k, 0).unwrap().is_some());
        assert!(h.client.chat(&p, c).await.unwrap().from_cache);
        // The own key carries no publik header.
        let own = own_key();
        let oc = call("own");
        let h = harness(vec![FakeEntry::reply(
            &chat_request(&own, &oc),
            200,
            answer("x"),
        )]);
        h.client.chat(&own, oc).await.unwrap();
        assert_eq!(idempotency_header(&h.fake.requests_seen()[0]), None);
    }

    #[tokio::test]
    async fn a_validation_retry_is_sent_even_when_k_is_cached() {
        let p = publik(Some("pk_live_a"));
        let c = call("hi");
        let req = chat_request(&p, &c);
        let h = harness(vec![
            FakeEntry::reply(&req, 200, answer("not valid for the schema")),
            FakeEntry::reply(&req, 200, answer("valid")),
        ]);
        h.client.chat(&p, c.clone()).await.unwrap();
        let mut retry = c.clone();
        retry.validation_retry = 1;
        let reply = h.client.chat(&p, retry).await.unwrap();
        assert!(!reply.from_cache);
        assert_eq!(reply.content, "valid");
        assert_eq!(h.fake.requests_seen().len(), 2);
        // The valid answer replaced the cached one under K.
        assert_eq!(h.client.chat(&p, c).await.unwrap().content, "valid");
    }

    #[derive(Debug, PartialEq)]
    enum CheckError {
        Ai(AiError),
        Rejected(String),
    }

    impl From<AiError> for CheckError {
        fn from(e: AiError) -> Self {
            CheckError::Ai(e)
        }
    }

    fn only_ok(content: &str) -> Result<usize, CheckError> {
        if content == "ok" {
            Ok(content.len())
        } else {
            Err(CheckError::Rejected(content.to_string()))
        }
    }

    #[tokio::test]
    async fn an_answer_that_fails_its_check_is_not_cached() {
        let p = own_key();
        let c = call("hi");
        let req = chat_request(&p, &c);
        let h = harness(vec![
            FakeEntry::reply(&req, 200, answer("bad")),
            FakeEntry::reply(&req, 200, answer("ok")),
        ]);
        assert_eq!(
            h.client
                .chat_validated(&p, c.clone(), only_ok)
                .await
                .unwrap_err(),
            CheckError::Rejected("bad".into())
        );
        assert_eq!(h.db.cache_len().unwrap(), 0);
        // The next call is sent, and its valid answer is cached and checked again from there.
        let (reply, n) = h
            .client
            .chat_validated(&p, c.clone(), only_ok)
            .await
            .unwrap();
        assert_eq!(
            (reply.content.as_str(), reply.from_cache, n),
            ("ok", false, 2)
        );
        assert_eq!(h.db.cache_len().unwrap(), 1);
        let (reply, _) = h.client.chat_validated(&p, c, only_ok).await.unwrap();
        assert!(reply.from_cache);
        assert_eq!(h.fake.requests_seen().len(), 2);
    }

    #[tokio::test]
    async fn a_cached_answer_that_fails_its_check_is_passed_over() {
        let p = own_key();
        let c = call("hi");
        let req = chat_request(&p, &c);
        let h = harness(vec![
            FakeEntry::reply(&req, 200, answer("old")),
            FakeEntry::reply(&req, 200, answer("ok")),
        ]);
        // `chat` checks nothing, so "old" is cached.
        assert_eq!(h.client.chat(&p, c.clone()).await.unwrap().content, "old");
        let (reply, _) = h.client.chat_validated(&p, c, only_ok).await.unwrap();
        assert_eq!((reply.content.as_str(), reply.from_cache), ("ok", false));
        assert_eq!(h.fake.requests_seen().len(), 2);
    }

    #[tokio::test]
    async fn call_errors_still_come_through_a_check() {
        let p = own_key();
        let c = call("hi");
        let h = harness(vec![FakeEntry::reply(
            &chat_request(&p, &c),
            401,
            json!({}),
        )]);
        assert_eq!(
            h.client.chat_validated(&p, c, only_ok).await.unwrap_err(),
            CheckError::Ai(AiError::OwnKeyRefused { code: 401 })
        );
    }

    struct ErrorSink(Mutex<Vec<AiError>>);
    impl StatusSink for ErrorSink {
        fn on_error(&self, _: ProviderKind, error: &AiError) {
            self.0.lock().unwrap().push(error.clone());
        }
    }

    #[tokio::test]
    async fn publik_errors_are_read_from_their_body_and_reported() {
        let p = publik(Some("pk_live_a"));
        let cases = [
            (
                402,
                json!({"error": {"message": "Link this computer for $0.05 of free use.",
                                 "top_up_url": "https://publikhq.com/claim/x"}}),
                AiError::NeedsCredit {
                    message: "Link this computer for $0.05 of free use.".into(),
                    top_up_url: Some("https://publikhq.com/claim/x".into()),
                },
            ),
            (
                401,
                json!({"error": {"code": "key_revoked"}}),
                AiError::Disconnected,
            ),
            (
                403,
                json!({"error": {"code": "install_revoked"}}),
                AiError::Disconnected,
            ),
            (503, json!({}), AiError::PublikBusy),
            (
                429,
                json!({"error": {"message": "Slow down."}}),
                AiError::PublikRateLimited {
                    message: "Slow down.".into(),
                },
            ),
            (
                500,
                json!({"error": {"message": "The model returned an error."}}),
                AiError::PublikHttp {
                    code: 500,
                    message: "The model returned an error.".into(),
                },
            ),
        ];
        for (status, body, expected) in cases {
            let c = call("hi");
            let fake = Arc::new(FakeTransport::scripted(vec![FakeEntry::reply(
                &chat_request(&p, &c),
                status,
                body,
            )]));
            let sink = Arc::new(ErrorSink(Mutex::new(Vec::new())));
            let db = Db::open_in_memory().unwrap();
            let client = AiClient::new(
                fake.clone(),
                db.clone(),
                Arc::new(FakeClock::at(NOW)),
                Arc::new(FakeSleeper::new()),
                sink.clone(),
            );
            assert_eq!(client.chat(&p, c).await.unwrap_err(), expected);
            assert_eq!(fake.requests_seen().len(), 1, "{status}: one request");
            assert_eq!(db.cache_len().unwrap(), 0, "{status}: nothing cached");
            assert_eq!(*sink.0.lock().unwrap(), vec![expected]);
        }
    }

    #[tokio::test]
    async fn publik_without_a_key_needs_consent_and_sends_nothing() {
        let h = harness(Vec::new());
        assert_eq!(
            h.client.chat(&publik(None), call("hi")).await.unwrap_err(),
            AiError::NeedsConsent
        );
        assert_eq!(
            h.client
                .chat(&publik(Some("  ")), call("hi"))
                .await
                .unwrap_err(),
            AiError::NeedsConsent
        );
        assert!(h.fake.requests_seen().is_empty());
    }

    #[tokio::test]
    async fn bad_answers_are_invalid_and_not_cached() {
        let p = own_key();
        for body in [
            json!("not json"),
            json!({"id": 1}),
            json!({"choices": []}),
            json!({"choices": [{"message": {"content": null}}]}),
        ] {
            let c = call("hi");
            let h = harness(vec![FakeEntry::reply(&chat_request(&p, &c), 200, body)]);
            assert_eq!(
                h.client.chat(&p, c).await.unwrap_err(),
                AiError::InvalidAnswer
            );
            assert_eq!(h.db.cache_len().unwrap(), 0);
        }
    }

    #[tokio::test]
    async fn offline_and_unrecorded_map_to_their_errors() {
        let p = own_key();
        let c = call("hi");
        let h = harness(vec![FakeEntry::failing(
            &chat_request(&p, &c),
            TransportError::Offline,
        )]);
        assert_eq!(h.client.chat(&p, c).await.unwrap_err(), AiError::Offline);
        assert_eq!(h.fake.requests_seen().len(), 1);

        let h = harness(Vec::new());
        let err = h.client.chat(&p, call("unrecorded")).await.unwrap_err();
        match err {
            AiError::Unrecorded { hash, path } => {
                assert_eq!(path, "/chat/completions");
                let _ = std::fs::remove_file(crate::net::fake::unrecorded_path(&hash));
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn an_own_key_without_its_parts_is_not_configured() {
        let h = harness(Vec::new());
        let mut no_key = own_key();
        no_key.key = None;
        let mut no_model = own_key();
        no_model.chat_model = String::new();
        let mut no_url = own_key();
        no_url.base_url = String::new();
        for p in [no_key, no_model, no_url] {
            assert_eq!(
                h.client.chat(&p, call("hi")).await.unwrap_err(),
                AiError::NotConfigured
            );
        }
        assert!(h.fake.requests_seen().is_empty());
    }

    #[tokio::test]
    async fn a_local_provider_must_be_on_this_computer() {
        let h = harness(Vec::new());
        let p = ResolvedProvider::local("http://192.168.1.5:11434/v1", "llama3.2:3b");
        assert_eq!(
            h.client.chat(&p, call("hi")).await.unwrap_err(),
            AiError::LocalNotLocal
        );
        let p = ResolvedProvider::local("http://192.168.1.5:11434/v1", "");
        assert_eq!(
            h.client.chat(&p, call("hi")).await.unwrap_err(),
            AiError::LocalNotLocal
        );
        assert_eq!(
            h.client.detect_local("http://192.168.1.5:11434/v1").await,
            Err(AiError::LocalNotLocal)
        );
        assert!(h.fake.requests_seen().is_empty());
    }

    #[tokio::test]
    async fn a_local_provider_without_a_model_asks_ollama() {
        let tags = HttpRequest::get("http://localhost:11434/api/tags");
        let picked = ResolvedProvider::local("http://localhost:11434/v1", "qwen2.5:14b");
        let mut c = call("hi");
        c.timeout = Duration::from_secs(120);
        let h = harness(vec![
            FakeEntry::reply(
                &tags,
                200,
                json!({"models": [
                    {"name": "llama3.2:3b", "details": {"parameter_size": "3.2B"}},
                    {"name": "qwen2.5:14b", "details": {"parameter_size": "14.8B"}}]}),
            ),
            FakeEntry::reply(&chat_request(&picked, &c), 200, answer("local")),
        ]);
        let p = ResolvedProvider::local("http://localhost:11434/v1", "");
        assert_eq!(h.client.chat(&p, c).await.unwrap().content, "local");
        let urls: Vec<String> = h.fake.requests_seen().into_iter().map(|r| r.url).collect();
        assert_eq!(
            urls,
            [
                "http://localhost:11434/api/tags",
                "http://localhost:11434/v1/chat/completions"
            ]
        );
    }

    #[tokio::test]
    async fn ollama_states_have_their_errors() {
        let tags = HttpRequest::get("http://localhost:11434/api/tags");
        let base = "http://localhost:11434/v1";
        let h = harness(vec![FakeEntry::failing(&tags, TransportError::Offline)]);
        assert_eq!(
            h.client.detect_local(base).await,
            Err(AiError::LocalNotRunning)
        );
        assert_eq!(h.fake.requests_seen().len(), 1);
        let h = harness(vec![FakeEntry::reply(&tags, 200, json!({"models": []}))]);
        assert_eq!(
            h.client.detect_local(base).await,
            Err(AiError::LocalNoModel)
        );
        let p = ResolvedProvider::local(base, "");
        assert_eq!(
            h.client.chat(&p, call("hi")).await.unwrap_err(),
            AiError::LocalNoModel
        );
        let h = harness(vec![FakeEntry::reply(&tags, 404, json!({}))]);
        assert_eq!(
            h.client.detect_local(base).await,
            Err(AiError::LocalNotRunning)
        );
        let local = ResolvedProvider::local(base, "m");
        let c = call("hi");
        let h = harness(vec![FakeEntry::failing(
            &chat_request(&local, &c),
            TransportError::Offline,
        )]);
        assert_eq!(
            h.client.chat(&local, c).await.unwrap_err(),
            AiError::LocalNotRunning
        );
    }

    #[tokio::test]
    async fn list_models_counts_the_data_array() {
        let p = own_key();
        let req = authorised(&p, HttpRequest::get(p.url(MODELS_PATH)));
        let data: Vec<Value> = (0..14).map(|i| json!({"id": format!("m{i}")})).collect();
        let h = harness(vec![FakeEntry::reply(&req, 200, json!({"data": data}))]);
        assert_eq!(h.client.list_models(&p).await, Ok(14));
        assert_eq!(
            h.client.provider_test(&p).await,
            ProviderTest {
                ok: true,
                message: "Key works: 14 models available.".into()
            }
        );
        let h = harness(vec![FakeEntry::reply(
            &req,
            200,
            json!({"data": [{"id": "a"}]}),
        )]);
        assert_eq!(
            h.client.provider_test(&p).await.message,
            "Key works: 1 model available."
        );
        let h = harness(vec![FakeEntry::reply(&req, 401, json!({}))]);
        assert_eq!(
            h.client.provider_test(&p).await,
            ProviderTest {
                ok: false,
                message: "Your key was refused (401). Check it in Settings → Providers.".into()
            }
        );
        let h = harness(vec![FakeEntry::reply(&req, 404, json!({}))]);
        assert_eq!(
            h.client.list_models(&p).await,
            Err(AiError::ProviderError { code: 404 })
        );
        let h = harness(vec![FakeEntry::reply(&req, 200, json!({"data": "x"}))]);
        assert_eq!(h.client.list_models(&p).await, Err(AiError::InvalidAnswer));
    }

    #[tokio::test]
    async fn the_local_test_names_ollamas_model() {
        let tags = HttpRequest::get("http://localhost:11434/api/tags");
        let h = harness(vec![FakeEntry::reply(
            &tags,
            200,
            json!({"models": [{"name": "llama3.2:3b", "details": {"parameter_size": "3.2B"}}]}),
        )]);
        let p = ResolvedProvider::local("http://localhost:11434/v1", "");
        assert_eq!(
            h.client.provider_test(&p).await,
            ProviderTest {
                ok: true,
                message: "Ollama is running. Redpen will use llama3.2:3b.".into()
            }
        );
        assert_eq!(
            h.client
                .local_models("http://localhost:11434/v1")
                .await
                .unwrap(),
            vec![LocalModel {
                name: "llama3.2:3b".into(),
                parameter_size: "3.2B".into()
            }]
        );
    }

    #[tokio::test]
    async fn usage_counts_sent_calls_not_cache_hits() {
        let p = own_key();
        let mut entries = Vec::new();
        let calls: Vec<ChatCall> = ["a", "b", "c"].iter().map(|t| call(t)).collect();
        for c in &calls {
            entries.push(FakeEntry::reply(&chat_request(&p, c), 200, answer("ok")));
        }
        let h = harness(entries);
        for c in &calls {
            h.client.chat(&p, c.clone()).await.unwrap();
        }
        assert!(
            h.client
                .chat(&p, calls[0].clone())
                .await
                .unwrap()
                .from_cache
        );
        let day = local_day(NOW, 0);
        let usage = h.db.usage(&day, "own_key", "fix-grammar").unwrap().unwrap();
        assert_eq!((usage.calls, usage.charge_micros), (3, 0));
    }

    struct ChargeSink;
    impl StatusSink for ChargeSink {
        fn on_response(
            &self,
            _: ProviderKind,
            _: Feature,
            _: u16,
            headers: &[(String, String)],
        ) -> Option<u64> {
            headers
                .iter()
                .find(|(n, _)| n == "x-publik-charge-micros")
                .and_then(|(_, v)| v.parse().ok())
        }
    }

    #[tokio::test]
    async fn the_sinks_charge_adds_up_and_is_returned() {
        let p = ResolvedProvider {
            kind: ProviderKind::Publik,
            base_url: "https://publikhq.com/api/v1".into(),
            key: Some("pk_live_a".into()),
            chat_model: "publik-fast".into(),
        };
        let (a, b) = (call("a"), call("b"));
        let mut ea = FakeEntry::reply(&chat_request(&p, &a), 200, answer("x"));
        ea.headers = vec![
            ("x-publik-charge-micros".into(), "210".into()),
            ("x-publik-request-id".into(), "req_1".into()),
        ];
        let mut eb = FakeEntry::reply(&chat_request(&p, &b), 200, answer("y"));
        eb.headers = vec![("x-publik-charge-micros".into(), "180".into())];
        let h = harness_with_sink(vec![ea, eb], Arc::new(ChargeSink));
        let reply = h.client.chat(&p, a).await.unwrap();
        assert_eq!(reply.charge_micros, Some(210));
        assert_eq!(reply.request_id.as_deref(), Some("req_1"));
        h.client.chat(&p, b).await.unwrap();
        let usage =
            h.db.usage(&local_day(NOW, 0), "publik", "fix-grammar")
                .unwrap()
                .unwrap();
        assert_eq!((usage.calls, usage.charge_micros), (2, 390));
    }

    #[tokio::test]
    async fn the_local_day_uses_the_clocks_offset() {
        let p = own_key();
        let c = call("hi");
        let h = harness(vec![FakeEntry::reply(
            &chat_request(&p, &c),
            200,
            answer("x"),
        )]);
        // 23:30 UTC is the next day at UTC+1.
        let late = 20_769 * DAY_MS - 30 * 60_000;
        h.clock.set(late);
        h.clock.set_utc_offset_minutes(60);
        h.client.chat(&p, c).await.unwrap();
        assert!(
            h.db.usage("2026-11-12", "own_key", "fix-grammar")
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn the_sent_log_records_only_while_on() {
        let p = own_key();
        let (a, b) = (call("first text"), call("second text"));
        let h = harness(vec![
            FakeEntry::reply(&chat_request(&p, &a), 200, answer("x")),
            FakeEntry::reply(&chat_request(&p, &b), 200, answer("y")),
        ]);
        h.client.chat(&p, a).await.unwrap();
        assert!(h.db.sent_log_list().unwrap().is_empty());
        h.client.set_sent_log(true);
        let body_len = canonical_json(&chat_body(&p, &b)).len() as u64;
        h.client.chat(&p, b.clone()).await.unwrap();
        h.client.chat(&p, b).await.unwrap(); // a cache hit sends nothing
        let log = h.db.sent_log_list().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].sent_text, "second text");
        assert_eq!(log[0].endpoint, "/chat/completions");
        assert_eq!(log[0].feature, "fix-grammar");
        assert_eq!(log[0].model, "example-chat");
        assert_eq!(log[0].provider, "own_key");
        assert_eq!(log[0].bytes, body_len);
        h.client.sent_log_clear().unwrap();
        assert!(h.db.sent_log_list().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_start_up_sweep_runs_in_new() {
        let db = Db::open_in_memory().unwrap();
        for (key, age_days) in [("old", 8), ("young", 6)] {
            db.cache_put(&CacheRow {
                key: key.into(),
                provider: "own_key".into(),
                model: "m".into(),
                prompt_id: "fix-grammar".into(),
                prompt_version: 1,
                response_json: r#"{"content":"x"}"#.into(),
                charge_micros: None,
                created_at_ms: NOW - age_days * DAY_MS,
            })
            .unwrap();
        }
        let _client = AiClient::new(
            Arc::new(FakeTransport::scripted(Vec::new())),
            db.clone(),
            Arc::new(FakeClock::at(NOW)),
            Arc::new(FakeSleeper::new()),
            Arc::new(NoopStatusSink),
        );
        assert!(db.cache_get("old", 0).unwrap().is_none());
        assert!(db.cache_get("young", 0).unwrap().is_some());
    }

    /// A sleeper that moves the fake clock by the requested time, and stops the loop (never
    /// returns) after `limit` sleeps.
    struct AdvancingSleeper {
        clock: Arc<FakeClock>,
        count: std::sync::atomic::AtomicUsize,
        limit: usize,
    }

    #[async_trait::async_trait]
    impl Sleeper for AdvancingSleeper {
        async fn sleep(&self, d: Duration) {
            let n = self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n >= self.limit {
                std::future::pending::<()>().await;
            }
            self.clock.advance(d);
        }
    }

    #[tokio::test]
    async fn keep_sweeping_sweeps_every_hour() {
        let clock = Arc::new(FakeClock::at(NOW));
        let db = Db::open_in_memory().unwrap();
        db.sent_log_add(
            NOW - 7 * DAY_MS + 30 * 60_000,
            "local",
            CHAT_PATH,
            "custom",
            "m",
            "t",
            1,
        )
        .unwrap();
        let sleeper = Arc::new(AdvancingSleeper {
            clock: clock.clone(),
            count: Default::default(),
            limit: 1,
        });
        let client = AiClient::new(
            Arc::new(FakeTransport::scripted(Vec::new())),
            db.clone(),
            clock.clone(),
            sleeper,
            Arc::new(NoopStatusSink),
        );
        assert_eq!(db.sent_log_list().unwrap().len(), 1);
        let _ = tokio::time::timeout(Duration::from_millis(50), client.keep_sweeping()).await;
        assert_eq!(clock.now_ms(), NOW + 3_600_000);
        assert!(db.sent_log_list().unwrap().is_empty());
    }

    #[tokio::test]
    async fn clearing_empties_disk_and_memory() {
        let p = own_key();
        let (a, mut b) = (call("a"), call("b"));
        b.cache = CachePolicy::Memory;
        let h = harness(vec![
            FakeEntry::reply(&chat_request(&p, &a), 200, answer("x")),
            FakeEntry::reply(&chat_request(&p, &b), 200, answer("y")),
        ]);
        h.client.chat(&p, a.clone()).await.unwrap();
        h.client.chat(&p, b.clone()).await.unwrap();
        h.client.cache_clear().unwrap();
        assert_eq!(h.db.cache_len().unwrap(), 0);
        assert!(!h.client.chat(&p, a).await.unwrap().from_cache);
        assert!(!h.client.chat(&p, b).await.unwrap().from_cache);
    }

    #[test]
    fn privacy_follows_the_settings() {
        let settings = PrivacySettings {
            remember_answers_days: 0,
            sent_log: true,
            ..Default::default()
        };
        assert_eq!(
            PrivacyOptions::from(&settings),
            PrivacyOptions {
                remember_answers_days: 0,
                sent_log: true
            }
        );
        assert_eq!(
            PrivacyOptions::default(),
            PrivacyOptions {
                remember_answers_days: 7,
                sent_log: false
            }
        );
    }
}
