//! `redpen rewrite --action A [--dialect D] [--style S] [--instruction S] [--redact]
//! --provider P [--recording R] FILE` (docs/modules/04-rewrite-engine.md): one rewrite of the
//! file's text, exactly as the popup runs it.
//!
//! The file is read as it is (its whitespace is part of the selection and comes back around the
//! result). The tier context is Standard and anonymous, the cache is an in-memory database, and
//! redaction is on only with `--redact`. It prints the outcome on one line: `{"text", "diff",
//! "warnings", "accept_default", "unchanged", "from_cache", "charge_micros"}`, the charge read
//! from `x-publik-charge-micros`. A refusal or an AI error prints `{"error": {"kind", "message",
//! ...}}` on standard output and its message on standard error, and exits 3; usage errors
//! exit 2.

use std::io::Write;
use std::sync::Arc;

use redpen_core::check::Dialect;
use redpen_core::clock::{SystemClock, TokioSleeper};
use redpen_core::provider::{AiClient, TierContext};
use redpen_core::rewrite::{self, Action, RewriteError, RewriteRequest, Style};
use redpen_core::store::Db;
use serde::Serialize;

use super::{AI_ERROR, Failure, fail, print, read_text};
use crate::providers::{self, ChargeSink, ProviderArg};

/// The options of `rewrite` and `prompt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteOpts {
    pub action: Action,
    pub dialect: Dialect,
    pub style: Style,
    pub instruction: Option<String>,
    pub redact: bool,
    pub provider: Option<ProviderArg>,
    pub recording: Option<String>,
    pub file: String,
}

impl RewriteOpts {
    /// The request these options describe for `text`.
    pub fn request(&self, text: String) -> RewriteRequest {
        RewriteRequest {
            action: self.action,
            text,
            instruction: self.instruction.clone(),
            dialect: self.dialect,
            style: self.style,
            redact: self.redact,
            variant: 0,
            validation_retry: 0,
        }
    }
}

/// Parses the options of `name` (`rewrite` takes `--provider` and `--recording`; `prompt` does
/// not).
pub fn parse(name: &str, args: &[String], with_provider: bool) -> Result<RewriteOpts, Failure> {
    let mut action = None;
    let mut dialect = Dialect::default();
    let mut style = Style::Own;
    let mut instruction = None;
    let mut redact = false;
    let mut provider = None;
    let mut recording = None;
    let mut file = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .ok_or_else(|| Failure::usage(format!("{flag} needs a value. Run redpen --help.")))
        };
        match arg.as_str() {
            "--action" => {
                action = Some(
                    value("--action")?
                        .parse::<Action>()
                        .map_err(|e| Failure::usage(e.to_string()))?,
                );
            }
            "--dialect" => {
                dialect = value("--dialect")?.parse().map_err(
                    |e: redpen_core::check::UnknownDialect| Failure::usage(e.to_string()),
                )?;
            }
            "--style" => style = parse_style(value("--style")?)?,
            "--instruction" => instruction = Some(value("--instruction")?.clone()),
            "--redact" => redact = true,
            "--provider" if with_provider => {
                provider = Some(value("--provider")?.parse::<ProviderArg>()?);
            }
            "--recording" if with_provider => recording = Some(value("--recording")?.clone()),
            other if other.starts_with('-') && other != "-" => {
                return Err(Failure::usage(format!(
                    "unknown argument {other}. Run redpen --help."
                )));
            }
            path if file.is_none() => file = Some(path.to_string()),
            extra => {
                return Err(Failure::usage(format!(
                    "unknown argument {extra}. Run redpen --help."
                )));
            }
        }
    }
    let action = action
        .ok_or_else(|| Failure::usage(format!("{name} needs --action. Run redpen --help.")))?;
    if with_provider && provider.is_none() {
        return Err(Failure::usage(format!(
            "{name} needs --provider. Run redpen --help."
        )));
    }
    let file =
        file.ok_or_else(|| Failure::usage(format!("{name} needs a FILE. Run redpen --help.")))?;
    Ok(RewriteOpts {
        action,
        dialect,
        style,
        instruction,
        redact,
        provider,
        recording,
        file,
    })
}

/// A `--style` value. Only `own` until style presets arrive.
fn parse_style(value: &str) -> Result<Style, Failure> {
    match value {
        "own" => Ok(Style::Own),
        other => Err(Failure::usage(format!(
            "Unknown style \"{other}\". Only own is available in this version."
        ))),
    }
}

/// A refusal or an AI error: `{"error": {...}}` on standard output, the message on standard
/// error, exit 3.
pub fn refused(e: &RewriteError, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    #[derive(Serialize)]
    struct Refusal<'a> {
        error: &'a RewriteError,
    }
    print(&Refusal { error: e }, out);
    let _ = writeln!(err, "error: {e}");
    AI_ERROR
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
    let opts = match parse("rewrite", args, true) {
        Ok(opts) => opts,
        Err(failure) => return fail(failure, err),
    };
    let text = match read_text(&opts.file) {
        Ok(text) => text,
        Err(failure) => return fail(failure, err),
    };
    let arg = opts.provider.expect("parse requires --provider");
    let setup = match providers::setup(arg, opts.recording.as_deref(), env) {
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
        Arc::new(ChargeSink),
    );
    let request = opts.request(text);
    let ctx = TierContext::default();
    match runtime.block_on(rewrite::rewrite(&client, &setup.provider, &ctx, request)) {
        Ok(outcome) => print(&outcome, out),
        Err(e) => refused(&e, out, err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redpen_core::net::{FakeEntry, HttpRequest, Recording};
    use redpen_core::provider::ResolvedProvider;
    use serde_json::{Value, json};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    fn run_capturing(list: &[&str]) -> (i32, Value, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with_env(&args(list), &mut out, &mut err, |_| None);
        let out = String::from_utf8(out).unwrap();
        let value = if out.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"))
        };
        (code, value, String::from_utf8(err).unwrap())
    }

    /// A directory with `input.txt` and a recording answering its Fix grammar with `content`.
    fn case(text: &str, content: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("input.txt"), text).unwrap();
        let p = ResolvedProvider::fake();
        let r = RewriteRequest {
            dialect: Dialect::British,
            redact: true,
            ..RewriteRequest::new(Action::FixGrammar, text)
        };
        let ctx = TierContext::default();
        let body = rewrite::request_body(&p, &ctx, &r).unwrap();
        let req = HttpRequest::post_json("http://fake.invalid/v1/chat/completions", body);
        let mut entry = FakeEntry::reply(
            &req,
            200,
            json!({"choices": [{"message": {"role": "assistant", "content": content}}]}),
        );
        entry.headers = vec![("x-publik-charge-micros".into(), "100".into())];
        std::fs::write(
            dir.path().join("recording.json"),
            serde_json::to_string(&Recording::new(vec![entry])).unwrap(),
        )
        .unwrap();
        dir
    }

    fn path(dir: &tempfile::TempDir, name: &str) -> String {
        dir.path().join(name).to_str().unwrap().to_string()
    }

    #[test]
    fn options_parse() {
        let opts = parse(
            "rewrite",
            &args(&[
                "--action",
                "custom",
                "--dialect",
                "british",
                "--style",
                "own",
                "--instruction",
                "translate to French",
                "--redact",
                "--provider",
                "fake",
                "--recording",
                "recording.json",
                "input.txt",
            ]),
            true,
        )
        .unwrap();
        assert_eq!(opts.action, Action::Custom);
        assert_eq!(opts.dialect, Dialect::British);
        assert_eq!(opts.instruction.as_deref(), Some("translate to French"));
        assert!(opts.redact);
        assert_eq!(opts.provider, Some(ProviderArg::Fake));
        assert_eq!(opts.file, "input.txt");
        let fail = |a: &[&str], with| parse("rewrite", &args(a), with).unwrap_err().message;
        assert_eq!(
            fail(&["--provider", "fake", "in.txt"], true),
            "rewrite needs --action. Run redpen --help."
        );
        assert_eq!(
            fail(&["--action", "clearer", "in.txt"], true),
            "rewrite needs --provider. Run redpen --help."
        );
        assert_eq!(
            fail(&["--action", "clearer", "--provider", "fake"], true),
            "rewrite needs a FILE. Run redpen --help."
        );
        assert_eq!(
            fail(&["--action", "rephrase", "in.txt"], false),
            "Unknown action \"rephrase\". Use fix-grammar, clearer, shorter, more-formal, friendlier or custom."
        );
        assert_eq!(
            fail(
                &["--action", "clearer", "--style", "academic", "in.txt"],
                false
            ),
            "Unknown style \"academic\". Only own is available in this version."
        );
        assert_eq!(
            fail(
                &["--action", "clearer", "--provider", "fake", "in.txt"],
                false
            ),
            "unknown argument --provider. Run redpen --help."
        );
    }

    #[test]
    fn a_recorded_answer_prints_the_outcome() {
        let text = "Their going to send the report tomorow, please forward it to ana@example.com when it arrive.";
        let dir = case(
            text,
            r#"{"text": "They're going to send the report tomorrow. Please forward it to [EMAIL_1] when it arrives."}"#,
        );
        let (code, value, err) = run_capturing(&[
            "--action",
            "fix-grammar",
            "--dialect",
            "british",
            "--redact",
            "--provider",
            "fake",
            "--recording",
            &path(&dir, "recording.json"),
            &path(&dir, "input.txt"),
        ]);
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(
            value["text"],
            "They're going to send the report tomorrow. Please forward it to ana@example.com when it arrives."
        );
        assert_eq!(value["diff"][0], json!({"delete": "Their"}));
        assert_eq!(value["warnings"], json!([]));
        assert_eq!(value["accept_default"], true);
        assert_eq!(value["unchanged"], false);
        assert_eq!(value["from_cache"], false);
        assert_eq!(value["charge_micros"], 100);
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys.len(), 7);
    }

    #[test]
    fn refusals_and_invalid_answers_exit_3() {
        let dir = case("Fine text here.", "hello");
        let (code, value, err) = run_capturing(&[
            "--action",
            "fix-grammar",
            "--dialect",
            "british",
            "--redact",
            "--provider",
            "fake",
            "--recording",
            &path(&dir, "recording.json"),
            &path(&dir, "input.txt"),
        ]);
        assert_eq!(code, 3);
        assert_eq!(
            value,
            json!({"error": {"kind": "InvalidAnswer", "message": "The AI answer could not be used."}})
        );
        assert_eq!(err, "error: The AI answer could not be used.\n");

        std::fs::write(dir.path().join("blank.txt"), "  \n").unwrap();
        let (code, value, _) = run_capturing(&[
            "--action",
            "clearer",
            "--provider",
            "fake",
            &path(&dir, "blank.txt"),
        ]);
        assert_eq!(code, 3);
        assert_eq!(
            value,
            json!({"error": {"kind": "Empty", "message": "Select some text first."}})
        );
    }

    #[test]
    fn usage_errors_exit_2() {
        let (code, value, err) = run_capturing(&[
            "--action",
            "clearer",
            "--provider",
            "own-key",
            "missing.txt",
        ]);
        assert_eq!(code, 2);
        assert_eq!(value, Value::Null);
        assert!(
            err.starts_with("error: could not read missing.txt"),
            "{err}"
        );
    }
}
