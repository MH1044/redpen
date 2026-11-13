//! Module 03 tests in the core (docs/modules/03-publik.md, Acceptance tests): the publik API half
//! of the one chat client, the new error variants, the tier rule, the idempotency key, the search
//! types and the copy rule. A test named `cNN_…` covers contract test NN (C1 to C19 in the spec),
//! `tNN_…` Redpen's own bullet NN, and `e_…` an error path or a Scope bullet the lists leave
//! implicit. The app's half (publik.rs, the resolver, the status sink, the commands, the tray,
//! the self-test and the proof) is in app/src-tauri/tests/03_publik.rs.
//!
//! Expected values come from the spec (Interfaces, the `AiError` table, acceptance tests C4 to C17
//! and 10), docs/PUBLIK-CONTRACT.md §6 to §9, docs/PUBLIK-INTEGRATION.md §1, §5 and §9, and
//! SHA-256 (FIPS 180-4) computed with `sha256sum` over the strings in
//! tests/cases/03-publik/REFERENCE.md. The responses are the hand-written files
//! tests/cases/03-publik/*.fake.json, whose invented values are true by construction. Nothing
//! here is taken from Redpen's own output.
//!
//! Transport: `Script` answers each `METHOD path` from its own queue, in order, and keeps every
//! request; a request with no answer left fails as unrecorded, so a retry shows as a failure.
//!
//! The spec's Interfaces name `idempotency_key`, `ProviderResolver`, `ClaimState`,
//! `RewriteQuality`, `TierContext`, `publik_tier`, the `AiError` variants and the search types,
//! but not every module path. Those names are reached through `mod api`, so a different path
//! changes one line there and no expected value.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use redpen_core::clock::{FakeClock, FakeSleeper};
use redpen_core::net::{
    Body, HeaderValue, HttpRequest, HttpResponse, HttpTransport, TransportError, canonical_json,
    request_hash, request_path, url_host,
};
use redpen_core::provider::{
    AiClient, AiError, CachePolicy, ChatCall, Feature, Message, NoopStatusSink, ProviderKind,
    ResolvedProvider, SecretString,
};
use redpen_core::store::Db;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------------------------
// The names of the spec's Interfaces whose module path the spec leaves open.

mod api {
    pub use redpen_core::provider::ProviderResolver;
    pub use redpen_core::provider::idempotency_key;
    pub use redpen_core::provider::search::{SearchError, SearchOutcome, SearchResult};
    pub use redpen_core::provider::tier::{ClaimState, RewriteQuality, TierContext, publik_tier};

    /// A search result as module 09 fakes one: the contract's fields (§6a).
    pub fn search_result(value: serde_json::Value) -> SearchResult {
        serde_json::from_value(value).expect("a search result has url, title, snippet, site, age")
    }

    /// `SearchOutcome {results, charge_micros}`, as a fresh search gives it.
    pub fn search_outcome(results: Vec<SearchResult>, charge_micros: u64) -> SearchOutcome {
        SearchOutcome {
            results,
            charge_micros: Some(charge_micros),
            from_cache: false,
        }
    }

    /// The error a 502 or 504 from `POST /search` gives.
    pub fn provider_failed() -> SearchError {
        SearchError::ProviderFailed
    }
}

/// The fake publik API key of contract test 16.
const KEY: &str = "pk_live_a";
const PUBLIK_BASE: &str = "https://publikhq.com/api/v1";
/// The own key's address and model, from the settings example of docs/ARCHITECTURE.md §4.
const OWN_BASE: &str = "https://api.example.com/v1";
const OWN_KEY: &str = "sk-test-a";
const OWN_MODEL: &str = "example-chat";
const CHAT: &str = "POST /chat/completions";
const SYSTEM: &str = "You correct grammar and spelling.";
const TEXT: &str = "Their going to the libary.";
/// 2026-09-21T14:13:20Z (tests/cases/02-provider-layer/REFERENCE.md).
const T0: i64 = 1_790_000_000_000;
/// The only model values a Redpen chat request may carry (docs/PUBLIK-INTEGRATION.md §1).
const TIERS: [&str; 3] = ["publik-fast", "publik-balanced", "publik-smart"];
/// The contract's request limit (§6), counted strictly in bytes.
const FOUR_MB: usize = 4_000_000;

/// The spec's K (bullet 10, module 00 test 2) and the cache keys of tests/cases/03-publik/REFERENCE.md.
const K_SPEC: &str = "22f53c07ac6b84e42b6fea37d9f1ce05a6c42780307b5932ccf2031f7ef66dd1";
const K_PUBLIK: &str = "748f0ab593f7379e156586aeba37f558d87039ad6da30400ee0c179dba3135b4";
const K_PUBLIK_V2: &str = "58caca32c98b3257e71ff220c47c6df33293b53f682c2b5db297a652e522d820";
const K_OWN: &str = "f17781d369d7294097135b31fafd42208a55e1aff0edafc7c9e4eec21156741c";
/// The idempotency keys of `K_PUBLIK` for n = 0, 1, 2, and of `K_PUBLIK_V2` for n = 0.
const IDEM_0: &str = "748f0ab593f7379e156586aeba37f558";
const IDEM_1: &str = "fc0efd139ba7f503da3f335030060003";
const IDEM_2: &str = "7ba59aa7d4199e866e157ae016c9b78c";
const IDEM_V2: &str = "58caca32c98b3257e71ff220c47c6df3";

// The spec's AiError table and docs/PUBLIK-CONTRACT.md §5.
const MSG_DISCONNECTED: &str = "publik API disconnected this computer.";
const MSG_BUSY: &str = "publik API is busy; try again in a moment";
const MSG_500: &str = "publik API 500: The model returned an error.";
const MSG_TIMEOUT: &str = "publik API did not answer within 30 seconds. Try again.";
const MSG_NO_APP_TOKEN: &str = "This build has no publik API app token. Use your own key for now.";
const MSG_PROVISION_500: &str =
    "publik API could not set up this computer (500). Use your own key for now.";
const MSG_UNREACHABLE: &str = "publik API is unreachable right now. Use your own key for now.";
// Module 02's messages, unchanged for publik API.
const MSG_OFFLINE: &str =
    "No connection. Rewrites need a connection or a local model; checking still works.";
const MSG_INVALID: &str = "The AI answer could not be used.";

// ---------------------------------------------------------------------------------------------
// Fake responses.

/// tests/cases/03-publik, from this crate's folder.
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

/// The response a fake file describes, header names lower-cased as every transport gives them.
fn fake(name: &str) -> Reply {
    let file = fake_file(name);
    let status = file["status"]
        .as_u64()
        .and_then(|s| u16::try_from(s).ok())
        .unwrap_or_else(|| panic!("{name}.fake.json has no status"));
    let headers = file["headers"]
        .as_object()
        .map(|map| {
            map.iter()
                .map(|(k, v)| {
                    let value = v.as_str().unwrap_or_else(|| panic!("{name}: header {k}"));
                    (k.to_ascii_lowercase(), value.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(HttpResponse {
        status,
        headers,
        body: serde_json::to_vec(&file["body"]).expect("a JSON value serialises"),
    })
}

/// `error.message` of a fake error body.
fn error_message(name: &str) -> String {
    fake_file(name)["body"]["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}.fake.json has no error.message"))
        .to_string()
}

/// `error.top_up_url` of a fake 402 body.
fn fake_top_up_url(name: &str) -> String {
    fake_file(name)["body"]["error"]["top_up_url"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}.fake.json has no error.top_up_url"))
        .to_string()
}

/// A 200 answer carrying `content`, with no headers.
fn answer(content: &str) -> Reply {
    let body = json!({"choices": [{"message": {"role": "assistant", "content": content}}]});
    Ok(HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: body.to_string().into_bytes(),
    })
}

// ---------------------------------------------------------------------------------------------
// A scripted transport.

type Reply = Result<HttpResponse, TransportError>;

/// Answers each `METHOD path` (module 00's `request_path`) from its own queue, in order, and
/// keeps every request. A request with no answer left fails as unrecorded.
#[derive(Default)]
struct Script {
    answers: Mutex<HashMap<String, VecDeque<Reply>>>,
    seen: Mutex<Vec<HttpRequest>>,
}

impl Script {
    fn on(self, route: &str, replies: impl IntoIterator<Item = Reply>) -> Self {
        self.answers
            .lock()
            .expect("answers are not poisoned")
            .entry(route.to_string())
            .or_default()
            .extend(replies);
        self
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.seen.lock().expect("log is not poisoned").clone()
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
        let route = format!("{} {path}", req.method);
        let next = self
            .answers
            .lock()
            .expect("answers are not poisoned")
            .get_mut(&route)
            .and_then(VecDeque::pop_front);
        next.unwrap_or_else(|| {
            Err(TransportError::Unrecorded {
                hash: request_hash(&req),
                path,
            })
        })
    }
}

/// A client over a script, an in-memory database, a fake clock at `T0` and a fake sleeper, with
/// the status sink that reads nothing (the app's sink is tested in the app crate).
struct Env {
    client: AiClient,
    db: Db,
    t: Arc<Script>,
    sleeper: Arc<FakeSleeper>,
}

fn env(script: Script) -> Env {
    let t = Arc::new(script);
    let db = Db::open_in_memory().expect("an in-memory redpen.db");
    let sleeper = Arc::new(FakeSleeper::new());
    let client = AiClient::new(
        t.clone(),
        db.clone(),
        Arc::new(FakeClock::at(T0)),
        sleeper.clone(),
        Arc::new(NoopStatusSink),
    );
    Env {
        client,
        db,
        t,
        sleeper,
    }
}

/// A runtime of its own, for tests that wrap a scenario in a log capture.
fn run<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(future)
}

// ---------------------------------------------------------------------------------------------
// Providers, calls and what a request carries.

/// publik API as `publik.rs` resolves it: the gateway, the install's key, a tier as the model.
fn publik() -> ResolvedProvider {
    ResolvedProvider {
        kind: ProviderKind::Publik,
        base_url: PUBLIK_BASE.to_string(),
        key: Some(SecretString::from(KEY.to_string())),
        chat_model: "publik-fast".to_string(),
    }
}

fn own_key() -> ResolvedProvider {
    ResolvedProvider::own_key(OWN_BASE, OWN_KEY.to_string(), OWN_MODEL)
}

/// The Fix grammar call of REFERENCE.md: prompt `fix-grammar` version 1, input text and dialect.
fn call(text: &str) -> ChatCall {
    ChatCall {
        feature: Feature::FixGrammar,
        prompt_id: "fix-grammar",
        prompt_version: 1,
        messages: vec![Message::system(SYSTEM), Message::user(text)],
        max_tokens: 256,
        temperature: 0.0,
        timeout: Duration::from_secs(30),
        cache_input: json!({"text": text, "dialect": "british"}),
        cache: CachePolicy::Disk,
        validation_retry: 0,
    }
}

/// The same call as the n-th Try again after an answer failed validation.
fn try_again(n: u32) -> ChatCall {
    ChatCall {
        validation_retry: n,
        ..call(TEXT)
    }
}

fn header<'a>(req: &'a HttpRequest, name: &str) -> Option<&'a HeaderValue> {
    req.headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

fn idempotency(req: &HttpRequest) -> Option<String> {
    header(req, "x-publik-idempotency-key").map(|v| v.expose().to_string())
}

fn body(req: &HttpRequest) -> Value {
    match &req.body {
        Body::Json(v) => v.clone(),
        Body::Empty => Value::Null,
    }
}

fn is_lower_hex_32(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The key travels only as `Authorization: Bearer <key>`, a secret header value: never in the
/// URL, the body, another header or `x-api-key` (contract §6, test 4).
fn key_only_as_bearer(req: &HttpRequest, key: &str) {
    let auth = header(req, "authorization").expect("an Authorization header");
    assert_eq!(auth.expose(), format!("Bearer {key}"));
    assert!(
        auth.is_secret(),
        "the Authorization header is a secret value"
    );
    assert!(header(req, "x-api-key").is_none(), "no x-api-key header");
    assert!(!req.url.contains(key), "the key is not in the URL");
    assert!(
        !canonical_json(&body(req)).contains(key),
        "the key is not in the body"
    );
    for (name, value) in &req.headers {
        if !name.eq_ignore_ascii_case("authorization") {
            assert!(!value.expose().contains(key), "the key is not in {name}");
        }
    }
}

/// Contract tests 4 and 15 over every request a test made: each body under 4 MB, none with an
/// `image_url` part, and every publik API chat request naming a tier.
fn check_requests(reqs: &[HttpRequest]) {
    for req in reqs {
        let text = canonical_json(&body(req));
        assert!(text.len() < FOUR_MB, "a {} byte request", text.len());
        assert!(!text.contains("image_url"), "Redpen sends no images");
        let publik_chat = url_host(&req.url).as_deref() == Some("publikhq.com")
            && request_path(&req.url) == "/chat/completions";
        if publik_chat {
            let model = body(req)["model"].as_str().unwrap_or_default().to_string();
            assert!(TIERS.contains(&model.as_str()), "model {model:?}");
        }
    }
}

/// Every distinct `http://` or `https://` address in the strings of a JSON value.
fn urls(value: &Value) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    collect_urls(value, &mut found);
    found
}

fn collect_urls(value: &Value, found: &mut BTreeSet<String>) {
    match value {
        Value::String(s) => found.extend(urls_in(s)),
        Value::Array(items) => items.iter().for_each(|v| collect_urls(v, found)),
        Value::Object(map) => map.values().for_each(|v| collect_urls(v, found)),
        _ => {}
    }
}

fn urls_in(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '<' | '>'))
        .filter(|w| w.starts_with("http://") || w.starts_with("https://"))
        .map(|w| w.trim_end_matches(['.', ',', ';', ':', ')']).to_string())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// C4 and bullet 10: the idempotency key.

// Bullet 10 (R7b), first sentence: the spec's three keys, from `sha256sum` of `K:1` and `K:2`;
// and the same rule for the cache key of this file's call (REFERENCE.md).
#[test]
fn t10_r7b_idempotency_keys_match_sha256sum() {
    assert_eq!(
        api::idempotency_key(K_SPEC, 0),
        "22f53c07ac6b84e42b6fea37d9f1ce05"
    );
    assert_eq!(
        api::idempotency_key(K_SPEC, 1),
        "b6058582879f1920e30934ad9402c4db"
    );
    assert_eq!(
        api::idempotency_key(K_SPEC, 2),
        "84491dd9d4f881c8ec68ba4c829a264a"
    );
    assert_eq!(api::idempotency_key(K_PUBLIK, 0), IDEM_0);
    assert_eq!(api::idempotency_key(K_PUBLIK, 1), IDEM_1);
    assert_eq!(api::idempotency_key(K_PUBLIK, 2), IDEM_2);
    assert_eq!(api::idempotency_key(K_PUBLIK_V2, 0), IDEM_V2);
}

// C4: every idempotency key is 32 lowercase hex; n = 0 is K's first 32 characters, and each
// Try again gives a key of its own.
#[test]
fn c04_idempotency_keys_are_32_lowercase_hex() {
    for k in [K_SPEC, K_PUBLIK, K_PUBLIK_V2, K_OWN] {
        let keys: Vec<String> = (0..6).map(|n| api::idempotency_key(k, n)).collect();
        for key in &keys {
            assert!(is_lower_hex_32(key), "{key:?} for {k}");
        }
        assert_eq!(keys[0], k[..32]);
        let distinct: BTreeSet<&String> = keys.iter().collect();
        assert_eq!(distinct.len(), keys.len(), "every retry has its own key");
    }
}

// C4: a publik API chat call carries the key only as a bearer header, a tier as the model and
// `x-publik-idempotency-key` = idempotency_key(K, 0), K being the cache key of REFERENCE.md.
#[tokio::test]
async fn c04_a_publik_call_carries_the_bearer_key_a_tier_and_the_idempotency_key() {
    let e = env(Script::default().on(CHAT, [fake("chat-200-status")]));
    let p = publik();
    let c = call(TEXT);
    assert_eq!(AiClient::cache_key(&p, &c), K_PUBLIK);
    let reply = e.client.chat(&p, c).await.expect("the answer");
    assert_eq!(reply.content, "ok");
    let reqs = e.t.requests();
    assert_eq!(reqs.len(), 1);
    let req = &reqs[0];
    assert_eq!(req.url, "https://publikhq.com/api/v1/chat/completions");
    key_only_as_bearer(req, KEY);
    assert_eq!(idempotency(req).as_deref(), Some(IDEM_0));
    assert_eq!(body(req)["model"], "publik-fast");
    check_requests(&reqs);
}

// C4: the idempotency key is publik API's (docs/PUBLIK-INTEGRATION.md §6); the own key never
// carries it.
#[tokio::test]
async fn c04_own_key_calls_carry_no_idempotency_key() {
    let e = env(Script::default().on(CHAT, [answer("ok")]));
    e.client
        .chat(&own_key(), call(TEXT))
        .await
        .expect("the answer");
    let reqs = e.t.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(idempotency(&reqs[0]), None);
    key_only_as_bearer(&reqs[0], OWN_KEY);
}

// ---------------------------------------------------------------------------------------------
// C6 to C10, C14: publik API's answers and errors through the one client.

// C6: a 402, anonymous and claimed: NeedsCredit with error.message and exactly one link, the
// top_up_url (though the body has others); one request; nothing cached, so the same input is
// sent again.
#[tokio::test]
async fn c06_a_402_is_needs_credit_with_one_link_one_request_and_nothing_cached() {
    for name in ["chat-402-anonymous", "chat-402-claimed"] {
        let e = env(Script::default().on(CHAT, [fake(name), fake(name)]));
        let err = e
            .client
            .chat(&publik(), call(TEXT))
            .await
            .expect_err("a 402 is an error");
        let AiError::NeedsCredit {
            message,
            top_up_url: link,
        } = &err
        else {
            panic!("{name}: expected NeedsCredit, got {err:?}");
        };
        assert_eq!(message, &error_message(name), "{name}");
        assert_eq!(
            link.as_deref(),
            Some(fake_top_up_url(name).as_str()),
            "{name}"
        );
        assert_eq!(err.kind(), "NeedsCredit");
        assert_eq!(
            urls(&err.to_json()),
            BTreeSet::from([fake_top_up_url(name)]),
            "{name}: exactly one link"
        );
        assert!(
            err.to_string().starts_with(&error_message(name)),
            "{name}: {err}"
        );
        assert_eq!(e.t.requests().len(), 1, "{name}: one request");
        assert_eq!(e.db.cache_len().expect("ai_cache"), 0, "{name}");
        let again = e
            .client
            .chat(&publik(), call(TEXT))
            .await
            .expect_err("still a 402");
        assert!(matches!(again, AiError::NeedsCredit { .. }), "{again:?}");
        assert_eq!(e.t.requests().len(), 2, "{name}: nothing was cached");
        check_requests(&e.t.requests());
    }
}

// C8: 401 invalid_api_key, 401 key_revoked and 403 install_revoked are Disconnected, one request
// each.
#[tokio::test]
async fn c08_401_and_403_install_revoked_are_disconnected() {
    for name in [
        "chat-401-invalid-api-key",
        "chat-401-key-revoked",
        "chat-403-install-revoked",
    ] {
        let e = env(Script::default().on(CHAT, [fake(name)]));
        let err = e
            .client
            .chat(&publik(), call(TEXT))
            .await
            .expect_err("an error");
        assert_eq!(err, AiError::Disconnected, "{name}");
        assert_eq!(err.to_string(), MSG_DISCONNECTED);
        assert_eq!(err.kind(), "Disconnected");
        assert_eq!(e.t.requests().len(), 1, "{name}: one request");
    }
}

// C9 (and invariant 9): 429, 503, 500 and another 4xx, one request each, never retried, with
// the spec's messages.
#[tokio::test]
async fn c09_429_503_500_and_400_one_request_each_with_their_messages() {
    let cases = [
        (
            "chat-429",
            AiError::PublikRateLimited {
                message: error_message("chat-429"),
            },
            error_message("chat-429"),
        ),
        ("chat-503", AiError::PublikBusy, MSG_BUSY.to_string()),
        (
            "chat-500",
            AiError::PublikHttp {
                code: 500,
                message: "The model returned an error.".to_string(),
            },
            MSG_500.to_string(),
        ),
        (
            "chat-400",
            AiError::PublikHttp {
                code: 400,
                message: error_message("chat-400"),
            },
            format!("publik API 400: {}", error_message("chat-400")),
        ),
    ];
    for (name, expected, message) in cases {
        let e = env(Script::default().on(CHAT, [fake(name), fake(name), fake(name)]));
        let err = e
            .client
            .chat(&publik(), call(TEXT))
            .await
            .expect_err("an error");
        assert_eq!(err, expected, "{name}");
        assert_eq!(err.to_string(), message, "{name}");
        assert_eq!(e.t.requests().len(), 1, "{name}: one request");
        assert!(e.sleeper.sleeps().is_empty(), "{name}: no retry wait");
        assert_eq!(e.db.cache_len().expect("ai_cache"), 0, "{name}");
    }
}

// C10 (and invariant 9): a publik API timeout is one request and PublikTimeout; an own-key
// timeout is retried twice after 0.5 s and 1 s (module 02 test 5).
#[tokio::test]
async fn c10_a_publik_timeout_is_not_retried_an_own_key_timeout_is() {
    let e = env(Script::default().on(
        CHAT,
        [
            Err(TransportError::Timeout),
            Err(TransportError::Timeout),
            Err(TransportError::Timeout),
        ],
    ));
    let err = e
        .client
        .chat(&publik(), call(TEXT))
        .await
        .expect_err("a timeout");
    assert_eq!(err, AiError::PublikTimeout { seconds: 30 });
    assert_eq!(err.to_string(), MSG_TIMEOUT);
    assert_eq!(e.t.requests().len(), 1, "one request");
    assert!(e.sleeper.sleeps().is_empty());

    let e = env(Script::default().on(
        CHAT,
        [
            Err(TransportError::Timeout),
            Err(TransportError::Timeout),
            Err(TransportError::Timeout),
        ],
    ));
    let err = e
        .client
        .chat(&own_key(), call(TEXT))
        .await
        .expect_err("a timeout");
    assert_eq!(
        err,
        AiError::Timeout {
            who: ProviderKind::OwnKey,
            seconds: 30
        }
    );
    assert_eq!(e.t.requests().len(), 3, "the own key tries three times");
    assert_eq!(
        e.sleeper.sleeps(),
        [Duration::from_millis(500), Duration::from_millis(1_000)]
    );
}

// Error path: no connection on publik API is module 02's Offline, one request.
#[tokio::test]
async fn e_offline_on_publik_is_one_request() {
    let e = env(Script::default().on(
        CHAT,
        [Err(TransportError::Offline), Err(TransportError::Offline)],
    ));
    let err = e
        .client
        .chat(&publik(), call(TEXT))
        .await
        .expect_err("offline");
    assert_eq!(err, AiError::Offline);
    assert_eq!(err.to_string(), MSG_OFFLINE);
    assert_eq!(e.t.requests().len(), 1);
}

// C14: a 200 without `choices` is InvalidAnswer, one request, nothing cached (schema validation
// of the answers themselves is module 04's R5).
#[tokio::test]
async fn c14_a_200_without_choices_is_an_invalid_answer() {
    let e = env(Script::default().on(CHAT, [fake("chat-200-no-choices")]));
    let err = e
        .client
        .chat(&publik(), call(TEXT))
        .await
        .expect_err("unusable");
    assert_eq!(err, AiError::InvalidAnswer);
    assert_eq!(err.to_string(), MSG_INVALID);
    assert_eq!(e.t.requests().len(), 1);
    assert_eq!(e.db.cache_len().expect("ai_cache"), 0);
}

// ---------------------------------------------------------------------------------------------
// C11: caches.

// C11: an identical publik API call is answered from the cache; the own key has a cache of its
// own; a prompt version bump misses. The rows sit under the reference keys.
#[tokio::test]
async fn c11_caches_are_per_provider_identical_input_never_spends_twice_a_version_bump_misses() {
    let e = env(Script::default().on(
        CHAT,
        [
            answer("publik one"),
            answer("own key one"),
            answer("publik two"),
        ],
    ));
    let first = e.client.chat(&publik(), call(TEXT)).await.expect("sent");
    assert_eq!(
        (first.content.as_str(), first.from_cache),
        ("publik one", false)
    );
    let again = e.client.chat(&publik(), call(TEXT)).await.expect("cached");
    assert_eq!(
        (again.content.as_str(), again.from_cache),
        ("publik one", true)
    );
    assert_eq!(
        e.t.requests().len(),
        1,
        "identical input never spends twice"
    );

    let own = e.client.chat(&own_key(), call(TEXT)).await.expect("sent");
    assert_eq!(
        (own.content.as_str(), own.from_cache),
        ("own key one", false)
    );
    assert_eq!(e.t.requests().len(), 2, "the own key has its own cache");

    let bumped = ChatCall {
        prompt_version: 2,
        ..call(TEXT)
    };
    let second = e.client.chat(&publik(), bumped).await.expect("sent");
    assert_eq!(
        (second.content.as_str(), second.from_cache),
        ("publik two", false)
    );
    let reqs = e.t.requests();
    assert_eq!(reqs.len(), 3, "a version bump misses");
    assert_eq!(idempotency(&reqs[0]).as_deref(), Some(IDEM_0));
    assert_eq!(idempotency(&reqs[2]).as_deref(), Some(IDEM_V2));

    for (key, provider, model, version) in [
        (K_PUBLIK, "publik", "publik-fast", 1),
        (K_OWN, "own_key", OWN_MODEL, 1),
        (K_PUBLIK_V2, "publik", "publik-fast", 2),
    ] {
        let row =
            e.db.cache_get(key, 0)
                .expect("ai_cache")
                .unwrap_or_else(|| panic!("no ai_cache row under {key}"));
        assert_eq!(
            (
                row.provider.as_str(),
                row.model.as_str(),
                row.prompt_version
            ),
            (provider, model, version)
        );
    }
    assert_eq!(e.db.cache_len().expect("ai_cache"), 3);
    check_requests(&reqs);
}

// ---------------------------------------------------------------------------------------------
// Bullet 10 (R7b): the idempotency key of each kind of Try again.

// Bullet 10: a first call carries (K, 0); after an invalid answer, Try again carries (K, 1),
// then (K, 2); the valid answer on the third press is cached under K itself.
#[tokio::test]
async fn t10_r7b_try_again_after_invalid_answers_sends_k1_then_k2() {
    let e = env(Script::default().on(
        CHAT,
        [
            fake("chat-200-no-choices"),
            fake("chat-200-no-choices"),
            answer("They're going to the library."),
        ],
    ));
    let first = e.client.chat(&publik(), try_again(0)).await;
    assert_eq!(first.expect_err("invalid"), AiError::InvalidAnswer);
    let second = e.client.chat(&publik(), try_again(1)).await;
    assert_eq!(second.expect_err("invalid"), AiError::InvalidAnswer);
    let third = e.client.chat(&publik(), try_again(2)).await.expect("valid");
    assert!(!third.from_cache);
    let keys: Vec<Option<String>> = e.t.requests().iter().map(idempotency).collect();
    assert_eq!(
        keys,
        [
            Some(IDEM_0.to_string()),
            Some(IDEM_1.to_string()),
            Some(IDEM_2.to_string())
        ]
    );
    let row = e.db.cache_get(K_PUBLIK, 0).expect("ai_cache");
    assert!(row.is_some(), "the valid answer is cached under K");
    let later = e.client.chat(&publik(), call(TEXT)).await.expect("cached");
    assert!(later.from_cache);
    assert_eq!(e.t.requests().len(), 3);
}

// Bullet 10: a valid answer on retry 1 is cached under K, so a later first call is answered
// from the cache.
#[tokio::test]
async fn t10_r7b_a_valid_answer_on_retry_1_is_cached_under_k() {
    let e = env(Script::default().on(
        CHAT,
        [
            fake("chat-200-no-choices"),
            answer("They're going to the library."),
        ],
    ));
    let first = e.client.chat(&publik(), try_again(0)).await;
    assert_eq!(first.expect_err("invalid"), AiError::InvalidAnswer);
    let retried = e.client.chat(&publik(), try_again(1)).await.expect("valid");
    assert_eq!(retried.content, "They're going to the library.");
    let keys: Vec<Option<String>> = e.t.requests().iter().map(idempotency).collect();
    assert_eq!(keys, [Some(IDEM_0.to_string()), Some(IDEM_1.to_string())]);
    let row =
        e.db.cache_get(K_PUBLIK, 0)
            .expect("ai_cache")
            .expect("cached under K");
    assert!(
        row.response_json.contains("library"),
        "{}",
        row.response_json
    );
    let later = e.client.chat(&publik(), call(TEXT)).await.expect("cached");
    assert!(later.from_cache);
    assert_eq!(later.content, "They're going to the library.");
    assert_eq!(e.t.requests().len(), 2);
}

// Bullet 10: after a 503, Try again sends the first key again.
#[tokio::test]
async fn t10_r7b_try_again_after_a_503_resends_the_first_key() {
    let e = env(Script::default().on(CHAT, [fake("chat-503"), answer("ok")]));
    let err = e
        .client
        .chat(&publik(), call(TEXT))
        .await
        .expect_err("busy");
    assert_eq!(err, AiError::PublikBusy);
    e.client
        .chat(&publik(), call(TEXT))
        .await
        .expect("answered");
    let keys: Vec<Option<String>> = e.t.requests().iter().map(idempotency).collect();
    assert_eq!(keys, [Some(IDEM_0.to_string()), Some(IDEM_0.to_string())]);
}

// ---------------------------------------------------------------------------------------------
// C13: the tier rule.

fn ctx(best: bool, claimed: bool) -> api::TierContext {
    api::TierContext {
        quality: if best {
            api::RewriteQuality::Best
        } else {
            api::RewriteQuality::Standard
        },
        claim_state: if claimed {
            api::ClaimState::Claimed
        } else {
            api::ClaimState::Anonymous
        },
    }
}

// C13: the spec's four cases.
#[test]
fn c13_publik_smart_falls_back_to_balanced_while_anonymous() {
    assert_eq!(
        api::publik_tier(Feature::Clearer, &ctx(true, false)),
        "publik-balanced"
    );
    assert_eq!(
        api::publik_tier(Feature::Clearer, &ctx(true, true)),
        "publik-smart"
    );
    assert_eq!(
        api::publik_tier(Feature::FixGrammar, &ctx(true, true)),
        "publik-fast"
    );
    assert_eq!(
        api::publik_tier(Feature::Clearer, &ctx(false, true)),
        "publik-balanced"
    );
}

// C13 (docs/PUBLIK-INTEGRATION.md §1): Fix grammar is always publik-fast; the other five are
// publik-balanced, and publik-smart only when Rewrite quality is Best and the computer is linked.
#[test]
fn c13_the_tier_table_for_every_action() {
    for best in [false, true] {
        for claimed in [false, true] {
            assert_eq!(
                api::publik_tier(Feature::FixGrammar, &ctx(best, claimed)),
                "publik-fast",
                "Fix grammar, best {best}, claimed {claimed}"
            );
            for feature in [
                Feature::Clearer,
                Feature::Shorter,
                Feature::MoreFormal,
                Feature::Friendlier,
                Feature::Custom,
            ] {
                let expected = if best && claimed {
                    "publik-smart"
                } else {
                    "publik-balanced"
                };
                assert_eq!(
                    api::publik_tier(feature, &ctx(best, claimed)),
                    expected,
                    "{feature}, best {best}, claimed {claimed}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// C15 and C16.

// C15 (docs/PUBLIK-INTEGRATION.md §7): the largest request Redpen builds, a 1,500-word selection
// with Custom's longer instructions, stays far under 4 MB and carries no image.
#[tokio::test]
async fn c15_the_largest_request_stays_under_4_mb_and_carries_no_image() {
    let selection = "The quick brown fox jumps over the lazy dog. ".repeat(167);
    assert!(selection.split_whitespace().count() >= 1_500);
    let selection_bytes = selection.len();
    let instruction = "Rewrite this so a ten-year-old can follow it.";
    let custom = ChatCall {
        feature: Feature::Custom,
        prompt_id: "custom",
        prompt_version: 1,
        messages: vec![
            Message::system(SYSTEM.repeat(20)),
            Message::user(format!(
                "<instruction>\n{instruction}\n</instruction>\n\n<text>\n{selection}\n</text>"
            )),
        ],
        max_tokens: 4_096,
        temperature: 0.4,
        timeout: Duration::from_secs(30),
        cache_input: json!({"instruction": instruction, "text": selection}),
        cache: CachePolicy::Disk,
        validation_retry: 0,
    };
    let p = ResolvedProvider {
        chat_model: "publik-balanced".to_string(),
        ..publik()
    };
    let e = env(Script::default().on(CHAT, [answer("{\"text\": \"ok\"}")]));
    e.client.chat(&p, custom).await.expect("answered");
    let reqs = e.t.requests();
    assert_eq!(reqs.len(), 1);
    let bytes = canonical_json(&body(&reqs[0])).len();
    assert!(
        bytes > selection_bytes,
        "the whole selection was sent ({bytes} bytes)"
    );
    check_requests(&reqs);
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

/// Every publik API answer this file uses, one call each, with what each gave: the reply's
/// `Debug`, or the error's `Display`, `Debug` and serialisation.
async fn every_publik_outcome() -> Vec<String> {
    let mut seen = Vec::new();
    let mut replies: Vec<Reply> = [
        "chat-200-status",
        "chat-200-no-choices",
        "chat-402-anonymous",
        "chat-402-claimed",
        "chat-401-invalid-api-key",
        "chat-401-key-revoked",
        "chat-403-install-revoked",
        "chat-429",
        "chat-503",
        "chat-500",
        "chat-400",
    ]
    .into_iter()
    .map(fake)
    .collect();
    replies.push(Err(TransportError::Timeout));
    replies.push(Err(TransportError::Offline));
    for reply in replies {
        let e = env(Script::default().on(CHAT, [reply]));
        match e.client.chat(&publik(), call(TEXT)).await {
            Ok(got) => seen.push(format!("{got:?}")),
            Err(err) => {
                seen.push(err.to_string());
                seen.push(format!("{err:?}"));
                seen.push(err.to_json().to_string());
            }
        }
        for req in e.t.requests() {
            seen.push(format!("{req:?}"));
        }
    }
    seen.push(format!("{:?}", publik()));
    seen
}

// C16 (invariant 6): the key `pk_live_a` never appears in what a capturing subscriber writes over
// every publik API answer above, nor in any reply, error message, error serialisation, request
// `Debug` or resolved provider's `Debug`. The scenario runs once before the capture, so every log
// call site it reaches is registered, and the capture rebuilds their interest: the tests running
// beside this one cannot leave a call site switched off for it.
#[test]
fn c16_the_key_never_appears_in_errors_replies_or_logs() {
    run(every_publik_outcome());
    let log = Captured::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let seen = tracing::subscriber::with_default(subscriber, || {
        tracing::callsite::rebuild_interest_cache();
        run(every_publik_outcome())
    });
    let text = log.text();
    assert!(!text.is_empty(), "the client logged its calls");
    assert!(!text.contains(KEY), "the key is in the log:\n{text}");
    assert!(seen.len() > 20);
    for s in &seen {
        assert!(!s.contains(KEY), "the key is in {s}");
    }
}

// ---------------------------------------------------------------------------------------------
// The error variants and the search types (Scope; the spec's AiError table).

// Error paths: every publik API variant has the spec's message, its own kind, and serialises as
// `{kind, message, …}`.
#[test]
fn e_publik_errors_have_the_spec_messages_and_kinds() {
    let cases = [
        (AiError::Disconnected, "Disconnected", MSG_DISCONNECTED),
        (
            AiError::PublikRateLimited {
                message: "Wait a minute, then try again.".to_string(),
            },
            "PublikRateLimited",
            "Wait a minute, then try again.",
        ),
        (AiError::PublikBusy, "PublikBusy", MSG_BUSY),
        (
            AiError::PublikHttp {
                code: 500,
                message: "The model returned an error.".to_string(),
            },
            "PublikHttp",
            MSG_500,
        ),
        (
            AiError::PublikTimeout { seconds: 30 },
            "PublikTimeout",
            MSG_TIMEOUT,
        ),
        (AiError::NoAppToken, "NoAppToken", MSG_NO_APP_TOKEN),
        (
            AiError::ProvisionFailed {
                message: MSG_PROVISION_500.to_string(),
            },
            "ProvisionFailed",
            MSG_PROVISION_500,
        ),
        (
            AiError::ProvisionFailed {
                message: MSG_UNREACHABLE.to_string(),
            },
            "ProvisionFailed",
            MSG_UNREACHABLE,
        ),
    ];
    for (err, kind, message) in cases {
        assert_eq!(err.kind(), kind, "{err:?}");
        assert_eq!(err.to_string(), message, "{err:?}");
        let json = err.to_json();
        assert_eq!(json["kind"], kind);
        assert_eq!(json["message"], message);
    }
    // NeedsConsent opens the consent view; it has a kind of its own.
    assert_eq!(AiError::NeedsConsent.kind(), "NeedsConsent");
    assert_eq!(AiError::NeedsConsent.to_json()["kind"], "NeedsConsent");
    // NeedsCredit: error.message, and the one link as a field of its own.
    let link = "https://publikhq.com/claim/fake-claim-token";
    let err = AiError::NeedsCredit {
        message: "Link this computer.".to_string(),
        top_up_url: Some(link.to_string()),
    };
    assert_eq!(err.kind(), "NeedsCredit");
    assert!(err.to_string().starts_with("Link this computer."), "{err}");
    let json = err.to_json();
    assert_eq!(json["kind"], "NeedsCredit");
    assert_eq!(json["top_up_url"], link);
    assert_eq!(urls(&json), BTreeSet::from([link.to_string()]));
}

// Scope: SearchResult, SearchOutcome and SearchError live in the core, with the contract's
// fields, so module 09 can fake them; a 502 or 504 says nothing was charged.
#[test]
fn e_search_types_carry_the_contract_fields() {
    let result = api::search_result(json!({
        "url": "https://reefs.example.org/great-barrier-reef",
        "title": "Great Barrier Reef",
        "snippet": "The Great Barrier Reef is the world's largest coral reef system.",
        "site": "reefs.example.org",
        "age": "3 days ago"
    }));
    assert_eq!(result.url, "https://reefs.example.org/great-barrier-reef");
    assert_eq!(result.title, "Great Barrier Reef");
    assert_eq!(result.site, "reefs.example.org");
    assert!(result.snippet.contains("largest coral reef system"));
    let outcome = api::search_outcome(vec![result], 10_000);
    assert_eq!(outcome.results.len(), 1);
    assert_eq!(outcome.charge_micros, Some(10_000));
    let failed = api::provider_failed();
    assert!(
        failed.to_string().contains("nothing was charged"),
        "{failed}"
    );
}

/// A resolver with a fixed answer.
struct Fixed(Option<ResolvedProvider>);

impl api::ProviderResolver for Fixed {
    fn chat(&self) -> Result<ResolvedProvider, AiError> {
        self.0.clone().ok_or(AiError::NeedsConsent)
    }
}

// Scope: `ProviderResolver` is a trait the app implements and the core shares across threads.
#[test]
fn e_a_provider_resolver_is_shared_across_threads() {
    let resolvers: [Arc<dyn api::ProviderResolver>; 2] =
        [Arc::new(Fixed(None)), Arc::new(Fixed(Some(publik())))];
    let [none, some] = resolvers;
    assert_eq!(none.chat().unwrap_err(), AiError::NeedsConsent);
    let kind = std::thread::spawn(move || some.chat().map(|p| p.kind))
        .join()
        .expect("the thread ends");
    assert_eq!(kind, Ok(ProviderKind::Publik));
}

// Files (the spec's list): CONTRIBUTING.md carries the publik API block of
// docs/PUBLIK-INTEGRATION.md §12 line for line, and SECURITY.md names `device_name` with its
// generic labels (A2) and the publik API requests of docs/ARCHITECTURE.md §8.
#[test]
fn e_contributing_and_security_describe_publik_api() {
    let root = repo_root();
    let integration = fs::read_to_string(root.join("docs/PUBLIK-INTEGRATION.md"))
        .expect("docs/PUBLIK-INTEGRATION.md");
    let section = integration
        .split("## 12. CONTRIBUTING.md block")
        .nth(1)
        .expect("docs/PUBLIK-INTEGRATION.md §12");
    let block: Vec<&str> = section
        .lines()
        .skip_while(|l| !l.starts_with("```md"))
        .skip(1)
        .take_while(|l| !l.starts_with("```"))
        .collect();
    assert!(block.len() >= 10, "{block:?}");
    let contributing = fs::read_to_string(root.join("CONTRIBUTING.md")).expect("CONTRIBUTING.md");
    for line in block {
        assert!(
            contributing
                .lines()
                .any(|l| l.trim_end() == line.trim_end()),
            "CONTRIBUTING.md lacks the line: {line}"
        );
    }
    let security = fs::read_to_string(root.join("SECURITY.md")).expect("SECURITY.md");
    for named in [
        "device_name",
        "Windows PC",
        "Linux PC",
        "/installs",
        "/installs/revoke",
        "/wallet",
        "/search",
    ] {
        assert!(
            security.contains(named),
            "SECURITY.md does not name {named}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// C17: the copy rule (docs/PUBLIK-CONTRACT.md §9, docs/TESTING.md §5).

/// The phrases the contract forbids, compared without case at the start of a word.
const COPY_RULE: [&str; 5] = [
    "publik credits",
    "in credits",
    "gemini credit",
    "chatgpt credit",
    "openai api",
];
/// docs/TESTING.md §5's folders, plus the Rust sources whose messages the interface shows (the
/// contract's "UI strings").
const COPY_RULE_ROOTS: [&str; 5] = [
    "app/src",
    "app/src-tauri/src",
    "crates/redpen-core/src",
    "README.md",
    "docs",
];
/// The contract and this file quote the phrases.
const COPY_RULE_EXEMPT: [&str; 2] = [
    "docs/PUBLIK-CONTRACT.md",
    "crates/redpen-core/tests/03_publik.rs",
];
const TEXT_EXTENSIONS: [&str; 12] = [
    "md", "rs", "ts", "tsx", "js", "jsx", "mjs", "css", "html", "json", "txt", "svg",
];
const SKIPPED_DIRS: [&str; 3] = ["node_modules", "target", "dist"];

/// The repository root: the first ancestor of this crate whose Cargo.toml is the workspace's.
fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|t| t.contains("[workspace]")) {
            return dir;
        }
        assert!(dir.pop(), "no workspace Cargo.toml above the crate");
    }
}

/// The text files under the copy rule's roots, relative to `root` with forward slashes: those
/// git tracks or would track, or, outside a repository, every file under the roots.
fn files_to_scan(root: &Path) -> Vec<String> {
    let listed = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .args(COPY_RULE_ROOTS)
        .output()
        .ok()
        .filter(|out| out.status.success());
    let mut files: Vec<String> = match listed {
        Some(out) => String::from_utf8_lossy(&out.stdout)
            .split('\0')
            .filter(|f| !f.is_empty())
            .map(str::to_string)
            .collect(),
        None => {
            let mut found = Vec::new();
            for rel in COPY_RULE_ROOTS {
                walk(root, Path::new(rel), &mut found);
            }
            found
        }
    };
    files.retain(|rel| {
        let text = Path::new(rel)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| TEXT_EXTENSIONS.contains(&e));
        text && root.join(rel).is_file()
    });
    files.sort();
    files.dedup();
    files
}

fn walk(root: &Path, rel: &Path, found: &mut Vec<String>) {
    let path = root.join(rel);
    if path.is_file() {
        found.push(rel.to_string_lossy().replace('\\', "/"));
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !SKIPPED_DIRS.contains(&name.as_str()) {
            walk(root, &rel.join(name), found);
        }
    }
}

/// Whether `lower` (already lower-cased) holds `phrase` at the start of a word.
fn holds_phrase(lower: &str, phrase: &str) -> bool {
    lower.match_indices(phrase).any(|(i, _)| {
        lower[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric())
    })
}

/// `path:line: "phrase"` for every forbidden phrase in the files under `root`.
fn copy_rule_findings(root: &Path) -> Vec<String> {
    let mut findings = Vec::new();
    for rel in files_to_scan(root) {
        if COPY_RULE_EXEMPT.contains(&rel.as_str()) {
            continue;
        }
        let Ok(text) = fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            let lower = line.to_lowercase();
            for phrase in COPY_RULE {
                if holds_phrase(&lower, phrase) {
                    findings.push(format!("{rel}:{}: \"{phrase}\"", n + 1));
                }
            }
        }
    }
    findings
}

fn plant(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("a parent folder")).expect("the folder is created");
    fs::write(path, text).expect("the file is written");
}

// C17: the repository's interface, README and docs talk in dollars.
#[test]
fn c17_the_copy_rule_passes_on_the_tree() {
    let root = repo_root();
    let scanned = files_to_scan(&root);
    assert!(scanned.iter().any(|f| f == "README.md"), "{scanned:?}");
    assert!(
        scanned.iter().any(|f| f.starts_with("docs/")),
        "{scanned:?}"
    );
    assert!(
        scanned.iter().any(|f| f.starts_with("app/src/")),
        "{scanned:?}"
    );
    let findings = copy_rule_findings(&root);
    assert!(
        findings.is_empty(),
        "Talk in dollars (docs/PUBLIK-CONTRACT.md §9); reword:\n{}",
        findings.join("\n")
    );
}

// C17: a planted interface string fails, each forbidden phrase in any case is found, and the
// contract's own quotation is exempt.
#[test]
fn c17_the_copy_rule_fails_on_a_planted_ui_string() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let root = dir.path();
    plant(
        root,
        "app/src/Planted.tsx",
        "export const topUp = \"Buy more publik credits\";\n",
    );
    plant(
        root,
        "app/src/Fine.tsx",
        "export const line = \"$3.12 of publik balance\";\n",
    );
    plant(
        root,
        "docs/PUBLIK-CONTRACT.md",
        "A test fails on \"publik credits\".\n",
    );
    plant(root, "README.md", "Redpen talks in dollars.\n");
    assert_eq!(
        copy_rule_findings(root),
        ["app/src/Planted.tsx:1: \"publik credits\""]
    );

    let lines = [
        "Top up your Publik Credits.",
        "Paid IN CREDITS.",
        "Gemini credit left: 3",
        "Your ChatGPT credits",
        "Uses the OpenAI API.",
    ];
    plant(root, "docs/Planted.md", &lines.join("\n"));
    let findings = copy_rule_findings(root);
    assert_eq!(findings.len(), 1 + COPY_RULE.len(), "{findings:?}");
    for (n, phrase) in COPY_RULE.iter().enumerate() {
        let expected = format!("docs/Planted.md:{}: \"{phrase}\"", n + 1);
        assert!(findings.contains(&expected), "{expected} in {findings:?}");
    }
}
