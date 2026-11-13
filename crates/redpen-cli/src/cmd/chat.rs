//! `redpen chat --provider P [--recording R] --message S`: one chat call, for diagnosis and the
//! golden tests (docs/modules/02-provider-layer.md, Design notes).
//!
//! It sends `{"model", "messages": [{"role": "user", "content": S}], "max_tokens": 256,
//! "temperature": 0}` to `/chat/completions` with cache policy `Off`, an in-memory database and
//! no settings file, and prints `{"content": "...", "from_cache": false}` on one line. An
//! `AiError` prints `{"error": {"kind", "message", ...}}` on standard output and its message on
//! standard error, and exits 3; usage errors exit 2.

use std::io::Write;
use std::sync::Arc;

use redpen_core::clock::{SystemClock, TokioSleeper};
use redpen_core::provider::{
    AiClient, AiError, CachePolicy, ChatCall, Feature, Message, NoopStatusSink, ProviderKind,
    TierContext, chat_timeout, publik_tier,
};
use redpen_core::store::Db;
use serde::Serialize;

use super::{AI_ERROR, Failure, fail, print};
use crate::providers::{self, ProviderArg};

/// The options of `chat`.
#[derive(Debug, PartialEq, Eq)]
struct Opts {
    provider: ProviderArg,
    recording: Option<String>,
    message: String,
}

fn parse(args: &[String]) -> Result<Opts, Failure> {
    let mut provider = None;
    let mut recording = None;
    let mut message = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .ok_or_else(|| Failure::usage(format!("{flag} needs a value. Run redpen --help.")))
        };
        match arg.as_str() {
            "--provider" => provider = Some(value("--provider")?.parse::<ProviderArg>()?),
            "--recording" => recording = Some(value("--recording")?.clone()),
            "--message" => message = Some(value("--message")?.clone()),
            other => {
                return Err(Failure::usage(format!(
                    "unknown argument {other}. Run redpen --help."
                )));
            }
        }
    }
    let provider =
        provider.ok_or_else(|| Failure::usage("chat needs --provider. Run redpen --help."))?;
    let message =
        message.ok_or_else(|| Failure::usage("chat needs --message. Run redpen --help."))?;
    Ok(Opts {
        provider,
        recording,
        message,
    })
}

#[derive(Serialize)]
struct Output<'a> {
    content: &'a str,
    from_cache: bool,
}

#[derive(Serialize)]
struct Refusal<'a> {
    error: &'a AiError,
}

pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    run_with_env(args, out, err, |name| std::env::var(name).ok())
}

fn run_with_env(
    args: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
    env: impl Fn(&str) -> Option<String>,
) -> i32 {
    let opts = match parse(args) {
        Ok(opts) => opts,
        Err(failure) => return fail(failure, err),
    };
    let setup = match providers::setup(opts.provider, opts.recording.as_deref(), env) {
        Ok(setup) => setup,
        Err(failure) => return fail(failure, err),
    };
    let db = match Db::open_in_memory() {
        Ok(db) => db,
        Err(e) => return fail(Failure::usage(e.to_string()), err),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => return fail(Failure::usage(format!("could not start: {e}")), err),
    };
    let client = AiClient::new(
        setup.transport,
        db,
        Arc::new(SystemClock),
        Arc::new(TokioSleeper),
        Arc::new(NoopStatusSink),
    );
    let call = ChatCall {
        feature: Feature::Test,
        prompt_id: "chat",
        prompt_version: 1,
        messages: vec![Message::user(opts.message.as_str())],
        max_tokens: 256,
        temperature: 0.0,
        timeout: chat_timeout(setup.provider.kind),
        cache_input: serde_json::json!({ "message": opts.message }),
        cache: CachePolicy::Off,
        validation_retry: 0,
    };
    // On publik API a message goes to the tier a provider's Test uses, the cheapest.
    let mut provider = setup.provider;
    if provider.kind == ProviderKind::Publik {
        provider.chat_model = publik_tier(Feature::Test, &TierContext::default()).to_string();
    }
    match runtime.block_on(client.chat(&provider, call)) {
        Ok(reply) => print(
            &Output {
                content: &reply.content,
                from_cache: reply.from_cache,
            },
            out,
        ),
        Err(e) => {
            print(&Refusal { error: &e }, out);
            let _ = writeln!(err, "error: {e}");
            AI_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redpen_core::net::{FakeEntry, HttpRequest, Recording};
    use serde_json::json;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    fn run_capturing(list: &[&str]) -> (i32, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with_env(&args(list), &mut out, &mut err, |_| None);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    /// A recording answering the fake provider's `hi` with `hello`.
    fn recording(dir: &tempfile::TempDir) -> String {
        let req = HttpRequest::post_json(
            "http://fake.invalid/v1/chat/completions",
            json!({"model": "fake-chat", "messages": [{"role": "user", "content": "hi"}],
                   "max_tokens": 256, "temperature": 0}),
        );
        let entry = FakeEntry::reply(
            &req,
            200,
            json!({"choices": [{"message": {"role": "assistant", "content": "hello"}}]}),
        );
        let path = dir.path().join("recording.json");
        std::fs::write(
            &path,
            serde_json::to_string(&Recording::new(vec![entry])).unwrap(),
        )
        .unwrap();
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn options_parse() {
        assert_eq!(
            parse(&args(&[
                "--provider",
                "fake",
                "--recording",
                "r.json",
                "--message",
                "hi"
            ]))
            .unwrap(),
            Opts {
                provider: ProviderArg::Fake,
                recording: Some("r.json".into()),
                message: "hi".into(),
            }
        );
        assert_eq!(
            parse(&args(&["--message", "hi"])).unwrap_err().message,
            "chat needs --provider. Run redpen --help."
        );
        assert_eq!(
            parse(&args(&["--provider", "local"])).unwrap_err().message,
            "chat needs --message. Run redpen --help."
        );
        assert_eq!(
            parse(&args(&["--provider", "fake", "--message"]))
                .unwrap_err()
                .message,
            "--message needs a value. Run redpen --help."
        );
        assert_eq!(
            parse(&args(&["--provider", "fake", "--message", "hi", "extra"]))
                .unwrap_err()
                .message,
            "unknown argument extra. Run redpen --help."
        );
    }

    #[test]
    fn a_recorded_answer_prints_the_content() {
        let dir = tempfile::tempdir().unwrap();
        let rec = recording(&dir);
        let (code, out, err) =
            run_capturing(&["--provider", "fake", "--recording", &rec, "--message", "hi"]);
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(out, "{\"content\":\"hello\",\"from_cache\":false}\n");
    }

    #[test]
    fn an_unrecorded_request_exits_3_with_the_transport_message() {
        let dir = tempfile::tempdir().unwrap();
        let rec = recording(&dir);
        let (code, out, err) = run_capturing(&[
            "--provider",
            "fake",
            "--recording",
            &rec,
            "--message",
            "bye",
        ]);
        assert_eq!(code, 3);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["error"]["kind"], "Unrecorded");
        let message = value["error"]["message"].as_str().unwrap();
        assert!(message.starts_with("Unrecorded request "), "{message}");
        assert!(message.contains(" to /chat/completions; "), "{message}");
        assert_eq!(err, format!("error: {message}\n"));
        let hash = value["error"]["hash"].as_str().unwrap();
        let _ = std::fs::remove_file(redpen_core::net::fake::unrecorded_path(hash));
    }

    #[test]
    fn usage_errors_exit_2() {
        let (code, out, err) = run_capturing(&["--provider", "own-key", "--message", "hi"]);
        assert_eq!(code, 2);
        assert!(out.is_empty());
        assert_eq!(
            err,
            "error: --provider own-key needs REDPEN_OWN_KEY, REDPEN_OWN_BASE_URL and REDPEN_OWN_MODEL.\n"
        );
        let (code, _, err) = run_capturing(&["--provider", "cloud", "--message", "hi"]);
        assert_eq!(code, 2);
        assert_eq!(
            err,
            "error: Unknown provider \"cloud\". Use local, own-key, publik-env or fake.\n"
        );
    }
}
