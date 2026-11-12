//! `redpen check-speed [--dialect D] [--runs N] FILE`: the measurement behind the `check-speed`
//! gate (docs/TESTING.md §4), run by `scripts/gates.sh check-speed` on a release build.
//!
//! FILE is the module 01 corpus (`corpus.jsonl`, one sentence per line in its `text` field) or
//! any plain text file. Its sentences are joined, repeated as needed, into a text of exactly
//! 1,000 words (UAX #29 words, the count Redpen uses everywhere). One untimed check loads the
//! dictionary; then each run builds a fresh checker, untimed, and times one check of the text,
//! so no run is served from the previous run's cache: that is the wait for a new selection. The
//! gated figure is the median of those runs; above 50 ms the gate fails with exit code 1.
//!
//! For information it also reports `recheck_median_us`: the same text checked again on one
//! checker, where Harper's per-chunk cache serves the unchanged text, as when the popup checks a
//! text again after a fix.

use std::io::Write;
use std::time::{Duration, Instant};

use redpen_core::check::{CheckOptions, Checker};
use serde::Serialize;
use unicode_segmentation::UnicodeSegmentation;

use super::{Accepts, Failure, GATE_FAILED, OK, fail, parse, print, read_text};

pub const ACCEPTS: Accepts = Accepts {
    disable: false,
    runs: true,
};

/// The size of the timed text, in words.
pub const WORDS: usize = 1_000;
/// Checks timed when `--runs` is not given.
pub const RUNS: usize = 10;
/// The gate's limit for the median check (docs/TESTING.md §4).
pub const LIMIT: Duration = Duration::from_millis(50);

/// What the gate prints: one line of JSON, times in whole microseconds.
#[derive(Debug, Serialize)]
struct Report {
    gate: &'static str,
    words: usize,
    chars: usize,
    findings: usize,
    runs: usize,
    median_us: u64,
    min_us: u64,
    max_us: u64,
    recheck_median_us: u64,
    limit_us: u64,
    pass: bool,
}

pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let report = match measure(args) {
        Ok(report) => report,
        Err(failure) => return fail(failure, err),
    };
    let _ = writeln!(
        err,
        "check-speed: median {} ms over {} checks of a {}-word text (limit {} ms): {}; \
         checking the same text again: {} ms",
        format_ms(report.median_us),
        report.runs,
        report.words,
        LIMIT.as_millis(),
        if report.pass { "pass" } else { "FAIL" },
        format_ms(report.recheck_median_us),
    );
    print(&report, out);
    if report.pass { OK } else { GATE_FAILED }
}

fn measure(args: &[String]) -> Result<Report, Failure> {
    let opts = parse("check-speed", args, ACCEPTS)?;
    let path = opts.file.as_deref().unwrap_or_default();
    let content = read_text(path)?;
    let sentences = if path.ends_with(".jsonl") {
        jsonl_sentences(&content).map_err(|e| Failure::usage(format!("{path}: {e}")))?
    } else {
        vec![content]
    };
    let text = text_of_words(&sentences, WORDS)
        .ok_or_else(|| Failure::usage(format!("{path} has no words to check.")))?;
    let runs = opts.runs.unwrap_or(RUNS);
    let options = CheckOptions {
        dialect: opts.dialect,
        disabled: Vec::new(),
    };

    let warm_up = Checker::new(options.clone());
    let findings = warm_up
        .check(&text)
        .map_err(|e| Failure::usage(e.to_string()))?
        .len();
    let mut times: Vec<Duration> = Vec::with_capacity(runs);
    for _ in 0..runs {
        let checker = Checker::new(options.clone());
        let started = Instant::now();
        let result = checker.check(&text);
        times.push(started.elapsed());
        result.map_err(|e| Failure::usage(e.to_string()))?;
    }
    let cold = median(&mut times);
    // The warm-up checker has seen the text once, so every check here finds its chunks cached.
    let mut rechecks: Vec<Duration> = Vec::with_capacity(runs);
    for _ in 0..runs {
        let started = Instant::now();
        let result = warm_up.check(&text);
        rechecks.push(started.elapsed());
        result.map_err(|e| Failure::usage(e.to_string()))?;
    }
    let recheck_median = median(&mut rechecks);
    Ok(Report {
        gate: "check-speed",
        words: WORDS,
        chars: text.chars().count(),
        findings,
        runs,
        median_us: micros(cold),
        min_us: micros(times[0]),
        max_us: micros(times[times.len() - 1]),
        recheck_median_us: micros(recheck_median),
        limit_us: micros(LIMIT),
        pass: passes(cold),
    })
}

/// Whether a median check passes the gate: at most [`LIMIT`], so the gate fails above 50 ms.
fn passes(median: Duration) -> bool {
    median <= LIMIT
}

/// The sentence of every non-empty line of a JSON Lines corpus.
fn jsonl_sentences(content: &str) -> Result<Vec<String>, String> {
    let mut sentences = Vec::new();
    for (i, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("line {} is not JSON: {e}", i + 1))?;
        let sentence = value["text"]
            .as_str()
            .ok_or_else(|| format!("line {} has no \"text\" field", i + 1))?;
        sentences.push(sentence.to_string());
    }
    Ok(sentences)
}

/// The sentences joined by spaces, repeated as needed, and cut just after word number `words`.
/// `None` when the sentences hold no word at all.
fn text_of_words(sentences: &[String], words: usize) -> Option<String> {
    let per_pass: usize = sentences.iter().map(|s| s.unicode_words().count()).sum();
    if per_pass == 0 || words == 0 {
        return None;
    }
    let passes = words.div_ceil(per_pass);
    let joined = vec![sentences.join(" "); passes].join(" ");
    let (start, last) = joined.unicode_word_indices().nth(words - 1)?;
    Some(joined[..start + last.len()].to_string())
}

/// The middle of the durations (the mean of the two middle ones for an even count); sorts them.
fn median(times: &mut [Duration]) -> Duration {
    times.sort();
    let n = times.len();
    if n % 2 == 1 {
        times[n / 2]
    } else {
        (times[n / 2 - 1] + times[n / 2]) / 2
    }
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// Microseconds as milliseconds with one decimal, by integer maths.
fn format_ms(us: u64) -> String {
    let tenths = (us + 50) / 100;
    format!("{}.{}", tenths / 10, tenths % 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_has_exactly_the_words_asked_for() {
        let sentences = vec![
            "I saw teh cat.".to_string(),
            "It sat, on an mat!".to_string(),
        ];
        let text = text_of_words(&sentences, 10).unwrap();
        assert_eq!(text.unicode_words().count(), 10);
        // 4 + 5 words in one pass, so the 10th is the first word of the second pass.
        assert_eq!(text, "I saw teh cat. It sat, on an mat! I");
        assert_eq!(
            text_of_words(&sentences, 1_000)
                .unwrap()
                .unicode_words()
                .count(),
            1_000
        );
        assert_eq!(text_of_words(&["...".to_string()], 10), None);
        assert_eq!(text_of_words(&[], 10), None);
    }

    /// `run` on a file holding `content`: the exit code, standard output and standard error,
    /// and the file's path.
    fn run_on(name: &str, content: &str) -> (i32, String, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        let path = path.to_str().unwrap().to_string();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(
            &["--runs".into(), "1".into(), path.clone()],
            &mut out,
            &mut err,
        );
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
            path,
        )
    }

    #[test]
    fn a_file_with_no_words_exits_2_with_the_message() {
        for (name, content) in [
            ("dots.txt", "..."),
            ("empty.txt", ""),
            ("dots.jsonl", "{\"text\": \"...\"}\n"),
        ] {
            let (code, out, err, path) = run_on(name, content);
            assert_eq!(code, 2, "{name}");
            assert!(out.is_empty(), "{name}: {out}");
            assert_eq!(err, format!("error: {path} has no words to check.\n"));
        }
    }

    #[test]
    fn a_corpus_line_that_is_not_json_exits_2_with_the_message() {
        let (code, out, err, path) = run_on("corpus.jsonl", "{\"text\": \"One.\"}\nnope\n");
        assert_eq!(code, 2);
        assert!(out.is_empty(), "{out}");
        assert!(
            err.starts_with(&format!("error: {path}: line 2 is not JSON")),
            "{err}"
        );
        let (code, _, err, path) = run_on("corpus.jsonl", "{\"id\": \"c01\"}\n");
        assert_eq!(code, 2);
        assert_eq!(
            err,
            format!("error: {path}: line 1 has no \"text\" field\n")
        );
    }

    #[test]
    fn a_text_over_the_limit_exits_2_with_the_message() {
        // 1,000 words of 30 letters and the 999 spaces between them: 30,999 characters.
        let (code, out, err, _) = run_on("long.txt", &"a".repeat(30));
        assert_eq!(code, 2);
        assert!(out.is_empty(), "{out}");
        assert_eq!(
            err,
            "error: This selection is too long to check (over 20,000 characters). Select less \
             text.\n"
        );
    }

    #[test]
    fn corpus_lines_give_their_text() {
        let corpus = "{\"id\": \"c01\", \"text\": \"I saw teh cat.\"}\n\n{\"text\": \"Two.\"}\n";
        assert_eq!(
            jsonl_sentences(corpus).unwrap(),
            vec!["I saw teh cat.".to_string(), "Two.".to_string()]
        );
        assert_eq!(
            jsonl_sentences("{\"id\": 1}").unwrap_err(),
            "line 1 has no \"text\" field"
        );
        assert!(
            jsonl_sentences("nope")
                .unwrap_err()
                .starts_with("line 1 is not JSON")
        );
    }

    #[test]
    fn median_of_odd_and_even_counts() {
        let ms = Duration::from_millis;
        assert_eq!(median(&mut [ms(5), ms(1), ms(3)]), ms(3));
        assert_eq!(median(&mut [ms(4), ms(1), ms(2), ms(10)]), ms(3));
    }

    // Acceptance test 15: the gate fails above 50 ms (docs/TESTING.md §4: at most 50 ms passes).
    #[test]
    fn the_gate_fails_above_fifty_milliseconds() {
        assert_eq!(LIMIT, Duration::from_millis(50));
        assert!(passes(Duration::ZERO));
        assert!(passes(Duration::from_millis(50)));
        assert!(!passes(
            Duration::from_millis(50) + Duration::from_micros(1)
        ));
        assert!(!passes(Duration::from_micros(71_300)));
    }

    #[test]
    fn milliseconds_with_one_decimal() {
        assert_eq!(format_ms(12_345), "12.3");
        assert_eq!(format_ms(12_350), "12.4");
        assert_eq!(format_ms(999), "1.0");
        assert_eq!(format_ms(0), "0.0");
    }
}
