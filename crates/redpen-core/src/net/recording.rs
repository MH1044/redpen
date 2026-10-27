//! Recordings (docs/TESTING.md §1 and §5): the file format the fake transport answers from, the
//! check every committed `recording.json` must pass, and the recorder that writes entries which
//! pass it.
//!
//! A recording carries no status headers: of a real response's `x-publik-*` headers only
//! `x-publik-charge-micros` is kept, with the synthetic value `"100"`, and of the body only what
//! a test needs (`choices` for chat; `results` and `charge_micros` for a publik search; `web` for
//! a Brave search). Response ids, timestamps, balances, request ids and usage counts are dropped,
//! so a recording never describes the account or the call that made it.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::hash::request_hash;
use super::{Body, HttpRequest, HttpResponse, HttpTransport, TransportError};

/// A `recording.json` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub format: String,
    pub entries: Vec<RecordingEntry>,
}

impl Recording {
    /// The one format this version reads and writes.
    pub const FORMAT: &'static str = "redpen-recording/1";

    pub fn new(entries: Vec<RecordingEntry>) -> Self {
        Self {
            format: Self::FORMAT.to_string(),
            entries,
        }
    }
}

/// One exchange: the request's hash and a copy of its body for readers, then the answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingEntry {
    pub request_sha256: String,
    /// `hand-written`, or `recorded <provider> <date>`.
    pub source: String,
    /// The request body, for readers of the file; the hash is what the fake matches on.
    pub request: Value,
    pub status: u16,
    #[serde(with = "header_map", default)]
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

/// The one response header a recording may carry, and the only value a recorded entry may give
/// it (docs/TESTING.md §1).
pub const CHARGE_HEADER: &str = "x-publik-charge-micros";
pub const SYNTHETIC_CHARGE: &str = "100";

/// Checks one entry against the rules of docs/TESTING.md §5. `file` names the file in messages.
pub fn validate_entry(entry: &RecordingEntry, file: &str) -> Result<(), String> {
    for (name, _) in &entry.headers {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("x-publik-") && lower != CHARGE_HEADER {
            return Err(format!(
                "Recordings keep only {CHARGE_HEADER} with a synthetic value; remove {lower} from {file}."
            ));
        }
    }
    let recorded = entry.source.starts_with("recorded");
    if recorded {
        for (name, value) in &entry.headers {
            if name.eq_ignore_ascii_case(CHARGE_HEADER) && value != SYNTHETIC_CHARGE {
                return Err(format!(
                    "Recordings keep only {CHARGE_HEADER} with a synthetic value; remove the real charge {value} from {file}."
                ));
            }
        }
    }
    let Some(object) = entry.body.as_object() else {
        return Err(format!(
            "Recordings keep only the answer's choices or results; the body of an entry in {file} is not an object."
        ));
    };
    let keys: Vec<&str> = object.keys().map(String::as_str).collect();
    let allowed: &[&[&str]] = &[&["choices"], &["results", "charge_micros"], &["web"]];
    let shape_ok = allowed
        .iter()
        .any(|shape| keys.iter().all(|k| shape.contains(k)) && !keys.is_empty());
    if !shape_ok {
        let extra: Vec<&str> = keys
            .iter()
            .copied()
            .filter(|k| !["choices", "results", "charge_micros", "web"].contains(k))
            .collect();
        let named = if extra.is_empty() {
            keys.join(", ")
        } else {
            extra.join(", ")
        };
        return Err(format!(
            "Recordings keep only the body keys choices (chat), results and charge_micros (publik search) or web (Brave search); remove {named} from {file}."
        ));
    }
    if recorded
        && let Some(charge) = object.get("charge_micros")
        && charge != &Value::from(100)
    {
        return Err(format!(
            "Recordings keep only {CHARGE_HEADER} with a synthetic value; remove the real charge_micros {charge} from {file}."
        ));
    }
    Ok(())
}

/// Checks a whole recording.
pub fn validate(recording: &Recording, file: &str) -> Result<(), String> {
    if recording.format != Recording::FORMAT {
        return Err(format!(
            "{file} has format {:?}; a recording has format {:?}.",
            recording.format,
            Recording::FORMAT
        ));
    }
    for entry in &recording.entries {
        validate_entry(entry, file)?;
    }
    Ok(())
}

/// Builds the entry a recording keeps for a real exchange: the request's hash and body, the
/// status, the synthetic charge header, and the scrubbed body. Everything else is dropped.
pub fn scrub(req: &HttpRequest, resp: &HttpResponse, source: &str) -> RecordingEntry {
    let request = match &req.body {
        Body::Empty => Value::Null,
        Body::Json(v) => v.clone(),
    };
    let had_charge = resp
        .headers
        .iter()
        .any(|(n, _)| n.eq_ignore_ascii_case(CHARGE_HEADER));
    let headers = if had_charge {
        vec![(CHARGE_HEADER.to_string(), SYNTHETIC_CHARGE.to_string())]
    } else {
        Vec::new()
    };
    let body = scrub_body(resp.json().unwrap_or(Value::Null));
    RecordingEntry {
        request_sha256: request_hash(req),
        source: source.to_string(),
        request,
        status: resp.status,
        headers,
        body,
    }
}

/// Keeps `choices[].message.{role, content}` for chat; `url`, `title`, `snippet`, `site` and
/// `age` of each result (with `charge_micros` 100) for a publik search; the same fields of
/// `web.results[]` for a Brave search.
fn scrub_body(body: Value) -> Value {
    let Some(object) = body.as_object() else {
        return Value::Object(Map::new());
    };
    let mut out = Map::new();
    if let Some(choices) = object.get("choices").and_then(Value::as_array) {
        let kept: Vec<Value> = choices
            .iter()
            .map(|choice| {
                let message = choice.get("message").cloned().unwrap_or(Value::Null);
                let mut m = Map::new();
                if let Some(role) = message.get("role") {
                    m.insert("role".into(), role.clone());
                }
                if let Some(content) = message.get("content") {
                    m.insert("content".into(), content.clone());
                }
                serde_json::json!({"message": Value::Object(m)})
            })
            .collect();
        out.insert("choices".into(), Value::Array(kept));
    } else if let Some(results) = object.get("results").and_then(Value::as_array) {
        out.insert("results".into(), Value::Array(scrub_results(results)));
        out.insert("charge_micros".into(), Value::from(100));
    } else if let Some(web) = object.get("web").and_then(Value::as_object) {
        let results = web
            .get("results")
            .and_then(Value::as_array)
            .map(|r| scrub_results(r))
            .unwrap_or_default();
        out.insert(
            "web".into(),
            serde_json::json!({"results": Value::Array(results)}),
        );
    }
    Value::Object(out)
}

fn scrub_results(results: &[Value]) -> Vec<Value> {
    results
        .iter()
        .map(|r| {
            let mut m = Map::new();
            for key in ["url", "title", "snippet", "site", "age"] {
                if let Some(v) = r.get(key) {
                    m.insert(key.into(), v.clone());
                }
            }
            Value::Object(m)
        })
        .collect()
}

/// A transport that forwards to another and keeps a scrubbed entry of every exchange. Behind
/// `UPDATE_RECORDINGS=1` (docs/TESTING.md §1) the maintainer's re-recording runs through it.
pub struct Recorder<T: HttpTransport> {
    inner: T,
    source: String,
    entries: Mutex<Vec<RecordingEntry>>,
}

impl<T: HttpTransport> Recorder<T> {
    /// Records through `inner`, labelling entries `source` (`recorded <provider> <date>`).
    pub fn new(inner: T, source: impl Into<String>) -> Self {
        Self {
            inner,
            source: source.into(),
            entries: Mutex::new(Vec::new()),
        }
    }

    /// The scrubbed entries so far, in order.
    pub fn entries(&self) -> Vec<RecordingEntry> {
        self.entries
            .lock()
            .expect("entry log is not poisoned")
            .clone()
    }

    /// The entries as a recording.
    pub fn recording(&self) -> Recording {
        Recording::new(self.entries())
    }

    /// The transport being recorded.
    pub fn inner(&self) -> &T {
        &self.inner
    }
}

#[async_trait::async_trait]
impl<T: HttpTransport> HttpTransport for Recorder<T> {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let resp = self.inner.send(req.clone()).await?;
        let entry = scrub(&req, &resp, &self.source);
        self.entries
            .lock()
            .expect("entry log is not poisoned")
            .push(entry);
        Ok(resp)
    }
}

/// `headers` is a JSON object in the file and an ordered list in memory.
mod header_map {
    use super::*;

    pub fn serialize<S: serde::Serializer>(
        headers: &[(String, String)],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let map: BTreeMap<&str, &str> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        map.serialize(s)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<Vec<(String, String)>, D::Error> {
        let map: BTreeMap<String, String> = BTreeMap::deserialize(d)?;
        Ok(map.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(headers: Vec<(&str, &str)>, body: Value, source: &str) -> RecordingEntry {
        RecordingEntry {
            request_sha256: "0".repeat(64),
            source: source.into(),
            request: json!({}),
            status: 200,
            headers: headers
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body,
        }
    }

    #[test]
    fn a_balance_header_is_refused_with_the_message() {
        let e = entry(
            vec![("x-publik-balance", "3120000")],
            json!({"choices": []}),
            "hand-written",
        );
        assert_eq!(
            validate_entry(&e, "tests/cases/x/recording.json").unwrap_err(),
            "Recordings keep only x-publik-charge-micros with a synthetic value; remove x-publik-balance from tests/cases/x/recording.json."
        );
    }

    #[test]
    fn a_body_key_id_is_refused() {
        let e = entry(vec![], json!({"id": "x", "choices": []}), "hand-written");
        let err = validate_entry(&e, "f").unwrap_err();
        assert!(err.contains("remove id from f."), "{err}");
    }

    #[test]
    fn a_recorded_entry_must_carry_the_synthetic_charge() {
        let e = entry(
            vec![("x-publik-charge-micros", "210")],
            json!({"choices": []}),
            "recorded publik-env 2026-10-27",
        );
        assert!(validate_entry(&e, "f").is_err());
        let ok = entry(
            vec![("x-publik-charge-micros", "210")],
            json!({"choices": []}),
            "hand-written",
        );
        assert!(validate_entry(&ok, "f").is_ok());
        let search = entry(
            vec![],
            json!({"results": [], "charge_micros": 10000}),
            "recorded publik-env 2026-10-27",
        );
        assert!(validate_entry(&search, "f").is_err());
    }

    #[test]
    fn round_trips_through_json() {
        let rec = Recording::new(vec![entry(
            vec![("x-publik-charge-micros", "100")],
            json!({"choices": []}),
            "hand-written",
        )]);
        let text = serde_json::to_string(&rec).unwrap();
        assert!(text.contains(r#""headers":{"x-publik-charge-micros":"100"}"#));
        let back: Recording = serde_json::from_str(&text).unwrap();
        assert_eq!(back, rec);
    }

    #[test]
    fn scrub_keeps_only_the_charge_header_and_the_choices() {
        let req = HttpRequest::post_json(
            "https://publikhq.com/api/v1/chat/completions",
            json!({"model": "publik-fast"}),
        );
        let resp = HttpResponse {
            status: 200,
            headers: vec![
                ("x-publik-balance".into(), "3120000".into()),
                ("x-publik-charge-micros".into(), "210".into()),
                ("x-publik-request-id".into(), "req_1".into()),
                ("x-publik-week-used".into(), "5".into()),
            ],
            body: serde_json::to_vec(&json!({
                "id": "chatcmpl-1", "created": 1790000000, "model": "x",
                "usage": {"total_tokens": 9},
                "choices": [{"index": 0, "finish_reason": "stop",
                             "message": {"role": "assistant", "content": "{\"text\": \"ok\"}"}}]
            }))
            .unwrap(),
        };
        let e = scrub(&req, &resp, "recorded publik-env 2026-10-27");
        assert_eq!(
            e.headers,
            vec![("x-publik-charge-micros".to_string(), "100".to_string())]
        );
        assert_eq!(
            e.body,
            json!({"choices": [{"message": {"role": "assistant", "content": "{\"text\": \"ok\"}"}}]})
        );
        assert_eq!(e.request, json!({"model": "publik-fast"}));
        assert!(validate_entry(&e, "f").is_ok());
    }

    #[test]
    fn scrub_keeps_search_results_with_the_synthetic_charge() {
        let req = HttpRequest::post_json(
            "https://publikhq.com/api/v1/search",
            json!({"q": "\"x\"", "count": 5}),
        );
        let resp = HttpResponse {
            status: 200,
            headers: vec![("x-publik-charge-micros".into(), "10000".into())],
            body: serde_json::to_vec(&json!({
                "object": "search.results", "query": "x", "count": 1, "more": false,
                "results": [{"url": "https://www.example.org/a", "title": "A", "snippet": "s",
                             "site": "example.org", "age": "2026-09-30", "rank": 1}],
                "charge_micros": 10000, "request_id": "req_2"
            }))
            .unwrap(),
        };
        let e = scrub(&req, &resp, "recorded publik-env 2026-10-27");
        assert_eq!(
            e.body,
            json!({"results": [{"url": "https://www.example.org/a", "title": "A", "snippet": "s",
                                "site": "example.org", "age": "2026-09-30"}],
                   "charge_micros": 100})
        );
        assert!(validate_entry(&e, "f").is_ok());
    }
}
