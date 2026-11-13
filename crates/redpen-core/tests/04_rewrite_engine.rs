//! Module 04 tests in the core (docs/modules/04-rewrite-engine.md, Acceptance tests): from a
//! selected text and an action to a validated, restored and cleaned result with its diff and
//! warnings, or a refusal. A test named `tNN_…` covers acceptance test NN (rows R3 to R10 and R7b
//! of docs/PUBLIK-INTEGRATION.md §11, as the spec maps them), `e_…` an error path or a Scope rule
//! the list leaves implicit, and `iN_…` invariant N of docs/ARCHITECTURE.md §7 that this module
//! proves (7: prompts are versioned files; 8: every AI answer is validated before use).
//!
//! Expected values come from docs/PUBLIK-INTEGRATION.md §6 and the spec, through the golden cases
//! of tests/cases/04-rewrite-engine (every `expected.json` of a `rewrite` or `prompt` case is also
//! checked here, against the core, with the same request and the same recorded answer), the
//! corpora there (true by construction) and the reference runs recorded in
//! tests/cases/04-rewrite-engine/REFERENCE.md. Nothing here is taken from Redpen's own output.
//!
//! Transport: `Script` answers each request with the next reply in its queue and keeps every
//! request; with the queue empty a request fails as unrecorded, so no test reaches the network.
//!
//! The spec's Interfaces name the types and functions but not every module path, nor the
//! signatures of the prompt loader and renderer. Those are reached through `mod api`, so a
//! different path changes one line there and no expected value.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use redpen_core::clock::{FakeClock, FakeSleeper};
use redpen_core::net::{
    Body, HttpRequest, HttpResponse, HttpTransport, TransportError, canonical_json, request_hash,
    request_path,
};
use redpen_core::provider::{
    AiClient, AiError, ChatCall, Feature, NoopStatusSink, ProviderKind, ResolvedProvider,
    SecretString,
};
use redpen_core::store::Db;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use api::{
    Action, ClaimState, Dialect, DiffOp, RewriteError, RewriteOutcome, RewriteQuality,
    RewriteRequest, Style, TierContext, Warning,
};

// ---------------------------------------------------------------------------------------------
// The names of the spec's Interfaces whose module path or signature the spec leaves open.

mod api {
    pub use redpen_core::check::Dialect;
    pub use redpen_core::diff::DiffOp;
    pub use redpen_core::provider::tier::{ClaimState, RewriteQuality, TierContext};
    pub use redpen_core::redact::{redact, restore};
    pub use redpen_core::rewrite::{
        Action, RewriteError, RewriteOutcome, RewriteRequest, Style, Warning, build_request,
        edit_ratio, rewrite,
    };

    /// `prompts::load` over one prompt file's name and text; `Err` carries the error's message.
    pub fn load_prompt(file_name: &str, text: &str) -> Result<(), String> {
        redpen_core::rewrite::prompts::load(file_name, text)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// `prompts::render` of one template with `values`; `Err` carries the error's message.
    pub fn render(template: &str, values: &[(&str, &str)]) -> Result<String, String> {
        redpen_core::rewrite::prompts::render(template, values).map_err(|e| e.to_string())
    }

    /// The embedded prompt file `file` rendered with `values`: its system and user contents;
    /// `Err` carries the error's message.
    pub fn render_file(file: &str, values: &[(&str, &str)]) -> Result<(String, String), String> {
        let prompt = redpen_core::rewrite::prompts::get(file).map_err(|e| e.to_string())?;
        redpen_core::rewrite::prompts::render_prompt(prompt, values)
            .map(|rendered| (rendered.system, rendered.user))
            .map_err(|e| e.to_string())
    }

    /// The text a warning shows (docs/PUBLIK-INTEGRATION.md §6.1 and §6.2).
    pub fn warning_message(w: &Warning) -> String {
        w.to_string()
    }
}

/// The fake publik API key of contract test 16.
const KEY: &str = "pk_live_a";
const PUBLIK_BASE: &str = "https://publikhq.com/api/v1";
/// The own key's address and model, from the settings example of docs/ARCHITECTURE.md §4.
const OWN_BASE: &str = "https://api.example.com/v1";
const OWN_KEY: &str = "sk-test-a";
const OWN_MODEL: &str = "example-chat";
const LOCAL_BASE: &str = "http://localhost:11434/v1";
const LOCAL_MODEL: &str = "qwen2.5:14b";
/// 2026-09-21T14:13:20Z (tests/cases/02-provider-layer/REFERENCE.md).
const T0: i64 = 1_790_000_000_000;

/// The six actions by their prompt ids.
const ACTIONS: [&str; 6] = [
    "fix-grammar",
    "clearer",
    "shorter",
    "more-formal",
    "friendlier",
    "custom",
];

/// The Fix grammar call whose cache key and idempotency keys tests/cases/03-publik/REFERENCE.md
/// computed with `sha256sum`: publik API, `publik-fast`, prompt version 1, dialect British.
const LIBARY: &str = "Their going to the libary.";
const LIBRARY: &str = "They're going to the library.";
const K_LIBARY: &str = "748f0ab593f7379e156586aeba37f558d87039ad6da30400ee0c179dba3135b4";
const IDEM_0: &str = "748f0ab593f7379e156586aeba37f558";
const IDEM_1: &str = "fc0efd139ba7f503da3f335030060003";
const IDEM_2: &str = "7ba59aa7d4199e866e157ae016c9b78c";

/// docs/PUBLIK-INTEGRATION.md §6.1: the example text and answer, with the marker as sent.
const T61: &str =
    "Their going to send the report tomorow, please forward it to [EMAIL_1] when it arrive.";
const A61: &str =
    "They're going to send the report tomorrow. Please forward it to [EMAIL_1] when it arrives.";
/// docs/PUBLIK-INTEGRATION.md §6.2: the example text, and a hand-written Clearer answer.
const T62: &str = "With regard to the matter of the invoice, which we discussed at some length on the phone last week, it would be appreciated if it could be paid at the earliest opportunity that is convenient.";
const A62: &str = "With regard to the matter of the invoice, which we discussed at some length on the phone last week, it would be appreciated if it could be settled at the earliest opportunity.";

/// The guard's examples (REFERENCE.md, "The guard").
const SIX_OLD: &str = "We go to the shop in the van at six.";
const SIX_NEW: &str = "They went to a shop on the bus by six.";
const FOUR_NEW: &str = "We went to a shop on the bus at six.";
const TWENTY_OLD: &str = "The old man walked his small dog to the park every morning and then sat on a green wooden bench.";
const TWENTY_NEW: &str = "The young woman took his big dog to the river every evening and then sat on a blue wooden bench.";

/// The user sections of docs/PUBLIK-INTEGRATION.md §6.1 (shared by the four presets) and §6.3.
const TEXT_USER: &str = "<text>\n{text}\n</text>";
const CUSTOM_USER: &str = "<instruction>\n{instruction}\n</instruction>\n\n<text>\n{text}\n</text>";
/// The reply line of §6.1: JSON in a prompt, which is no placeholder.
const REPLY_LINE: &str = "Reply with one JSON object and nothing else, in this form: {\"text\": \"<the corrected text>\"}";

// The spec's messages (Interfaces) and docs/PUBLIK-INTEGRATION.md §6.
const MSG_EMPTY: &str = "Select some text first.";
const MSG_INSTRUCTION: &str = "Keep the instruction under 300 characters.";
/// Custom with no instruction (the spec's Design notes, "Refusal order").
const MSG_INSTRUCTION_MISSING: &str = "Type an instruction for Custom first.";
const MSG_COLLISION: &str = "This text already contains a marker like [EMAIL_1], so redaction can't run safely. Turn off redaction in Settings → Privacy, or edit the text.";
const MSG_DROPPED_PHONE: &str = "The AI answer dropped [PHONE_1], so it was not used. Try again, or turn off redaction for this text.";
const MSG_INVENTED: &str =
    "The AI answer added a marker that was not in your text, so it was not used.";
const MSG_INVALID: &str = "The AI answer could not be used.";
const MSG_NOT_SHORTER: &str = "This is not shorter than your text.";

/// How long a gate script may take to print its question and read `n`.
const GATE_TIMEOUT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------------------------
// A scripted transport.

type Reply = Result<HttpResponse, TransportError>;

/// Answers each request with the next reply in its queue and keeps every request. A request
/// with no reply left fails as unrecorded.
struct Script {
    replies: Mutex<VecDeque<Reply>>,
    seen: Mutex<Vec<HttpRequest>>,
}

impl Script {
    fn requests(&self) -> Vec<HttpRequest> {
        self.seen.lock().expect("the log is not poisoned").clone()
    }
}

#[async_trait::async_trait]
impl HttpTransport for Script {
    async fn send(&self, req: HttpRequest) -> Reply {
        self.seen
            .lock()
            .expect("the log is not poisoned")
            .push(req.clone());
        let next = self
            .replies
            .lock()
            .expect("the replies are not poisoned")
            .pop_front();
        next.unwrap_or_else(|| {
            Err(TransportError::Unrecorded {
                hash: request_hash(&req),
                path: request_path(&req.url),
            })
        })
    }
}

/// A client over a script, an in-memory database (answers kept on disk for 7 days, the default),
/// a fake clock at `T0` and a fake sleeper, with the status sink that reads nothing.
struct Env {
    client: AiClient,
    db: Db,
    t: Arc<Script>,
}

fn env(replies: Vec<Reply>) -> Env {
    let t = Arc::new(Script {
        replies: Mutex::new(replies.into()),
        seen: Mutex::new(Vec::new()),
    });
    let db = Db::open_in_memory().expect("an in-memory redpen.db");
    let client = AiClient::new(
        t.clone(),
        db.clone(),
        Arc::new(FakeClock::at(T0)),
        Arc::new(FakeSleeper::new()),
        Arc::new(NoopStatusSink),
    );
    Env { client, db, t }
}

impl Env {
    /// A rewrite with the command line's tier context: Standard and anonymous.
    async fn run(
        &self,
        p: &ResolvedProvider,
        r: RewriteRequest,
    ) -> Result<RewriteOutcome, RewriteError> {
        api::rewrite(&self.client, p, &TierContext::default(), r).await
    }

    async fn run_in(
        &self,
        p: &ResolvedProvider,
        ctx: &TierContext,
        r: RewriteRequest,
    ) -> Result<RewriteOutcome, RewriteError> {
        api::rewrite(&self.client, p, ctx, r).await
    }

    /// The rows of `ai_cache`.
    fn cached(&self) -> usize {
        self.db.cache_len().expect("ai_cache can be counted")
    }

    /// Every request sent so far.
    fn sent(&self) -> Vec<HttpRequest> {
        self.t.requests()
    }
}

fn reply(status: u16, body: &Value) -> Reply {
    Ok(HttpResponse {
        status,
        headers: Vec::new(),
        body: serde_json::to_vec(body).expect("a JSON value serialises"),
    })
}

/// A 200 answer whose `choices[0].message.content` is `content`.
fn answer(content: &str) -> Reply {
    reply(
        200,
        &json!({"choices": [{"message": {"role": "assistant", "content": content}}]}),
    )
}

/// A 200 answer `{"text": text}`.
fn text_answer(text: &str) -> Reply {
    answer(&json!({ "text": text }).to_string())
}

/// A 200 Custom answer `{"text", "changes_language"}`.
fn custom_answer(text: &str, changes_language: bool) -> Reply {
    answer(&json!({"text": text, "changes_language": changes_language}).to_string())
}

/// publik API's 503 (tests/cases/03-publik/chat-503.fake.json).
fn busy() -> Reply {
    reply(
        503,
        &json!({"error": {"type": "overloaded", "code": "overloaded", "message": "The service is overloaded."}}),
    )
}

// ---------------------------------------------------------------------------------------------
// Providers, requests and results.

fn fake() -> ResolvedProvider {
    ResolvedProvider::fake()
}

/// publik API as the app resolves it: the gateway, the install's key and a tier.
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

fn local() -> ResolvedProvider {
    ResolvedProvider::local(LOCAL_BASE, LOCAL_MODEL)
}

fn action(name: &str) -> Action {
    match name {
        "fix-grammar" => Action::FixGrammar,
        "clearer" => Action::Clearer,
        "shorter" => Action::Shorter,
        "more-formal" => Action::MoreFormal,
        "friendlier" => Action::Friendlier,
        "custom" => Action::Custom,
        other => panic!("no action {other}"),
    }
}

fn feature(name: &str) -> Feature {
    match name {
        "fix-grammar" => Feature::FixGrammar,
        "clearer" => Feature::Clearer,
        "shorter" => Feature::Shorter,
        "more-formal" => Feature::MoreFormal,
        "friendlier" => Feature::Friendlier,
        "custom" => Feature::Custom,
        other => panic!("no feature {other}"),
    }
}

/// A first press: British English, the author's own style, no redaction, no variant.
fn req(action: Action, text: &str) -> RewriteRequest {
    RewriteRequest {
        action,
        text: text.to_string(),
        instruction: None,
        dialect: Dialect::British,
        style: Style::Own,
        redact: false,
        variant: 0,
        validation_retry: 0,
    }
}

fn custom(text: &str, instruction: &str) -> RewriteRequest {
    RewriteRequest {
        instruction: Some(instruction.to_string()),
        ..req(Action::Custom, text)
    }
}

fn redacted(action: Action, text: &str) -> RewriteRequest {
    RewriteRequest {
        redact: true,
        ..req(action, text)
    }
}

/// A press of the named action; Custom gets a short instruction.
fn press(name: &str, text: &str) -> RewriteRequest {
    if name == "custom" {
        custom(text, "make it more formal")
    } else {
        req(action(name), text)
    }
}

/// `n` words: `w1 w2 … wn`, each a letter followed by digits, one UAX #29 word.
fn words_text(n: usize) -> String {
    (1..=n)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn ok(result: Result<RewriteOutcome, RewriteError>, what: &str) -> RewriteOutcome {
    match result {
        Ok(outcome) => outcome,
        Err(e) => panic!("{what}: expected a result, got the error \"{e}\""),
    }
}

fn err(result: Result<RewriteOutcome, RewriteError>, what: &str) -> RewriteError {
    match result {
        Ok(outcome) => panic!(
            "{what}: expected an error, got the result {:?}",
            outcome.text
        ),
        Err(e) => e,
    }
}

fn ok_call(result: Result<ChatCall, RewriteError>, what: &str) -> ChatCall {
    match result {
        Ok(call) => call,
        Err(e) => panic!("{what}: expected a request, got the error \"{e}\""),
    }
}

fn body(req: &HttpRequest) -> Value {
    match &req.body {
        Body::Json(v) => v.clone(),
        Body::Empty => Value::Null,
    }
}

fn user_content(req: &HttpRequest) -> String {
    body(req)["messages"][1]["content"]
        .as_str()
        .expect("a user message")
        .to_string()
}

fn idempotency(req: &HttpRequest) -> String {
    req.headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-publik-idempotency-key"))
        .map(|(_, value)| value.expose().to_string())
        .expect("a publik API request carries x-publik-idempotency-key")
}

/// The first request a press sends, whatever the answer (here: none).
async fn sent_body(p: &ResolvedProvider, ctx: &TierContext, r: RewriteRequest) -> Value {
    let e = env(Vec::new());
    let _ = e.run_in(p, ctx, r).await;
    let sent = e.sent();
    let first = sent.first().expect("the press sent a request");
    body(first)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// K as docs/ARCHITECTURE.md §4 defines it: SHA-256 of the canonical
/// `{"provider", "model", "prompt", "prompt_version", "input"}`.
fn cache_key(provider: &str, model: &str, call: &ChatCall) -> String {
    let input = json!({
        "provider": provider,
        "model": model,
        "prompt": call.prompt_id,
        "prompt_version": call.prompt_version,
        "input": call.cache_input,
    });
    hex(&Sha256::digest(canonical_json(&input).as_bytes()))
}

/// The idempotency key of K after `n` invalid answers (docs/PUBLIK-INTEGRATION.md §5).
fn idempotency_key(k: &str, n: u32) -> String {
    let full = if n == 0 {
        k.to_string()
    } else {
        hex(&Sha256::digest(format!("{k}:{n}").as_bytes()))
    };
    full[..32].to_string()
}

// ---------------------------------------------------------------------------------------------
// The command line's JSON (the spec's Design notes), built from the core's values, so a golden
// case's `expected.json` can be checked against the core.

fn diff_json(diff: &[DiffOp]) -> Value {
    Value::Array(
        diff.iter()
            .map(|op| match op {
                DiffOp::Equal(s) => json!({ "equal": s }),
                DiffOp::Insert(s) => json!({ "insert": s }),
                DiffOp::Delete(s) => json!({ "delete": s }),
            })
            .collect(),
    )
}

fn warning_json(w: &Warning) -> Value {
    let message = api::warning_message(w);
    match w {
        Warning::Guard { changed, total } => {
            json!({"kind": "guard", "message": message, "changed": changed, "total": total})
        }
        Warning::Language { expected, got } => {
            json!({"kind": "language", "message": message, "expected": expected, "got": got})
        }
        Warning::NotShorter => json!({"kind": "not_shorter", "message": message}),
    }
}

fn outcome_json(o: &RewriteOutcome) -> Value {
    json!({
        "text": o.text,
        "diff": diff_json(&o.diff),
        "warnings": o.warnings.iter().map(warning_json).collect::<Vec<_>>(),
        "accept_default": o.accept_default,
        "unchanged": o.unchanged,
        "from_cache": o.from_cache,
        "charge_micros": o.charge_micros,
    })
}

/// `{"kind", "message"}` plus the variant's fields; an AI error is the `AiError` itself, as
/// module 02 prints it.
fn error_json(e: &RewriteError) -> Value {
    let message = e.to_string();
    match e {
        RewriteError::Ai(ai) => ai.to_json(),
        RewriteError::Empty => json!({"kind": "Empty", "message": message}),
        RewriteError::TooLong { words } => {
            json!({"kind": "TooLong", "message": message, "words": words})
        }
        RewriteError::InstructionTooLong => {
            json!({"kind": "InstructionTooLong", "message": message})
        }
        RewriteError::RedactionCollision => {
            json!({"kind": "RedactionCollision", "message": message})
        }
        RewriteError::PlaceholderDropped { marker } => {
            json!({"kind": "PlaceholderDropped", "message": message, "marker": marker})
        }
        RewriteError::PlaceholderInvented { marker } => {
            json!({"kind": "PlaceholderInvented", "message": message, "marker": marker})
        }
        RewriteError::InstructionMissing => {
            json!({"kind": "InstructionMissing", "message": message})
        }
    }
}

/// The warnings of a result, short: `guard 6 of 10`, `language English to French`, `not shorter`.
fn warnings(o: &RewriteOutcome) -> Vec<String> {
    o.warnings
        .iter()
        .map(|w| match w {
            Warning::Guard { changed, total } => format!("guard {changed} of {total}"),
            Warning::Language { expected, got } => format!("language {expected} to {got}"),
            Warning::NotShorter => "not shorter".to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Files: the golden cases, the corpora and the prompt files.

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn cases_dir() -> PathBuf {
    repo_root().join("tests/cases/04-rewrite-engine")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{} could not be read: {e}", path.display()))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&read(path))
        .unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The lines of a `.jsonl` file of the cases folder.
fn jsonl(name: &str) -> Vec<Value> {
    read(&cases_dir().join(name))
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{name}: {e}: {line}")))
        .collect()
}

/// One golden case: its command line and its `expected.json`.
struct Golden {
    name: String,
    dir: PathBuf,
    args: Vec<String>,
    exit: i64,
    expected: Value,
}

fn golden(dir: &Path) -> Golden {
    let cmd = read_json(&dir.join("cmd.json"));
    let args = cmd["args"]
        .as_array()
        .expect("cmd.json has args")
        .iter()
        .map(|a| a.as_str().expect("every argument is a string").to_string())
        .collect();
    Golden {
        name: file_name(dir),
        dir: dir.to_path_buf(),
        args,
        exit: cmd["exit"].as_i64().unwrap_or(0),
        expected: read_json(&dir.join("expected.json")),
    }
}

/// The golden cases whose folder name starts with `prefix`, in name order.
fn goldens(prefix: &str) -> Vec<Golden> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(cases_dir())
        .expect("tests/cases/04-rewrite-engine can be listed")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("cmd.json").is_file() && file_name(path).starts_with(prefix))
        .collect();
    dirs.sort();
    dirs.iter().map(PathBuf::as_path).map(golden).collect()
}

impl Golden {
    fn command(&self) -> &str {
        self.args.first().map(String::as_str).unwrap_or_default()
    }

    /// The value after `flag`.
    fn flag(&self, flag: &str) -> Option<&str> {
        self.args
            .windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].as_str())
    }

    /// The request a `rewrite` or `prompt` command makes of the core.
    fn request(&self) -> RewriteRequest {
        let name = self
            .flag("--action")
            .unwrap_or_else(|| panic!("{}: no --action", self.name));
        let file = self.args.last().expect("a FILE argument");
        RewriteRequest {
            action: action(name),
            text: read(&self.dir.join(file)),
            instruction: self.flag("--instruction").map(str::to_string),
            dialect: self
                .flag("--dialect")
                .unwrap_or("american")
                .parse()
                .unwrap_or_else(|_| panic!("{}: unknown dialect", self.name)),
            style: Style::Own,
            redact: self.args.iter().any(|a| a == "--redact"),
            variant: 0,
            validation_retry: 0,
        }
    }

    /// The recorded entries: each request's hash and its reply.
    fn entries(&self) -> Vec<(String, Reply)> {
        let path = self.dir.join("recording.json");
        if !path.is_file() {
            return Vec::new();
        }
        read_json(&path)["entries"]
            .as_array()
            .expect("a recording has entries")
            .iter()
            .map(|entry| {
                let hash = entry["request_sha256"]
                    .as_str()
                    .expect("an entry has a request_sha256")
                    .to_string();
                let status = entry["status"]
                    .as_u64()
                    .and_then(|s| u16::try_from(s).ok())
                    .expect("an entry has a status");
                (hash, reply(status, &entry["body"]))
            })
            .collect()
    }
}

/// Runs one golden `rewrite` case through the core with the fake provider and the command line's
/// tier context, and checks the request against the recording, the result against
/// `expected.json`, and `ai_cache` (a usable answer is cached, anything else is not).
///
/// A rewrite sends at most one chat call (docs/PUBLIK-INTEGRATION.md §8): one for a case with a
/// recording, none for a refusal. A recording may hold more than one acceptable request where the
/// spec leaves the request open (REFERENCE.md); the one sent must be among them.
async fn check_rewrite_case(g: &Golden) {
    let entries = g.entries();
    let recorded: Vec<String> = entries.iter().map(|(hash, _)| hash.clone()).collect();
    let e = env(entries.into_iter().map(|(_, r)| r).collect());
    let result = e.run(&fake(), g.request()).await;

    let sent: Vec<String> = e.sent().iter().map(request_hash).collect();
    assert_eq!(
        sent.len(),
        usize::from(!recorded.is_empty()),
        "{}: requests sent",
        g.name
    );
    for hash in &sent {
        assert!(
            recorded.contains(hash),
            "{}: the request sent ({hash}) is not in the recording",
            g.name
        );
    }

    let actual = match &result {
        Ok(o) => outcome_json(o),
        Err(error) => json!({ "error": error_json(error) }),
    };
    assert_eq!(
        actual, g.expected,
        "{}: the core's result differs from expected.json",
        g.name
    );
    assert_eq!(
        g.exit == 0,
        result.is_ok(),
        "{}: exit {} and the core's result disagree",
        g.name,
        g.exit
    );
    assert_eq!(
        e.cached(),
        usize::from(result.is_ok()),
        "{}: only a usable answer is cached",
        g.name
    );
}

/// Every golden `rewrite` case starting with `prefix`; `count` guards against a missing folder.
async fn check_rewrite_cases(prefix: &str, count: usize) {
    let cases: Vec<Golden> = goldens(prefix)
        .into_iter()
        .filter(|g| g.command() == "rewrite")
        .collect();
    assert_eq!(cases.len(), count, "golden rewrite cases named {prefix}*");
    for g in &cases {
        check_rewrite_case(g).await;
    }
}

/// `(id, language, text)` of every paragraph of languages.jsonl, in file order.
fn paragraphs() -> Vec<(String, String, String)> {
    jsonl("languages.jsonl")
        .iter()
        .map(|p| {
            let field = |k: &str| p[k].as_str().expect("id, lang and text").to_string();
            (field("id"), field("lang"), field("text"))
        })
        .collect()
}

fn paragraph(id: &str) -> String {
    paragraphs()
        .into_iter()
        .find(|(pid, _, _)| pid == id)
        .map(|(_, _, text)| text)
        .unwrap_or_else(|| panic!("languages.jsonl has no {id}"))
}

/// A prompt file split by the spec's Design notes: the `key: value` lines between the two `---`
/// lines, and each `## name` section's text up to the next `## ` line or the end, leading and
/// trailing line breaks removed.
fn parse_prompt_file(
    name: &str,
    text: &str,
) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let rest = text
        .strip_prefix("---\n")
        .unwrap_or_else(|| panic!("{name} starts with a --- line"));
    let (front, rest) = rest
        .split_once("\n---\n")
        .unwrap_or_else(|| panic!("{name}: the front matter ends with a --- line"));
    let mut front_matter = BTreeMap::new();
    for line in front.lines() {
        let (key, value) = line
            .split_once(": ")
            .unwrap_or_else(|| panic!("{name}: front matter line {line:?} is not key: value"));
        front_matter.insert(key.to_string(), value.to_string());
    }
    let mut sections = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut lines: Vec<&str> = Vec::new();
    for line in rest.split('\n') {
        if let Some(title) = line.strip_prefix("## ") {
            if let Some(section) = current.take() {
                sections.insert(section, lines.join("\n").trim_matches('\n').to_string());
            }
            current = Some(title.trim().to_string());
            lines.clear();
        } else {
            lines.push(line);
        }
    }
    if let Some(section) = current {
        sections.insert(section, lines.join("\n").trim_matches('\n').to_string());
    }
    (front_matter, sections)
}

/// §6's front matter for an action.
fn expected_front_matter(name: &str) -> BTreeMap<String, String> {
    let (tier, temperature, extra, schema) = match name {
        "fix-grammar" => ("publik-fast", "0", 100, "rewrite-result.v1"),
        "custom" => ("publik-balanced", "0.4", 150, "custom-result.v1"),
        _ => ("publik-balanced", "0.4", 100, "rewrite-result.v1"),
    };
    [
        ("id", name.to_string()),
        ("version", "1".to_string()),
        ("tier", tier.to_string()),
        ("temperature", temperature.to_string()),
        (
            "max_tokens",
            format!("clamp(2 * words(text) + {extra}, 256, 4096)"),
        ),
        ("schema", schema.to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// §6's sections for an action: the system content of the golden `t01-prompt-<action>` body
/// (a §6 example body, or one built as §6.2 says) with the dialect name and the style put back
/// as their placeholders, and the user section of §6.1 or §6.3.
fn expected_sections(name: &str) -> BTreeMap<String, String> {
    let g = golden(&cases_dir().join(format!("t01-prompt-{name}")));
    let system = g.expected["messages"][0]["content"]
        .as_str()
        .expect("a system message")
        .replace("British English", "{dialect_name}")
        .replace("Keep the author's own style.", "{style}");
    let user = if name == "custom" {
        CUSTOM_USER
    } else {
        TEXT_USER
    };
    BTreeMap::from([
        ("system".to_string(), system),
        ("user".to_string(), user.to_string()),
    ])
}

/// A prompt file in the spec's format, with Fix grammar's front matter under `id` and `version`.
fn prompt_file(id: &str, version: u32, system: &str, user: Option<&str>) -> String {
    let user_section = user
        .map(|u| format!("\n## user\n\n{u}\n"))
        .unwrap_or_default();
    format!(
        "---\nid: {id}\nversion: {version}\ntier: publik-fast\ntemperature: 0\nmax_tokens: clamp(2 * words(text) + 100, 256, 4096)\nschema: rewrite-result.v1\n---\n\n## system\n\n{system}\n{user_section}"
    )
}

// ---------------------------------------------------------------------------------------------
// Test 1 (R3): the rendered request of each action is the §6 body.

// Bullet 1: `redpen prompt` for each action prints exactly the §6 body (the golden t01 cases);
// the core sends that same body, model included, through the fake provider.
#[tokio::test]
async fn t01_r3_each_action_sends_exactly_its_section_6_body() {
    let cases = goldens("t01-");
    assert_eq!(cases.len(), 7, "the t01 golden cases");
    for g in &cases {
        assert_eq!(g.command(), "prompt", "{}", g.name);
        let e = env(Vec::new());
        let _ = e.run(&fake(), g.request()).await;
        let sent = e.sent();
        assert_eq!(sent.len(), 1, "{}: one request", g.name);
        assert_eq!(
            request_path(&sent[0].url),
            "/chat/completions",
            "{}",
            g.name
        );
        assert_eq!(body(&sent[0]), g.expected, "{}: the body sent", g.name);
    }
}

// Bullet 1: `build_request` gives the §6 messages, `max_tokens` and temperature, the prompt id
// and version 1, and the action's feature.
#[test]
fn t01_r3_build_request_holds_the_section_6_messages() {
    for g in goldens("t01-") {
        let name = g.flag("--action").expect("an action").to_string();
        let call = ok_call(
            api::build_request(&fake(), &TierContext::default(), &g.request()),
            &g.name,
        );
        let messages: Vec<Value> = call
            .messages
            .iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect();
        assert_eq!(Value::Array(messages), g.expected["messages"], "{}", g.name);
        assert_eq!(
            json!(call.max_tokens),
            g.expected["max_tokens"],
            "{}",
            g.name
        );
        assert_eq!(
            redpen_core::provider::client::shortest_number(call.temperature),
            g.expected["temperature"],
            "{}",
            g.name
        );
        assert_eq!(call.prompt_id, name, "{}", g.name);
        assert_eq!(call.prompt_version, 1, "{}", g.name);
        assert_eq!(call.feature, feature(&name), "{}", g.name);
    }
}

// ---------------------------------------------------------------------------------------------
// Invariant 7: prompts are versioned files; a change is a new version.

// Every prompt file is `prompts/<id>.v<N>.md` and names its own id and version; every schema is
// `prompts/schemas/<name>.v<N>.json` with that `$id`; the six prompts and two schemas of v1 exist.
#[test]
fn i7_every_prompt_file_is_named_by_its_id_and_version() {
    let dir = repo_root().join("prompts");
    let mut prompts = Vec::new();
    for entry in fs::read_dir(&dir).expect("prompts/ can be listed") {
        let path = entry.expect("a directory entry").path();
        if !path.is_file() {
            continue;
        }
        let name = file_name(&path);
        let stem = name
            .strip_suffix(".md")
            .unwrap_or_else(|| panic!("prompts/{name}: prompt files are <id>.v<N>.md"));
        let (id, version) = stem
            .rsplit_once(".v")
            .unwrap_or_else(|| panic!("prompts/{name}: prompt files are <id>.v<N>.md"));
        let n: u32 = version
            .parse()
            .unwrap_or_else(|_| panic!("prompts/{name}: the version is a whole number"));
        assert!(n >= 1, "prompts/{name}: versions start at 1");
        assert!(
            !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'),
            "prompts/{name}: the id is lower-case words joined by hyphens"
        );
        let (front_matter, _) = parse_prompt_file(&name, &read(&path));
        assert_eq!(
            front_matter.get("id").map(String::as_str),
            Some(id),
            "prompts/{name}: the file name matches its id"
        );
        assert_eq!(
            front_matter.get("version").map(String::as_str),
            Some(version),
            "prompts/{name}: the file name matches its version"
        );
        prompts.push(name);
    }
    for name in ACTIONS {
        assert!(
            prompts.contains(&format!("{name}.v1.md")),
            "prompts/{name}.v1.md exists"
        );
    }

    let mut schemas = Vec::new();
    for entry in fs::read_dir(dir.join("schemas")).expect("prompts/schemas/ can be listed") {
        let path = entry.expect("a directory entry").path();
        let name = file_name(&path);
        let stem = name
            .strip_suffix(".json")
            .unwrap_or_else(|| panic!("prompts/schemas/{name}: schemas are <name>.v<N>.json"));
        let (_, version) = stem
            .rsplit_once(".v")
            .unwrap_or_else(|| panic!("prompts/schemas/{name}: schemas are <name>.v<N>.json"));
        assert!(
            version.parse::<u32>().is_ok_and(|n| n >= 1),
            "prompts/schemas/{name}: the version is a whole number from 1"
        );
        assert_eq!(
            read_json(&path)["$id"],
            json!(stem),
            "prompts/schemas/{name}: $id is the file name"
        );
        schemas.push(name);
    }
    for name in ["rewrite-result.v1.json", "custom-result.v1.json"] {
        assert!(
            schemas.contains(&name.to_string()),
            "prompts/schemas/{name} exists"
        );
    }
}

// The six v1 files hold exactly the §6 front matter and sections, so a change to one fails here:
// a change is a new version (ADR-0004).
#[test]
fn i7_v1_prompt_files_hold_exactly_the_section_6_text() {
    for name in ACTIONS {
        let file = format!("{name}.v1.md");
        let text = read(&repo_root().join("prompts").join(&file));
        let (front_matter, sections) = parse_prompt_file(&file, &text);
        assert_eq!(
            front_matter,
            expected_front_matter(name),
            "prompts/{file}: the §6 front matter (a change to a prompt is a new version, v2)"
        );
        assert_eq!(
            sections,
            expected_sections(name),
            "prompts/{file}: the §6 system and user sections (a change to a prompt is a new version, v2)"
        );
    }
}

// The two v1 schemas are exactly §6.1's and §6.3's.
#[test]
fn i7_v1_schemas_are_exactly_the_section_6_schemas() {
    let rewrite = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "rewrite-result.v1",
        "type": "object",
        "additionalProperties": false,
        "required": ["text"],
        "properties": {"text": {"type": "string", "minLength": 1, "maxLength": 40000}}
    });
    let custom = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "custom-result.v1",
        "type": "object",
        "additionalProperties": false,
        "required": ["text", "changes_language"],
        "properties": {
            "text": {"type": "string", "minLength": 1, "maxLength": 40000},
            "changes_language": {"type": "boolean"}
        }
    });
    let schemas = repo_root().join("prompts/schemas");
    assert_eq!(read_json(&schemas.join("rewrite-result.v1.json")), rewrite);
    assert_eq!(read_json(&schemas.join("custom-result.v1.json")), custom);
}

// ---------------------------------------------------------------------------------------------
// Test 2 (R4): each action's recorded answer gives the expected text and diff.

// Bullet 2: the six golden rewrite cases, one per action; Fix grammar is the §6.1 answer restored
// with ana@example.com in place of [EMAIL_1].
#[tokio::test]
async fn t02_r4_each_actions_recorded_answer_gives_its_text_and_diff() {
    check_rewrite_cases("t02-", 6).await;
}

// The recordings of this module hash the bodies they hold (module 00's rule), so a golden case
// can only be answered for the request it names.
#[test]
fn e_recordings_hash_the_bodies_they_hold() {
    let mut checked = 0;
    for entry in fs::read_dir(cases_dir()).expect("the cases folder can be listed") {
        let path = entry
            .expect("a directory entry")
            .path()
            .join("recording.json");
        if !path.is_file() {
            continue;
        }
        let recording = read_json(&path);
        assert_eq!(
            recording["format"],
            "redpen-recording/1",
            "{}",
            path.display()
        );
        for e in recording["entries"].as_array().expect("entries") {
            let canonical = format!("POST /chat/completions\n{}", canonical_json(&e["request"]));
            assert_eq!(
                e["request_sha256"],
                json!(hex(&Sha256::digest(canonical.as_bytes()))),
                "{}",
                path.display()
            );
            assert_eq!(e["source"], "hand-written", "{}", path.display());
            assert_eq!(e["headers"], json!({}), "{}", path.display());
            checked += 1;
        }
    }
    assert!(checked >= 20, "{checked} recorded entries");
}

// ---------------------------------------------------------------------------------------------
// Test 3 (R5) and invariant 8: an unusable answer is refused, never cached, never half-applied.

/// The four bad bodies of R5.
const BAD_ANSWERS: [&str; 4] = [
    "{\"txt\": \"x\"}",
    "{\"text\": \"x\", \"extra\": 1}",
    "{\"text\": 5}",
    "hello",
];

// Bullet 3: missing `text`, an extra key, a wrong type and no JSON → `InvalidAnswer`, "The AI
// answer could not be used.", and no `ai_cache` row (the golden t03 cases).
#[tokio::test]
async fn t03_r5_unusable_answers_are_refused_and_not_cached() {
    check_rewrite_cases("t03-", 4).await;
}

// Invariant 8: after an unusable answer nothing is cached anywhere, so the same press is sent
// again, and its usable answer is the one used.
#[tokio::test]
async fn i8_an_unusable_answer_is_discarded_and_the_next_press_is_sent() {
    for content in BAD_ANSWERS {
        let e = env(vec![answer(content), text_answer(LIBRARY)]);
        let first = err(
            e.run(&fake(), req(Action::FixGrammar, LIBARY)).await,
            content,
        );
        assert!(
            matches!(first, RewriteError::Ai(AiError::InvalidAnswer)),
            "{content}: {first}"
        );
        assert_eq!(first.to_string(), MSG_INVALID, "{content}");
        assert_eq!(e.cached(), 0, "{content}: nothing cached");
        let second = ok(
            e.run(&fake(), req(Action::FixGrammar, LIBARY)).await,
            content,
        );
        assert!(!second.from_cache, "{content}: the press was sent again");
        assert_eq!(second.text, LIBRARY, "{content}");
        assert_eq!(e.sent().len(), 2, "{content}");
        assert_eq!(e.cached(), 1, "{content}: the usable answer is cached");
    }
}

// The schemas of §6: `text` of at least 1 and at most 40,000 characters, nothing else in a
// rewrite answer; Custom's `changes_language` is a required boolean.
#[tokio::test]
async fn e_answers_outside_their_schema_are_unusable() {
    let long = "a".repeat(40_001);
    let rewrite_bad = [
        json!({"text": ""}).to_string(),
        json!({ "text": long }).to_string(),
        json!({"text": null}).to_string(),
        json!(["x"]).to_string(),
        json!({"text": "x", "changes_language": false}).to_string(),
        "null".to_string(),
    ];
    for content in &rewrite_bad {
        let e = env(vec![answer(content)]);
        let error = err(e.run(&fake(), req(Action::Clearer, T62)).await, content);
        assert!(
            matches!(error, RewriteError::Ai(AiError::InvalidAnswer)),
            "{content}: {error}"
        );
        assert_eq!(e.cached(), 0, "{content}");
    }
    let custom_bad = [
        json!({"text": "x"}).to_string(),
        json!({"text": "x", "changes_language": "false"}).to_string(),
        json!({"text": "x", "changes_language": false, "note": "y"}).to_string(),
        json!({"text": "", "changes_language": false}).to_string(),
    ];
    for content in &custom_bad {
        let e = env(vec![answer(content)]);
        let r = custom(T62, "make it shorter");
        let error = err(e.run(&fake(), r).await, content);
        assert!(
            matches!(error, RewriteError::Ai(AiError::InvalidAnswer)),
            "{content}: {error}"
        );
        assert_eq!(e.cached(), 0, "{content}");
    }
}

// ---------------------------------------------------------------------------------------------
// Test 4 (R6): fences, preambles, wrapping quotes and whitespace.

// Bullet 4: a fenced answer, a preamble before the JSON (brace fallback), curly quotes around an
// unquoted original, and an original with two leading spaces and a trailing line break (the golden
// t04 cases, with a preamble line inside `text` and quotes kept around a quoted original).
#[tokio::test]
async fn t04_r6_fences_preambles_quotes_and_whitespace_are_cleaned() {
    check_rewrite_cases("t04-", 6).await;
}

// The rest of §6's parsing and cleaning rules: a bare fence, space around the fence, prose on both
// sides of the JSON, single and straight double quotes, each preamble opener, a preamble the
// original itself has, and whitespace other than spaces.
#[tokio::test]
async fn e_parsing_and_cleaning_follow_section_6() {
    let cases: [(&str, String, &str); 10] = [
        (
            "fine.",
            "```\n{\"text\": \"Fine.\"}\n```".to_string(),
            "Fine.",
        ),
        (
            "fine.",
            "  \n```json\n{\"text\": \"Fine.\"}\n```\n ".to_string(),
            "Fine.",
        ),
        (
            "fine.",
            "Here you go: {\"text\": \"Fine.\"} Hope it helps.".to_string(),
            "Fine.",
        ),
        ("fine.", json!({"text": "'Fine.'"}).to_string(), "Fine."),
        ("fine.", json!({"text": "\"Fine.\""}).to_string(), "Fine."),
        (
            "fine.",
            json!({"text": "Certainly, here it is:\nFine."}).to_string(),
            "Fine.",
        ),
        (
            "fine.",
            json!({"text": "Of course:\nFine."}).to_string(),
            "Fine.",
        ),
        (
            "fine.",
            json!({"text": "Here's the fix:  \nFine."}).to_string(),
            "Fine.",
        ),
        (
            "Here is the plan:\nwe meet at noon.",
            json!({"text": "Here is the plan:\nWe meet at noon."}).to_string(),
            "Here is the plan:\nWe meet at noon.",
        ),
        (
            "\n\tSee you soon.  ",
            json!({"text": "See you soon!"}).to_string(),
            "\n\tSee you soon!  ",
        ),
    ];
    for (original, content, expected) in &cases {
        let e = env(vec![answer(content)]);
        let o = ok(
            e.run(&fake(), req(Action::FixGrammar, original)).await,
            content,
        );
        assert_eq!(o.text, *expected, "{content}");
    }
}

// ---------------------------------------------------------------------------------------------
// Test 5 (R7): the minimal-edit guard.

// Bullet 5: 6 of 10 words changed warns "6 of 10" with `accept_default: false`; 3 of 5 does not
// (fewer than 4) (the golden t05 cases); 4 of 15, the §6.1 example (ratio 0.27), does not.
#[tokio::test]
async fn t05_r7_the_guard_fires_at_6_of_10_and_not_at_4_of_15_or_3_of_5() {
    check_rewrite_cases("t05-", 2).await;

    let e = env(vec![text_answer(A61)]);
    let original = T61.replace("[EMAIL_1]", "ana@example.com");
    let o = ok(
        e.run(&fake(), redacted(Action::FixGrammar, &original))
            .await,
        "4 of 15",
    );
    assert!(o.warnings.is_empty(), "4 of 15: {:?}", warnings(&o));
    assert!(o.accept_default, "4 of 15");
}

// `edit_ratio` counts changed and original words: each hunk counts max(deleted, inserted), tokens
// compare exactly (case-sensitive), punctuation is not a word.
#[test]
fn t05_edit_ratio_counts_hunks_over_word_tokens() {
    assert_eq!(api::edit_ratio(T61, A61), (4, 15));
    assert_eq!(api::edit_ratio(SIX_OLD, SIX_NEW), (6, 10));
    assert_eq!(
        api::edit_ratio("Me and him goes there.", "He and I go there."),
        (3, 5)
    );
    assert_eq!(api::edit_ratio(SIX_OLD, FOUR_NEW), (4, 10));
    assert_eq!(api::edit_ratio(TWENTY_OLD, TWENTY_NEW), (7, 20));
    assert_eq!(api::edit_ratio("a please b", "a Please b"), (1, 3));
    assert_eq!(api::edit_ratio(SIX_OLD, SIX_OLD), (0, 10));
    assert_eq!(api::edit_ratio("the cat sat", "the cat sat down"), (1, 3));
}

// The guard's thresholds: 4 of 10 (0.4, and 4 words) warns; 7 of 20 (0.35, not above) does not;
// and only Fix grammar has a guard.
#[tokio::test]
async fn e_guard_thresholds_and_fix_grammar_only() {
    let e = env(vec![text_answer(FOUR_NEW)]);
    let o = ok(
        e.run(&fake(), req(Action::FixGrammar, SIX_OLD)).await,
        "4 of 10",
    );
    assert_eq!(warnings(&o), ["guard 4 of 10"]);
    assert_eq!(
        api::warning_message(&o.warnings[0]),
        "This changes more than a grammar fix should (4 of 10 words). Check it before accepting."
    );
    assert!(!o.accept_default, "4 of 10");

    let e = env(vec![text_answer(TWENTY_NEW)]);
    let o = ok(
        e.run(&fake(), req(Action::FixGrammar, TWENTY_OLD)).await,
        "7 of 20",
    );
    assert!(o.warnings.is_empty(), "7 of 20: {:?}", warnings(&o));
    assert!(o.accept_default, "7 of 20");

    for name in ["clearer", "shorter", "more-formal", "friendlier"] {
        let e = env(vec![text_answer(SIX_NEW)]);
        let o = ok(e.run(&fake(), press(name, SIX_OLD)).await, name);
        assert!(
            !warnings(&o).iter().any(|w| w.starts_with("guard")),
            "{name}: {:?}",
            warnings(&o)
        );
    }
    let e = env(vec![custom_answer(SIX_NEW, false)]);
    let o = ok(e.run(&fake(), press("custom", SIX_OLD)).await, "custom");
    assert!(o.warnings.is_empty(), "custom: {:?}", warnings(&o));
}

// The `eval-rewrite` gate's Fix grammar corpus (docs/TESTING.md §4), scaled down to the full
// check: fix-grammar.jsonl has 100 sentences, each fixed by changing (or removing) one word, so its
// own correction as the answer changes 1 word of at least 8 (ratio at most 0.15, the gate's median
// limit), never warns, and is the default action.
#[tokio::test]
async fn e_the_gate_corpus_answered_with_its_corrections_never_warns() {
    let lines = jsonl("fix-grammar.jsonl");
    assert_eq!(lines.len(), 100, "docs/TESTING.md §4: 100 corpus sentences");
    let field = |line: &Value, key: &str| -> String {
        line[key]
            .as_str()
            .unwrap_or_else(|| panic!("fix-grammar.jsonl: no {key} in {line}"))
            .to_string()
    };
    let e = env(lines
        .iter()
        .map(|line| text_answer(&field(line, "corrected")))
        .collect());
    for line in &lines {
        let (id, text, corrected) = (
            field(line, "id"),
            field(line, "text"),
            field(line, "corrected"),
        );
        let dialect = match field(line, "dialect").as_str() {
            "american" => Dialect::American,
            "british" => Dialect::British,
            other => panic!("{id}: no dialect {other}"),
        };
        let (changed, total) = api::edit_ratio(&text, &corrected);
        assert_eq!(changed, 1, "{id}: one word fixes {text}");
        assert!(
            total >= 8 && changed * 100 <= 15 * total,
            "{id}: {changed} of {total}"
        );
        let o = ok(
            e.run(
                &fake(),
                RewriteRequest {
                    dialect,
                    ..req(Action::FixGrammar, &text)
                },
            )
            .await,
            &id,
        );
        assert_eq!(o.text, corrected, "{id}");
        assert!(o.warnings.is_empty(), "{id}: {:?}", warnings(&o));
        assert!(o.accept_default && !o.unchanged, "{id}");
    }
    assert_eq!(e.sent().len(), 100, "one request per sentence");
}

// ---------------------------------------------------------------------------------------------
// Test 6 (R8): language keeping.

// Bullet 6: an English original and a French answer, both over 40 characters → `Language
// {expected: "English", got: "French"}`, "The answer is in French, but your text is in English.",
// and the result is never the default action.
#[tokio::test]
async fn t06_r8_a_french_answer_to_english_text_warns() {
    let english = paragraph("en-1");
    let french = paragraph("fr-1");
    let e = env(vec![text_answer(&french)]);
    let o = ok(
        e.run(&fake(), req(Action::Clearer, &english)).await,
        "en-1 to fr-1",
    );
    assert_eq!(warnings(&o), ["language English to French"]);
    assert_eq!(
        api::warning_message(&o.warnings[0]),
        "The answer is in French, but your text is in English."
    );
    assert!(!o.accept_default);
    assert_eq!(o.text, french, "the result is still shown");
}

// Bullet 6: Custom with `changes_language: true` is not checked; with `false` it is.
#[tokio::test]
async fn t06_r8_custom_that_changes_language_is_not_checked() {
    let english = paragraph("en-1");
    let french = paragraph("fr-1");
    let e = env(vec![custom_answer(&french, true)]);
    let o = ok(
        e.run(&fake(), custom(&english, "translate to French"))
            .await,
        "changes_language true",
    );
    assert!(o.warnings.is_empty(), "{:?}", warnings(&o));
    assert_eq!(o.text, french);

    let e = env(vec![custom_answer(&french, false)]);
    let o = ok(
        e.run(&fake(), custom(&english, "translate to French"))
            .await,
        "changes_language false",
    );
    assert_eq!(warnings(&o), ["language English to French"]);
}

// Bullet 6: a 20-character answer is not checked.
#[tokio::test]
async fn t06_r8_a_20_character_answer_is_not_checked() {
    let short = "Merci beaucoup, ami.";
    assert_eq!(short.chars().count(), 20);
    let e = env(vec![text_answer(short)]);
    let o = ok(
        e.run(&fake(), req(Action::Clearer, &paragraph("en-1")))
            .await,
        "20 characters",
    );
    assert!(o.warnings.is_empty(), "{:?}", warnings(&o));
}

// Over languages.jsonl, whose 40 paragraphs whatlang 0.18.0 detects reliably: each paragraph
// answered by the same topic in the next language warns with whatlang's English names; answered
// by another paragraph in its own language, it does not.
#[tokio::test]
async fn t06_language_keeping_over_the_corpus() {
    let corpus = paragraphs();
    assert_eq!(corpus.len(), 40, "5 paragraphs in each of 8 languages");
    for (i, (id, lang, text)) in corpus.iter().enumerate() {
        let (other_id, other_lang, other_text) = &corpus[(i + 5) % 40];
        let e = env(vec![text_answer(other_text)]);
        let o = ok(e.run(&fake(), req(Action::Clearer, text)).await, id);
        assert_eq!(
            warnings(&o),
            [format!("language {lang} to {other_lang}")],
            "{id} answered by {other_id}"
        );

        let (same_id, same_lang, same_text) = &corpus[i - i % 5 + (i % 5 + 1) % 5];
        assert_eq!(same_lang, lang);
        let e = env(vec![text_answer(same_text)]);
        let o = ok(e.run(&fake(), req(Action::Clearer, text)).await, id);
        assert!(
            o.warnings.is_empty(),
            "{id} answered by {same_id}: {:?}",
            warnings(&o)
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Test 7 (R9): redaction.

// Bullet 7: every line of redaction.jsonl redacts to its expected text, one entity per marker,
// and round trips exactly (a repeated email, two emails, an email next to a phone number, the
// IBAN, the card, a 9-digit order number; the date and time are not redacted).
#[test]
fn t07_r9_redaction_round_trips_the_corpus() {
    let lines = jsonl("redaction.jsonl");
    assert!(lines.len() >= 19, "redaction.jsonl has its cases");
    for line in &lines {
        let id = line["id"].as_str().expect("an id");
        let text = line["text"].as_str().expect("a text");
        let (redacted_text, entities) = match api::redact(text) {
            Ok(redacted) => redacted,
            Err(e) => panic!("{id}: {e}"),
        };
        assert_eq!(redacted_text, line["redacted"], "{id}");
        assert_eq!(
            entities.len(),
            line["entities"].as_array().expect("entities").len(),
            "{id}: one entity per marker"
        );
        assert_eq!(
            api::restore(&redacted_text, &entities),
            text,
            "{id}: restore(redact(x)) == x"
        );
    }
}

// Restoring puts back every occurrence of a marker.
#[test]
fn t07_r9_restore_puts_back_every_occurrence() {
    let (_, entities) = match api::redact("Ask ana@example.com or ben@example.org for the keys.") {
        Ok(redacted) => redacted,
        Err(e) => panic!("{e}"),
    };
    assert_eq!(
        api::restore(
            "[EMAIL_2] wrote to [EMAIL_1], then [EMAIL_1] answered.",
            &entities
        ),
        "ben@example.org wrote to ana@example.com, then ana@example.com answered."
    );
}

// Bullet 7: the golden `redact` cases hold in the core: their redacted text and one entity per
// marker.
#[test]
fn t07_r9_redact_command_cases_hold_in_the_core() {
    let cases: Vec<Golden> = goldens("t07-")
        .into_iter()
        .filter(|g| g.command() == "redact")
        .collect();
    assert_eq!(cases.len(), 2, "the golden redact cases");
    for g in &cases {
        let text = read(&g.dir.join("input.txt"));
        let (redacted_text, entities) = match api::redact(&text) {
            Ok(redacted) => redacted,
            Err(e) => panic!("{}: {e}", g.name),
        };
        assert_eq!(redacted_text, g.expected["text"], "{}", g.name);
        assert_eq!(
            entities.len(),
            g.expected["entities"].as_array().expect("entities").len(),
            "{}",
            g.name
        );
    }
}

// Bullet 7: an answer that keeps the markers is restored; one missing [PHONE_1] is refused with
// `PlaceholderDropped`; one with [EMAIL_2], which was not sent, with `PlaceholderInvented` (the
// golden t07 rewrite cases).
#[tokio::test]
async fn t07_r9_markers_are_restored_and_dropped_or_invented_ones_refused() {
    check_rewrite_cases("t07-", 3).await;
}

// With redaction on, only the markers leave: neither the email nor the phone number is in the
// request; with it off, the text is sent as it is.
#[tokio::test]
async fn t07_with_redaction_on_only_markers_leave() {
    let text = "Please call me on +44 20 7946 0958 or write to ana@example.com tomorow.";
    let e = env(vec![text_answer(
        "Please call me on [PHONE_1] or write to [EMAIL_1] tomorrow.",
    )]);
    let o = ok(
        e.run(&fake(), redacted(Action::FixGrammar, text)).await,
        "redacted",
    );
    assert_eq!(
        o.text,
        "Please call me on +44 20 7946 0958 or write to ana@example.com tomorrow."
    );
    let sent = canonical_json(&body(&e.sent()[0]));
    assert!(!sent.contains("ana@example.com"), "{sent}");
    assert!(!sent.contains("7946"), "{sent}");
    assert!(
        sent.contains("[EMAIL_1]") && sent.contains("[PHONE_1]"),
        "{sent}"
    );

    let e = env(Vec::new());
    let _ = e.run(&fake(), req(Action::FixGrammar, text)).await;
    assert_eq!(
        user_content(&e.sent()[0]),
        format!("<text>\n{text}\n</text>")
    );
}

// The marker errors' messages, and neither error leaves anything cached.
#[tokio::test]
async fn e_marker_errors_have_their_messages_and_cache_nothing() {
    let text = "Please call me on +44 20 7946 0958 or write to ana@example.com tomorow.";
    let e = env(vec![text_answer(
        "Please call me on the phone or write to [EMAIL_1] tomorrow.",
    )]);
    let error = err(
        e.run(&fake(), redacted(Action::Clearer, text)).await,
        "dropped",
    );
    assert!(
        matches!(&error, RewriteError::PlaceholderDropped { marker } if marker == "[PHONE_1]"),
        "{error}"
    );
    assert_eq!(error.to_string(), MSG_DROPPED_PHONE);
    assert_eq!(e.cached(), 0);

    let e = env(vec![text_answer(
        "Please call me on [PHONE_1] or write to [EMAIL_1] or [NUMBER_1] tomorrow.",
    )]);
    let error = err(
        e.run(&fake(), redacted(Action::Clearer, text)).await,
        "invented",
    );
    assert!(
        matches!(&error, RewriteError::PlaceholderInvented { marker } if marker == "[NUMBER_1]"),
        "{error}"
    );
    assert_eq!(error.to_string(), MSG_INVENTED);
    assert_eq!(e.cached(), 0);
}

// ---------------------------------------------------------------------------------------------
// Test 8: a text that already holds a marker.

// Bullet 8: `[EMAIL_1]` (or any marker) in the text with redaction on → `RedactionCollision`, and
// no request; with redaction off the text is sent as it is.
#[tokio::test]
async fn t08_a_marker_in_the_text_is_refused_before_any_request() {
    check_rewrite_cases("t08-", 1).await;
    for text in [
        "Send it to [EMAIL_1] please.",
        "Call [PHONE_12] now.",
        "IBAN [IBAN_3] attached.",
        "[CARD_1]",
        "Order [NUMBER_7].",
    ] {
        assert!(
            matches!(api::redact(text), Err(RewriteError::RedactionCollision)),
            "redact({text:?})"
        );
        let e = env(Vec::new());
        let error = err(
            e.run(&fake(), redacted(Action::FixGrammar, text)).await,
            text,
        );
        assert!(
            matches!(error, RewriteError::RedactionCollision),
            "{text}: {error}"
        );
        assert_eq!(error.to_string(), MSG_COLLISION, "{text}");
        assert!(e.sent().is_empty(), "{text}: no request");
    }
    let e = env(Vec::new());
    let _ = e
        .run(
            &fake(),
            req(Action::FixGrammar, "Send it to [EMAIL_1] please."),
        )
        .await;
    assert_eq!(e.sent().len(), 1, "with redaction off the text is sent");
}

// ---------------------------------------------------------------------------------------------
// Test 9 (R10): tier, temperature and max_tokens.

// Bullet 9: the model is the tier `publik_tier` gives each action: `publik-fast` for Fix grammar,
// `publik-balanced` for the other five, `publik-smart` only with Best and a linked computer.
#[tokio::test]
async fn t09_r10_the_tier_follows_publik_tier_per_action() {
    let contexts = [
        (
            RewriteQuality::Standard,
            ClaimState::Anonymous,
            "publik-balanced",
        ),
        (
            RewriteQuality::Standard,
            ClaimState::Claimed,
            "publik-balanced",
        ),
        (
            RewriteQuality::Best,
            ClaimState::Anonymous,
            "publik-balanced",
        ),
        (RewriteQuality::Best, ClaimState::Claimed, "publik-smart"),
    ];
    for (quality, claim_state, five) in contexts {
        let ctx = TierContext {
            quality,
            claim_state,
        };
        for name in ACTIONS {
            let sent = sent_body(&publik(), &ctx, press(name, LIBARY)).await;
            let tier = if name == "fix-grammar" {
                "publik-fast"
            } else {
                five
            };
            assert_eq!(sent["model"], tier, "{name}, {quality:?}, {claim_state:?}");
        }
    }
}

// The model per provider (the spec's Design notes): the fake provider sends the tier as publik API
// does; the own key and a local model send their configured model.
#[tokio::test]
async fn t09_the_fake_provider_sends_the_tier_and_the_others_their_model() {
    let ctx = TierContext::default();
    for name in ACTIONS {
        let tier = if name == "fix-grammar" {
            "publik-fast"
        } else {
            "publik-balanced"
        };
        assert_eq!(
            sent_body(&fake(), &ctx, press(name, LIBARY)).await["model"],
            tier,
            "{name}"
        );
        assert_eq!(
            sent_body(&own_key(), &ctx, press(name, LIBARY)).await["model"],
            OWN_MODEL,
            "{name}"
        );
        assert_eq!(
            sent_body(&local(), &ctx, press(name, LIBARY)).await["model"],
            LOCAL_MODEL,
            "{name}"
        );
    }
}

// Bullet 9: temperature 0 (written as the integer 0) for Fix grammar and 0.4 for the others.
#[tokio::test]
async fn t09_r10_temperature_is_0_for_fix_grammar_and_0_4_for_the_others() {
    for name in ACTIONS {
        let sent = sent_body(&fake(), &TierContext::default(), press(name, LIBARY)).await;
        let expected = if name == "fix-grammar" {
            json!(0)
        } else {
            json!(0.4)
        };
        assert_eq!(sent["temperature"], expected, "{name}");
    }
}

// Bullet 9: `max_tokens` is clamp(2 * words + 100, 256, 4096), Custom + 150: 256 for 20 words,
// 300 for 100 and 3,100 for 1,500 (Custom 256, 350 and 3,150), with the clamp's edges.
#[tokio::test]
async fn t09_r10_max_tokens_follow_the_formula() {
    let table = [
        (20, 256, 256),
        (53, 256, 256),
        (54, 256, 258),
        (78, 256, 306),
        (79, 258, 308),
        (100, 300, 350),
        (1500, 3100, 3150),
    ];
    for (n, rewrite_tokens, custom_tokens) in table {
        let text = words_text(n);
        for name in ACTIONS {
            let sent = sent_body(&fake(), &TierContext::default(), press(name, &text)).await;
            let expected = if name == "custom" {
                custom_tokens
            } else {
                rewrite_tokens
            };
            assert_eq!(sent["max_tokens"], expected, "{name}, {n} words");
        }
    }
}

// Words are UAX #29 words with a letter or a digit: punctuation, dashes and emoji are not words;
// a contraction, a decimal and a domain are one word each, a hyphenated word two.
#[tokio::test]
async fn t09_words_count_only_segments_with_a_letter_or_a_digit() {
    let text = (1..=100)
        .map(|i| {
            if i % 10 == 0 {
                format!("w{i}. \u{1F44D}")
            } else {
                format!("w{i}")
            }
        })
        .collect::<Vec<_>>()
        .join(" - ");
    let sent = sent_body(
        &fake(),
        &TierContext::default(),
        press("fix-grammar", &text),
    )
    .await;
    assert_eq!(sent["max_tokens"], 300, "100 words");
    for (x, n) in [
        ("don't", 1),
        ("example.com", 1),
        ("3.14", 1),
        ("e-mail", 2),
        ("[EMAIL_1]", 1),
        ("w1 - w2", 2),
        ("w1 \u{2014} w2!", 2),
    ] {
        assert_eq!(api::edit_ratio(x, x), (0, n), "{x}");
    }
}

// ---------------------------------------------------------------------------------------------
// Test 10: limits, refused before any request.

// Bullet 10: 1,500 words → a request; 1,501 → `TooLong` naming 1,501, no request; whitespace
// only → `Empty`; a 301-character instruction → `InstructionTooLong` (and 300 is sent).
#[tokio::test]
async fn t10_limits_are_checked_before_any_request() {
    check_rewrite_cases("t10-", 3).await;
    let ctx = TierContext::default();

    let e = env(Vec::new());
    let _ = e
        .run(&fake(), req(Action::FixGrammar, &words_text(1500)))
        .await;
    assert_eq!(e.sent().len(), 1, "1,500 words are sent");

    for name in ACTIONS {
        let e = env(Vec::new());
        let error = err(e.run(&fake(), press(name, &words_text(1501))).await, name);
        assert!(
            matches!(error, RewriteError::TooLong { words: 1501 }),
            "{name}: {error}"
        );
        assert_eq!(
            error.to_string(),
            "Select at most 1,500 words for a rewrite (this selection has 1,501).",
            "{name}"
        );
        assert!(e.sent().is_empty(), "{name}: no request");
        assert!(
            matches!(
                api::build_request(&fake(), &ctx, &press(name, &words_text(1501))),
                Err(RewriteError::TooLong { words: 1501 })
            ),
            "{name}: build_request refuses it too"
        );
    }

    for text in ["", " ", "  \n\t \r\n", "\n"] {
        let e = env(Vec::new());
        let error = err(e.run(&fake(), req(Action::Clearer, text)).await, text);
        assert!(matches!(error, RewriteError::Empty), "{text:?}: {error}");
        assert_eq!(error.to_string(), MSG_EMPTY);
        assert!(e.sent().is_empty(), "{text:?}: no request");
    }

    // Characters, not bytes: 300 'é' are 600 bytes and still allowed.
    for (instruction, allowed) in [
        ("a".repeat(301), false),
        ("é".repeat(301), false),
        ("a".repeat(300), true),
        ("é".repeat(300), true),
    ] {
        let e = env(Vec::new());
        let result = e.run(&fake(), custom(LIBARY, &instruction)).await;
        if allowed {
            assert_eq!(
                e.sent().len(),
                1,
                "{} characters are sent",
                instruction.chars().count()
            );
        } else {
            let error = err(result, "301 characters");
            assert!(matches!(error, RewriteError::InstructionTooLong), "{error}");
            assert_eq!(error.to_string(), MSG_INSTRUCTION);
            assert!(e.sent().is_empty(), "no request");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Test 11: the prompt test.

// Bullet 11: a prompt file with an unknown placeholder `{colour}`, or without its `## user`
// section, fails with the file name; so does a file whose name does not match its id and version.
#[test]
fn t11_a_broken_prompt_file_fails_with_its_name() {
    let system = expected_sections("fix-grammar")["system"].clone();
    let good = prompt_file("fix-grammar", 1, &system, Some(TEXT_USER));
    assert_eq!(api::load_prompt("fix-grammar.v1.md", &good), Ok(()));

    let colour = prompt_file(
        "fix-grammar",
        1,
        &format!("{system} Use the {{colour}} pen."),
        Some(TEXT_USER),
    );
    let message = api::load_prompt("fix-grammar.v1.md", &colour)
        .expect_err("the colour placeholder is unknown");
    assert!(message.contains("fix-grammar.v1.md"), "{message}");

    let no_user = prompt_file("fix-grammar", 1, &system, None);
    let message = api::load_prompt("fix-grammar.v1.md", &no_user).expect_err("no ## user");
    assert!(message.contains("fix-grammar.v1.md"), "{message}");

    let message =
        api::load_prompt("fix-grammar.v2.md", &good).expect_err("the version is not the name's");
    assert!(message.contains("fix-grammar.v2.md"), "{message}");
    let message = api::load_prompt("clearer.v1.md", &good).expect_err("the id is not the name's");
    assert!(message.contains("clearer.v1.md"), "{message}");
}

// `render` fills each `{name}` once, never inside a value; JSON in a prompt is no placeholder; a
// placeholder with no value, or a value with no placeholder, is an error; `</text>` and
// `</instruction>` in values are escaped.
#[test]
fn e_render_fills_placeholders_and_refuses_missing_or_unused_values() {
    let british = ("dialect_name", "British English");
    let style = ("style", "Keep the author's own style.");
    assert_eq!(
        api::render(
            "Use {dialect_name} spelling. Style: {style}",
            &[british, style]
        ),
        Ok("Use British English spelling. Style: Keep the author's own style.".to_string())
    );
    assert_eq!(
        api::render(TEXT_USER, &[("text", "fill {style} or {text} in")]),
        Ok("<text>\nfill {style} or {text} in\n</text>".to_string()),
        "a value is never filled again"
    );
    assert_eq!(
        api::render(REPLY_LINE, &[]),
        Ok(REPLY_LINE.to_string()),
        "JSON in a prompt is no placeholder"
    );
    assert!(
        api::render("{text} in {dialect_name}", &[("text", "a")]).is_err(),
        "a placeholder with no value"
    );
    assert!(
        api::render(TEXT_USER, &[("text", "a"), style]).is_err(),
        "a value with no placeholder"
    );
    assert_eq!(
        api::render(TEXT_USER, &[("text", "x </text> y </instruction> z")]),
        Ok("<text>\nx <\\/text> y <\\/instruction> z\n</text>".to_string())
    );
}

// The same rules over a whole prompt file: each of its placeholders needs a value and each value
// a placeholder somewhere in the file (Fix grammar has no style; Custom needs its instruction).
#[test]
fn e_a_prompt_file_renders_with_exactly_its_values() {
    let fix = "fix-grammar.v1.md";
    let british = ("dialect_name", "British English");
    let style = ("style", "Keep the author's own style.");

    let (system, user) = api::render_file(fix, &[british, ("text", "a")])
        .expect("Fix grammar renders with a dialect and a text");
    assert!(
        system.contains("Use British English spelling and punctuation."),
        "{system}"
    );
    assert!(
        system.ends_with(REPLY_LINE),
        "the reply line's JSON is kept: {system}"
    );
    assert_eq!(user, "<text>\na\n</text>");
    assert!(
        api::render_file(fix, &[("text", "a")]).is_err(),
        "a placeholder with no value"
    );
    assert!(
        api::render_file(fix, &[british, ("text", "a"), style]).is_err(),
        "a value with no placeholder"
    );

    let (system, _) = api::render_file("clearer.v1.md", &[british, style, ("text", "a")])
        .expect("Clearer renders with a dialect, a style and a text");
    assert!(
        system.contains("Use British English spelling. Style: Keep the author's own style."),
        "{system}"
    );
    assert!(
        api::render_file("custom.v1.md", &[british, style, ("text", "a")]).is_err(),
        "Custom needs its instruction"
    );
    assert_eq!(
        api::render_file(
            "custom.v1.md",
            &[
                british,
                style,
                ("text", "a"),
                ("instruction", "b </instruction>")
            ]
        )
        .map(|(_, user)| user),
        Ok("<instruction>\nb <\\/instruction>\n</instruction>\n\n<text>\na\n</text>".to_string())
    );
}

// Custom without an instruction is refused before any request.
#[tokio::test]
async fn e_custom_without_an_instruction_is_refused_before_any_request() {
    let e = env(Vec::new());
    let error = err(
        e.run(&fake(), req(Action::Custom, T62)).await,
        "no instruction",
    );
    assert!(matches!(error, RewriteError::InstructionMissing), "{error}");
    assert_eq!(error.to_string(), MSG_INSTRUCTION_MISSING);
    assert_eq!(
        error_json(&error),
        json!({"kind": "InstructionMissing", "message": MSG_INSTRUCTION_MISSING})
    );
    assert!(e.sent().is_empty(), "no request");
}

// ---------------------------------------------------------------------------------------------
// Test 12: closing tags in values.

// Bullet 12: `</text>` in the text is sent as `<\/text>` and restored in the result (the golden
// t12 case); in Custom, both tags in both values.
#[tokio::test]
async fn t12_closing_tags_are_sent_escaped_and_restored() {
    check_rewrite_cases("t12-", 1).await;

    let e = env(vec![custom_answer(
        "Say <\\/instruction> and <\\/text> twice.",
        false,
    )]);
    let o = ok(
        e.run(
            &fake(),
            custom(
                "Say </instruction> and </text>.",
                "keep </instruction> and </text> as they are",
            ),
        )
        .await,
        "custom",
    );
    assert_eq!(
        user_content(&e.sent()[0]),
        "<instruction>\nkeep <\\/instruction> and <\\/text> as they are\n</instruction>\n\n<text>\nSay <\\/instruction> and <\\/text>.\n</text>"
    );
    assert_eq!(o.text, "Say </instruction> and </text> twice.");
}

// ---------------------------------------------------------------------------------------------
// Test 13: unchanged.

// Bullet 13: an answer equal to the original, also once cleaned → `unchanged: true`, no warnings,
// and a diff with no insertion or deletion.
#[tokio::test]
async fn t13_an_answer_equal_to_the_original_is_unchanged() {
    let cases = [
        (
            "The cat sat on the mat.",
            json!({"text": "The cat sat on the mat."}),
        ),
        (
            "The cat sat on the mat.",
            json!({"text": "\u{201c}The cat sat on the mat.\u{201d}"}),
        ),
        (
            "  The cat sat on the mat.\n",
            json!({"text": "The cat sat on the mat."}),
        ),
    ];
    for (original, content) in &cases {
        for name in ["fix-grammar", "clearer"] {
            let e = env(vec![answer(&content.to_string())]);
            let o = ok(e.run(&fake(), press(name, original)).await, original);
            assert!(o.unchanged, "{name}: {original:?}");
            assert!(o.warnings.is_empty(), "{name}: {:?}", warnings(&o));
            assert_eq!(o.text, *original, "{name}");
            assert!(
                o.diff.iter().all(|op| matches!(op, DiffOp::Equal(_))),
                "{name}: {}",
                diff_json(&o.diff)
            );
        }
    }
    let e = env(vec![text_answer("The cat sat on a mat.")]);
    let o = ok(
        e.run(&fake(), req(Action::FixGrammar, "The cat sat on the mat."))
            .await,
        "changed",
    );
    assert!(!o.unchanged);
}

// ---------------------------------------------------------------------------------------------
// Test 14: not shorter.

// Bullet 14: Shorter with an answer longer than the original, or as long in words → `NotShorter`;
// a shorter answer, or another action, has no such warning.
#[tokio::test]
async fn t14_shorter_warns_when_the_answer_is_not_shorter() {
    let original = "Please send the file today.";
    for (answer_text, warns) in [
        ("Please send the whole file to me today.", true),
        ("Kindly send the file today.", true),
        ("Send the file today.", false),
    ] {
        let e = env(vec![text_answer(answer_text)]);
        let o = ok(
            e.run(&fake(), req(Action::Shorter, original)).await,
            answer_text,
        );
        if warns {
            assert_eq!(warnings(&o), ["not shorter"], "{answer_text}");
            assert_eq!(api::warning_message(&o.warnings[0]), MSG_NOT_SHORTER);
        } else {
            assert!(o.warnings.is_empty(), "{answer_text}: {:?}", warnings(&o));
        }
    }
    for name in ["clearer", "more-formal", "friendlier"] {
        let e = env(vec![text_answer("Please send the whole file to me today.")]);
        let o = ok(e.run(&fake(), press(name, original)).await, name);
        assert!(o.warnings.is_empty(), "{name}: {:?}", warnings(&o));
    }
}

// ---------------------------------------------------------------------------------------------
// Test 15: the word diff.

// Bullet 15: `the cat sat` to `the dog sat` → the golden t15 diff, and `edit_ratio` → (1, 3).
#[tokio::test]
async fn t15_the_diff_of_the_cat_sat_and_its_edit_ratio() {
    let cases = goldens("t15-");
    assert_eq!(cases.len(), 1);
    let e = env(vec![text_answer("the dog sat")]);
    let o = ok(
        e.run(&fake(), req(Action::FixGrammar, "the cat sat")).await,
        "the cat sat",
    );
    assert_eq!(diff_json(&o.diff), cases[0].expected);
    assert_eq!(
        cases[0].expected,
        json!([{"equal": "the "}, {"delete": "cat"}, {"insert": "dog"}, {"equal": " sat"}])
    );
    assert_eq!(api::edit_ratio("the cat sat", "the dog sat"), (1, 3));
}

// ---------------------------------------------------------------------------------------------
// Test 16 (R7b): the cache key, variants and Try again.

// Bullet 16: `variant` 1 changes K for Clearer; Fix grammar has no variant (its cache input is
// `{text, dialect}` only); Custom's adds the instruction; the text is the redacted one.
#[test]
fn t16_the_cache_input_of_each_action_and_variant() {
    let p = publik();
    let ctx = TierContext::default();
    let call = |r: &RewriteRequest| ok_call(api::build_request(&p, &ctx, r), "build_request");

    let clearer_0 = call(&req(Action::Clearer, T62));
    let clearer_1 = call(&RewriteRequest {
        variant: 1,
        ..req(Action::Clearer, T62)
    });
    let keys = |v: &Value| {
        v.as_object().map(|o| {
            let mut names: Vec<String> = o.keys().cloned().collect();
            names.sort();
            names
        })
    };
    assert_eq!(
        keys(&clearer_0.cache_input),
        Some(vec![
            "dialect".to_string(),
            "style".to_string(),
            "text".to_string(),
            "variant".to_string()
        ])
    );
    assert_eq!(clearer_0.cache_input["text"], T62);
    assert_eq!(clearer_0.cache_input["dialect"], "british");
    assert_eq!(clearer_0.cache_input["variant"], 0);
    assert_eq!(clearer_1.cache_input["variant"], 1);
    assert_ne!(
        cache_key("publik", "publik-balanced", &clearer_0),
        cache_key("publik", "publik-balanced", &clearer_1),
        "variant 1 is another K"
    );

    let fix_0 = call(&req(Action::FixGrammar, LIBARY));
    let fix_1 = call(&RewriteRequest {
        variant: 1,
        ..req(Action::FixGrammar, LIBARY)
    });
    assert_eq!(
        fix_0.cache_input,
        json!({"text": LIBARY, "dialect": "british"})
    );
    assert_eq!(
        fix_1.cache_input, fix_0.cache_input,
        "Fix grammar has no variant"
    );
    assert_eq!(cache_key("publik", "publik-fast", &fix_0), K_LIBARY);

    let custom_0 = call(&custom(T62, "make it shorter"));
    assert_eq!(
        keys(&custom_0.cache_input),
        Some(vec![
            "dialect".to_string(),
            "instruction".to_string(),
            "style".to_string(),
            "text".to_string(),
            "variant".to_string()
        ])
    );
    assert_eq!(custom_0.cache_input["instruction"], "make it shorter");

    let email = call(&redacted(
        Action::FixGrammar,
        &T61.replace("[EMAIL_1]", "ana@example.com"),
    ));
    assert_eq!(
        email.cache_input,
        json!({"text": T61, "dialect": "british"})
    );
    assert_eq!(email.prompt_id, "fix-grammar");
    assert_eq!(email.prompt_version, 1);
}

// Bullet 16 (R7b): after an invalid answer, Try again for Fix grammar keeps K and sends the
// idempotency key of SHA-256(`<K>:1`), a second invalid answer then `<K>:2`; the valid answer is
// cached under K, so the next press (and variant 1, the same K) comes from the cache.
#[tokio::test]
async fn t16_r7b_try_again_after_invalid_answers_for_fix_grammar() {
    let e = env(vec![
        answer(BAD_ANSWERS[0]),
        answer(BAD_ANSWERS[3]),
        text_answer(LIBRARY),
    ]);
    let p = publik();
    let try_again = |n: u32| RewriteRequest {
        validation_retry: n,
        ..req(Action::FixGrammar, LIBARY)
    };

    let first = err(e.run(&p, try_again(0)).await, "the first answer");
    assert!(
        matches!(first, RewriteError::Ai(AiError::InvalidAnswer)),
        "{first}"
    );
    let second = err(e.run(&p, try_again(1)).await, "the second answer");
    assert!(
        matches!(second, RewriteError::Ai(AiError::InvalidAnswer)),
        "{second}"
    );
    assert_eq!(e.cached(), 0);
    let third = ok(e.run(&p, try_again(2)).await, "the third answer");
    assert_eq!(third.text, LIBRARY);
    assert!(!third.from_cache);
    assert_eq!(e.cached(), 1, "the valid answer is cached");

    let sent = e.sent();
    let keys: Vec<String> = sent.iter().map(idempotency).collect();
    assert_eq!(keys, [IDEM_0, IDEM_1, IDEM_2]);
    assert_eq!(
        body(&sent[1]),
        body(&sent[0]),
        "the same request: K is kept"
    );
    assert_eq!(
        body(&sent[2]),
        body(&sent[0]),
        "the same request: K is kept"
    );

    let again = ok(e.run(&p, try_again(0)).await, "the next press");
    assert!(again.from_cache, "cached under K");
    assert_eq!(again.text, LIBRARY);
    let variant = ok(
        e.run(
            &p,
            RewriteRequest {
                variant: 1,
                ..req(Action::FixGrammar, LIBARY)
            },
        )
        .await,
        "variant 1",
    );
    assert!(variant.from_cache, "Fix grammar has no variant: the same K");
    assert_eq!(e.sent().len(), 3, "nothing more was sent");
}

// Bullet 16 (R7b): after a 503 the Try again sends K's own key.
#[tokio::test]
async fn t16_r7b_try_again_after_a_503_sends_ks_own_key() {
    let e = env(vec![busy(), text_answer(LIBRARY)]);
    let p = publik();
    let first = err(e.run(&p, req(Action::FixGrammar, LIBARY)).await, "503");
    assert!(
        matches!(first, RewriteError::Ai(AiError::PublikBusy)),
        "{first}"
    );
    assert_eq!(e.cached(), 0);
    let second = ok(
        e.run(&p, req(Action::FixGrammar, LIBARY)).await,
        "Try again",
    );
    assert_eq!(second.text, LIBRARY);
    let keys: Vec<String> = e.sent().iter().map(idempotency).collect();
    assert_eq!(keys, [IDEM_0, IDEM_0]);
}

// Bullet 16: the same for Clearer, whose K is computed from its cache input and the tier it sends;
// variant 1 is another K, so it is sent with that K's own key.
#[tokio::test]
async fn t16_r7b_try_again_and_another_version_for_clearer() {
    let p = publik();
    let ctx = TierContext::default();
    let k = |variant: u32| {
        let r = RewriteRequest {
            variant,
            ..req(Action::Clearer, T62)
        };
        cache_key(
            "publik",
            "publik-balanced",
            &ok_call(api::build_request(&p, &ctx, &r), "build_request"),
        )
    };
    let (k0, k1) = (k(0), k(1));

    let another =
        "With regard to the invoice we discussed on the phone last week, please pay it soon.";
    let e = env(vec![
        answer(BAD_ANSWERS[1]),
        answer(BAD_ANSWERS[2]),
        text_answer(A62),
        text_answer(another),
    ]);
    let try_again = |n: u32| RewriteRequest {
        validation_retry: n,
        ..req(Action::Clearer, T62)
    };
    let _ = err(e.run(&p, try_again(0)).await, "the first answer");
    let _ = err(e.run(&p, try_again(1)).await, "the second answer");
    let third = ok(e.run(&p, try_again(2)).await, "the third answer");
    assert_eq!(third.text, A62);
    let next = ok(
        e.run(
            &p,
            RewriteRequest {
                variant: 1,
                ..req(Action::Clearer, T62)
            },
        )
        .await,
        "Another version",
    );
    assert!(!next.from_cache, "variant 1 is another K");
    assert_eq!(next.text, another);

    let sent = e.sent();
    for request in &sent {
        assert_eq!(body(request)["model"], "publik-balanced");
    }
    let keys: Vec<String> = sent.iter().map(idempotency).collect();
    assert_eq!(
        keys,
        [
            idempotency_key(&k0, 0),
            idempotency_key(&k0, 1),
            idempotency_key(&k0, 2),
            idempotency_key(&k1, 0),
        ]
    );
    assert_eq!(e.cached(), 2, "each version is cached under its own K");
}

// ---------------------------------------------------------------------------------------------
// Test 17: the eval-rewrite gate.

// Bullet 17: `scripts/gates.sh eval-rewrite` (and gates.ps1) prints the number of calls and the
// tiers and asks y/N; on `n` it exits 0 having sent nothing: both providers point at a listener
// on this computer that must see no connection.
#[test]
fn t17_eval_rewrite_asks_first_and_n_sends_nothing() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    listener
        .set_nonblocking(true)
        .expect("a non-blocking listener");
    let port = listener.local_addr().expect("its address").port();

    let runs = gate_runs(port);
    assert!(
        !runs.is_empty(),
        "neither sh nor PowerShell is installed to run a gate script"
    );
    for (script, code, said) in &runs {
        assert_eq!(*code, Some(0), "{script} eval-rewrite, n: {said}");
        assert!(
            said.contains("publik-fast") && said.contains("publik-balanced"),
            "{script}: the tiers: {said}"
        );
        assert!(
            said.contains("calls") && said.bytes().any(|b| b.is_ascii_digit()),
            "{script}: the number of calls: {said}"
        );
        assert!(said.contains("y/N"), "{script}: the question: {said}");
    }
    match listener.accept() {
        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
        Ok((_, from)) => panic!("eval-rewrite connected from {from} after the answer n"),
        Err(e) => panic!("the listener failed: {e}"),
    }
}

// The Scope's gate is docs/TESTING.md §4's: Fix grammar on 100 corpus sentences (publik-fast) and
// 40 paragraphs × 5 actions (publik-balanced), so it announces 100 and 200 calls, 300 in all.
#[test]
fn e_eval_rewrite_announces_the_calls_of_testing_section_4() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    listener
        .set_nonblocking(true)
        .expect("a non-blocking listener");
    let port = listener.local_addr().expect("its address").port();
    let runs = gate_runs(port);
    assert!(!runs.is_empty(), "no shell ran a gate script");
    for (script, _, said) in &runs {
        for count in ["100", "200", "300"] {
            assert!(
                said.contains(count),
                "{script}: docs/TESTING.md §4 makes {count} calls: {said}"
            );
        }
    }
}

/// The gate scripts start the CLI with `cargo run -p redpen-cli`, which builds it with the CLI's
/// own set of dependency features, not the workspace's. On a machine that has never built it that
/// way the build takes minutes, so it happens here, once and untimed: what `GATE_TIMEOUT` times
/// is the gate's answer to `n`.
fn build_the_cli_once() {
    static BUILT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    BUILT.get_or_init(|| {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let status = Command::new(cargo)
            .args(["build", "-q", "-p", "redpen-cli"])
            .current_dir(repo_root())
            .status()
            .expect("cargo could not be started to build redpen-cli");
        assert!(status.success(), "cargo build -p redpen-cli failed");
    });
}

/// `eval-rewrite` run with each installed shell, answering `n`, with both providers pointing at
/// `port`: `(script, exit code, what it printed)`.
fn gate_runs(port: u16) -> Vec<(String, Option<i32>, String)> {
    build_the_cli_once();
    let (powershell, flags): (&str, &[&str]) = if cfg!(windows) {
        (
            "powershell",
            &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"],
        )
    } else {
        ("pwsh", &["-NoProfile", "-File"])
    };
    [
        ("sh", &[][..], "scripts/gates.sh"),
        (powershell, flags, "scripts/gates.ps1"),
    ]
    .into_iter()
    .filter_map(|(shell, flags, script)| {
        run_gate(shell, flags, script, port).map(|(code, said)| (script.to_string(), code, said))
    })
    .collect()
}

/// Runs `script eval-rewrite` with `shell` and answers `n`: its exit code and what it printed,
/// or `None` when the shell is not installed.
fn run_gate(shell: &str, flags: &[&str], script: &str, port: u16) -> Option<(Option<i32>, String)> {
    let spawned = Command::new(shell)
        .args(flags)
        .arg(script)
        .arg("eval-rewrite")
        .current_dir(repo_root())
        .env("PUBLIK_API_KEY", KEY)
        .env(
            "PUBLIK_API_BASE_URL",
            format!("http://127.0.0.1:{port}/api/v1"),
        )
        .env("REDPEN_OWN_KEY", OWN_KEY)
        .env("REDPEN_OWN_BASE_URL", format!("http://127.0.0.1:{port}/v1"))
        .env("REDPEN_OWN_MODEL", OWN_MODEL)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("{shell} is not installed here; {script} is not run");
            return None;
        }
        Err(e) => panic!("{shell} could not be started: {e}"),
    };
    if let Some(mut stdin) = child.stdin.take() {
        // The script may end before it reads; a closed pipe is no failure here.
        let _ = stdin.write_all(b"n\n");
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + GATE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                panic!(
                    "{script} eval-rewrite did not end within {} s of the answer n: the gate must print the calls and the tier and ask before it builds or sends anything",
                    GATE_TIMEOUT.as_secs()
                );
            }
            Err(e) => panic!("{script}: {e}"),
        }
    };
    let said = format!("{}{}", joined(stdout), joined(stderr)).replace("\r\n", "\n");
    Some((status.code(), said))
}

/// Reads a child's pipe to the end on its own thread, so a full pipe never blocks the child.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_string(&mut text);
        }
        text
    })
}

fn joined(handle: JoinHandle<String>) -> String {
    handle.join().unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// The spec's messages.

// Every refusal and warning reads as the spec's Interfaces and docs/PUBLIK-INTEGRATION.md §6 say.
#[test]
fn e_refusal_and_warning_messages_are_the_spec_strings() {
    let refusals = [
        (RewriteError::Empty, MSG_EMPTY.to_string()),
        (
            RewriteError::TooLong { words: 1812 },
            "Select at most 1,500 words for a rewrite (this selection has 1,812).".to_string(),
        ),
        (RewriteError::InstructionTooLong, MSG_INSTRUCTION.to_string()),
        (
            RewriteError::InstructionMissing,
            MSG_INSTRUCTION_MISSING.to_string(),
        ),
        (RewriteError::RedactionCollision, MSG_COLLISION.to_string()),
        (
            RewriteError::PlaceholderDropped {
                marker: "[PHONE_1]".to_string(),
            },
            MSG_DROPPED_PHONE.to_string(),
        ),
        (
            RewriteError::PlaceholderDropped {
                marker: "[EMAIL_2]".to_string(),
            },
            "The AI answer dropped [EMAIL_2], so it was not used. Try again, or turn off redaction for this text.".to_string(),
        ),
        (
            RewriteError::PlaceholderInvented {
                marker: "[EMAIL_2]".to_string(),
            },
            MSG_INVENTED.to_string(),
        ),
        (RewriteError::Ai(AiError::InvalidAnswer), MSG_INVALID.to_string()),
    ];
    for (error, message) in &refusals {
        assert_eq!(error.to_string(), *message);
    }
    let shown = [
        (
            Warning::Guard {
                changed: 9,
                total: 14,
            },
            "This changes more than a grammar fix should (9 of 14 words). Check it before accepting.",
        ),
        (
            Warning::Language {
                expected: "English".to_string(),
                got: "French".to_string(),
            },
            "The answer is in French, but your text is in English.",
        ),
        (Warning::NotShorter, MSG_NOT_SHORTER),
    ];
    for (warning, message) in &shown {
        assert_eq!(api::warning_message(warning), *message);
    }
}

// A rewrite's request waits as long as module 02 says: 30 seconds, and 120 on a local model.
#[test]
fn e_build_request_uses_the_chat_timeout_of_the_provider() {
    let ctx = TierContext::default();
    for (p, seconds) in [
        (publik(), 30),
        (own_key(), 30),
        (fake(), 30),
        (local(), 120),
    ] {
        let call = ok_call(
            api::build_request(&p, &ctx, &req(Action::Clearer, T62)),
            "build_request",
        );
        assert_eq!(call.timeout, Duration::from_secs(seconds), "{:?}", p.kind);
    }
}
