//! `redpen redact FILE` (docs/modules/04-rewrite-engine.md): prints the file's text with its
//! private details replaced by markers, and the entities in order of first appearance:
//! `{"text": "<redacted text>", "entities": [{"marker": "[EMAIL_1]", "kind": "email", "value":
//! "…"}]}`. The file is read as it is. A text that already holds a marker prints
//! `{"error": {...}}` and exits 3; usage errors exit 2.

use std::io::Write;

use redpen_core::redact::{Entity, redact};
use serde::Serialize;

use super::rewrite::refused;
use super::{Failure, fail, files, print, read_text};

#[derive(Serialize)]
struct Output<'a> {
    text: &'a str,
    entities: &'a [Entity],
}

pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let files = match files(args, 1) {
        Ok(files) if files.len() == 1 => files,
        Ok(_) => {
            return fail(
                Failure::usage("redact needs a FILE. Run redpen --help."),
                err,
            );
        }
        Err(failure) => return fail(failure, err),
    };
    let text = match read_text(files[0]) {
        Ok(text) => text,
        Err(failure) => return fail(failure, err),
    };
    match redact(&text) {
        Ok((text, entities)) => print(
            &Output {
                text: &text,
                entities: &entities,
            },
            out,
        ),
        Err(e) => refused(&e, out, err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn run_on(content: &str) -> (i32, Value) {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.txt");
        std::fs::write(&input, content).unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&[input.to_str().unwrap().to_string()], &mut out, &mut err);
        (code, serde_json::from_slice(&out).unwrap())
    }

    #[test]
    fn prints_the_text_and_its_entities() {
        let (code, value) = run_on("Mail ana@example.com or call 020 7946 0958.\n");
        assert_eq!(code, 0);
        assert_eq!(
            value,
            json!({
                "text": "Mail [EMAIL_1] or call [PHONE_1].\n",
                "entities": [
                    {"marker": "[EMAIL_1]", "kind": "email", "value": "ana@example.com"},
                    {"marker": "[PHONE_1]", "kind": "phone", "value": "020 7946 0958"}
                ]
            })
        );
    }

    #[test]
    fn a_marker_in_the_text_exits_3() {
        let (code, value) = run_on("Mail [EMAIL_1].");
        assert_eq!(code, 3);
        assert_eq!(value["error"]["kind"], "RedactionCollision");
    }

    #[test]
    fn usage_errors_exit_2() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(run(&[], &mut out, &mut err), 2);
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "error: redact needs a FILE. Run redpen --help.\n"
        );
        let mut err = Vec::new();
        let two = ["a.txt".to_string(), "b.txt".to_string()];
        assert_eq!(run(&two, &mut out, &mut err), 2);
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "error: unknown argument b.txt. Run redpen --help.\n"
        );
        assert!(out.is_empty());
    }
}
