//! `redpen`: the command-line tool the golden tests drive (docs/ARCHITECTURE.md §5). It never
//! provisions Publik. Module 00 gives it `--version`; `check`, `fix-all`, `rewrite`, `prompt`,
//! `redact`, `diff`, `chat` and `plagiarism` arrive with their modules.

use std::io::Write;

const NAME: &str = env!("CARGO_BIN_NAME");
const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "Usage: redpen --version [--json]\n       redpen --help\n";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let code = run(&args, &mut stdout.lock(), &mut stderr.lock());
    std::process::exit(code);
}

/// Runs the tool and returns its exit code: 0 for success, 2 for a usage error.
fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
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
        let code = run(&args, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
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
    }
}
