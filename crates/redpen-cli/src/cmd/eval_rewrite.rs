//! `redpen eval-rewrite [--provider P] [--recording R] CORPUS LANGUAGES`: the measurement behind
//! the `eval-rewrite` gate (docs/TESTING.md §4), which `scripts/gates.sh eval-rewrite` runs on
//! module 04's `fix-grammar.jsonl` (100 sentences) and `languages.jsonl` (40 paragraphs), 300
//! calls in all. It spends money on publik API, so it is the maintainer's to run.
//!
//! - **Fix grammar** on the first 100 sentences of CORPUS (JSON lines with `text`, and optionally
//!   `dialect` and `corrected`): the median edit ratio ([`redpen_core::diff::edit_ratio`]) of the
//!   answers, at most 0.15, and the share of answers with a guard warning, at most 5%. Every
//!   corpus sentence needs a one-word fix, so a guard warning there is an over-edit or a false
//!   alarm.
//! - **Language keeping** on every paragraph of LANGUAGES (JSON lines with `text`) with Clearer,
//!   Shorter, More formal, Friendlier and Custom ("Make it sound more confident."): the share of
//!   answers without a language warning, at least 98%.
//!
//! Before anything is sent it prints the number of calls, the tiers and the estimated cost, and
//! asks `Send them? [y/N]`; anything but `y` or `yes` sends nothing and exits 0. After the first
//! call it prints what that call cost (`x-publik-charge-micros`). A provider error that is not
//! an unusable answer (a 401, a 402, no connection) stops the run: nothing more is sent, it prints
//! `eval-rewrite: stopped after N calls: <the error>` and the gate fails. It ends with one JSON
//! line and exits 0 when every threshold holds, 1 when one does not. The thresholds are confirmed or set by the first run (module 04).
//! `--provider` is `publik-env` (the default) or `own-key`; `fake` and `local` serve tests.
//! Every comparison is in whole numbers.

use std::io::{BufRead, Write};
use std::sync::Arc;

use redpen_core::check::Dialect;
use redpen_core::clock::{SystemClock, TokioSleeper};
use redpen_core::diff::edit_ratio;
use redpen_core::provider::tier::{PUBLIK_BALANCED, PUBLIK_FAST};
use redpen_core::provider::{AiClient, TierContext};
use redpen_core::rewrite::{self, Action, RewriteOutcome, RewriteRequest, Warning};
use redpen_core::store::Db;
use serde::Serialize;
use serde_json::Value;

use super::{Failure, GATE_FAILED, OK, fail, print, read_text};
use crate::providers::{self, ChargeSink, ProviderArg};

/// Fix grammar runs on at most this many corpus sentences.
pub const MAX_SENTENCES: usize = 100;
/// The median edit ratio of Fix grammar may be at most this, in per cent (0.15).
pub const MEDIAN_LIMIT_PERCENT: u128 = 15;
/// Guard warnings may be on at most this share of Fix grammar answers, in per cent.
pub const GUARD_LIMIT_PERCENT: usize = 5;
/// At least this share of rewrites must keep the text's language, in per cent.
pub const KEEPING_LIMIT_PERCENT: usize = 98;
/// The actions language keeping runs on each paragraph.
pub const KEEPING_ACTIONS: [Action; 5] = [
    Action::Clearer,
    Action::Shorter,
    Action::MoreFormal,
    Action::Friendlier,
    Action::Custom,
];
/// Custom's instruction in the gate: an edit that keeps the language.
pub const CUSTOM_INSTRUCTION: &str = "Make it sound more confident.";
/// The published estimates per call (docs/PUBLIK-INTEGRATION.md §8), in micro-dollars.
const FAST_ESTIMATE_MICROS: u64 = 200;
const BALANCED_ESTIMATE_MICROS: u64 = 2_100;

/// One corpus sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sentence {
    text: String,
    dialect: Dialect,
    corrected: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct Opts {
    provider: ProviderArg,
    recording: Option<String>,
    corpus: String,
    languages: String,
}

fn parse(args: &[String]) -> Result<Opts, Failure> {
    let mut provider = ProviderArg::PublikEnv;
    let mut recording = None;
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .ok_or_else(|| Failure::usage(format!("{flag} needs a value. Run redpen --help.")))
        };
        match arg.as_str() {
            "--provider" => provider = value("--provider")?.parse()?,
            "--recording" => recording = Some(value("--recording")?.clone()),
            other if other.starts_with('-') || files.len() == 2 => {
                return Err(Failure::usage(format!(
                    "unknown argument {other}. Run redpen --help."
                )));
            }
            file => files.push(file.to_string()),
        }
    }
    let [corpus, languages]: [String; 2] = files.try_into().map_err(|_| {
        Failure::usage("eval-rewrite needs a CORPUS and a LANGUAGES file. Run redpen --help.")
    })?;
    Ok(Opts {
        provider,
        recording,
        corpus,
        languages,
    })
}

/// The JSON objects of a JSON-lines file, skipping blank lines.
fn json_lines(path: &str) -> Result<Vec<Value>, Failure> {
    read_text(path)?
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(i, line)| {
            serde_json::from_str(line)
                .map_err(|e| Failure::usage(format!("{path} line {}: {e}", i + 1)))
        })
        .collect()
}

fn sentences(path: &str) -> Result<Vec<Sentence>, Failure> {
    json_lines(path)?
        .into_iter()
        .take(MAX_SENTENCES)
        .map(|v| {
            let text = v["text"]
                .as_str()
                .ok_or_else(|| Failure::usage(format!("{path}: a line has no \"text\".")))?;
            Ok(Sentence {
                text: text.to_string(),
                dialect: v["dialect"]
                    .as_str()
                    .and_then(Dialect::parse)
                    .unwrap_or_default(),
                corrected: v["corrected"].as_str().map(str::to_string),
            })
        })
        .collect()
}

fn paragraphs(path: &str) -> Result<Vec<String>, Failure> {
    json_lines(path)?
        .into_iter()
        .map(|v| {
            v["text"]
                .as_str()
                .or_else(|| v["paragraph"].as_str())
                .map(str::to_string)
                .ok_or_else(|| Failure::usage(format!("{path}: a line has no \"text\".")))
        })
        .collect()
}

/// `n / d` as a decimal with `places` digits, rounded half up, in whole numbers.
fn decimal(n: u128, d: u128, places: u32) -> String {
    let scale = 10u128.pow(places);
    let scaled = if d == 0 {
        0
    } else {
        (n * scale * 2 + d) / (d * 2)
    };
    let whole = scaled / scale;
    if places == 0 {
        return whole.to_string();
    }
    format!(
        "{whole}.{:0width$}",
        scaled % scale,
        width = places as usize
    )
}

/// Dollars from micro-dollars, to the cent.
fn dollars(micros: u64) -> String {
    format!("${}", decimal(u128::from(micros), 1_000_000, 2))
}

/// The median of the ratios `changed / max(1, total)`, as a fraction (numerator, denominator):
/// the middle one, or the mean of the two middle ones.
fn median_ratio(ratios: &[(usize, usize)]) -> Option<(u128, u128)> {
    let mut r: Vec<(u128, u128)> = ratios
        .iter()
        .map(|&(c, t)| (c as u128, t.max(1) as u128))
        .collect();
    if r.is_empty() {
        return None;
    }
    r.sort_by(|a, b| (a.0 * b.1).cmp(&(b.0 * a.1)));
    let mid = r.len() / 2;
    if r.len() % 2 == 1 {
        Some(r[mid])
    } else {
        let (a, b) = (r[mid - 1], r[mid]);
        Some((a.0 * b.1 + b.0 * a.1, 2 * a.1 * b.1))
    }
}

/// The plan printed before the question.
fn plan(provider: ProviderArg, fix: usize, rewrites: usize, paragraphs: usize) -> String {
    let (fast, balanced, cost) = match provider {
        ProviderArg::PublikEnv | ProviderArg::Fake => {
            let micros =
                fix as u64 * FAST_ESTIMATE_MICROS + rewrites as u64 * BALANCED_ESTIMATE_MICROS;
            (
                PUBLIK_FAST,
                PUBLIK_BALANCED,
                format!(", about {} on publik API", dollars(micros)),
            )
        }
        ProviderArg::OwnKey => (
            "your own key's model",
            "your own key's model",
            ", billed by your own key's provider".to_string(),
        ),
        ProviderArg::Local => ("the local model", "the local model", ", free".to_string()),
    };
    format!(
        "eval-rewrite: {} calls: {fix} Fix grammar on {fast} (one per corpus sentence, at most {MAX_SENTENCES}) and {rewrites} rewrites ({paragraphs} paragraphs × Clearer, Shorter, More formal, Friendlier and Custom) on {balanced}{cost}.\nSend them? [y/N] ",
        fix + rewrites
    )
}

#[derive(Serialize)]
struct FixReport {
    sentences: usize,
    answered: usize,
    median_edit_ratio: String,
    median_limit: String,
    guard_warnings: usize,
    guard_warning_percent: String,
    guard_limit_percent: usize,
    exact: usize,
}

#[derive(Serialize)]
struct KeepingReport {
    rewrites: usize,
    answered: usize,
    kept: usize,
    kept_percent: String,
    kept_limit_percent: usize,
}

#[derive(Serialize)]
struct Report {
    gate: &'static str,
    provider: &'static str,
    calls: usize,
    errors: usize,
    charge_micros: u64,
    fix_grammar: FixReport,
    language_keeping: KeepingReport,
    pass: bool,
}

pub fn run(
    args: &[String],
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    run_with_env(args, input, out, err, |name| std::env::var(name).ok())
}

fn run_with_env(
    args: &[String],
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
    env: impl Fn(&str) -> Option<String>,
) -> i32 {
    let opts = match parse(args) {
        Ok(opts) => opts,
        Err(failure) => return fail(failure, err),
    };
    let (sentences, paragraphs) = match (sentences(&opts.corpus), paragraphs(&opts.languages)) {
        (Ok(s), Ok(p)) => (s, p),
        (Err(failure), _) | (_, Err(failure)) => return fail(failure, err),
    };
    let rewrites = paragraphs.len() * KEEPING_ACTIONS.len();
    let _ = write!(
        out,
        "{}",
        plan(opts.provider, sentences.len(), rewrites, paragraphs.len())
    );
    let _ = out.flush();
    let mut answer = String::new();
    let _ = input.read_line(&mut answer);
    // End the question's line, so what follows, the report's JSON line included, starts on its
    // own line even when the answer came from a pipe.
    let _ = writeln!(out);
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        let _ = writeln!(out, "eval-rewrite: nothing was sent.");
        return OK;
    }

    // Only now is the provider set up, so a declined run needs no key.
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
        Arc::new(ChargeSink),
    );
    let ctx = TierContext::default();

    let mut calls = 0;
    let mut errors = 0;
    let mut charge: u64 = 0;
    let mut stopped = None;
    let mut send = |r: RewriteRequest| -> Option<RewriteOutcome> {
        if stopped.is_some() {
            return None;
        }
        let result = runtime.block_on(rewrite::rewrite(&client, &setup.provider, &ctx, r));
        calls += 1;
        match &result {
            Ok(o) => {
                let micros = o.charge_micros.unwrap_or(0);
                charge += micros;
                if calls == 1 {
                    let _ = match o.charge_micros {
                        Some(m) => writeln!(
                            out,
                            "eval-rewrite: the first call cost ${} ({m} micro-dollars).",
                            decimal(u128::from(m), 1_000_000, 6)
                        ),
                        None => writeln!(out, "eval-rewrite: the first call reported no charge."),
                    };
                }
            }
            Err(e) if e.is_invalid_answer() => errors += 1,
            Err(e) => {
                errors += 1;
                stopped = Some(e.to_string());
            }
        }
        result.ok()
    };

    let mut ratios = Vec::new();
    let mut guard_warnings = 0;
    let mut exact = 0;
    for s in &sentences {
        let r = RewriteRequest {
            dialect: s.dialect,
            ..RewriteRequest::new(Action::FixGrammar, s.text.clone())
        };
        if let Some(o) = send(r) {
            ratios.push(edit_ratio(&s.text, &o.text));
            if o.warnings
                .iter()
                .any(|w| matches!(w, Warning::Guard { .. }))
            {
                guard_warnings += 1;
            }
            if s.corrected.as_deref() == Some(o.text.as_str()) {
                exact += 1;
            }
        }
    }
    let mut kept = 0;
    let mut answered_rewrites = 0;
    for p in &paragraphs {
        for action in KEEPING_ACTIONS {
            let mut r = RewriteRequest::new(action, p.clone());
            if action == Action::Custom {
                r.instruction = Some(CUSTOM_INSTRUCTION.to_string());
            }
            if let Some(o) = send(r) {
                answered_rewrites += 1;
                if !o
                    .warnings
                    .iter()
                    .any(|w| matches!(w, Warning::Language { .. }))
                {
                    kept += 1;
                }
            }
        }
    }
    if let Some(reason) = &stopped {
        let plural = if calls == 1 { "" } else { "s" };
        let _ = writeln!(
            err,
            "eval-rewrite: stopped after {calls} call{plural}: {reason}"
        );
    }

    let answered_fix = ratios.len();
    let median = median_ratio(&ratios);
    let median_ok = median.is_some_and(|(n, d)| n * 100 <= MEDIAN_LIMIT_PERCENT * d);
    let guard_ok = answered_fix > 0 && guard_warnings * 100 <= GUARD_LIMIT_PERCENT * answered_fix;
    let keeping_ok =
        answered_rewrites > 0 && kept * 100 >= KEEPING_LIMIT_PERCENT * answered_rewrites;
    let pass = stopped.is_none() && median_ok && guard_ok && keeping_ok;
    let (median_n, median_d) = median.unwrap_or((0, 1));
    let report = Report {
        gate: "eval-rewrite",
        provider: opts.provider.as_str(),
        calls,
        errors,
        charge_micros: charge,
        fix_grammar: FixReport {
            sentences: sentences.len(),
            answered: answered_fix,
            median_edit_ratio: decimal(median_n, median_d, 3),
            median_limit: decimal(MEDIAN_LIMIT_PERCENT, 100, 2),
            guard_warnings,
            guard_warning_percent: decimal(
                guard_warnings as u128 * 100,
                answered_fix.max(1) as u128,
                1,
            ),
            guard_limit_percent: GUARD_LIMIT_PERCENT,
            exact,
        },
        language_keeping: KeepingReport {
            rewrites,
            answered: answered_rewrites,
            kept,
            kept_percent: decimal(kept as u128 * 100, answered_rewrites.max(1) as u128, 1),
            kept_limit_percent: KEEPING_LIMIT_PERCENT,
        },
        pass,
    };
    let _ = writeln!(
        err,
        "eval-rewrite: median edit ratio {} (limit 0.15), guard warnings {}% (limit 5%), language kept {}% (limit 98%), ${} spent",
        report.fix_grammar.median_edit_ratio,
        report.fix_grammar.guard_warning_percent,
        report.language_keeping.kept_percent,
        decimal(u128::from(charge), 1_000_000, 4),
    );
    print(&report, out);
    if pass { OK } else { GATE_FAILED }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redpen_core::net::{FakeEntry, HttpRequest, Recording};
    use redpen_core::provider::ResolvedProvider;
    use serde_json::json;

    const ENGLISH: &str = "With regard to the matter of the invoice, which we discussed at some length on the phone last week, it would be appreciated if it could be paid at the earliest opportunity that is convenient.";

    fn files(dir: &tempfile::TempDir) -> (String, String) {
        let corpus = dir.path().join("corpus.jsonl");
        std::fs::write(
            &corpus,
            "{\"text\": \"I saw teh cat.\", \"dialect\": \"american\", \"corrected\": \"I saw the cat.\"}\n\n",
        )
        .unwrap();
        let languages = dir.path().join("languages.jsonl");
        std::fs::write(
            &languages,
            json!({"language": "English", "text": ENGLISH}).to_string(),
        )
        .unwrap();
        (
            corpus.to_str().unwrap().to_string(),
            languages.to_str().unwrap().to_string(),
        )
    }

    fn run_capturing(list: &[&str], input: &str) -> (i32, String, String) {
        run_in_env(list, input, |_| None)
    }

    fn run_in_env(
        list: &[&str],
        input: &str,
        env: impl Fn(&str) -> Option<String>,
    ) -> (i32, String, String) {
        let args: Vec<String> = list.iter().map(|a| a.to_string()).collect();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with_env(&args, &mut input.as_bytes(), &mut out, &mut err, env);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    /// The entry answering `r` as the fake provider sends it with `status` and `body`. The hash
    /// leaves out the host and the headers, so publik API's request matches it too.
    fn reply(r: &RewriteRequest, status: u16, body: Value) -> FakeEntry {
        let sent =
            rewrite::request_body(&ResolvedProvider::fake(), &TierContext::default(), r).unwrap();
        let req = HttpRequest::post_json("http://fake.invalid/v1/chat/completions", sent);
        FakeEntry::reply(&req, status, body)
    }

    /// The entry answering `r` with `content`, charged the synthetic 100 micro-dollars.
    fn answer(r: &RewriteRequest, content: Value) -> FakeEntry {
        let mut e = reply(
            r,
            200,
            json!({"choices": [{"message": {"role": "assistant", "content": content.to_string()}}]}),
        );
        e.headers = vec![("x-publik-charge-micros".into(), "100".into())];
        e
    }

    /// The Fix grammar request of `files`' one corpus sentence.
    fn fix_request() -> RewriteRequest {
        RewriteRequest::new(Action::FixGrammar, "I saw teh cat.")
    }

    /// A valid answer to each of the run's six requests over `files`, in the order they are sent.
    fn six_answers() -> Vec<FakeEntry> {
        let mut entries = vec![answer(&fix_request(), json!({"text": "I saw the cat."}))];
        for action in KEEPING_ACTIONS {
            let mut r = RewriteRequest::new(action, ENGLISH);
            let mut content = json!({"text": ENGLISH});
            if action == Action::Custom {
                r.instruction = Some(CUSTOM_INSTRUCTION.into());
                content["changes_language"] = json!(false);
            }
            entries.push(answer(&r, content));
        }
        entries
    }

    fn write_recording(dir: &tempfile::TempDir, entries: Vec<FakeEntry>) -> String {
        let recording = dir.path().join("recording.json");
        std::fs::write(
            &recording,
            serde_json::to_string(&Recording::new(entries)).unwrap(),
        )
        .unwrap();
        recording.to_str().unwrap().to_string()
    }

    #[test]
    fn a_corpus_line_that_is_not_json_exits_2_with_the_message() {
        let dir = tempfile::tempdir().unwrap();
        let (corpus, languages) = files(&dir);
        let broken = dir.path().join("broken.jsonl");
        // Line 3 of the file, the blank line 2 counted.
        std::fs::write(
            &broken,
            "{\"text\": \"I saw teh cat.\"}\n\n{\"text\": \"I saw\n",
        )
        .unwrap();
        let broken = broken.to_str().unwrap();
        for args in [[broken, languages.as_str()], [corpus.as_str(), broken]] {
            let (code, out, err) = run_capturing(&args, "y\n");
            assert_eq!(code, 2, "{args:?}: {err}");
            // Refused before the plan: nothing is asked and nothing is sent.
            assert!(out.is_empty(), "{args:?}: {out}");
            assert!(
                err.starts_with(&format!("error: {broken} line 3: ")),
                "{args:?}: {err}"
            );
            assert_eq!(err.lines().count(), 1, "{err}");
        }
    }

    #[test]
    fn a_corpus_line_with_no_text_exits_2_with_the_message() {
        let dir = tempfile::tempdir().unwrap();
        let (corpus, languages) = files(&dir);
        let no_text = dir.path().join("no-text.jsonl");
        std::fs::write(
            &no_text,
            "{\"text\": \"I saw teh cat.\"}\n{\"sentence\": \"I saw teh dog.\"}\n",
        )
        .unwrap();
        let no_text = no_text.to_str().unwrap();
        for args in [[no_text, languages.as_str()], [corpus.as_str(), no_text]] {
            let (code, out, err) = run_capturing(&args, "y\n");
            assert_eq!(code, 2, "{args:?}: {err}");
            assert!(out.is_empty(), "{args:?}: {out}");
            assert_eq!(err, format!("error: {no_text}: a line has no \"text\".\n"));
        }
    }

    // A provider error that is not an unusable answer stops the run at once, so a key that is
    // refused or a balance that runs out does not go on to spend on the remaining calls.
    #[test]
    fn a_provider_error_stops_the_run_and_sends_nothing_more() {
        // The messages of docs/modules/03-publik.md (402 with no message of its own, and 401)
        // and docs/modules/02-provider-layer.md (a key refused).
        let cases = [
            (
                "publik-env",
                402,
                "Your publik balance does not cover this request.",
            ),
            ("publik-env", 401, "publik API disconnected this computer."),
            (
                "fake",
                401,
                "Your key was refused (401). Check it in Settings → Providers.",
            ),
        ];
        for (provider, status, message) in cases {
            let dir = tempfile::tempdir().unwrap();
            let (corpus, languages) = files(&dir);
            // Every later request has a valid answer, so a run that went on would answer them.
            let mut entries = six_answers();
            entries[0] = reply(&fix_request(), status, json!({}));
            let recording = write_recording(&dir, entries);
            let env = |name: &str| (name == "PUBLIK_API_KEY").then(|| "pk_live_a".to_string());
            let (code, out, err) = run_in_env(
                &[
                    "--provider",
                    provider,
                    "--recording",
                    &recording,
                    &corpus,
                    &languages,
                ],
                "y\n",
                env,
            );
            let case = format!("{provider} {status}");
            assert_eq!(code, 1, "{case}: {out}\n{err}");
            assert_eq!(
                err.lines().next(),
                Some(format!("eval-rewrite: stopped after 1 call: {message}").as_str()),
                "{case}: {err}"
            );
            // The plan's two lines, then the report on a line of its own.
            assert_eq!(out.lines().count(), 3, "{case}: {out}");
            assert!(out.contains("Send them? [y/N] \n{"), "{case}: {out}");
            let last: Value = serde_json::from_str(out.lines().last().unwrap()).unwrap();
            assert_eq!(last["provider"], provider, "{case}");
            assert_eq!(last["calls"], 1, "{case}: nothing more is sent");
            assert_eq!(last["errors"], 1, "{case}");
            assert_eq!(last["charge_micros"], 0, "{case}");
            assert_eq!(last["fix_grammar"]["answered"], 0, "{case}");
            assert_eq!(last["language_keeping"]["answered"], 0, "{case}");
            assert_eq!(last["pass"], false, "{case}");
        }
    }

    // An unusable answer is counted and the run goes on.
    #[test]
    fn an_unusable_answer_is_counted_and_the_run_goes_on() {
        let dir = tempfile::tempdir().unwrap();
        let (corpus, languages) = files(&dir);
        let mut entries = six_answers();
        entries[0] = answer(&fix_request(), json!({"txt": "I saw the cat."}));
        let recording = write_recording(&dir, entries);
        let (code, out, err) = run_capturing(
            &[
                "--provider",
                "fake",
                "--recording",
                &recording,
                &corpus,
                &languages,
            ],
            "y\n",
        );
        // No Fix grammar answer to measure, so the gate fails, but every call is made.
        assert_eq!(code, 1, "{out}\n{err}");
        assert!(!err.contains("stopped"), "{err}");
        let last: Value = serde_json::from_str(out.lines().last().unwrap()).unwrap();
        assert_eq!(last["calls"], 6);
        assert_eq!(last["errors"], 1);
        assert_eq!(last["fix_grammar"]["answered"], 0);
        assert_eq!(last["language_keeping"]["answered"], 5);
        assert_eq!(last["language_keeping"]["kept"], 5);
        assert_eq!(last["pass"], false);
    }

    #[test]
    fn a_first_call_without_a_charge_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let (corpus, languages) = files(&dir);
        let mut entries = six_answers();
        for e in &mut entries {
            e.headers.clear();
        }
        let recording = write_recording(&dir, entries);
        let (_, out, err) = run_capturing(
            &[
                "--provider",
                "fake",
                "--recording",
                &recording,
                &corpus,
                &languages,
            ],
            "yes\n",
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[2], "eval-rewrite: the first call reported no charge.",
            "{out}\n{err}"
        );
        let last: Value = serde_json::from_str(lines[3]).unwrap();
        assert_eq!(last["calls"], 6);
        assert_eq!(last["charge_micros"], 0);
    }

    #[test]
    fn declining_sends_nothing_and_exits_0() {
        let dir = tempfile::tempdir().unwrap();
        let (corpus, languages) = files(&dir);
        for input in ["n\n", "\n", "", "no\n"] {
            // publik-env without a key: nothing is set up before the answer.
            let (code, out, err) = run_capturing(&[&corpus, &languages], input);
            assert_eq!((code, err.as_str()), (0, ""), "{input:?}");
            assert!(out.starts_with("eval-rewrite: 6 calls: 1 Fix grammar on publik-fast (one per corpus sentence, at most 100) and 5 rewrites (1 paragraphs × Clearer, Shorter, More formal, Friendlier and Custom) on publik-balanced, about $0.01 on publik API.\nSend them? [y/N] "), "{out}");
            assert!(out.ends_with("eval-rewrite: nothing was sent.\n"), "{out}");
        }
    }

    #[test]
    fn a_recorded_run_reports_its_measures() {
        let dir = tempfile::tempdir().unwrap();
        let (corpus, languages) = files(&dir);
        let recording = write_recording(&dir, six_answers());
        let (code, out, err) = run_capturing(
            &[
                "--provider",
                "fake",
                "--recording",
                &recording,
                &corpus,
                &languages,
            ],
            "y\n",
        );
        // One of four words changed is above the 0.15 median, so the gate fails with exit 1.
        assert_eq!(code, 1, "{out}\n{err}");
        assert!(
            out.contains("the first call cost $0.000100 (100 micro-dollars)."),
            "{out}"
        );
        let last: Value = serde_json::from_str(out.lines().last().unwrap()).unwrap();
        assert_eq!(last["calls"], 6);
        assert_eq!(last["errors"], 0);
        assert_eq!(last["charge_micros"], 600);
        assert_eq!(last["fix_grammar"]["median_edit_ratio"], "0.250");
        assert_eq!(last["fix_grammar"]["exact"], 1);
        assert_eq!(last["language_keeping"]["kept"], 5);
        // One of four words changed is above the 0.15 median, so the gate fails.
        assert_eq!(last["pass"], false);
        assert!(
            err.contains("median edit ratio 0.250 (limit 0.15)"),
            "{err}"
        );
    }

    #[test]
    fn medians_and_decimals_use_whole_numbers() {
        assert_eq!(median_ratio(&[]), None);
        assert_eq!(median_ratio(&[(1, 4)]), Some((1, 4)));
        assert_eq!(median_ratio(&[(3, 4), (0, 5), (1, 10)]), Some((1, 10)));
        // Even count: the mean of 1/10 and 1/5 is 3/20.
        let (n, d) = median_ratio(&[(1, 5), (0, 3), (1, 10), (9, 9)]).unwrap();
        assert_eq!(n * 20, 3 * d);
        assert_eq!(median_ratio(&[(0, 0)]), Some((0, 1)));
        assert_eq!(decimal(1, 4, 3), "0.250");
        assert_eq!(decimal(2, 3, 2), "0.67");
        assert_eq!(decimal(429_600, 1_000_000, 2), "0.43");
        assert_eq!(decimal(5, 1, 0), "5");
        assert_eq!(dollars(429_600), "$0.43");
    }

    #[test]
    fn options_parse() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse(&args(&["c.jsonl", "l.jsonl"])).unwrap(),
            Opts {
                provider: ProviderArg::PublikEnv,
                recording: None,
                corpus: "c.jsonl".into(),
                languages: "l.jsonl".into(),
            }
        );
        assert_eq!(
            parse(&args(&["--provider", "own-key", "c", "l"]))
                .unwrap()
                .provider,
            ProviderArg::OwnKey
        );
        assert_eq!(
            parse(&args(&["c"])).unwrap_err().message,
            "eval-rewrite needs a CORPUS and a LANGUAGES file. Run redpen --help."
        );
        assert_eq!(
            parse(&args(&["c", "l", "x"])).unwrap_err().message,
            "unknown argument x. Run redpen --help."
        );
        // The gate's own numbers: 100 Fix grammar calls and 200 rewrites (docs/TESTING.md §4).
        assert!(plan(ProviderArg::OwnKey, 100, 200, 40).contains("on your own key's model"));
        assert!(plan(ProviderArg::PublikEnv, 100, 200, 40).contains("300 calls: 100 Fix grammar"));
        assert!(plan(ProviderArg::PublikEnv, 100, 200, 40).contains("about $0.44 on publik API"));
    }
}
