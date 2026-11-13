//! `redpen prompt --action A [--dialect D] [--style S] [--instruction S] [--redact] FILE`
//! (docs/modules/04-rewrite-engine.md): prints the request body a rewrite of the file's text
//! sends to publik API, exactly as sent (after redaction with `--redact`), on one line, and sends
//! nothing. The tier context is Standard and anonymous. A refusal prints `{"error": {...}}` and
//! exits 3; usage errors exit 2.

use std::io::Write;

use redpen_core::provider::{ProviderKind, ResolvedProvider, TierContext};
use redpen_core::rewrite;

use super::rewrite::{parse, refused};
use super::{fail, print, read_text};
use crate::providers::PUBLIK_BASE_URL;

pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let opts = match parse("prompt", args, false) {
        Ok(opts) => opts,
        Err(failure) => return fail(failure, err),
    };
    let text = match read_text(&opts.file) {
        Ok(text) => text,
        Err(failure) => return fail(failure, err),
    };
    // publik API's provider, for the body only: no key is needed, since nothing is sent.
    let publik = ResolvedProvider {
        kind: ProviderKind::Publik,
        base_url: PUBLIK_BASE_URL.to_string(),
        key: None,
        chat_model: String::new(),
    };
    match rewrite::request_body(&publik, &TierContext::default(), &opts.request(text)) {
        Ok(body) => print(&body, out),
        Err(e) => refused(&e, out, err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn run_capturing(list: &[&str]) -> (i32, Value) {
        let args: Vec<String> = list.iter().map(|a| a.to_string()).collect();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&args, &mut out, &mut err);
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out.lines().count(), 1, "{out}");
        (code, serde_json::from_str(&out).unwrap())
    }

    #[test]
    fn prints_the_body_after_redaction() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.txt");
        std::fs::write(&input, "Mail ana@example.com today\n").unwrap();
        let input = input.to_str().unwrap();
        let (code, body) = run_capturing(&["--action", "fix-grammar", "--redact", input]);
        assert_eq!(code, 0);
        assert_eq!(body["model"], "publik-fast");
        assert_eq!(
            body["messages"][1],
            json!({"role": "user", "content": "<text>\nMail [EMAIL_1] today\n\n</text>"})
        );
        assert_eq!(body["max_tokens"], 256);
        assert_eq!(body["temperature"], 0);
        assert!(
            body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("Use American English spelling")
        );
        // Without --redact the text goes as it is.
        let (_, body) = run_capturing(&["--action", "clearer", input]);
        assert_eq!(
            body["messages"][1]["content"],
            "<text>\nMail ana@example.com today\n\n</text>"
        );
        assert_eq!(body["model"], "publik-balanced");
    }

    #[test]
    fn a_refusal_exits_3() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.txt");
        std::fs::write(&input, "Mail [EMAIL_1] today").unwrap();
        let (code, body) =
            run_capturing(&["--action", "clearer", "--redact", input.to_str().unwrap()]);
        assert_eq!(code, 3);
        assert_eq!(body["error"]["kind"], "RedactionCollision");
        let (code, body) = run_capturing(&["--action", "custom", input.to_str().unwrap()]);
        assert_eq!(code, 3);
        assert_eq!(
            body["error"],
            json!({"kind": "InstructionMissing", "message": "Type an instruction for Custom first."})
        );
    }
}
