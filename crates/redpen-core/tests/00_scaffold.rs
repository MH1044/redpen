//! Module 00 tests (docs/modules/00-scaffold.md, Acceptance tests). Each test names the bullet
//! it covers. Bullet 1 is the golden case `tests/cases/00-scaffold/version`, bullet 9 is
//! `crates/redpen-cli/tests/hygiene.rs`, bullet 11 is the launch smoke in CI, bullet 12 is
//! `app/src/00-scaffold.smoke.test.tsx` and bullet 14 is `crates/redpen-cli/tests/recordings.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use redpen_core::clock::{Clock, FakeClock, FakeSleeper, Sleeper};
use redpen_core::net::fake::unrecorded_path;
use redpen_core::net::{
    FakeEntry, FakeTransport, HeaderValue, HttpRequest, HttpTransport, TransportError, request_hash,
};
use serde_json::json;

const PUBLIK_CHAT: &str = "https://publikhq.com/api/v1/chat/completions";

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

fn read(path: impl AsRef<Path>) -> String {
    let path = repo_root().join(path);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} could not be read: {e}", path.display()))
}

fn entry_for(req: &HttpRequest, status: u16, body: serde_json::Value) -> FakeEntry {
    FakeEntry {
        request_sha256: request_hash(req),
        source: "hand-written".into(),
        request: json!({}),
        status,
        headers: vec![("x-publik-charge-micros".into(), "210".into())],
        body,
    }
}

// Bullet 2: the two reference hashes, computed with sha256sum over the canonical string.
#[test]
fn t02_request_hash_matches_the_reference() {
    let req = HttpRequest::post_json(PUBLIK_CHAT, json!({"b": 1, "a": 2}));
    assert_eq!(
        request_hash(&req),
        "82cab4cdacc60d06a2ccbd424c8fb469aa308bfc7df538fd03a95348e2eeef93"
    );
    let req = HttpRequest::post_json(
        PUBLIK_CHAT,
        json!({"model": "publik-fast", "messages": [{"role": "user", "content": "hi"}]}),
    );
    assert_eq!(
        request_hash(&req),
        "22f53c07ac6b84e42b6fea37d9f1ce05a6c42780307b5932ccf2031f7ef66dd1"
    );
}

// Bullet 3: key order, source whitespace and headers never change the hash; a value does.
#[test]
fn t03_hash_ignores_order_whitespace_and_headers_but_not_values() {
    let base = HttpRequest::post_json(PUBLIK_CHAT, json!({"b": 1, "a": 2}));
    let reordered = HttpRequest::post_json(PUBLIK_CHAT, json!({"a": 2, "b": 1}));
    let spaced: serde_json::Value = serde_json::from_str("{ \"b\" : 1 ,\n \"a\" :  2 }").unwrap();
    let spaced = HttpRequest::post_json(PUBLIK_CHAT, spaced);
    let with_key = HttpRequest::post_json(PUBLIK_CHAT, json!({"b": 1, "a": 2}))
        .header("Authorization", HeaderValue::secret("Bearer pk_live_a"));
    let other_key = HttpRequest::post_json(PUBLIK_CHAT, json!({"b": 1, "a": 2}))
        .header("Authorization", HeaderValue::secret("Bearer sk-test-b"));
    let expected = request_hash(&base);
    assert_eq!(request_hash(&reordered), expected);
    assert_eq!(request_hash(&spaced), expected);
    assert_eq!(request_hash(&with_key), expected);
    assert_eq!(request_hash(&other_key), expected);
    let changed = HttpRequest::post_json(PUBLIK_CHAT, json!({"b": 1, "a": 3}));
    assert_ne!(request_hash(&changed), expected);
}

// Bullet 4: a recording with one entry answers the matching request with its status, headers
// and body.
#[tokio::test]
async fn t04_from_file_answers_a_recorded_request() {
    let req = HttpRequest::post_json(PUBLIK_CHAT, json!({"model": "publik-fast", "messages": []}));
    let recording = json!({
        "format": "redpen-recording/1",
        "entries": [{
            "request_sha256": request_hash(&req),
            "source": "hand-written",
            "request": {"model": "publik-fast", "messages": []},
            "status": 200,
            "headers": {"x-publik-charge-micros": "210"},
            "body": {"choices": [{"message": {"role": "assistant", "content": "{\"text\": \"ok\"}"}}]}
        }]
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recording.json");
    fs::write(&path, serde_json::to_string_pretty(&recording).unwrap()).unwrap();

    let fake = FakeTransport::from_file(&path).unwrap();
    let resp = fake.send(req).await.unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(
        resp.headers,
        vec![("x-publik-charge-micros".to_string(), "210".to_string())]
    );
    assert_eq!(
        resp.json().unwrap(),
        json!({"choices": [{"message": {"role": "assistant", "content": "{\"text\": \"ok\"}"}}]})
    );
}

// Bullet 5: an unrecorded request fails with the documented message and writes the body file.
#[tokio::test]
async fn t05_unrecorded_request_fails_and_writes_the_body() {
    let fake = FakeTransport::scripted(Vec::new());
    let req = HttpRequest::post_json(
        PUBLIK_CHAT,
        json!({"model": "publik-fast", "messages": [{"role": "user", "content": "unrecorded"}]}),
    );
    let hash = request_hash(&req);
    let err = fake.send(req).await.unwrap_err();
    assert_eq!(
        err,
        TransportError::Unrecorded {
            hash: hash.clone(),
            path: "/chat/completions".into()
        }
    );
    assert_eq!(
        err.to_string(),
        format!(
            "Unrecorded request {hash} to /chat/completions; body written to target/unrecorded/{hash}.json. Add a hand-written entry, or ask the maintainer to record it."
        )
    );
    let file = unrecorded_path(&hash);
    assert!(file.is_file(), "{} was not written", file.display());
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(written["path"], "/chat/completions");
    assert_eq!(written["body"]["model"], "publik-fast");
    let _ = fs::remove_file(file);
}

// Bullet 6: every sent request is kept, in order, answered or not.
#[tokio::test]
async fn t06_requests_seen_in_order() {
    let first = HttpRequest::post_json(PUBLIK_CHAT, json!({"n": 1}));
    let second = HttpRequest::get("https://publikhq.com/api/v1/wallet");
    let third = HttpRequest::post_json(PUBLIK_CHAT, json!({"n": 3}));
    let fake = FakeTransport::scripted(vec![
        entry_for(&first, 200, json!({"choices": []})),
        entry_for(&second, 200, json!({"choices": []})),
    ]);
    fake.send(first.clone()).await.unwrap();
    fake.send(second.clone()).await.unwrap();
    let err = fake.send(third.clone()).await.unwrap_err();
    if let TransportError::Unrecorded { hash, .. } = &err {
        let _ = fs::remove_file(unrecorded_path(hash));
    }
    let seen = fake.requests_seen();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].body, first.body);
    assert_eq!(seen[1].url, second.url);
    assert_eq!(seen[2].body, third.body);
}

// Bullet 7: a secret header's Debug output is `***`, never the value.
#[test]
fn t07_secret_header_value_debug_is_stars() {
    let value = HeaderValue::secret("Bearer pk_live_a");
    assert_eq!(format!("{value:?}"), "***");
    let req = HttpRequest::get("https://publikhq.com/api/v1/wallet").header("authorization", value);
    assert!(!format!("{req:?}").contains("pk_live_a"));
}

// Bullet 8: the fake clock returns the set time and advances; the fake sleeper records a
// 500 ms sleep and returns without waiting.
#[tokio::test]
async fn t08_fake_clock_and_sleeper() {
    let clock = FakeClock::at(1_790_000_000_000);
    assert_eq!(clock.now_ms(), 1_790_000_000_000);
    clock.advance(Duration::from_secs(60));
    assert_eq!(clock.now_ms(), 1_790_000_060_000);

    let sleeper = FakeSleeper::new();
    let started = std::time::Instant::now();
    sleeper.sleep(Duration::from_millis(500)).await;
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "the fake sleeper waited"
    );
    assert_eq!(sleeper.sleeps(), vec![Duration::from_millis(500)]);
}

// Bullet 10: the licence gate's allow list contains no copyleft licence. `cargo deny check
// licenses` itself runs in scripts/verify.
#[test]
fn t10_deny_toml_allows_no_copyleft_licence() {
    let deny: toml::Table = read("deny.toml").parse().expect("deny.toml parses");
    let allow: Vec<String> = deny["licenses"]["allow"]
        .as_array()
        .expect("licenses.allow is an array")
        .iter()
        .map(|v| v.as_str().expect("licence ids are strings").to_string())
        .collect();
    assert!(allow.iter().any(|id| id == "MIT"));
    assert!(allow.iter().any(|id| id == "Apache-2.0"));
    for forbidden in [
        "GPL-2.0", "GPL-3.0", "AGPL-3.0", "LGPL-3.0", "SSPL-1.0", "BUSL-1.1",
    ] {
        assert!(
            !allow.iter().any(|id| id.starts_with(forbidden)),
            "{forbidden} is in the allow list"
        );
    }
    for id in &allow {
        let upper = id.to_uppercase();
        for family in ["GPL", "SSPL", "BUSL"] {
            assert!(!upper.contains(family), "{id} is in the allow list");
        }
    }
}

// Bullet 13: every document the Scope names exists; docs/adr/ holds ADRs 0001 to 0009, each
// with a Status line and listed in the index; CONTRIBUTING.md carries the publik API block.
#[test]
fn t13_documents_and_adrs_exist() {
    let root = repo_root();
    for doc in [
        "README.md",
        "CONTRIBUTING.md",
        "SECURITY.md",
        "RELEASING.md",
        "THIRD_PARTY.md",
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "docs/HANDBOOK.md",
        "docs/PUBLIK-CONTRACT.md",
        "docs/PUBLIK-INTEGRATION.md",
        "docs/adr/README.md",
        "docs/adr/template.md",
    ] {
        assert!(root.join(doc).is_file(), "{doc} is missing");
    }
    assert!(read("CONTRIBUTING.md").contains("## publik API"));

    let index = read("docs/adr/README.md");
    let mut adr_files: Vec<String> = fs::read_dir(root.join("docs").join("adr"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.len() > 5 && name[..4].chars().all(|c| c.is_ascii_digit()) && name.ends_with(".md")
        })
        .collect();
    adr_files.sort();
    assert_eq!(adr_files.len(), 9, "ADR files: {adr_files:?}");
    for (i, name) in adr_files.iter().enumerate() {
        assert!(
            name.starts_with(&format!("{:04}-", i + 1)),
            "{name} is out of sequence"
        );
        assert!(
            index.contains(name),
            "docs/adr/README.md does not list {name}"
        );
        let body = read(format!("docs/adr/{name}"));
        let status = body
            .lines()
            .find(|line| line.contains("Status:"))
            .unwrap_or_else(|| panic!("{name} has no Status: line"));
        // 0007 (name and slug) waits for the maintainer; the rest are decided.
        let expected = if i + 1 == 7 { "Proposed" } else { "Accepted" };
        assert!(
            status.contains(expected),
            "{name}: expected {expected}, found {status:?}"
        );
    }
}
