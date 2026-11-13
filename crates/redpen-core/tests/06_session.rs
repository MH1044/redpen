//! Module 06 tests in the core (docs/modules/06-popup-and-settings-ui.md, Acceptance tests): the
//! session API the shell's commands call, driven end to end with `FakeCapture` and
//! `FakeTransport`, and the popup's placement. A test named `tNN_…` covers bullet NN (R1 and R2 of
//! docs/PUBLIK-INTEGRATION.md §11 are bullets 1 and 2; bullets 3 and 4 have their reducer half in
//! app/src/state/06-popup-and-settings-ui.test.ts and their core half here), `e_…` an error path
//! of the session commands (docs/ARCHITECTURE.md §5), and `s_…` a Scope bullet the numbered list
//! leaves implicit. Invariants 1 and 2 (docs/ARCHITECTURE.md §7) are proved by bullets 1 and 2.
//!
//! Expected values come from the spec, docs/ARCHITECTURE.md §4, §5 and §8,
//! docs/PUBLIK-INTEGRATION.md §4 to §6 and §9, docs/TESTING.md §5, module 01's reference run, module
//! 03's idempotency keys and the two hand-written recordings in
//! tests/cases/06-popup-and-settings-ui, with the arithmetic in that folder's REFERENCE.md. Nothing
//! here is taken from Redpen's own output.
//!
//! The spec's Design notes name `Sessions` in `redpen_core::flow::session`, its calls (`open`,
//! `get`, `close`, `apply_fix`, `rewrite`, `accept`, `copy_result`, `undo`) and the five parts it
//! is written against, but not the calls' signatures. Every call goes through `mod api`, which
//! hands results and errors back as the JSON the interface receives (docs/ARCHITECTURE.md §5:
//! `Session {id, text, findings, dialect, english}`, `RewriteOutcome {result_id, text, …,
//! provider_line, charge_micros?}`, errors `{kind, message, …}`), so a different signature
//! changes one line there and no expected value.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use redpen_core::check::Dialect;
use redpen_core::clock::{FakeClock, FakeSleeper};
use redpen_core::net::{
    Body, FakeEntry, FakeTransport, HttpRequest, Recording, canonical_json, request_hash,
};
use redpen_core::platform::placement::place;
use redpen_core::platform::{CaptureError, FakeCapture, Foreground, Point, Rect, WindowRef};
use redpen_core::provider::{
    AiClient, AiError, FixedResolver, NoopStatusSink, ProviderKind, ResolvedProvider, SecretString,
};
use redpen_core::store::settings::ChatChoice;
use redpen_core::store::{Db, Settings};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------------------------
// The session API. Everything outside this module uses only names the spec or earlier modules
// give.

mod api {
    use std::sync::Arc;

    use redpen_core::clock::{Clock, Sleeper};
    use redpen_core::platform::Capture;
    use redpen_core::provider::{AiClient, ProviderResolver};
    use redpen_core::rewrite::Action;
    use redpen_core::store::Settings;
    use serde::Serialize;
    use serde_json::Value;

    pub use redpen_core::flow::session::Sessions;

    fn json<T: Serialize>(value: &T) -> Value {
        serde_json::to_value(value).expect("a session result serialises")
    }

    /// `Sessions` over the five parts the Design notes name.
    pub fn sessions(
        capture: Arc<dyn Capture>,
        client: Arc<AiClient>,
        resolver: Arc<dyn ProviderResolver>,
        clock: Arc<dyn Clock>,
        sleeper: Arc<dyn Sleeper>,
    ) -> Sessions {
        Sessions::new(capture, client, resolver, clock, sleeper)
    }

    /// The hotkey's path: capture the selection in the window in front and check it with
    /// Harper. The new session as `session_get` returns it, or the error as the popup gets it.
    pub async fn open(s: &Sessions, settings: &Settings) -> Result<Value, Value> {
        s.open(settings)
            .await
            .map(|x| json(&x))
            .map_err(|e| json(&e))
    }

    /// `session_get`.
    pub async fn get(s: &Sessions, session_id: &str) -> Result<Value, Value> {
        s.get(session_id).map(|x| json(&x)).map_err(|e| json(&e))
    }

    /// `close_session`: drops the session and its text.
    pub async fn close(s: &Sessions, session_id: &str) {
        s.close(session_id);
    }

    /// `apply_fix`: the session after the fix, checked again.
    pub async fn apply_fix(
        s: &Sessions,
        settings: &Settings,
        session_id: &str,
        finding_id: &str,
        suggestion_index: usize,
    ) -> Result<Value, Value> {
        s.apply_fix(session_id, finding_id, suggestion_index, settings)
            .map(|x| json(&x))
            .map_err(|e| json(&e))
    }

    /// `rewrite` with one of the six actions by its id (`fix-grammar` … `custom`).
    pub async fn rewrite(
        s: &Sessions,
        settings: &Settings,
        session_id: &str,
        action: &str,
        instruction: Option<&str>,
        variant: u32,
    ) -> Result<Value, Value> {
        let action = Action::parse(action).expect("one of the six actions");
        s.rewrite(
            session_id,
            action,
            instruction.map(str::to_string),
            variant,
            settings,
        )
        .await
        .map(|x| json(&x))
        .map_err(|e| json(&e))
    }

    /// `accept`: types the result (or, without `result_id`, the session's text) into the window
    /// the selection came from.
    pub async fn accept(
        s: &Sessions,
        settings: &Settings,
        session_id: &str,
        result_id: Option<&str>,
    ) -> Result<(), Value> {
        s.accept(session_id, result_id, settings)
            .await
            .map_err(|e| json(&e))
    }

    /// `copy_result`: puts the result (or, without `result_id`, the session's text) on the
    /// clipboard.
    pub async fn copy_result(
        s: &Sessions,
        session_id: &str,
        result_id: Option<&str>,
    ) -> Result<(), Value> {
        s.copy_result(session_id, result_id)
            .await
            .map_err(|e| json(&e))
    }

    /// `undo_last_replace`: `"undone"` or `"on_clipboard"`.
    pub async fn undo(s: &Sessions) -> Result<Value, Value> {
        s.undo().await.map(|x| json(&x)).map_err(|e| json(&e))
    }
}

// ---------------------------------------------------------------------------------------------
// Fixtures.

/// 2026-09-21T14:13:20Z, as in modules 02 to 05.
const T0: i64 = 1_790_000_000_000;
/// A fake publik API key, short enough for the hygiene test's secret shapes.
const KEY: &str = "pk_live_r2fake";
const PUBLIK_BASE: &str = "https://publikhq.com/api/v1";
/// The own key and local model of earlier modules (docs/ARCHITECTURE.md §4, module 02).
const OWN_BASE: &str = "https://api.example.com/v1";
const OWN_KEY: &str = "sk-test-six";
const OWN_MODEL: &str = "example-chat";
const LOCAL_BASE: &str = "http://localhost:11434/v1";
const LOCAL_MODEL: &str = "qwen2.5:14b";
const IDEMPOTENCY: &str = "x-publik-idempotency-key";

/// The app the selection comes from: a process name no log may carry.
const PROCESS: &str = "quillnote-fake.exe";
const WINDOW: WindowRef = WindowRef {
    id: 0x5106,
    pid: 6106,
};
/// What the clipboard held before the capture: it never leaves the computer.
const CLIPBOARD: &str = "clipboard text from before the capture";

/// R1: corpus sentence c01 (tests/cases/01-harper-check), American English.
const C01: &str = "I went to teh store yesterday.";
const C01_FIXED: &str = "I went to the store yesterday.";
const C01_FINDING: &str = "10-13-spelling-57a49948";
const C01_MESSAGE: &str = "Did you mean the definite article?";

/// R2 (bullet 2), British English, with its redaction, the recorded answer and the result.
const R2_TEXT: &str = "Please mail ana@example.com today about the invoice.";
const R2_SENT: &str = "Please mail [EMAIL_1] today about the invoice.";
const R2_ANSWER: &str = "Please email [EMAIL_1] about the invoice today.";
const R2_RESULT: &str = "Please email ana@example.com about the invoice today.";
const EMAIL: &str = "ana@example.com";

/// Try again after an invalid answer (tests/cases/03-publik/REFERENCE.md).
const LIBARY: &str = "Their going to the libary.";
const LIBRARY: &str = "They're going to the library.";
const IDEM_0: &str = "748f0ab593f7379e156586aeba37f558";
const IDEM_1: &str = "fc0efd139ba7f503da3f335030060003";

// Messages (docs/ARCHITECTURE.md §8, modules 01, 03, 04 and 05).
const MSG_INVALID: &str = "The AI answer could not be used.";
const MSG_NEEDS_CONSENT: &str = "Choose how Redpen uses AI.";
const MSG_PASSWORD: &str = "Redpen doesn't read or type into password fields.";
const MSG_NOTHING_COPIED: &str = "Nothing was copied. Select some text first.";
const MSG_WINDOW_GONE: &str = "The window you started in has closed. Copy the text instead.";
const MSG_CLIPBOARD_BUSY: &str = "Another app is using the clipboard. Try again.";
const MSG_NOTHING_TO_UNDO: &str = "There is nothing to undo.";
const MSG_STALE: &str = "That suggestion no longer fits the text. Check again.";
const MSG_NO_SUGGESTION: &str = "That suggestion is not available any more.";

/// The repository root: the first ancestor of this crate that holds the workspace Cargo.toml.
fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if let Ok(text) = fs::read_to_string(dir.join("Cargo.toml"))
            && text.contains("[workspace]")
        {
            return dir;
        }
        assert!(dir.pop(), "no workspace Cargo.toml above the crate");
    }
}

fn recording_path(case: &str) -> PathBuf {
    repo_root()
        .join("tests")
        .join("cases")
        .join("06-popup-and-settings-ui")
        .join(case)
        .join("recording.json")
}

fn recording(case: &str) -> Recording {
    let path = recording_path(case);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} could not be read: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not a recording: {e}", path.display()))
}

fn recorded(case: &str) -> FakeTransport {
    FakeTransport::from_file(&recording_path(case)).expect("the recording loads")
}

/// No entries: any request fails the test as unrecorded (invariant 11).
fn nothing_recorded() -> FakeTransport {
    FakeTransport::scripted(Vec::new())
}

/// A chat answer whose content is `{"text": text}`.
fn answer_body(text: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": json!({"text": text}).to_string()}}]})
}

/// The `r2-clearer` request with another model, for the own key and a local model.
fn r2_request_for(model: &str) -> Value {
    let mut body = recording("r2-clearer").entries[0].request.clone();
    body["model"] = Value::from(model);
    body
}

/// publik API as the app resolves it: the gateway, the install's key, the default tier.
fn publik() -> ResolvedProvider {
    ResolvedProvider {
        kind: ProviderKind::Publik,
        base_url: PUBLIK_BASE.to_string(),
        key: Some(SecretString::from(KEY.to_string())),
        chat_model: "publik-balanced".to_string(),
    }
}

fn own_key() -> ResolvedProvider {
    ResolvedProvider::own_key(OWN_BASE, OWN_KEY.to_string(), OWN_MODEL)
}

fn local() -> ResolvedProvider {
    ResolvedProvider::local(LOCAL_BASE, LOCAL_MODEL)
}

/// The window in front: an ordinary text field in `PROCESS`, with a caret.
fn front() -> Foreground {
    Foreground {
        window: WINDOW,
        process: PROCESS.to_string(),
        caret: Some(Rect {
            x: 640,
            y: 480,
            width: 2,
            height: 18,
        }),
        cursor: Point { x: 700, y: 500 },
        ..Foreground::default()
    }
}

/// A desk with `front()` in front, `selection` selected and `CLIPBOARD` on the clipboard.
fn desk(selection: &str) -> FakeCapture {
    FakeCapture::with_foreground(front())
        .with_selection(selection)
        .with_clipboard(CLIPBOARD)
}

/// Everything a session needs, with the fakes kept for the assertions.
struct Env {
    sessions: api::Sessions,
    settings: Settings,
    desk: Arc<FakeCapture>,
    net: Arc<FakeTransport>,
}

fn env_on(
    desk: FakeCapture,
    net: FakeTransport,
    provider: Result<ResolvedProvider, AiError>,
    dialect: Dialect,
) -> Env {
    let desk = Arc::new(desk);
    let net = Arc::new(net);
    let clock = Arc::new(FakeClock::at(T0));
    let sleeper = Arc::new(FakeSleeper::new());
    let client = Arc::new(AiClient::new(
        net.clone(),
        Db::open_in_memory().expect("an in-memory redpen.db"),
        clock.clone(),
        sleeper.clone(),
        Arc::new(NoopStatusSink),
    ));
    // Settings → Providers names the provider the resolver gives, so a session that reads the
    // choice from Settings and one that asks the resolver see the same thing; with no provider
    // the choice is unset, as on a fresh install.
    let chat = match &provider {
        Ok(p) => Some(match p.kind {
            ProviderKind::Publik => ChatChoice::Publik,
            ProviderKind::OwnKey => ChatChoice::OwnKey,
            ProviderKind::Local => ChatChoice::Local,
            other => panic!("no Settings choice for {other:?}"),
        }),
        Err(_) => None,
    };
    let resolver = Arc::new(match provider {
        Ok(p) => FixedResolver::new(p),
        Err(e) => FixedResolver::failing(e),
    });
    let mut settings = Settings::default();
    settings.check.dialect = dialect;
    settings.providers.chat = chat;
    let sessions = api::sessions(desk.clone(), client, resolver, clock, sleeper);
    Env {
        sessions,
        settings,
        desk,
        net,
    }
}

fn env(
    selection: &str,
    net: FakeTransport,
    provider: Result<ResolvedProvider, AiError>,
    dialect: Dialect,
) -> Env {
    env_on(desk(selection), net, provider, dialect)
}

impl Env {
    /// The hotkey's capture and check; panics on a refusal.
    async fn open(&self) -> Value {
        api::open(&self.sessions, &self.settings)
            .await
            .unwrap_or_else(|e| panic!("the capture was refused: {e}"))
    }

    async fn rewrite(&self, id: &str, action: &str, variant: u32) -> Result<Value, Value> {
        api::rewrite(&self.sessions, &self.settings, id, action, None, variant).await
    }

    async fn accept(&self, id: &str, result_id: Option<&str>) -> Result<(), Value> {
        api::accept(&self.sessions, &self.settings, id, result_id).await
    }

    /// The texts `paste` typed, each into `WINDOW`.
    fn typed(&self) -> Vec<String> {
        self.desk
            .typed()
            .into_iter()
            .map(|(window, text)| {
                assert_eq!(window, WINDOW, "Accept typed into another window");
                text
            })
            .collect()
    }

    /// Every request that would have left the computer.
    fn sent(&self) -> Vec<HttpRequest> {
        self.net.requests_seen()
    }
}

fn id_of(session: &Value) -> String {
    session["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a session has a string id: {session}"))
        .to_string()
}

fn result_id_of(outcome: &Value) -> Option<String> {
    outcome["result_id"].as_str().map(str::to_string)
}

fn finding_ids(session: &Value) -> Vec<String> {
    session["findings"]
        .as_array()
        .unwrap_or_else(|| panic!("a session has a findings list: {session}"))
        .iter()
        .map(|f| f["id"].as_str().expect("a finding has an id").to_string())
        .collect()
}

fn json_body(req: &HttpRequest) -> Value {
    match &req.body {
        Body::Json(v) => v.clone(),
        Body::Empty => panic!("the request to {} has no body", req.url),
    }
}

fn header(req: &HttpRequest, name: &str) -> Option<String> {
    req.headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.expose().to_string())
}

/// The error's kind and message, for comparing with the spec's.
fn kind_and_message(e: &Value) -> (String, String) {
    (
        e["kind"].as_str().unwrap_or_default().to_string(),
        e["message"].as_str().unwrap_or_default().to_string(),
    )
}

fn is(kind: &str, message: &str) -> (String, String) {
    (kind.to_string(), message.to_string())
}

// ---------------------------------------------------------------------------------------------
// Bullets 1 and 2: R1 and R2.

// Bullet 1 (R1, invariant 1): capture, check, apply one fix, Accept → `requests_seen()` is empty
// and `paste` received the fixed text.
#[tokio::test]
async fn t01_r1_check_fix_and_accept_send_nothing() {
    let e = env(C01, nothing_recorded(), Ok(publik()), Dialect::American);

    let session = e.open().await;
    assert_eq!(session["text"], C01);
    assert_eq!(session["dialect"], "american");
    assert_eq!(session["english"], true);
    assert_eq!(finding_ids(&session), [C01_FINDING]);
    let finding = &session["findings"][0];
    assert_eq!(finding["category"], "spelling");
    assert_eq!(finding["message"], C01_MESSAGE);
    assert_eq!(
        finding["suggestions"],
        json!([{"kind": "replace", "text": "the"}])
    );
    let id = id_of(&session);

    let fixed = api::apply_fix(&e.sessions, &e.settings, &id, C01_FINDING, 0)
        .await
        .unwrap_or_else(|err| panic!("the fix was refused: {err}"));
    assert_eq!(fixed["text"], C01_FIXED);
    assert_eq!(id_of(&fixed), id, "a fix keeps the session");
    assert!(
        !finding_ids(&fixed).iter().any(|f| f == C01_FINDING),
        "the fixed text was checked again: {fixed}"
    );
    let kept = api::get(&e.sessions, &id)
        .await
        .expect("the session is open");
    assert_eq!(kept["text"], C01_FIXED);

    e.accept(&id, None)
        .await
        .unwrap_or_else(|err| panic!("Accept was refused: {err}"));
    assert_eq!(e.typed(), [C01_FIXED]);
    assert!(
        e.sent().is_empty(),
        "a check-only session sent {} request(s)",
        e.sent().len()
    );
}

// Bullet 2 (R2, invariant 2): Clearer on `Please mail ana@example.com today about the invoice.`,
// then Accept → exactly one request, its body with `model`, `messages`, `max_tokens` and
// `temperature` only, the user message holding `[EMAIL_1]` and not the address; `paste` received
// the restored text.
#[tokio::test]
async fn t02_r2_a_rewrite_sends_only_the_redacted_text_and_its_prompt() {
    let rec = recording("r2-clearer");
    let e = env(
        R2_TEXT,
        recorded("r2-clearer"),
        Ok(publik()),
        Dialect::British,
    );
    let id = id_of(&e.open().await);

    let outcome = e
        .rewrite(&id, "clearer", 0)
        .await
        .unwrap_or_else(|err| panic!("Clearer failed: {err}"));
    assert_eq!(outcome["text"], R2_RESULT);
    e.accept(&id, result_id_of(&outcome).as_deref())
        .await
        .unwrap_or_else(|err| panic!("Accept was refused: {err}"));

    let sent = e.sent();
    assert_eq!(
        sent.len(),
        1,
        "Clearer and Accept sent {} requests",
        sent.len()
    );
    let req = &sent[0];
    assert_eq!(req.method.as_str(), "POST");
    assert_eq!(req.url, format!("{PUBLIK_BASE}/chat/completions"));

    let body = json_body(req);
    let mut keys: Vec<&str> = body
        .as_object()
        .expect("the body is a JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["max_tokens", "messages", "model", "temperature"]);

    let messages = body["messages"].as_array().expect("messages is a list");
    let user = messages
        .iter()
        .find(|m| m["role"] == "user")
        .expect("a user message");
    let content = user["content"].as_str().expect("the user content is text");
    assert!(content.contains("[EMAIL_1]"), "{content}");
    assert!(!content.contains(EMAIL), "{content}");
    assert!(content.contains(R2_SENT), "{content}");

    // Exactly the §6.2 request, and nothing the data-path sentence leaves out: not the address,
    // the window's process, the clipboard or the key.
    assert_eq!(body, rec.entries[0].request);
    assert_eq!(request_hash(req), rec.entries[0].request_sha256);
    let whole = format!("{} {}", req.url, canonical_json(&body));
    for never in [EMAIL, PROCESS, CLIPBOARD, KEY] {
        assert!(!whole.contains(never), "the request carries {never:?}");
    }
    for (name, value) in &req.headers {
        for never in [EMAIL, PROCESS, CLIPBOARD] {
            assert!(
                !value.expose().contains(never),
                "the header {name} carries {never:?}"
            );
        }
    }

    assert_eq!(e.typed(), [R2_RESULT]);
}

// Both recordings answer the request they hold: each entry's hash is module 00's hash of its
// `request` field sent to the chat path.
#[test]
fn s_the_recordings_hash_their_requests() {
    for case in ["r2-clearer", "t03-try-again"] {
        let rec = recording(case);
        assert_eq!(rec.format, "redpen-recording/1");
        assert!(!rec.entries.is_empty(), "{case} has no entry");
        for entry in &rec.entries {
            let req = HttpRequest::post_json(
                format!("{PUBLIK_BASE}/chat/completions"),
                entry.request.clone(),
            );
            assert_eq!(request_hash(&req), entry.request_sha256, "{case}");
            assert_eq!(entry.source, "hand-written", "{case}");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Bullets 3 and 4, the core half.

// Bullet 3: Try again after an `InvalidAnswer` repeats the call with the same arguments, and the
// session sets the validation retry: the second request carries the idempotency key of
// SHA-256(`<K>:1`), not the first call's.
#[tokio::test]
async fn t03_try_again_after_an_invalid_answer_is_a_new_request() {
    let e = env(
        LIBARY,
        recorded("t03-try-again"),
        Ok(publik()),
        Dialect::British,
    );
    let id = id_of(&e.open().await);

    let err = e
        .rewrite(&id, "fix-grammar", 0)
        .await
        .expect_err("the first answer is not JSON");
    assert_eq!(kind_and_message(&err), is("InvalidAnswer", MSG_INVALID));

    let outcome = e
        .rewrite(&id, "fix-grammar", 0)
        .await
        .unwrap_or_else(|err| panic!("Try again failed: {err}"));
    assert_eq!(outcome["text"], LIBRARY);

    let keys: Vec<Option<String>> = e.sent().iter().map(|r| header(r, IDEMPOTENCY)).collect();
    assert_eq!(
        keys,
        [Some(IDEM_0.to_string()), Some(IDEM_1.to_string())],
        "the first call, then the first Try again after an invalid answer"
    );
    assert!(e.typed().is_empty(), "nothing is typed before Accept");
}

// Bullet 3: Another version (`variant` 1) is a new request for the same body, while repeating
// variant 0 is answered from the cache and sends nothing (docs/PUBLIK-INTEGRATION.md §5).
#[tokio::test]
async fn t03_another_version_is_a_new_request_and_a_repeat_is_not() {
    let e = env(
        R2_TEXT,
        recorded("r2-clearer"),
        Ok(publik()),
        Dialect::British,
    );
    let id = id_of(&e.open().await);

    let first = e.rewrite(&id, "clearer", 0).await.expect("Clearer");
    assert_eq!(first["text"], R2_RESULT);
    let again = e.rewrite(&id, "clearer", 0).await.expect("Clearer again");
    assert_eq!(again["text"], R2_RESULT);
    assert_eq!(e.sent().len(), 1, "the same input never spends twice");

    let other = e.rewrite(&id, "clearer", 1).await.expect("Another version");
    assert_eq!(other["text"], R2_RESULT);
    let sent = e.sent();
    assert_eq!(sent.len(), 2, "Another version is a new request");
    assert_eq!(json_body(&sent[0]), json_body(&sent[1]));
    assert_ne!(
        header(&sent[0], IDEMPOTENCY),
        header(&sent[1], IDEMPOTENCY),
        "the variant is part of the cache key, so of the idempotency key"
    );
}

// Bullet 4 and R12: with no provider set up the rewrite stops with `NeedsConsent`, which the
// popup turns into the consent view; nothing is sent and the findings stay usable.
#[tokio::test]
async fn t04_needs_consent_sends_nothing_and_keeps_the_findings() {
    let e = env(
        C01,
        nothing_recorded(),
        Err(AiError::NeedsConsent),
        Dialect::American,
    );
    let id = id_of(&e.open().await);

    let err = e
        .rewrite(&id, "clearer", 0)
        .await
        .expect_err("no provider is set up");
    assert_eq!(
        kind_and_message(&err),
        is("NeedsConsent", MSG_NEEDS_CONSENT)
    );
    assert!(e.sent().is_empty(), "declining sends nothing");

    let session = api::get(&e.sessions, &id)
        .await
        .expect("the session stays open");
    assert_eq!(finding_ids(&session), [C01_FINDING]);
    let fixed = api::apply_fix(&e.sessions, &e.settings, &id, C01_FINDING, 0)
        .await
        .expect("the findings are still usable");
    assert_eq!(fixed["text"], C01_FIXED);
}

// ---------------------------------------------------------------------------------------------
// Bullet 10: popup placement.

const POPUP: (u32, u32) = (440, 320);

fn area(x: i32, y: i32, width: i32, height: i32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn caret(x: i32, y: i32) -> Option<Rect> {
    Some(Rect {
        x,
        y,
        width: 2,
        height: 20,
    })
}

fn inside(p: Point, a: Rect) -> bool {
    let (w, h) = (POPUP.0 as i32, POPUP.1 as i32);
    p.x >= a.x && p.y >= a.y && p.x + w <= a.x + a.width && p.y + h <= a.y + a.height
}

/// A 1920 × 1080 monitor less a 40 px taskbar.
const MAIN: Rect = Rect {
    x: 0,
    y: 0,
    width: 1920,
    height: 1040,
};
const SECOND: Rect = Rect {
    x: 1920,
    y: 0,
    width: 2560,
    height: 1400,
};

// Bullet 10 (Design notes): with room below, the popup goes 8 px below the caret rectangle.
#[test]
fn t10_the_popup_goes_below_the_caret() {
    let p = place(caret(600, 400), Point { x: 1200, y: 300 }, &[MAIN], POPUP);
    assert_eq!(p, Point { x: 600, y: 428 });
}

// Bullet 10: a caret at the bottom edge of a 1920 × 1040 work area puts the popup above it.
#[test]
fn t10_a_caret_at_the_bottom_edge_flips_the_popup_above() {
    let c = caret(600, 1016);
    let p = place(c, Point { x: 1200, y: 300 }, &[MAIN], POPUP);
    assert!(
        p.y + POPUP.1 as i32 <= 1016,
        "the popup must end above the caret: {p:?}"
    );
    assert!(inside(p, MAIN), "{p:?}");
    assert_eq!(p, Point { x: 600, y: 688 });
}

// Bullet 10: with no caret the popup goes at the mouse cursor.
#[test]
fn t10_without_a_caret_the_popup_goes_at_the_cursor() {
    let p = place(None, Point { x: 1200, y: 300 }, &[MAIN], POPUP);
    assert_eq!(p, Point { x: 1200, y: 300 });
}

// Bullet 10: a caret on the second monitor keeps the popup inside that monitor, whatever the
// cursor, clamped at its edges.
#[test]
fn t10_a_caret_on_the_second_monitor_keeps_the_popup_there() {
    let p = place(
        caret(2500, 400),
        Point { x: 100, y: 100 },
        &[MAIN, SECOND],
        POPUP,
    );
    assert!(inside(p, SECOND), "{p:?}");
    assert_eq!(p, Point { x: 2500, y: 428 });

    let p = place(
        caret(4400, 1370),
        Point { x: 100, y: 100 },
        &[MAIN, SECOND],
        POPUP,
    );
    assert!(inside(p, SECOND), "{p:?}");
    assert_eq!(p, Point { x: 4040, y: 1042 });
}

// Bullet 10: a monitor left of the main one has negative coordinates; the popup is clamped into it.
#[test]
fn t10_a_monitor_at_negative_coordinates() {
    let left = area(-1920, 0, 1920, 1040);
    let p = place(
        caret(-100, 300),
        Point { x: 100, y: 100 },
        &[left, MAIN],
        POPUP,
    );
    assert!(inside(p, left), "{p:?}");
    assert_eq!(p, Point { x: -440, y: 328 });
}

// Bullet 10: with no caret, the work area that holds the cursor clamps the popup.
#[test]
fn t10_without_a_caret_the_cursors_monitor_holds_the_popup() {
    let p = place(None, Point { x: 4400, y: 1300 }, &[MAIN, SECOND], POPUP);
    assert!(inside(p, SECOND), "{p:?}");
}

// ---------------------------------------------------------------------------------------------
// Bullet 14: a closed session is gone.

// Bullet 14: `close_session` then `session_get` → `SessionGone`; every other call on the closed
// session is refused the same way and nothing is typed, copied or sent.
#[tokio::test]
async fn t14_a_closed_session_is_gone() {
    let e = env(C01, nothing_recorded(), Ok(publik()), Dialect::American);
    let id = id_of(&e.open().await);
    let clipboard_writes = e.desk.count("set_clipboard_text");

    api::close(&e.sessions, &id).await;

    let err = api::get(&e.sessions, &id)
        .await
        .expect_err("the session was closed");
    assert_eq!(err["kind"], "SessionGone");
    assert!(
        err["message"].as_str().is_some_and(|m| !m.is_empty()),
        "SessionGone has a message: {err}"
    );
    assert!(!err.to_string().contains("teh"), "the error holds the text");

    let fix = api::apply_fix(&e.sessions, &e.settings, &id, C01_FINDING, 0).await;
    assert_eq!(fix.expect_err("closed")["kind"], "SessionGone");
    let rewrite = e.rewrite(&id, "clearer", 0).await;
    assert_eq!(rewrite.expect_err("closed")["kind"], "SessionGone");
    let accept = e.accept(&id, None).await;
    assert_eq!(accept.expect_err("closed")["kind"], "SessionGone");
    let copy = api::copy_result(&e.sessions, &id, None).await;
    assert_eq!(copy.expect_err("closed")["kind"], "SessionGone");

    assert!(e.typed().is_empty());
    assert!(e.sent().is_empty());
    assert_eq!(e.desk.count("set_clipboard_text"), clipboard_writes);

    // Closing again is harmless; an id that never existed is gone too.
    api::close(&e.sessions, &id).await;
    let unknown = api::get(&e.sessions, "no-such-session").await;
    assert_eq!(unknown.expect_err("unknown")["kind"], "SessionGone");
}

// ---------------------------------------------------------------------------------------------
// Bullet 16: log hygiene.

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("the log is not poisoned")).into_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("the log is not poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The R2 answer with an invented balance header, written here and never in a recording.
fn r2_with_balance() -> FakeTransport {
    let mut entry: FakeEntry = recording("r2-clearer").entries[0].clone();
    entry
        .headers
        .push(("x-publik-balance".to_string(), "3120000".to_string()));
    FakeTransport::scripted(vec![entry])
}

// Bullet 16 (docs/TESTING.md §5, docs/ARCHITECTURE.md §8): a full scripted session's `tracing`
// output holds neither the text, `ana@example.com`, the fake key, the fake balance nor the
// process name.
#[test]
fn t16_a_full_session_logs_no_text_key_balance_or_process_name() {
    let log = LogBuffer::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        runtime.block_on(async {
            let e = env(R2_TEXT, r2_with_balance(), Ok(publik()), Dialect::British);
            let id = id_of(&e.open().await);
            let outcome = e.rewrite(&id, "clearer", 0).await.expect("Clearer");
            let result_id = result_id_of(&outcome);
            api::copy_result(&e.sessions, &id, result_id.as_deref())
                .await
                .expect("Copy");
            e.accept(&id, result_id.as_deref()).await.expect("Accept");
            api::undo(&e.sessions).await.expect("Undo");
            api::close(&e.sessions, &id).await;
            assert_eq!(e.sent().len(), 1);
        });
    });

    let written = log.text();
    assert!(
        !written.trim().is_empty(),
        "the session wrote no log line, so this test proves nothing"
    );
    for never in [
        R2_TEXT,
        R2_SENT,
        R2_ANSWER,
        R2_RESULT,
        EMAIL,
        "about the invoice",
        KEY,
        "3120000",
        "$3.12",
        PROCESS,
        CLIPBOARD,
    ] {
        assert!(
            !written.contains(never),
            "the log carries {never:?}:\n{written}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Scope: the footer's provider line, Copy, Undo.

// Scope (footer): the provider part of the footer is the tier on publik API, `Your own key ·
// <model>` and `Local · <model>` otherwise; only publik API reports a charge.
#[tokio::test]
async fn s_the_footer_names_the_provider_and_the_model() {
    let e = env(
        R2_TEXT,
        recorded("r2-clearer"),
        Ok(publik()),
        Dialect::British,
    );
    let id = id_of(&e.open().await);
    let outcome = e
        .rewrite(&id, "clearer", 0)
        .await
        .expect("Clearer on publik API");
    let line = outcome["provider_line"].as_str().expect("a provider line");
    assert!(line.starts_with("publik-balanced"), "{line}");
    assert_eq!(outcome["charge_micros"], 2100);
    // Invariant 6: what the interface receives carries no key.
    assert!(!outcome.to_string().contains(KEY), "{outcome}");

    let own_req = HttpRequest::post_json(
        format!("{OWN_BASE}/chat/completions"),
        r2_request_for(OWN_MODEL),
    );
    let net = FakeTransport::scripted(vec![FakeEntry::reply(
        &own_req,
        200,
        answer_body(R2_ANSWER),
    )]);
    let e = env(R2_TEXT, net, Ok(own_key()), Dialect::British);
    let id = id_of(&e.open().await);
    let outcome = e
        .rewrite(&id, "clearer", 0)
        .await
        .expect("Clearer on the own key");
    assert_eq!(outcome["provider_line"], "Your own key · example-chat");
    assert!(
        outcome.get("charge_micros").is_none_or(Value::is_null),
        "{outcome}"
    );
    assert!(!outcome.to_string().contains(OWN_KEY), "{outcome}");

    let local_req = HttpRequest::post_json(
        format!("{LOCAL_BASE}/chat/completions"),
        r2_request_for(LOCAL_MODEL),
    );
    let net = FakeTransport::scripted(vec![FakeEntry::reply(
        &local_req,
        200,
        answer_body(R2_ANSWER),
    )]);
    let e = env(R2_TEXT, net, Ok(local()), Dialect::British);
    let id = id_of(&e.open().await);
    let outcome = e
        .rewrite(&id, "clearer", 0)
        .await
        .expect("Clearer on a local model");
    assert_eq!(outcome["provider_line"], "Local · qwen2.5:14b");
    assert!(
        outcome.get("charge_micros").is_none_or(Value::is_null),
        "{outcome}"
    );
}

// Scope (Copy → `set_clipboard_text`): Copy puts the result on the clipboard, or in the check view
// the session's text; nothing is typed and nothing more is sent.
#[tokio::test]
async fn s_copy_puts_the_result_or_the_text_on_the_clipboard() {
    let e = env(
        R2_TEXT,
        recorded("r2-clearer"),
        Ok(publik()),
        Dialect::British,
    );
    let id = id_of(&e.open().await);
    assert_eq!(
        e.desk.clipboard().as_deref(),
        Some(CLIPBOARD),
        "capture restores it"
    );

    let outcome = e.rewrite(&id, "clearer", 0).await.expect("Clearer");
    api::copy_result(&e.sessions, &id, result_id_of(&outcome).as_deref())
        .await
        .expect("Copy");
    assert_eq!(e.desk.clipboard().as_deref(), Some(R2_RESULT));

    api::copy_result(&e.sessions, &id, None)
        .await
        .expect("Copy in the check view");
    assert_eq!(e.desk.clipboard().as_deref(), Some(R2_TEXT));

    assert!(e.typed().is_empty());
    assert_eq!(e.sent().len(), 1);
}

// Scope (Undo → the undo buffer): Undo after Accept sends one Ctrl+Z to the window the text went
// to, once.
#[tokio::test]
async fn s_undo_after_accept_sends_one_ctrl_z() {
    let e = env(C01, nothing_recorded(), Ok(publik()), Dialect::American);
    let id = id_of(&e.open().await);
    e.accept(&id, None).await.expect("Accept");

    let undone = api::undo(&e.sessions).await.expect("Undo");
    assert_eq!(undone, "undone");
    assert_eq!(e.desk.undone(), [WINDOW]);

    let again = api::undo(&e.sessions)
        .await
        .expect_err("one Undo per replace");
    assert_eq!(
        kind_and_message(&again),
        is("NothingToUndo", MSG_NOTHING_TO_UNDO)
    );
}

// ---------------------------------------------------------------------------------------------
// Error paths of the session commands.

// `undo_last_replace` before any Accept → `NothingToUndo`.
#[tokio::test]
async fn e_undo_with_nothing_to_undo() {
    let e = env(C01, nothing_recorded(), Ok(publik()), Dialect::American);
    let err = api::undo(&e.sessions)
        .await
        .expect_err("nothing was replaced");
    assert_eq!(
        kind_and_message(&err),
        is("NothingToUndo", MSG_NOTHING_TO_UNDO)
    );
    assert_eq!(e.desk.count("send_undo"), 0);
}

// The hotkey's capture refuses a password field before any key, and an empty selection; no session
// opens.
#[tokio::test]
async fn e_a_refused_capture_opens_no_session() {
    let password = Foreground {
        is_password: true,
        ..front()
    };
    let e = env_on(
        FakeCapture::with_foreground(password).with_selection("hunter2"),
        nothing_recorded(),
        Ok(publik()),
        Dialect::American,
    );
    let err = api::open(&e.sessions, &e.settings)
        .await
        .expect_err("a password field is refused");
    assert_eq!(kind_and_message(&err), is("PasswordField", MSG_PASSWORD));
    assert_eq!(e.desk.count("read_selection"), 0, "no copy was attempted");
    assert!(!err.to_string().contains("hunter2"));

    let e = env_on(
        FakeCapture::with_foreground(front()),
        nothing_recorded(),
        Ok(publik()),
        Dialect::American,
    );
    let err = api::open(&e.sessions, &e.settings)
        .await
        .expect_err("nothing is selected");
    assert_eq!(
        kind_and_message(&err),
        is("NothingCopied", MSG_NOTHING_COPIED)
    );
}

// `apply_fix` with a suggestion index the finding does not have → `NoSuchSuggestion`; the same
// finding applied twice → `StaleFinding`. Neither changes the text.
#[tokio::test]
async fn e_apply_fix_refuses_a_missing_suggestion_and_a_stale_finding() {
    let e = env(C01, nothing_recorded(), Ok(publik()), Dialect::American);
    let id = id_of(&e.open().await);

    let err = api::apply_fix(&e.sessions, &e.settings, &id, C01_FINDING, 1)
        .await
        .expect_err("c01's finding has one suggestion");
    assert_eq!(
        kind_and_message(&err),
        is("NoSuchSuggestion", MSG_NO_SUGGESTION)
    );
    let kept = api::get(&e.sessions, &id).await.expect("open");
    assert_eq!(kept["text"], C01);

    api::apply_fix(&e.sessions, &e.settings, &id, C01_FINDING, 0)
        .await
        .expect("the first fix applies");
    let err = api::apply_fix(&e.sessions, &e.settings, &id, C01_FINDING, 0)
        .await
        .expect_err("the finding was fixed already");
    assert_eq!(kind_and_message(&err), is("StaleFinding", MSG_STALE));
    let kept = api::get(&e.sessions, &id).await.expect("open");
    assert_eq!(kept["text"], C01_FIXED);
    assert!(e.sent().is_empty());
}

// `accept` into a window that has closed → `WindowGone` and nothing typed; the session stays, so
// Copy still works.
#[tokio::test]
async fn e_accept_into_a_closed_window_types_nothing() {
    let e = env(C01, nothing_recorded(), Ok(publik()), Dialect::American);
    let id = id_of(&e.open().await);
    e.desk.close_window(&WINDOW);

    let err = e.accept(&id, None).await.expect_err("the window is gone");
    assert_eq!(kind_and_message(&err), is("WindowGone", MSG_WINDOW_GONE));
    assert!(e.typed().is_empty());

    api::copy_result(&e.sessions, &id, None)
        .await
        .expect("Copy still works");
    assert_eq!(e.desk.clipboard().as_deref(), Some(C01));
}

// `copy_result` while another app holds the clipboard → `ClipboardBusy`.
#[tokio::test]
async fn e_copy_while_the_clipboard_is_busy() {
    let e = env(C01, nothing_recorded(), Ok(publik()), Dialect::American);
    let id = id_of(&e.open().await);
    e.desk
        .script_set_clipboard_text(Err(CaptureError::ClipboardBusy));

    let err = api::copy_result(&e.sessions, &id, None)
        .await
        .expect_err("the clipboard is busy");
    assert_eq!(
        kind_and_message(&err),
        is("ClipboardBusy", MSG_CLIPBOARD_BUSY)
    );
}
