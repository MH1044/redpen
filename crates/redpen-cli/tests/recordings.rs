//! The recordings check (docs/TESTING.md §5): every committed `tests/cases/**/recording.json`
//! keeps no status headers and no response fields beyond what a test needs, so a recording never
//! describes the account or the call that made it. Module 00 acceptance test 14.

use std::fs;
use std::path::{Path, PathBuf};

use redpen_core::net::recording::{Recorder, Recording, validate};
use redpen_core::net::{FakeEntry, FakeTransport, HttpRequest, HttpTransport, request_hash};
use serde_json::json;

/// The repository root: the first ancestor of this crate that holds the workspace Cargo.toml.
fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if let Ok(text) = fs::read_to_string(dir.join("Cargo.toml"))
            && text.contains("[workspace]")
        {
            return dir;
        }
        assert!(dir.pop(), "no workspace Cargo.toml above the crate");
    }
}

/// Every `recording.json` below `dir`, in path order.
fn recordings_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            found.extend(recordings_under(&path));
        } else if path.file_name().is_some_and(|n| n == "recording.json") {
            found.push(path);
        }
    }
    found
}

/// Parses and checks one file; messages name it by `shown`.
fn check_file(path: &Path, shown: &str) -> Result<(), String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{shown} could not be read: {e}"))?;
    let recording: Recording =
        serde_json::from_str(&text).map_err(|e| format!("{shown} is not a recording: {e}"))?;
    validate(&recording, shown)
}

fn write_recording(dir: &Path, body: serde_json::Value, headers: serde_json::Value) -> PathBuf {
    let path = dir.join("recording.json");
    let text = json!({
        "format": "redpen-recording/1",
        "entries": [{
            "request_sha256": "0".repeat(64),
            "source": "hand-written",
            "request": {},
            "status": 200,
            "headers": headers,
            "body": body
        }]
    });
    fs::write(&path, serde_json::to_string_pretty(&text).unwrap()).unwrap();
    path
}

// Test 14, first part: every committed recording passes.
#[test]
fn committed_recordings_pass() {
    let cases = repo_root().join("tests").join("cases");
    let mut failures = Vec::new();
    for path in recordings_under(&cases) {
        let shown = path
            .strip_prefix(repo_root())
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if let Err(message) = check_file(&path, &shown) {
            failures.push(message);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// Test 14: a planted recording with a balance header fails with the exact message.
#[test]
fn a_balance_header_fails_with_the_message() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_recording(
        dir.path(),
        json!({"choices": []}),
        json!({"x-publik-charge-micros": "100", "x-publik-balance": "3120000"}),
    );
    let shown = "tests/cases/03-publik/planted/recording.json";
    assert_eq!(
        check_file(&path, shown).unwrap_err(),
        format!(
            "Recordings keep only x-publik-charge-micros with a synthetic value; remove x-publik-balance from {shown}."
        )
    );
}

// Test 14: a body key `id` fails.
#[test]
fn a_body_key_id_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_recording(
        dir.path(),
        json!({"id": "chatcmpl-1", "choices": []}),
        json!({}),
    );
    let err = check_file(&path, "f").unwrap_err();
    assert!(err.contains("remove id from f."), "{err}");
}

// Test 14: the recorder, run over the fake transport with a response carrying the status headers
// and the response fields a real server adds, writes an entry with only the synthetic charge
// header and the choices, and that entry passes the check.
#[tokio::test]
async fn the_recorder_writes_entries_that_pass() {
    let req = HttpRequest::post_json(
        "https://publikhq.com/api/v1/chat/completions",
        json!({"model": "publik-fast", "messages": [{"role": "user", "content": "hi"}]}),
    );
    let real_answer = FakeEntry {
        request_sha256: request_hash(&req),
        source: "scripted".into(),
        request: json!({}),
        status: 200,
        headers: vec![
            ("x-publik-balance".into(), "3120000".into()),
            ("x-publik-charge-micros".into(), "210".into()),
            ("x-publik-request-id".into(), "req_abc".into()),
            ("x-publik-week-used".into(), "4200".into()),
        ],
        body: json!({
            "id": "chatcmpl-1",
            "usage": {"prompt_tokens": 3, "completion_tokens": 2},
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}}]
        }),
    };
    let recorder = Recorder::new(
        FakeTransport::scripted(vec![real_answer]),
        "recorded publik-env 2026-10-27",
    );
    let resp = recorder.send(req).await.unwrap();
    assert_eq!(resp.status, 200);

    let entries = recorder.entries();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(
        entry.headers,
        vec![("x-publik-charge-micros".to_string(), "100".to_string())]
    );
    let keys: Vec<&String> = entry.body.as_object().unwrap().keys().collect();
    assert_eq!(keys, vec!["choices"]);
    assert_eq!(
        entry.body["choices"][0]["message"],
        json!({"role": "assistant", "content": "ok"})
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recording.json");
    fs::write(
        &path,
        serde_json::to_string_pretty(&recorder.recording()).unwrap(),
    )
    .unwrap();
    assert_eq!(check_file(&path, "f"), Ok(()));
}
