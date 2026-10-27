//! Canonical request hashing (docs/modules/00-scaffold.md, Interfaces).
//!
//! `request_hash` is the SHA-256, as 64 lowercase hex characters, of
//! `METHOD + " " + path + "\n" + canonical`, where `path` is the URL path after the provider's
//! base path (`/api/v1` for Publik, `/v1` for OpenAI-compatible servers, `/res/v1` for Brave
//! Search) followed by the query string, if any, with its parameters sorted by name and
//! percent-encoded as sent, and `canonical` is the JSON body with keys sorted, no whitespace, as
//! UTF-8 (never `\u`-escaped), or empty for a request without a body. Headers never count, so
//! the same request hashes the same whatever key it carries.
//!
//! Every base path above ends in a `v1` segment, so the rule is: the path after its last `v1`
//! segment, or the whole path when there is none (Ollama's `/api/tags`).

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{Body, HttpRequest};

/// The 64-hex SHA-256 that identifies a request in a recording.
pub fn request_hash(req: &HttpRequest) -> String {
    let canonical = match &req.body {
        Body::Empty => String::new(),
        Body::Json(value) => canonical_json(value),
    };
    let input = format!("{} {}\n{}", req.method, request_path(&req.url), canonical);
    hex(&Sha256::digest(input.as_bytes()))
}

/// The part of a URL that enters the hash: the path after the provider's base path, plus the
/// query string with its parameters sorted by name. The scheme, host and port never count.
pub fn request_path(url: &str) -> String {
    let after_scheme = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => url,
    };
    let path_and_rest = match after_scheme.find('/') {
        Some(i) => &after_scheme[i..],
        None => "/",
    };
    let path_and_rest = path_and_rest.split('#').next().unwrap_or("");
    let (path, query) = match path_and_rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_and_rest, None),
    };
    let mut out = strip_base_path(path).to_string();
    if let Some(query) = query.filter(|q| !q.is_empty()) {
        let mut params: Vec<(&str, &str)> = query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| p.split_once('=').unwrap_or((p, "")))
            .collect();
        // A stable sort keeps repeated names in the order they were sent.
        params.sort_by(|a, b| a.0.cmp(b.0));
        out.push('?');
        for (i, (name, value)) in params.iter().enumerate() {
            if i > 0 {
                out.push('&');
            }
            out.push_str(name);
            out.push('=');
            out.push_str(value);
        }
    }
    out
}

/// The path after the last `v1` segment: `/api/v1/chat/completions` → `/chat/completions`.
fn strip_base_path(path: &str) -> &str {
    let mut rest = path;
    let mut found = None;
    while let Some(i) = rest.find("/v1/") {
        found = Some(i + 3);
        rest = &rest[i + 3..];
    }
    match found {
        Some(_) => rest,
        None if path.ends_with("/v1") => "/",
        None => path,
    }
}

/// The canonical form of a JSON value: object keys sorted at every level, no whitespace, UTF-8.
/// The answer cache of module 02 uses the same function for its keys.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).expect("a string serialises"));
                out.push(':');
                write_canonical(&map[*key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        // serde_json writes strings as UTF-8 and escapes only what JSON requires.
        other => out.push_str(&serde_json::to_string(other).expect("a scalar serialises")),
    }
}

/// Lowercase hex of a digest.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths_lose_the_base_path_and_sort_the_query() {
        assert_eq!(
            request_path("https://publikhq.com/api/v1/chat/completions"),
            "/chat/completions"
        );
        assert_eq!(
            request_path("http://localhost:11434/v1/chat/completions"),
            "/chat/completions"
        );
        assert_eq!(
            request_path("https://api.search.brave.com/res/v1/web/search?q=%22a%20b%22&count=5"),
            "/web/search?count=5&q=%22a%20b%22"
        );
        assert_eq!(request_path("http://localhost:11434/api/tags"), "/api/tags");
        assert_eq!(request_path("https://publikhq.com/api/v1"), "/");
        assert_eq!(request_path("https://example.com"), "/");
    }

    #[test]
    fn canonical_json_sorts_keys_at_every_level_and_keeps_utf8() {
        let value = json!({"b": {"z": 1, "a": [3, {"y": 2, "x": 1}]}, "a": "é ✓"});
        assert_eq!(
            canonical_json(&value),
            r#"{"a":"é ✓","b":{"a":[3,{"x":1,"y":2}],"z":1}}"#
        );
    }

    #[test]
    fn hashes_match_the_spec() {
        let req = HttpRequest::post_json(
            "https://publikhq.com/api/v1/chat/completions",
            json!({"b": 1, "a": 2}),
        );
        assert_eq!(
            request_hash(&req),
            "82cab4cdacc60d06a2ccbd424c8fb469aa308bfc7df538fd03a95348e2eeef93"
        );
    }
}
