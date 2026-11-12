//! The subcommands of `redpen` (docs/ARCHITECTURE.md §5), one file each, and what they share:
//! option parsing, reading the input file, and the exit codes.
//!
//! Exit codes: 0 success; 1 a gate that measured a failure (`check-speed`); 2 a usage error, a
//! file that cannot be read as UTF-8 text, or a text the core refused (`TooLong`).

pub mod check;
pub mod check_speed;
pub mod fix_all;

use std::io::Write;

use redpen_core::check::{Category, CheckError, Dialect};
use serde::Serialize;

pub const OK: i32 = 0;
pub const GATE_FAILED: i32 = 1;
pub const USAGE: i32 = 2;

/// A failed command: what to print on standard error, and the exit code.
#[derive(Debug, PartialEq, Eq)]
pub struct Failure {
    pub code: i32,
    pub message: String,
}

impl Failure {
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            code: USAGE,
            message: message.into(),
        }
    }
}

/// A command's result as one line of JSON on standard output, fields in declaration order.
pub fn print(value: &impl Serialize, out: &mut dyn Write) -> i32 {
    let line = serde_json::to_string(value).expect("command output serialises");
    let _ = writeln!(out, "{line}");
    OK
}

/// A failed command: its message on standard error, and its exit code.
pub fn fail(failure: Failure, err: &mut dyn Write) -> i32 {
    let _ = writeln!(err, "error: {}", failure.message);
    failure.code
}

/// A refusal from the core: `{"error": {"kind", "message", ...}}` on standard output, so the
/// golden harness can compare it, and the message on standard error. Exit 2.
pub fn refused(e: &CheckError, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    #[derive(Serialize)]
    struct Refusal<'a> {
        error: &'a CheckError,
    }
    print(&Refusal { error: e }, out);
    let _ = writeln!(err, "error: {e}");
    USAGE
}

/// The options shared by the checking commands, parsed from `args`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Opts {
    pub dialect: Dialect,
    pub disabled: Vec<Category>,
    pub runs: Option<usize>,
    pub file: Option<String>,
}

/// Which options a command accepts besides `FILE`.
#[derive(Debug, Clone, Copy)]
pub struct Accepts {
    pub disable: bool,
    pub runs: bool,
}

/// Parses `[--dialect D] [--disable C]... [--runs N] FILE` for the command `name`.
pub fn parse(name: &str, args: &[String], accepts: Accepts) -> Result<Opts, Failure> {
    let mut opts = Opts::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .ok_or_else(|| Failure::usage(format!("{flag} needs a value. Run redpen --help.")))
        };
        match arg.as_str() {
            "--dialect" => {
                opts.dialect = value("--dialect")?.parse().map_err(
                    |e: redpen_core::check::UnknownDialect| Failure::usage(e.to_string()),
                )?;
            }
            "--disable" if accepts.disable => {
                let category: Category = value("--disable")?.parse().map_err(
                    |e: redpen_core::check::UnknownCategory| Failure::usage(e.to_string()),
                )?;
                if !opts.disabled.contains(&category) {
                    opts.disabled.push(category);
                }
            }
            "--runs" if accepts.runs => {
                let runs = value("--runs")?;
                opts.runs = Some(runs.parse().ok().filter(|n: &usize| *n > 0).ok_or_else(
                    || Failure::usage(format!("--runs takes a whole number above 0, not {runs}.")),
                )?);
            }
            other if other.starts_with('-') && other != "-" => {
                return Err(Failure::usage(format!(
                    "unknown argument {other}. Run redpen --help."
                )));
            }
            file if opts.file.is_none() => opts.file = Some(file.to_string()),
            extra => {
                return Err(Failure::usage(format!(
                    "unknown argument {extra}. Run redpen --help."
                )));
            }
        }
    }
    if opts.file.is_none() {
        return Err(Failure::usage(format!(
            "{name} needs a FILE. Run redpen --help."
        )));
    }
    Ok(opts)
}

/// The whole of `path` as UTF-8 text, exactly as stored: no line ending or byte-order mark is
/// removed, as Harper's own linter reads a `.txt` file.
pub fn read_text(path: &str) -> Result<String, Failure> {
    let bytes =
        std::fs::read(path).map_err(|e| Failure::usage(format!("could not read {path}: {e}")))?;
    String::from_utf8(bytes).map_err(|_| Failure::usage(format!("{path} is not UTF-8 text.")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    const CHECK: Accepts = Accepts {
        disable: true,
        runs: false,
    };
    const FIX_ALL: Accepts = Accepts {
        disable: false,
        runs: false,
    };

    #[test]
    fn options_parse() {
        let opts = parse(
            "check",
            &args(&[
                "--dialect",
                "british",
                "--disable",
                "spelling",
                "--disable",
                "style",
                "--disable",
                "spelling",
                "in.txt",
            ]),
            CHECK,
        )
        .unwrap();
        assert_eq!(
            opts,
            Opts {
                dialect: Dialect::British,
                disabled: vec![Category::Spelling, Category::Style],
                runs: None,
                file: Some("in.txt".into()),
            }
        );
        let opts = parse("fix-all", &args(&["in.txt"]), FIX_ALL).unwrap();
        assert_eq!(opts.dialect, Dialect::American);
        assert!(opts.disabled.is_empty());
    }

    #[test]
    fn usage_errors_say_what_to_do() {
        let fail = |a: &[&str], accepts| parse("check", &args(a), accepts).unwrap_err();
        assert_eq!(
            fail(&[], CHECK),
            Failure::usage("check needs a FILE. Run redpen --help.")
        );
        assert_eq!(
            fail(&["--dialect"], CHECK).message,
            "--dialect needs a value. Run redpen --help."
        );
        assert_eq!(
            fail(&["--dialect", "us", "f"], CHECK).message,
            "Unknown dialect \"us\". Use american, british, australian, canadian or indian."
        );
        assert_eq!(
            fail(&["--disable", "tone", "f"], CHECK).message,
            "Unknown category \"tone\". Use spelling, grammar, punctuation or style."
        );
        assert_eq!(
            fail(&["--disable", "spelling", "f"], FIX_ALL).message,
            "unknown argument --disable. Run redpen --help."
        );
        assert_eq!(
            fail(&["a.txt", "b.txt"], CHECK).message,
            "unknown argument b.txt. Run redpen --help."
        );
        let runs = Accepts {
            disable: false,
            runs: true,
        };
        assert_eq!(
            fail(&["--runs", "0", "f"], runs).message,
            "--runs takes a whole number above 0, not 0."
        );
        assert_eq!(
            parse("check-speed", &args(&["--runs", "3", "f"]), runs)
                .unwrap()
                .runs,
            Some(3)
        );
        assert_eq!(
            fail(&["--runs", "3", "f"], CHECK).message,
            "unknown argument --runs. Run redpen --help."
        );
    }

    #[test]
    fn read_text_keeps_the_file_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("in.txt");
        std::fs::write(&path, "\u{feff}teh\r\n").unwrap();
        assert_eq!(
            read_text(path.to_str().unwrap()).unwrap(),
            "\u{feff}teh\r\n"
        );
        std::fs::write(&path, [0x66, 0xff]).unwrap();
        let failure = read_text(path.to_str().unwrap()).unwrap_err();
        assert_eq!(failure.code, USAGE);
        assert!(failure.message.ends_with("is not UTF-8 text."));
        let missing = dir.path().join("missing.txt");
        let failure = read_text(missing.to_str().unwrap()).unwrap_err();
        assert!(failure.message.starts_with("could not read "));
    }
}
