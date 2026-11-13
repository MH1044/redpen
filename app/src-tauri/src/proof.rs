//! The maintainer's proof (`scripts/publik-proof.sh`, `src/bin/publik-proof.rs`): one real
//! publik API call, as the setup skill's proof call does (contract §11). It spends money, so it
//! runs only when the maintainer starts it, and asks first.
//!
//! - Without an app token it prints the `NoAppToken` message and exits 2, sending nothing.
//! - With no key on this computer it shows [`DISCLOSURE`] and asks y/N before provisioning.
//! - By default: one `publik-fast` call, "Reply with the word ok."; it prints the reply, the
//!   `x-publik-charge-micros` line and the names (never the values) of the `x-publik-*` headers
//!   received. It never prints the key or the balance.
//! - `--models`: the model ids of `GET /models`.
//! - `--probe-json`: whether `response_format: {"type": "json_object"}` is honoured.
//! - `--search`: after its own y/N, one web search (one cent) for `"largest coral reef system"`;
//!   the first result's title and site, and the charge.

use std::io::{BufRead, Write};
use std::time::Duration;

use redpen_core::net::{HeaderValue, HttpRequest, HttpResponse, HttpTransport};
use redpen_core::provider::client::{CHAT_PATH, IDEMPOTENCY_HEADER, MODELS_PATH, chat_body};
use redpen_core::provider::tier::PUBLIK_FAST;
use redpen_core::provider::{
    AiError, CachePolicy, ChatCall, ExposeSecret, Feature, Message, cache_key, idempotency_key,
    map_publik_error,
};
use serde_json::{Value, json};

use crate::publik::{
    Credential, DATA_PATH, DISCLOSURE, PublikOptions, Resolution, provision_with, search,
};

/// What the default call asks.
pub const PROOF_MESSAGE: &str = "Reply with the word ok.";
/// What `--probe-json` asks.
pub const PROBE_MESSAGE: &str = "Reply with a JSON object with one key, \"ok\", set to true.";
/// The one search of `--search`.
pub const SEARCH_QUERY: &str = "\"largest coral reef system\"";
/// The command line.
pub const USAGE: &str = "usage: publik-proof [--models] [--probe-json] [--search]";

/// Which checks to run. With none, the one `publik-fast` call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProofOptions {
    pub models: bool,
    pub probe_json: bool,
    pub search: bool,
}

/// Reads the arguments after the program name.
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<ProofOptions, String> {
    let mut o = ProofOptions::default();
    for arg in args {
        match arg.as_str() {
            "--models" => o.models = true,
            "--probe-json" => o.probe_json = true,
            "--search" => o.search = true,
            other => return Err(format!("Unknown argument {other:?}. {USAGE}")),
        }
    }
    Ok(o)
}

/// Runs the proof over `t`, reading answers from `input` and writing to `out`. Returns the exit
/// code: 0 done, 1 declined or failed, 2 no app token.
pub async fn run(
    t: &dyn HttpTransport,
    opts: &ProofOptions,
    publik: &PublikOptions,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> i32 {
    match run_inner(t, opts, publik, input, out).await {
        Ok(()) => 0,
        Err(code) => code,
    }
}

async fn run_inner(
    t: &dyn HttpTransport,
    opts: &ProofOptions,
    publik: &PublikOptions,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<(), i32> {
    if publik.app_token.is_none() {
        say(out, &AiError::NoAppToken.to_string());
        return Err(2);
    }
    let cred = match publik.resolve(None) {
        Resolution::Env(c) | Resolution::File(c) => c,
        Resolution::OwnKey(_) | Resolution::NeedsConsent => {
            say(out, DATA_PATH);
            say(out, DISCLOSURE);
            if !ask(
                input,
                out,
                "Set up this computer with publik API now? [y/N] ",
            ) {
                say(out, "Nothing was sent.");
                return Err(1);
            }
            provision_with(t, publik).await.map_err(|e| {
                say(out, &format!("error: {e}"));
                1
            })?
        }
    };
    if !opts.models && !opts.probe_json && !opts.search {
        return one_call(t, &cred, out).await;
    }
    if opts.models {
        models(t, &cred, out).await?;
    }
    if opts.probe_json {
        probe_json(t, &cred, out).await?;
    }
    if opts.search {
        one_search(t, &cred, input, out).await?;
    }
    Ok(())
}

/// The one `publik-fast` call.
async fn one_call(
    t: &dyn HttpTransport,
    cred: &Credential,
    out: &mut dyn Write,
) -> Result<(), i32> {
    let resp = chat(t, cred, PROOF_MESSAGE, None)
        .await
        .map_err(|e| fail(out, &e))?;
    let reply = content(&resp).ok_or_else(|| fail(out, &AiError::InvalidAnswer))?;
    say(out, &format!("reply: {}", reply.trim()));
    say(
        out,
        &format!(
            "x-publik-charge-micros: {}",
            resp.header("x-publik-charge-micros").unwrap_or("(none)")
        ),
    );
    let mut names: Vec<&str> = resp
        .headers
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| n.to_ascii_lowercase().starts_with("x-publik-"))
        .collect();
    names.sort_unstable();
    names.dedup();
    say(
        out,
        &format!("x-publik-* headers received: {}", names.join(", ")),
    );
    Ok(())
}

/// `--models`: the ids `GET /models` lists.
async fn models(t: &dyn HttpTransport, cred: &Credential, out: &mut dyn Write) -> Result<(), i32> {
    let req = bearer(HttpRequest::get(cred.url(MODELS_PATH)), cred);
    let resp = send(t, req).await.map_err(|e| fail(out, &e))?;
    let ids: Vec<String> = resp
        .json()
        .and_then(|v| v.get("data").and_then(Value::as_array).cloned())
        .ok_or_else(|| fail(out, &AiError::InvalidAnswer))?
        .iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    for id in ids {
        say(out, &format!("model: {id}"));
    }
    Ok(())
}

/// `--probe-json`: whether a `json_object` response format comes back as one JSON object.
async fn probe_json(
    t: &dyn HttpTransport,
    cred: &Credential,
    out: &mut dyn Write,
) -> Result<(), i32> {
    let format = json!({"type": "json_object"});
    let honoured = match chat(t, cred, PROBE_MESSAGE, Some(format)).await {
        Ok(resp) => content(&resp)
            .and_then(|c| serde_json::from_str::<Value>(c.trim()).ok())
            .is_some_and(|v| v.is_object()),
        // The tier refuses the field itself.
        Err(AiError::PublikHttp {
            code: 400 | 422, ..
        }) => false,
        Err(e) => return Err(fail(out, &e)),
    };
    say(
        out,
        &format!(
            "response_format json_object: {}",
            if honoured { "honoured" } else { "not honoured" }
        ),
    );
    Ok(())
}

/// `--search`: one web search, after its own question.
async fn one_search(
    t: &dyn HttpTransport,
    cred: &Credential,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<(), i32> {
    let question = format!("One web search for {SEARCH_QUERY} costs one cent. Search now? [y/N] ");
    if !ask(input, out, &question) {
        say(out, "No search was sent.");
        return Err(1);
    }
    let outcome = search(t, cred, SEARCH_QUERY, 5).await.map_err(|e| {
        say(out, &format!("error: {e}"));
        1
    })?;
    match outcome.results.first() {
        Some(first) => say(
            out,
            &format!("first result: {} ({})", first.title, first.site),
        ),
        None => say(out, "no results"),
    }
    say(
        out,
        &format!(
            "charge_micros: {}",
            outcome
                .charge_micros
                .map_or("(none)".to_string(), |c| c.to_string())
        ),
    );
    Ok(())
}

/// One `publik-fast` chat request, with the idempotency key of its cache key.
async fn chat(
    t: &dyn HttpTransport,
    cred: &Credential,
    message: &str,
    response_format: Option<Value>,
) -> Result<HttpResponse, AiError> {
    let p = cred.provider(PUBLIK_FAST);
    let call = ChatCall {
        feature: Feature::Test,
        prompt_id: "publik-proof",
        prompt_version: 1,
        messages: vec![Message::user(message)],
        max_tokens: 256,
        temperature: 0.0,
        timeout: Duration::from_secs(30),
        cache_input: json!({"message": message, "response_format": response_format}),
        cache: CachePolicy::Off,
        validation_retry: 0,
    };
    let mut body = chat_body(&p, &call);
    if let Some(format) = response_format {
        body["response_format"] = format;
    }
    let req = bearer(HttpRequest::post_json(p.url(CHAT_PATH), body), cred)
        .header("Content-Type", HeaderValue::plain("application/json"))
        .header(
            IDEMPOTENCY_HEADER,
            HeaderValue::plain(idempotency_key(&cache_key(&p, &call), 0)),
        )
        .timeout(call.timeout);
    send(t, req).await
}

/// Sends once; a non-2xx is the publik error for it.
async fn send(t: &dyn HttpTransport, req: HttpRequest) -> Result<HttpResponse, AiError> {
    let resp = t.send(req).await.map_err(|e| match e {
        redpen_core::net::TransportError::Timeout => AiError::PublikTimeout { seconds: 30 },
        redpen_core::net::TransportError::Unrecorded { hash, path } => {
            AiError::Unrecorded { hash, path }
        }
        _ => AiError::Offline,
    })?;
    if (200..=299).contains(&resp.status) {
        Ok(resp)
    } else {
        Err(map_publik_error(resp.status, &resp.body))
    }
}

fn bearer(req: HttpRequest, cred: &Credential) -> HttpRequest {
    match &cred.key {
        Some(key) => req.header(
            "Authorization",
            HeaderValue::secret(format!("Bearer {}", key.expose_secret())),
        ),
        None => req,
    }
}

fn content(resp: &HttpResponse) -> Option<String> {
    resp.json()?
        .pointer("/choices/0/message/content")?
        .as_str()
        .map(str::to_string)
}

fn say(out: &mut dyn Write, line: &str) {
    let _ = writeln!(out, "{line}");
}

fn fail(out: &mut dyn Write, e: &AiError) -> i32 {
    say(out, &format!("error: {e}"));
    1
}

/// Asks a y/N question; only `y` or `yes` is a yes.
fn ask(input: &mut dyn BufRead, out: &mut dyn Write, question: &str) -> bool {
    let _ = write!(out, "{question}");
    let _ = out.flush();
    let mut answer = String::new();
    if input.read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use redpen_core::net::{FakeEntry, FakeTransport};
    use redpen_core::provider::SecretString;

    const KEY: &str = "pk_live_a";

    fn options(dir: &std::path::Path, token: Option<&str>) -> PublikOptions {
        PublikOptions {
            credential_path: dir.join("redpen.json"),
            app_token: token.map(str::to_string),
            base_url: "https://publikhq.com/api/v1".into(),
            app_version: "0.1.0".into(),
            new_install_id: || "5f0c8a52-6e0b-4c55-9a3e-2a3f1d9b7c11".into(),
            env: |_| None,
        }
    }

    fn saved_credential(dir: &std::path::Path) -> Credential {
        let c = Credential {
            key: Some(SecretString::from(KEY)),
            install_id: "5f0c8a52-6e0b-4c55-9a3e-2a3f1d9b7c11".into(),
            ..Credential::default()
        };
        c.save(&dir.join("redpen.json")).unwrap();
        c
    }

    fn chat_req(cred: &Credential, message: &str, format: Option<Value>) -> HttpRequest {
        let p = cred.provider(PUBLIK_FAST);
        let call = ChatCall {
            feature: Feature::Test,
            prompt_id: "publik-proof",
            prompt_version: 1,
            messages: vec![Message::user(message)],
            max_tokens: 256,
            temperature: 0.0,
            timeout: Duration::from_secs(30),
            cache_input: json!({}),
            cache: CachePolicy::Off,
            validation_retry: 0,
        };
        let mut body = chat_body(&p, &call);
        if let Some(f) = format {
            body["response_format"] = f;
        }
        HttpRequest::post_json(p.url(CHAT_PATH), body)
    }

    async fn run_with(
        fake: &FakeTransport,
        opts: ProofOptions,
        o: &PublikOptions,
        answers: &str,
    ) -> (i32, String) {
        let mut input = std::io::Cursor::new(answers.as_bytes().to_vec());
        let mut out = Vec::new();
        let code = run(fake, &opts, o, &mut input, &mut out).await;
        (code, String::from_utf8(out).unwrap())
    }

    #[tokio::test]
    async fn without_a_token_it_says_so_and_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeTransport::scripted(Vec::new());
        let (code, out) = run_with(
            &fake,
            ProofOptions::default(),
            &options(dir.path(), None),
            "y\n",
        )
        .await;
        assert_eq!(code, 2);
        assert_eq!(
            out.trim(),
            "This build has no publik API app token. Use your own key for now."
        );
        assert!(fake.requests_seen().is_empty());
    }

    #[tokio::test]
    async fn the_call_prints_reply_charge_and_header_names_only() {
        let dir = tempfile::tempdir().unwrap();
        let cred = saved_credential(dir.path());
        let mut entry = FakeEntry::reply(
            &chat_req(&cred, PROOF_MESSAGE, None),
            200,
            json!({"choices": [{"message": {"role": "assistant", "content": "ok"}}]}),
        );
        entry.headers = vec![
            ("x-publik-balance".into(), "3120000".into()),
            ("x-publik-charge-micros".into(), "210".into()),
            ("x-publik-request-id".into(), "req_abc".into()),
            ("content-type".into(), "application/json".into()),
        ];
        let fake = FakeTransport::scripted(vec![entry]);
        let o = options(dir.path(), Some("pat_redpen_fake"));
        let (code, out) = run_with(&fake, ProofOptions::default(), &o, "").await;
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("reply: ok"), "{out}");
        assert!(out.contains("x-publik-charge-micros: 210"), "{out}");
        assert!(
            out.contains(
                "x-publik-* headers received: x-publik-balance, x-publik-charge-micros, x-publik-request-id"
            ),
            "{out}"
        );
        assert!(!out.contains(KEY) && !out.contains("3120000") && !out.contains("3.12"));
        // The request carried the key in the header only, and an idempotency key.
        let seen = &fake.requests_seen()[0];
        assert!(
            seen.headers
                .iter()
                .any(|(n, v)| n == "Authorization" && v.is_secret())
        );
        assert!(seen.headers.iter().any(|(n, _)| n == IDEMPOTENCY_HEADER));
        assert!(!seen.url.contains(KEY));
    }

    #[tokio::test]
    async fn models_probe_and_search() {
        let dir = tempfile::tempdir().unwrap();
        let cred = saved_credential(dir.path());
        let models_req = HttpRequest::get(cred.url(MODELS_PATH));
        let format = json!({"type": "json_object"});
        let probe = chat_req(&cred, PROBE_MESSAGE, Some(format.clone()));
        let search_req =
            HttpRequest::post_json(cred.url("/search"), json!({"q": SEARCH_QUERY, "count": 5}));
        let fake = FakeTransport::scripted(vec![
            FakeEntry::reply(
                &models_req,
                200,
                json!({"object": "list", "data": [{"id": "publik-fast"}, {"id": "publik-balanced"}]}),
            ),
            FakeEntry::reply(
                &probe,
                200,
                json!({"choices": [{"message": {"content": "{\"ok\": true}"}}]}),
            ),
            FakeEntry::reply(
                &search_req,
                200,
                json!({"results": [{"url": "https://en.example.org/wiki/Reef", "title": "Great Barrier Reef",
                                    "snippet": "s", "site": "en.example.org"}], "charge_micros": 10000}),
            ),
        ]);
        let o = options(dir.path(), Some("pat_redpen_fake"));
        let all = ProofOptions {
            models: true,
            probe_json: true,
            search: true,
        };
        let (code, out) = run_with(&fake, all, &o, "y\n").await;
        assert_eq!(code, 0, "{out}");
        assert!(
            out.contains("model: publik-fast\nmodel: publik-balanced"),
            "{out}"
        );
        assert!(
            out.contains("response_format json_object: honoured"),
            "{out}"
        );
        assert!(
            out.contains("first result: Great Barrier Reef (en.example.org)"),
            "{out}"
        );
        assert!(out.contains("charge_micros: 10000"), "{out}");

        let fake = FakeTransport::scripted(vec![FakeEntry::reply(
            &probe,
            200,
            json!({"choices": [{"message": {"content": "ok"}}]}),
        )]);
        let probe_only = ProofOptions {
            probe_json: true,
            ..ProofOptions::default()
        };
        let (_, out) = run_with(&fake, probe_only, &o, "").await;
        assert!(
            out.contains("response_format json_object: not honoured"),
            "{out}"
        );
    }

    #[tokio::test]
    async fn no_means_nothing_is_sent() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeTransport::scripted(Vec::new());
        let o = options(dir.path(), Some("pat_redpen_fake"));
        let (code, out) = run_with(&fake, ProofOptions::default(), &o, "n\n").await;
        assert_eq!(code, 1);
        assert!(out.contains(DISCLOSURE));
        assert!(out.contains("Nothing was sent."));
        assert!(fake.requests_seen().is_empty());
        saved_credential(dir.path());
        let search_only = ProofOptions {
            search: true,
            ..ProofOptions::default()
        };
        let (code, out) = run_with(&fake, search_only, &o, "\n").await;
        assert_eq!(code, 1);
        assert!(out.contains("No search was sent."));
        assert!(fake.requests_seen().is_empty());
    }

    #[test]
    fn arguments_are_the_three_flags() {
        assert_eq!(parse_args(Vec::new()), Ok(ProofOptions::default()));
        assert_eq!(
            parse_args(["--models".to_string(), "--search".to_string()]),
            Ok(ProofOptions {
                models: true,
                probe_json: false,
                search: true
            })
        );
        assert!(
            parse_args(["--key".to_string()])
                .unwrap_err()
                .contains(USAGE)
        );
    }
}
