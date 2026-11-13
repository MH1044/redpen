//! Parsing every chat answer (docs/PUBLIK-INTEGRATION.md §6, invariant 8): trim; drop one leading
//! fence line (` ```json ` or ` ``` `) and one trailing fence; parse as JSON; if that fails, parse
//! the substring from the first `{` to the last `}`; then validate against the action's schema
//! with `jsonschema`. Anything else is `AiError::InvalidAnswer`, "The AI answer could not be
//! used.", and nothing of it is shown, inserted or cached.
//!
//! The schemas are the embedded `prompts/schemas/*.json`, compiled once. `jsonschema` is built
//! without its resolvers, so a schema can never make it fetch a URL or read a file.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde_json::Value;

use super::prompts::{FILES, schema_source};
use crate::provider::AiError;

static VALIDATORS: LazyLock<HashMap<&'static str, jsonschema::Validator>> = LazyLock::new(|| {
    FILES
        .iter()
        .filter_map(|(path, source)| {
            let name = path.strip_prefix("schemas/")?.strip_suffix(".json")?;
            let schema: Value = serde_json::from_str(source).ok()?;
            let validator = jsonschema::validator_for(&schema).ok()?;
            Some((name, validator))
        })
        .collect()
});

/// Whether the embedded schema `name` exists and compiles.
pub fn schema_compiles(name: &str) -> bool {
    schema_source(name).is_some() && VALIDATORS.contains_key(name)
}

/// `content` trimmed, without one leading fence line (three backticks, optionally followed by a
/// language tag such as `json`) and one trailing fence line.
pub fn strip_fences(content: &str) -> &str {
    let mut s = content.trim();
    if let Some(rest) = s.strip_prefix("```") {
        let (first, after) = rest.split_once('\n').unwrap_or((rest, ""));
        if first.trim().chars().all(|c| c.is_ascii_alphanumeric()) {
            s = after;
        }
    }
    let trimmed = s.trim_end();
    if let Some(before) = trimmed.strip_suffix("```") {
        // Only a fence on a line of its own (or the whole remaining text).
        if before.is_empty() || before.ends_with('\n') {
            s = before;
        }
    }
    s.trim()
}

/// The JSON in `content`: the whole of it once fences are dropped, else the substring from the
/// first `{` to the last `}`.
pub fn extract_json(content: &str) -> Option<Value> {
    let s = strip_fences(content);
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        return Some(v);
    }
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&s[start..=end]).ok()
}

/// The answer object of `content`, valid against the schema `schema` (`rewrite-result.v1`), or
/// `InvalidAnswer`.
pub fn parse_answer(content: &str, schema: &str) -> Result<Value, AiError> {
    let value = extract_json(content).ok_or(AiError::InvalidAnswer)?;
    let validator = VALIDATORS.get(schema).ok_or(AiError::InvalidAnswer)?;
    if validator.is_valid(&value) {
        Ok(value)
    } else {
        Err(AiError::InvalidAnswer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn both_schemas_compile() {
        assert!(schema_compiles("rewrite-result.v1"));
        assert!(schema_compiles("custom-result.v1"));
        assert!(!schema_compiles("no-such-schema.v1"));
    }

    #[test]
    fn fences_are_dropped() {
        assert_eq!(
            strip_fences("```json\n{\"text\": \"Fine.\"}\n```"),
            "{\"text\": \"Fine.\"}"
        );
        assert_eq!(strip_fences("  ```\n{}\n```  \n"), "{}");
        assert_eq!(strip_fences("```JSON\n{}"), "{}");
        assert_eq!(strip_fences("{}\n```"), "{}");
        assert_eq!(
            strip_fences("{\"text\": \"a ``` b\"}"),
            "{\"text\": \"a ``` b\"}"
        );
        assert_eq!(strip_fences("plain"), "plain");
    }

    #[test]
    fn the_brace_fallback_finds_the_object() {
        assert_eq!(
            extract_json("Sure! Here is the text:\n{\"text\": \"Fine.\"}"),
            Some(json!({"text": "Fine."}))
        );
        assert_eq!(
            extract_json("{\"text\": \"x\"} I hope this helps."),
            Some(json!({"text": "x"}))
        );
        assert_eq!(extract_json("hello"), None);
        assert_eq!(extract_json("} backwards {"), None);
    }

    #[test]
    fn the_four_bad_answers_are_invalid() {
        for content in [
            r#"{"txt": "x"}"#,
            r#"{"text": "x", "extra": 1}"#,
            r#"{"text": 5}"#,
            "hello",
        ] {
            assert_eq!(
                parse_answer(content, "rewrite-result.v1"),
                Err(AiError::InvalidAnswer),
                "{content}"
            );
        }
        assert_eq!(
            parse_answer(r#"{"text": ""}"#, "rewrite-result.v1"),
            Err(AiError::InvalidAnswer)
        );
        let long = json!({"text": "x".repeat(40_001)}).to_string();
        assert_eq!(
            parse_answer(&long, "rewrite-result.v1"),
            Err(AiError::InvalidAnswer)
        );
        assert_eq!(
            parse_answer(r#"{"text": "Fine."}"#, "rewrite-result.v1"),
            Ok(json!({"text": "Fine."}))
        );
    }

    #[test]
    fn custom_answers_need_changes_language() {
        let schema = "custom-result.v1";
        assert!(parse_answer(r#"{"text": "Bonjour", "changes_language": true}"#, schema).is_ok());
        assert!(parse_answer(r#"{"text": "Bonjour"}"#, schema).is_err());
        assert!(parse_answer(r#"{"text": "Bonjour", "changes_language": "yes"}"#, schema).is_err());
        assert!(parse_answer(r#"{"text": "x"}"#, "unknown.v1").is_err());
    }
}
