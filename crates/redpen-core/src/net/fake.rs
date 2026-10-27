//! The fake transport (docs/TESTING.md §1): the only HTTP in tests, for the whole project.
//!
//! It answers from a `recording.json` (or from scripted entries) by the [`request_hash`] of the
//! request, fails on any request it has no entry for (invariant 11), writing that request's body
//! under `target/unrecorded/` so a hand-written entry can be added, and keeps every request it
//! saw so privacy tests can assert on what would have left the machine.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use super::hash::request_hash;
use super::recording::{Recording, RecordingEntry};
use super::{Body, HttpRequest, HttpResponse, HttpTransport, TransportError};

/// One recorded exchange. The same shape as an entry of `recording.json`.
pub type FakeEntry = RecordingEntry;

/// Why a recording could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum FakeError {
    #[error("{path} could not be read: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not a recording: {message}")]
    Format { path: String, message: String },
}

/// A transport that answers only recorded requests.
#[derive(Debug)]
pub struct FakeTransport {
    entries: Vec<FakeEntry>,
    seen: Mutex<Vec<HttpRequest>>,
}

impl FakeTransport {
    /// Loads `recording.json` (format `redpen-recording/1`).
    pub fn from_file(path: &Path) -> Result<Self, FakeError> {
        let shown = path.display().to_string();
        let text = fs::read_to_string(path).map_err(|source| FakeError::Io {
            path: shown.clone(),
            source,
        })?;
        let recording: Recording = serde_json::from_str(&text).map_err(|e| FakeError::Format {
            path: shown.clone(),
            message: e.to_string(),
        })?;
        if recording.format != Recording::FORMAT {
            return Err(FakeError::Format {
                path: shown,
                message: format!(
                    "format is {:?}, expected {:?}",
                    recording.format,
                    Recording::FORMAT
                ),
            });
        }
        Ok(Self::scripted(recording.entries))
    }

    /// A transport answering exactly these entries.
    pub fn scripted(entries: Vec<FakeEntry>) -> Self {
        Self {
            entries,
            seen: Mutex::new(Vec::new()),
        }
    }

    /// Every request sent through this transport, in order, answered or not.
    pub fn requests_seen(&self) -> Vec<HttpRequest> {
        self.seen
            .lock()
            .expect("request log is not poisoned")
            .clone()
    }

    /// The entries this transport answers.
    pub fn entries(&self) -> &[FakeEntry] {
        &self.entries
    }
}

/// Where an unrecorded request's body is written: `target/unrecorded/<hash>.json` under the
/// workspace root, found by walking up from the current directory; `CARGO_TARGET_DIR` wins.
pub fn unrecorded_path(hash: &str) -> PathBuf {
    unrecorded_dir().join(format!("{hash}.json"))
}

fn unrecorded_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR") {
        return PathBuf::from(dir).join("unrecorded");
    }
    let start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut dir = start.clone();
    loop {
        if let Ok(text) = fs::read_to_string(dir.join("Cargo.toml"))
            && text.contains("[workspace]")
        {
            return dir.join("target").join("unrecorded");
        }
        if !dir.pop() {
            return start.join("target").join("unrecorded");
        }
    }
}

#[async_trait::async_trait]
impl HttpTransport for FakeTransport {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let hash = request_hash(&req);
        self.seen
            .lock()
            .expect("request log is not poisoned")
            .push(req.clone());
        if let Some(entry) = self.entries.iter().find(|e| e.request_sha256 == hash) {
            let body = serde_json::to_vec(&entry.body).expect("a JSON value serialises");
            let headers = entry
                .headers
                .iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
                .collect();
            return Ok(HttpResponse {
                status: entry.status,
                headers,
                body,
            });
        }
        let path = super::hash::request_path(&req.url);
        let file = unrecorded_path(&hash);
        let written = serde_json::json!({
            "request_sha256": hash,
            "method": req.method.as_str(),
            "path": path,
            "body": match &req.body { Body::Empty => Value::Null, Body::Json(v) => v.clone() },
        });
        // Best effort: the error below is the signal, the file is a convenience.
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(
            &file,
            serde_json::to_string_pretty(&written).expect("a JSON value serialises"),
        );
        Err(TransportError::Unrecorded { hash, path })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(req: &HttpRequest, status: u16, body: Value) -> FakeEntry {
        FakeEntry {
            request_sha256: request_hash(req),
            source: "hand-written".into(),
            request: json!({}),
            status,
            headers: vec![("X-Publik-Charge-Micros".into(), "210".into())],
            body,
        }
    }

    #[tokio::test]
    async fn answers_a_recorded_request_and_lower_cases_headers() {
        let req = HttpRequest::post_json(
            "https://publikhq.com/api/v1/chat/completions",
            json!({"a": 1}),
        );
        let fake = FakeTransport::scripted(vec![entry(&req, 200, json!({"choices": []}))]);
        let resp = fake.send(req).await.unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.header("x-publik-charge-micros"), Some("210"));
        assert_eq!(resp.headers[0].0, "x-publik-charge-micros");
        assert_eq!(resp.json(), Some(json!({"choices": []})));
        assert_eq!(fake.requests_seen().len(), 1);
    }

    #[tokio::test]
    async fn an_unrecorded_request_fails_and_is_still_seen() {
        let fake = FakeTransport::scripted(Vec::new());
        let req = HttpRequest::get("https://publikhq.com/api/v1/wallet");
        let err = fake.send(req).await.unwrap_err();
        match &err {
            TransportError::Unrecorded { hash, path } => {
                assert_eq!(hash.len(), 64);
                assert_eq!(path, "/wallet");
                assert!(unrecorded_path(hash).is_file());
                let _ = fs::remove_file(unrecorded_path(hash));
            }
            other => panic!("unexpected error {other:?}"),
        }
        assert_eq!(fake.requests_seen().len(), 1);
    }

    #[test]
    fn a_file_with_the_wrong_format_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recording.json");
        fs::write(&path, r#"{"format": "other/1", "entries": []}"#).unwrap();
        let err = FakeTransport::from_file(&path).unwrap_err();
        assert!(err.to_string().contains("is not a recording"), "{err}");
        let err = FakeTransport::from_file(&dir.path().join("missing.json")).unwrap_err();
        assert!(err.to_string().contains("could not be read"), "{err}");
    }
}
