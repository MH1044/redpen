//! `redpen`: the command-line tool the golden tests and the maintainer's scripts drive
//! (docs/ARCHITECTURE.md §5). It never provisions Publik. Module 00 gives it `--version`; module
//! 01 `check`, `fix-all` and the `check-speed` measurement; module 02 `chat` and the
//! `--provider` choice; module 04 `rewrite`, `prompt`, `redact`, `diff`, the `eval-rewrite`
//! measurement and `--provider publik-env`; `plagiarism` arrives with its module.

mod cmd;
mod providers;

use std::io::{BufRead, Write};

const NAME: &str = env!("CARGO_BIN_NAME");
const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
Usage: redpen --version [--json]
       redpen check [--dialect D] [--disable C]... FILE
       redpen fix-all [--dialect D] FILE
       redpen check-speed [--dialect D] [--runs N] FILE
       redpen chat --provider P [--recording R] --message S
       redpen rewrite --action A [--dialect D] [--style S] [--instruction I] [--redact]
                      --provider P [--recording R] FILE
       redpen prompt --action A [--dialect D] [--style S] [--instruction I] [--redact] FILE
       redpen redact FILE
       redpen diff OLD NEW
       redpen eval-rewrite [--provider P] [--recording R] CORPUS LANGUAGES
       redpen --help

D: american (the default), british, australian, canadian or indian.
C: spelling, grammar, punctuation or style.
A: fix-grammar, clearer, shorter, more-formal, friendlier or custom (needs --instruction).
S: own (the default; style presets arrive in a later version).
P: fake, local (Ollama on this computer), own-key (reads REDPEN_OWN_KEY,
   REDPEN_OWN_BASE_URL and REDPEN_OWN_MODEL) or publik-env (reads PUBLIK_API_KEY and
   PUBLIK_API_BASE_URL). R: a recording.json to answer from.
eval-rewrite spends money on publik API: it asks before sending anything.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let code = run(
        &args,
        &mut stdin.lock(),
        &mut stdout.lock(),
        &mut stderr.lock(),
    );
    std::process::exit(code);
}

/// Runs the tool and returns its exit code (`cmd` lists them).
fn run(args: &[String], input: &mut dyn BufRead, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match args.first().map(String::as_str) {
        Some("check") => return cmd::check::run(&args[1..], out, err),
        Some("fix-all") => return cmd::fix_all::run(&args[1..], out, err),
        Some("check-speed") => return cmd::check_speed::run(&args[1..], out, err),
        Some("chat") => return cmd::chat::run(&args[1..], out, err),
        Some("rewrite") => return cmd::rewrite::run(&args[1..], out, err),
        Some("prompt") => return cmd::prompt::run(&args[1..], out, err),
        Some("redact") => return cmd::redact::run(&args[1..], out, err),
        Some("diff") => return cmd::diff::run(&args[1..], out, err),
        Some("eval-rewrite") => return cmd::eval_rewrite::run(&args[1..], input, out, err),
        _ => {}
    }
    let mut version = false;
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--version" | "-V" => version = true,
            "--json" => json = true,
            "--help" | "-h" => {
                let _ = out.write_all(USAGE.as_bytes());
                return 0;
            }
            other => {
                let _ = writeln!(err, "error: unknown argument {other}. Run redpen --help.");
                return 2;
            }
        }
    }
    if version {
        let line = if json {
            serde_json::json!({"name": NAME, "version": VERSION}).to_string()
        } else {
            format!("{NAME} {VERSION}")
        };
        let _ = writeln!(out, "{line}");
        return 0;
    }
    let _ = out.write_all(USAGE.as_bytes());
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_capturing(args: &[&str]) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&args, &mut "".as_bytes(), &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    /// A temporary directory holding `name` with `content`, and the file's path.
    fn file(name: &str, content: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        let path = path.to_str().unwrap().to_string();
        (dir, path)
    }

    fn json(text: &str) -> serde_json::Value {
        serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
    }

    #[test]
    fn version_json_is_the_one_line_of_the_spec() {
        let (code, out, err) = run_capturing(&["--version", "--json"]);
        assert_eq!(code, 0);
        assert_eq!(out, "{\"name\":\"redpen\",\"version\":\"0.1.0\"}\n");
        assert!(err.is_empty());
    }

    #[test]
    fn version_plain_and_help() {
        let (code, out, _) = run_capturing(&["--version"]);
        assert_eq!((code, out.as_str()), (0, "redpen 0.1.0\n"));
        let (code, out, _) = run_capturing(&["--help"]);
        assert_eq!(code, 0);
        assert!(out.starts_with("Usage: redpen"));
        assert!(out.contains("redpen check [--dialect D] [--disable C]... FILE"));
        assert!(out.contains("redpen fix-all [--dialect D] FILE"));
        assert!(out.contains("redpen chat --provider P [--recording R] --message S"));
        assert!(out.contains("redpen redact FILE"));
        assert!(out.contains("redpen diff OLD NEW"));
        assert!(out.contains("redpen prompt --action A"));
        assert!(out.contains("redpen rewrite --action A"));
        let (code, out, _) = run_capturing(&[]);
        assert_eq!(code, 0);
        assert!(out.starts_with("Usage: redpen"));
    }

    #[test]
    fn an_unknown_argument_exits_2_with_the_message() {
        let (code, out, err) = run_capturing(&["--bogus"]);
        assert_eq!(code, 2);
        assert!(out.is_empty());
        assert_eq!(err, "error: unknown argument --bogus. Run redpen --help.\n");
        let (code, _, err) = run_capturing(&["checks"]);
        assert_eq!(code, 2);
        assert_eq!(err, "error: unknown argument checks. Run redpen --help.\n");
    }

    #[test]
    fn check_prints_the_findings() {
        // Harper 3.0.0's own linter reports a Typo on "teh" with "the" first.
        let (_dir, path) = file("in.txt", "I saw teh cat.");
        let (code, out, err) = run_capturing(&["check", "--dialect", "american", &path]);
        assert_eq!(code, 0, "{err}");
        let value = json(&out);
        let findings = value["findings"].as_array().unwrap();
        assert_eq!(findings.len(), 1, "{value}");
        assert_eq!(findings[0]["category"], "spelling");
        assert_eq!(findings[0]["harper_kind"], "Typo");
        assert_eq!(findings[0]["span"]["chars"], serde_json::json!([6, 9]));
        assert_eq!(
            findings[0]["suggestions"][0],
            serde_json::json!({"kind": "replace", "text": "the"})
        );
        assert!(out.ends_with("}\n") && out.lines().count() == 1);
    }

    #[test]
    fn check_drops_a_disabled_category() {
        let (_dir, path) = file("in.txt", "I saw teh cat.");
        let (code, out, _) = run_capturing(&["check", "--disable", "spelling", &path]);
        assert_eq!(code, 0);
        assert_eq!(json(&out), serde_json::json!({"findings": []}));
    }

    #[test]
    fn check_of_an_empty_file_has_no_findings() {
        let (_dir, path) = file("empty.txt", "");
        let (code, out, err) = run_capturing(&["check", &path]);
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(json(&out), serde_json::json!({"findings": []}));
    }

    #[test]
    fn fix_all_prints_the_fixed_text() {
        let (_dir, path) = file("in.txt", "I saw teh cat.");
        let (code, out, err) = run_capturing(&["fix-all", "--dialect", "british", &path]);
        assert_eq!(code, 0, "{err}");
        assert_eq!(json(&out), serde_json::json!({"text": "I saw the cat."}));
    }

    #[test]
    fn a_text_over_the_limit_is_refused_with_exit_2() {
        let (_dir, path) = file("long.txt", &"a ".repeat(10_000).replacen(' ', "b ", 1));
        let message =
            "This selection is too long to check (over 20,000 characters). Select less text.";
        for command in ["check", "fix-all"] {
            let (code, out, err) = run_capturing(&[command, &path]);
            assert_eq!(code, 2, "{command}");
            assert_eq!(
                json(&out),
                serde_json::json!({"error": {"kind": "TooLong", "message": message, "chars": 20_001}})
            );
            assert_eq!(err, format!("error: {message}\n"));
        }
    }

    #[test]
    fn a_missing_file_and_bad_options_exit_2() {
        let (code, out, err) = run_capturing(&["check", "no-such-file.txt"]);
        assert_eq!(code, 2);
        assert!(out.is_empty());
        assert!(
            err.starts_with("error: could not read no-such-file.txt: "),
            "{err}"
        );
        let (code, _, err) = run_capturing(&["fix-all", "--disable", "style", "x.txt"]);
        assert_eq!(code, 2);
        assert_eq!(
            err,
            "error: unknown argument --disable. Run redpen --help.\n"
        );
        let (code, _, err) = run_capturing(&["check"]);
        assert_eq!(code, 2);
        assert_eq!(err, "error: check needs a FILE. Run redpen --help.\n");
    }

    #[test]
    fn check_speed_reports_the_median() {
        let (_dir, path) = file(
            "corpus.jsonl",
            "{\"text\": \"I saw teh cat.\"}\n{\"text\": \"She will recieve the letter.\"}\n",
        );
        let (code, out, err) = run_capturing(&["check-speed", "--runs", "3", &path]);
        let value = json(&out);
        assert_eq!(value["gate"], "check-speed");
        assert_eq!(value["words"], 1_000);
        assert_eq!(value["runs"], 3);
        assert_eq!(value["limit_us"], 50_000);
        let pass = value["pass"].as_bool().unwrap();
        assert_eq!(code, if pass { 0 } else { 1 });
        assert!(err.starts_with("check-speed: median "), "{err}");
        assert!(
            err.contains("over 3 checks of a 1000-word text (limit 50 ms)"),
            "{err}"
        );
    }
}
