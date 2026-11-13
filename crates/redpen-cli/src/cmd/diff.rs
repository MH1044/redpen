//! `redpen diff OLD NEW` (docs/modules/04-rewrite-engine.md): prints the word diff from the text
//! of OLD to the text of NEW as one line, a list of one-key objects:
//! `[{"equal":"the "},{"delete":"cat"},{"insert":"dog"},{"equal":" sat"}]`.
//!
//! A file's final line ending (`\n` or `\r\n`) is not part of its text, so two one-line files
//! saved by an editor that ends every file with one diff as their lines do. Usage errors exit 2.

use std::io::Write;

use redpen_core::diff::word_diff;

use super::{Failure, fail, files, print, read_text};

/// `text` without one final `\n` or `\r\n`.
fn without_final_line_ending(text: &str) -> &str {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text)
}

pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let files = match files(args, 2) {
        Ok(files) if files.len() == 2 => files,
        Ok(_) => {
            return fail(
                Failure::usage("diff needs OLD and NEW files. Run redpen --help."),
                err,
            );
        }
        Err(failure) => return fail(failure, err),
    };
    let old = match read_text(files[0]) {
        Ok(text) => text,
        Err(failure) => return fail(failure, err),
    };
    let new = match read_text(files[1]) {
        Ok(text) => text,
        Err(failure) => return fail(failure, err),
    };
    print(
        &word_diff(
            without_final_line_ending(&old),
            without_final_line_ending(&new),
        ),
        out,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_on(old: &str, new: &str) -> (i32, String) {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("old.txt"), dir.path().join("new.txt"));
        std::fs::write(&a, old).unwrap();
        std::fs::write(&b, new).unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(
            &[
                a.to_str().unwrap().to_string(),
                b.to_str().unwrap().to_string(),
            ],
            &mut out,
            &mut err,
        );
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn the_cat_and_the_dog() {
        let expected = "[{\"equal\":\"the \"},{\"delete\":\"cat\"},{\"insert\":\"dog\"},{\"equal\":\" sat\"}]\n";
        assert_eq!(
            run_on("the cat sat", "the dog sat"),
            (0, expected.to_string())
        );
        assert_eq!(
            run_on("the cat sat\n", "the dog sat\r\n"),
            (0, expected.to_string())
        );
        // Only one final line ending goes.
        let (_, out) = run_on("a\n\n", "a\n\n");
        assert_eq!(out, "[{\"equal\":\"a\\n\"}]\n");
    }

    #[test]
    fn usage_errors_exit_2() {
        let mut out = Vec::new();
        for (args, message) in [
            (
                vec![],
                "error: diff needs OLD and NEW files. Run redpen --help.\n",
            ),
            (
                vec!["a"],
                "error: diff needs OLD and NEW files. Run redpen --help.\n",
            ),
            (
                vec!["a", "b", "c"],
                "error: unknown argument c. Run redpen --help.\n",
            ),
            (
                vec!["--json", "a"],
                "error: unknown argument --json. Run redpen --help.\n",
            ),
        ] {
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            let mut err = Vec::new();
            assert_eq!(run(&args, &mut out, &mut err), 2);
            assert_eq!(String::from_utf8(err).unwrap(), message);
        }
        assert!(out.is_empty());
    }
}
