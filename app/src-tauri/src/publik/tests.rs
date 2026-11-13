//! Unit tests of `publik.rs`, over the fake transport and a temporary directory. Expected values
//! come from docs/PUBLIK-CONTRACT.md, docs/PUBLIK-INTEGRATION.md and the module's spec.

use std::cell::Cell;
use std::sync::{Arc, RwLock};

use redpen_core::clock::{FakeClock, FakeSleeper};
use redpen_core::net::{FakeEntry, FakeTransport, HttpRequest, TransportError};
use redpen_core::provider::{
    AiClient, CachePolicy, ChatCall, Feature, Message, ProviderKind, SecretString,
};
use redpen_core::store::{Db, Settings, local_day};
use serde_json::json;

use super::*;

const KEY: &str = "pk_live_a";
const TOKEN: &str = "pat_redpen_fake";
const KEPT_ID: &str = "5f0c8a52-6e0b-4c55-9a3e-2a3f1d9b7c11";
const BASE: &str = "https://publikhq.com/api/v1";
const NOW: i64 = 1_790_000_000_000;

thread_local! {
    static NEXT_ID: Cell<u32> = const { Cell::new(0) };
}

/// Fresh install ids in turn: `…000000000001`, `…000000000002`.
fn counting_id() -> String {
    NEXT_ID.with(|n| {
        let v = n.get() + 1;
        n.set(v);
        format!("00000000-0000-4000-8000-{v:012}")
    })
}

fn fresh_id(n: u32) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}

fn no_env(_: &str) -> Option<String> {
    None
}

fn options(dir: &Path, token: Option<&str>) -> PublikOptions {
    NEXT_ID.with(|n| n.set(0));
    PublikOptions {
        credential_path: dir.join("publik").join("apps").join("redpen.json"),
        app_token: token.map(str::to_string),
        base_url: BASE.into(),
        app_version: "0.1.0".into(),
        new_install_id: counting_id,
        env: no_env,
    }
}

fn installs(id: &str) -> HttpRequest {
    HttpRequest::post_json(
        format!("{BASE}/installs"),
        installs_body(TOKEN, "0.1.0", id),
    )
}

fn created(id: &str, balance: i64) -> Value {
    json!({
        "key": KEY, "key_id": "key_1", "base_url": BASE, "install_id": id,
        "claim_url": "https://publikhq.com/claim/abc", "claim_state": "anonymous",
        "balance_micros": balance,
        "wallet": {"balance_micros": balance, "add_credit_url": "https://publikhq.com/wallet/add",
                   "top_up_url": "https://publikhq.com/claim/abc", "claim_state": "anonymous"},
        "disclosure": {"version": 1, "cost": "A rewrite costs about $0.002.",
                       "data_path": "publik's servers"}
    })
}

fn kept_credential(dir: &Path, key: Option<&str>) -> Credential {
    let c = Credential {
        key: key.map(SecretString::from),
        base_url: BASE.into(),
        install_id: KEPT_ID.into(),
        claim_url: "https://publikhq.com/claim/old".into(),
        claim_state: ClaimState::Anonymous,
        add_credit_url: "https://publikhq.com/wallet/add".into(),
        cost_sentence: "old".into(),
        balance_micros: 0,
    };
    c.save(&options(dir, None).credential_path).unwrap();
    c
}

// ---------------------------------------------------------------------------------------------
// Texts and money.

#[test]
fn the_public_names_and_texts() {
    assert_eq!(APP_SLUG, "redpen");
    assert_eq!(APP_VERSION, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        DISCLOSURE,
        "Requests go through publik's servers to a shared AI model account. publik does not train on them and keeps no request bodies beyond a 24-hour fingerprint that stops a request being charged twice. The AI provider's own abuse monitoring applies."
    );
    let info = consent_info();
    assert_eq!(info.title, "Choose how Redpen uses AI");
    assert_eq!(
        info.data_path,
        "Checking is done on this computer and needs no AI. Only the text you select and the action you choose go through publik's servers, and only when you press a rewrite button."
    );
    assert_eq!(info.disclosure, DISCLOSURE);
    assert_eq!(info.disclosure_version, 1);
}

#[test]
fn the_balance_line() {
    use ClaimState::*;
    assert_eq!(
        balance_line(3_120_000, Anonymous),
        "$3.12 of publik balance"
    );
    assert_eq!(balance_line(3_120_000, Claimed), "$3.12 of publik balance");
    assert_eq!(balance_line(2_100, Claimed), "$0.0021 of publik balance");
    assert_eq!(
        balance_line(0, Anonymous),
        "$0.00 · link this computer for $0.05 of free use"
    );
    assert_eq!(balance_line(0, Claimed), "$0.00 of publik balance");
    assert_eq!(balance_line(-5_000, Claimed), "$0.00 of publik balance");
    assert_eq!(
        balance_line(-1, Anonymous),
        "$0.00 · link this computer for $0.05 of free use"
    );
    assert_eq!(balance_line(50_000, Anonymous), "$0.05 of publik balance");
}

#[test]
fn dollars_round_down_in_integer_arithmetic() {
    assert_eq!(dollars(10_000), "$0.01");
    assert_eq!(dollars(19_999), "$0.01");
    assert_eq!(dollars(9_999), "$0.0099");
    assert_eq!(dollars(210), "$0.0002");
    assert_eq!(dollars(1_250_000_000), "$1,250.00");
    assert_eq!(dollars(100_000_000), "$100.00");
    assert_eq!(dollars(1_234_567_890_000), "$1,234,567.89");
    assert_eq!(dollars(0), "$0.0000");
}

// ---------------------------------------------------------------------------------------------
// The token, the credential, the provider choice.

#[test]
fn the_token_comes_from_the_file_or_the_environment() {
    assert_eq!(parse_app_token("# No token yet.\n"), None);
    assert_eq!(parse_app_token(""), None);
    assert_eq!(parse_app_token("pat_redpen_fake\n").as_deref(), Some(TOKEN));
    assert_eq!(
        parse_app_token("# comment\n\n  pat_redpen_fake  \n").as_deref(),
        Some(TOKEN)
    );
    assert_eq!(parse_app_token("pk_live_a"), None);
    assert_eq!(parse_app_token("pat_"), None);
    assert_eq!(parse_app_token("pat_a b"), None);
    assert_eq!(
        app_token_from(Some("pat_redpen_env"), "pat_redpen_file").as_deref(),
        Some("pat_redpen_env")
    );
    assert_eq!(
        app_token_from(None, "pat_redpen_file").as_deref(),
        Some("pat_redpen_file")
    );
    assert_eq!(
        app_token_from(Some(""), "pat_redpen_file").as_deref(),
        Some("pat_redpen_file")
    );
    assert_eq!(app_token_from(None, ""), None);
}

#[test]
fn the_committed_placeholder_has_no_token() {
    let text = include_str!("../../publik-app-token.txt");
    assert_eq!(text.lines().count(), 1);
    assert!(text.starts_with('#'));
    assert_eq!(parse_app_token(text), None);
}

#[test]
fn the_credential_path_is_the_contracts() {
    let path = credential_path_from(|name| match name {
        "LOCALAPPDATA" => Some("C:/Local".into()),
        "XDG_CONFIG_HOME" => Some("/cfg".into()),
        "HOME" => Some("/home-dir".into()),
        _ => None,
    });
    let expected = if cfg!(windows) {
        PathBuf::from("C:/Local")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/home-dir/Library/Application Support")
    } else {
        PathBuf::from("/cfg")
    };
    assert_eq!(
        path,
        expected.join("publik").join("apps").join("redpen.json")
    );
    if !cfg!(windows) && !cfg!(target_os = "macos") {
        let fallback = credential_path_from(|name| match name {
            "HOME" => Some("/home-dir".into()),
            _ => None,
        });
        assert_eq!(
            fallback,
            PathBuf::from("/home-dir/.config/publik/apps/redpen.json")
        );
    }
}

#[test]
fn the_credential_round_trips_and_never_prints_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let c = kept_credential(dir.path(), Some(KEY));
    let path = options(dir.path(), None).credential_path;
    let back = Credential::load(&path).unwrap();
    assert_eq!(back.key.as_ref().unwrap().expose_secret(), KEY);
    assert_eq!(back.install_id, KEPT_ID);
    assert!(!format!("{c:?}").contains(KEY));
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut fields: Vec<&str> = written
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    // Contract §3: the credential's fields.
    assert_eq!(
        fields,
        [
            "add_credit_url",
            "balance_micros",
            "base_url",
            "claim_state",
            "claim_url",
            "cost_sentence",
            "install_id",
            "key"
        ]
    );
    let mut forgotten = back.clone();
    forgotten.key = None;
    forgotten.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("\"key\""));
    assert!(!Credential::load(&path).unwrap().has_key());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn own_key_beats_the_environment_beats_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = options(dir.path(), None).credential_path;
    let env = |name: &str| match name {
        "PUBLIK_API_KEY" => Some("pk_live_env".to_string()),
        "PUBLIK_API_BASE_URL" => Some("http://localhost:8787/api/v1".to_string()),
        _ => None,
    };
    let own = OwnKey {
        base_url: "https://api.example.com/v1".into(),
        key: SecretString::from("sk-test-a"),
        chat_model: "example-chat".into(),
    };
    assert!(matches!(
        resolve_provider_with(None, &no_env, &path),
        Resolution::NeedsConsent
    ));
    kept_credential(dir.path(), None);
    assert!(
        matches!(
            resolve_provider_with(None, &no_env, &path),
            Resolution::NeedsConsent
        ),
        "a credential without its key is not set up"
    );
    kept_credential(dir.path(), Some(KEY));
    match resolve_provider_with(None, &no_env, &path) {
        Resolution::File(c) => assert_eq!(c.key.unwrap().expose_secret(), KEY),
        other => panic!("{other:?}"),
    }
    match resolve_provider_with(None, &env, &path) {
        Resolution::Env(c) => {
            assert_eq!(c.key.unwrap().expose_secret(), "pk_live_env");
            assert_eq!(c.base_url, "http://localhost:8787/api/v1");
        }
        other => panic!("{other:?}"),
    }
    match resolve_provider_with(Some(&own), &env, &path) {
        Resolution::OwnKey(o) => assert_eq!(o.provider().kind, ProviderKind::OwnKey),
        other => panic!("{other:?}"),
    }
    assert!(!format!("{own:?}").contains("sk-test-a"));
}

#[test]
fn a_credential_resolves_to_a_publik_provider() {
    let c = Credential {
        key: Some(SecretString::from(KEY)),
        base_url: format!("{BASE}/"),
        ..Credential::default()
    };
    let p = c.provider("publik-fast");
    assert_eq!(p.kind, ProviderKind::Publik);
    assert_eq!(p.chat_model, "publik-fast");
    assert_eq!(
        p.url("/chat/completions"),
        format!("{BASE}/chat/completions")
    );
    assert_eq!(c.url("/wallet"), format!("{BASE}/wallet"));
}

// ---------------------------------------------------------------------------------------------
// Provisioning.

#[test]
fn the_installs_body_is_the_contracts_with_a_generic_device_name() {
    let body = installs_body(TOKEN, "0.1.0", KEPT_ID);
    let device = if cfg!(windows) {
        "Windows PC"
    } else if cfg!(target_os = "macos") {
        "Mac"
    } else {
        "Linux PC"
    };
    assert_eq!(
        body,
        json!({
            "app_token": TOKEN, "app_slug": "redpen", "app_version": "0.1.0",
            "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
            "device_name": device,
            "install_id": KEPT_ID, "disclosure_version": 1, "dialects": ["chat_completions"]
        })
    );
    if cfg!(windows) {
        assert_eq!(body["os"], "windows");
    }
    let text = body.to_string();
    for name in ["COMPUTERNAME", "HOSTNAME", "USERNAME", "USER"] {
        if let Ok(v) = std::env::var(name)
            && v.len() > 3
        {
            assert!(!text.contains(&v), "the body names {name}");
        }
    }
}

#[tokio::test]
async fn a_201_writes_the_credential_before_returning() {
    let dir = tempfile::tempdir().unwrap();
    let o = options(dir.path(), Some(TOKEN));
    let fake = FakeTransport::scripted(vec![FakeEntry::reply(
        &installs(&fresh_id(1)),
        201,
        created(&fresh_id(1), 0),
    )]);
    let c = provision_with(&fake, &o).await.unwrap();
    assert_eq!(c.key.as_ref().unwrap().expose_secret(), KEY);
    assert_eq!(c.install_id, fresh_id(1));
    assert_eq!(c.claim_url, "https://publikhq.com/claim/abc");
    assert_eq!(c.add_credit_url, "https://publikhq.com/wallet/add");
    assert_eq!(c.cost_sentence, "A rewrite costs about $0.002.");
    assert_eq!(c.claim_state, ClaimState::Anonymous);
    let on_disk = Credential::load(&o.credential_path).unwrap();
    assert_eq!(on_disk.key.unwrap().expose_secret(), KEY);
    let seen = fake.requests_seen();
    assert_eq!(seen.len(), 1);
    assert!(
        !seen[0]
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("authorization")),
        "POST /installs carries no auth header"
    );
}

#[tokio::test]
async fn the_kept_install_id_is_reused_and_a_null_key_keeps_the_key_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    kept_credential(dir.path(), Some(KEY));
    let o = options(dir.path(), Some(TOKEN));
    let mut replay = created(KEPT_ID, 2_100);
    replay["key"] = Value::Null;
    let fake = FakeTransport::scripted(vec![FakeEntry::reply(&installs(KEPT_ID), 200, replay)]);
    let c = provision_with(&fake, &o).await.unwrap();
    assert_eq!(c.key.unwrap().expose_secret(), KEY);
    assert_eq!(c.install_id, KEPT_ID);
    assert_eq!(c.balance_micros, 2_100);
    assert_eq!(fake.requests_seen().len(), 1);
}

#[tokio::test]
async fn a_null_key_with_no_key_on_disk_mints_once_more_under_a_fresh_id() {
    let dir = tempfile::tempdir().unwrap();
    // After Disconnect: the install id stays, the key is gone.
    kept_credential(dir.path(), None);
    let o = options(dir.path(), Some(TOKEN));
    let mut replay = created(KEPT_ID, 0);
    replay["key"] = Value::Null;
    let fake = FakeTransport::scripted(vec![
        FakeEntry::reply(&installs(KEPT_ID), 200, replay),
        FakeEntry::reply(&installs(&fresh_id(1)), 201, created(&fresh_id(1), 0)),
    ]);
    let c = provision_with(&fake, &o).await.unwrap();
    assert_eq!(c.install_id, fresh_id(1));
    assert_eq!(fake.requests_seen().len(), 2);
}

#[tokio::test]
async fn install_revoked_mints_once_more_and_a_second_failure_gives_its_message() {
    let dir = tempfile::tempdir().unwrap();
    kept_credential(dir.path(), Some(KEY));
    let o = options(dir.path(), Some(TOKEN));
    let revoked =
        json!({"error": {"code": "install_revoked", "message": "This install was revoked."}});
    let fake = FakeTransport::scripted(vec![
        FakeEntry::reply(&installs(KEPT_ID), 403, revoked.clone()),
        FakeEntry::reply(&installs(&fresh_id(1)), 201, created(&fresh_id(1), 50_000)),
    ]);
    let c = provision_with(&fake, &o).await.unwrap();
    assert_eq!(c.install_id, fresh_id(1));
    assert_eq!(c.balance_micros, 50_000);

    let dir = tempfile::tempdir().unwrap();
    kept_credential(dir.path(), Some(KEY));
    let o = options(dir.path(), Some(TOKEN));
    let fake = FakeTransport::scripted(vec![
        FakeEntry::reply(&installs(KEPT_ID), 403, revoked.clone()),
        FakeEntry::reply(&installs(&fresh_id(1)), 403, revoked),
    ]);
    assert_eq!(
        provision_with(&fake, &o).await.unwrap_err(),
        ProvisionError::Failed {
            message: "This install was revoked.".into()
        }
    );
    assert_eq!(fake.requests_seen().len(), 2, "never a third attempt");
}

#[tokio::test]
async fn failures_have_the_contracts_messages() {
    let cases = [
        (
            429,
            json!({"error": {"message": "publik API is busy setting up computers."}}),
            "publik API is busy setting up computers.",
        ),
        (429, json!({}), PROVISION_BUSY_FALLBACK),
        (
            500,
            json!({}),
            "publik API could not set up this computer (500). Use your own key for now.",
        ),
        (
            400,
            json!({"error": {"message": "The app token is not valid."}}),
            "The app token is not valid.",
        ),
        (
            201,
            json!({"key": null}),
            "publik API could not set up this computer (201). Use your own key for now.",
        ),
    ];
    for (status, body, message) in cases {
        let dir = tempfile::tempdir().unwrap();
        let o = options(dir.path(), Some(TOKEN));
        let fake = FakeTransport::scripted(vec![FakeEntry::reply(
            &installs(&fresh_id(1)),
            status,
            body,
        )]);
        let err = provision_with(&fake, &o).await.unwrap_err();
        assert_eq!(err.to_string(), message, "{status}");
        assert_eq!(fake.requests_seen().len(), 1, "{status}: one request");
        assert!(Credential::load(&o.credential_path).is_none());
    }
    let dir = tempfile::tempdir().unwrap();
    let o = options(dir.path(), Some(TOKEN));
    let fake = FakeTransport::scripted(vec![FakeEntry::failing(
        &installs(&fresh_id(1)),
        TransportError::Offline,
    )]);
    assert_eq!(
        provision_with(&fake, &o).await.unwrap_err().to_string(),
        "publik API is unreachable right now. Use your own key for now."
    );
}

#[tokio::test]
async fn no_token_means_no_request() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTransport::scripted(Vec::new());
    let err = provision_with(&fake, &options(dir.path(), None))
        .await
        .unwrap_err();
    assert_eq!(err, ProvisionError::NoAppToken);
    assert_eq!(
        err.to_string(),
        "This build has no publik API app token. Use your own key for now."
    );
    assert_eq!(AiError::from(err), AiError::NoAppToken);
    assert!(fake.requests_seen().is_empty());
    assert_eq!(
        AiError::from(ProvisionError::Failed {
            message: "m".into()
        }),
        AiError::ProvisionFailed {
            message: "m".into()
        }
    );
}

// ---------------------------------------------------------------------------------------------
// Disconnect, the wallet, the self-test.

fn revoke_request() -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        url: format!("{BASE}/installs/revoke"),
        headers: Vec::new(),
        body: Body::Empty,
        timeout: HOUSEKEEPING_TIMEOUT,
    }
}

#[tokio::test]
async fn disconnect_revokes_then_forgets_the_key_and_keeps_the_install_id() {
    for revoke in [
        FakeEntry::reply(&revoke_request(), 200, json!({"revoked": true})),
        FakeEntry::failing(&revoke_request(), TransportError::Offline),
    ] {
        let dir = tempfile::tempdir().unwrap();
        kept_credential(dir.path(), Some(KEY));
        let path = options(dir.path(), None).credential_path;
        let fake = FakeTransport::scripted(vec![revoke]);
        disconnect_at(&fake, &path).await;
        let seen = fake.requests_seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].url, format!("{BASE}/installs/revoke"));
        let auth = seen[0]
            .headers
            .iter()
            .find(|(n, _)| n == "Authorization")
            .unwrap();
        assert!(auth.1.is_secret());
        assert_eq!(auth.1.expose(), format!("Bearer {KEY}"));
        let after = Credential::load(&path).unwrap();
        assert!(!after.has_key());
        assert_eq!(after.install_id, KEPT_ID);
    }
    // No credential: nothing to do, nothing sent.
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTransport::scripted(Vec::new());
    disconnect_at(&fake, &dir.path().join("none.json")).await;
    assert!(fake.requests_seen().is_empty());
}

#[tokio::test]
async fn the_self_test_provisions_then_reads_the_wallet() {
    let dir = tempfile::tempdir().unwrap();
    let o = options(dir.path(), Some(TOKEN));
    // The fresh id the fake gives back is the contract's example.
    let fake = FakeTransport::scripted(vec![
        FakeEntry::reply(&installs(&fresh_id(1)), 201, created(KEPT_ID, 0)),
        FakeEntry::reply(
            &HttpRequest::get(format!("{BASE}/wallet")),
            200,
            json!({"balance_micros": 0, "claim_state": "anonymous"}),
        ),
    ]);
    assert_eq!(
        self_test_with(&fake, &o).await,
        "publik self-test: ok redpen 0.1.0 (install 5f0c8a52)"
    );
    assert!(Credential::load(&o.credential_path).unwrap().has_key());
    let urls: Vec<String> = fake.requests_seen().into_iter().map(|r| r.url).collect();
    assert_eq!(urls, [format!("{BASE}/installs"), format!("{BASE}/wallet")]);

    let dir = tempfile::tempdir().unwrap();
    let o = options(dir.path(), Some(TOKEN));
    let fake = FakeTransport::scripted(vec![FakeEntry::reply(
        &installs(&fresh_id(1)),
        500,
        json!({}),
    )]);
    assert_eq!(
        self_test_with(&fake, &o).await,
        "publik self-test: failed: publik API could not set up this computer (500). Use your own key for now."
    );
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTransport::scripted(Vec::new());
    assert_eq!(
        self_test_with(&fake, &options(dir.path(), None)).await,
        "publik self-test: failed: This build has no publik API app token. Use your own key for now."
    );
    assert!(fake.requests_seen().is_empty());
}

#[tokio::test]
async fn a_failed_wallet_fails_the_self_test() {
    let dir = tempfile::tempdir().unwrap();
    let o = options(dir.path(), Some(TOKEN));
    let fake = FakeTransport::scripted(vec![
        FakeEntry::reply(&installs(&fresh_id(1)), 201, created(KEPT_ID, 0)),
        FakeEntry::reply(
            &HttpRequest::get(format!("{BASE}/wallet")),
            401,
            json!({"error": {"code": "invalid_api_key"}}),
        ),
    ]);
    assert_eq!(
        self_test_with(&fake, &o).await,
        "publik self-test: failed: publik API disconnected this computer."
    );
}

// ---------------------------------------------------------------------------------------------
// Web search.

fn search_request(q: &str, count: u8) -> HttpRequest {
    HttpRequest::post_json(format!("{BASE}/search"), json!({"q": q, "count": count}))
}

fn key_credential() -> Credential {
    Credential {
        key: Some(SecretString::from(KEY)),
        base_url: BASE.into(),
        install_id: KEPT_ID.into(),
        ..Credential::default()
    }
}

#[tokio::test]
async fn a_search_sends_q_and_count_with_the_key_in_the_header_only() {
    let q = "\"largest coral reef system\"";
    let fake = FakeTransport::scripted(vec![FakeEntry::reply(
        &search_request(q, 5),
        200,
        json!({"object": "search.results", "query": q, "count": 5, "more": false,
               "results": [
                 {"url": "https://en.example.org/wiki/Great_Barrier_Reef", "title": "Great Barrier Reef",
                  "snippet": "the world's largest coral reef system", "site": "en.example.org", "age": "2026-09-30"},
                 {"url": "https://www.example.com/reefs", "title": "Reefs", "snippet": "…", "site": "www.example.com"}],
               "charge_micros": 10000, "request_id": "req_1"}),
    )]);
    let outcome = search(&fake, &key_credential(), q, 5).await.unwrap();
    assert_eq!(outcome.results.len(), 2);
    assert_eq!(outcome.charge_micros, Some(10_000));
    let seen = fake.requests_seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url, format!("{BASE}/search"));
    assert_eq!(
        seen[0].body,
        Body::Json(json!({"q": q, "count": 5})),
        "exactly q and count"
    );
    assert!(!seen[0].url.contains(KEY));
    let with_key: Vec<&str> = seen[0]
        .headers
        .iter()
        .filter(|(_, v)| v.expose().contains(KEY))
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(with_key, ["Authorization"]);
    assert!(
        !seen[0]
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("x-publik-idempotency-key")),
        "a search carries no idempotency key"
    );
}

#[tokio::test]
async fn search_errors_are_one_request_each() {
    let q = "\"a phrase\"";
    let credit = json!({"error": {"message": "Add a plan or pack.", "top_up_url": "https://publikhq.com/wallet/add"}});
    let cases = [
        (
            402,
            credit,
            SearchError::NeedsCredit {
                message: "Add a plan or pack.".into(),
                top_up_url: Some("https://publikhq.com/wallet/add".into()),
            },
        ),
        (502, json!({}), SearchError::ProviderFailed),
        (504, json!({}), SearchError::ProviderFailed),
        (503, json!({}), SearchError::Busy),
    ];
    for (status, body, expected) in cases {
        let fake =
            FakeTransport::scripted(vec![FakeEntry::reply(&search_request(q, 5), status, body)]);
        assert_eq!(
            search(&fake, &key_credential(), q, 5).await.unwrap_err(),
            expected
        );
        assert_eq!(fake.requests_seen().len(), 1, "{status}: never retried");
    }
    let fake = FakeTransport::scripted(Vec::new());
    assert_eq!(
        search(&fake, &Credential::default(), q, 5)
            .await
            .unwrap_err(),
        SearchError::Disconnected
    );
    assert!(fake.requests_seen().is_empty());
}

// ---------------------------------------------------------------------------------------------
// The service.

struct Service {
    dir: tempfile::TempDir,
    fake: Arc<FakeTransport>,
    service: PublikService,
}

fn service(chat: Option<ChatChoice>, entries: Vec<FakeEntry>, token: Option<&str>) -> Service {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Settings::default();
    s.providers.chat = chat;
    let fake = Arc::new(FakeTransport::scripted(entries));
    let status = Arc::new(StatusStore::new(
        &dir.path().join("data"),
        Arc::new(FakeClock::at(NOW)),
    ));
    let service = PublikService::new(
        fake.clone(),
        options(dir.path(), token),
        dir.path().join("config"),
        Arc::new(RwLock::new(s)),
        status,
    );
    Service { dir, fake, service }
}

#[tokio::test]
async fn connect_gives_the_first_run_card_and_chooses_publik() {
    for (balance, run_pending) in [(0, false), (50_000, true)] {
        let s = service(
            None,
            vec![FakeEntry::reply(
                &installs(&fresh_id(1)),
                201,
                created(&fresh_id(1), balance),
            )],
            Some(TOKEN),
        );
        let outcome = s.service.connect().await.unwrap();
        assert_eq!(outcome.run_pending, run_pending);
        assert_eq!(
            outcome.balance_line,
            balance_line(balance, ClaimState::Anonymous)
        );
        assert_eq!(outcome.cost_sentence, "A rewrite costs about $0.002.");
        assert_eq!(outcome.claim_url, "https://publikhq.com/claim/abc");
        assert_eq!(outcome.claim_state, ClaimState::Anonymous);
        assert_eq!(outcome.status.balance_micros, balance);
        assert_eq!(
            s.service.settings().providers.chat,
            Some(ChatChoice::Publik)
        );
        let saved = Settings::load(&s.dir.path().join("config")).unwrap();
        assert_eq!(saved.providers.chat, Some(ChatChoice::Publik));
        let shown = serde_json::to_string(&outcome).unwrap();
        assert!(!shown.contains(KEY));
        let status_file =
            std::fs::read_to_string(s.dir.path().join("data").join("status.json")).unwrap();
        assert!(!status_file.contains(KEY));
        assert_eq!(s.fake.requests_seen().len(), 1);
    }
    let s = service(None, Vec::new(), None);
    assert_eq!(s.service.connect().await.unwrap_err(), AiError::NoAppToken);
    assert!(s.fake.requests_seen().is_empty());
}

fn wallet_entry() -> FakeEntry {
    let mut e = FakeEntry::reply(
        &HttpRequest::get(format!("{BASE}/wallet")),
        200,
        json!({"balance_micros": 3120000, "claim_state": "claimed"}),
    );
    e.headers = vec![("x-publik-week-used".into(), "4200".into())];
    e
}

#[tokio::test]
async fn the_wallet_is_read_only_while_publik_is_chosen_and_set_up() {
    // No credential: nothing at start, NeedsConsent on refresh, no request.
    let s = service(Some(ChatChoice::Publik), vec![wallet_entry()], None);
    assert!(s.service.refresh_at_start().await.is_none());
    assert_eq!(
        s.service.refresh().await.unwrap_err(),
        AiError::NeedsConsent
    );
    assert!(s.fake.requests_seen().is_empty());
    // A credential and publik API chosen: exactly one GET /wallet.
    let s = service(Some(ChatChoice::Publik), vec![wallet_entry()], None);
    kept_credential(s.dir.path(), Some(KEY));
    let status = s.service.refresh_at_start().await.unwrap().unwrap();
    assert_eq!(status.balance_micros, 3_120_000);
    assert_eq!(status.claim_state, ClaimState::Claimed);
    assert_eq!(status.week_used_micros, 4_200);
    let seen = s.fake.requests_seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, Method::Get);
    assert_eq!(seen[0].url, format!("{BASE}/wallet"));
    // A credential on disk, but the own key, a local model or no choice: none.
    for chat in [Some(ChatChoice::OwnKey), Some(ChatChoice::Local), None] {
        let s = service(chat, vec![wallet_entry()], None);
        kept_credential(s.dir.path(), Some(KEY));
        assert!(s.service.refresh_at_start().await.is_none());
        assert_eq!(
            s.service.refresh().await.unwrap_err(),
            AiError::NotConfigured
        );
        assert!(s.fake.requests_seen().is_empty(), "{chat:?}");
    }
}

#[tokio::test]
async fn provider_status_and_links() {
    let s = service(Some(ChatChoice::Publik), vec![wallet_entry()], None);
    let empty = s.service.provider_status();
    assert_eq!(empty.chat, Some(ChatChoice::Publik));
    assert_eq!(empty.publik, None);
    assert_eq!(empty.balance_line, None);
    assert_eq!(
        s.service.account_link(LinkKind::TopUp),
        Err(LinkError::NoLink)
    );
    assert_eq!(
        s.service.account_link(LinkKind::Claim),
        Err(LinkError::NoLink)
    );
    kept_credential(s.dir.path(), Some(KEY));
    s.service.refresh().await.unwrap();
    let status = s.service.provider_status();
    assert_eq!(
        status.balance_line.as_deref(),
        Some("$3.12 of publik balance")
    );
    assert!(!serde_json::to_string(&status).unwrap().contains(KEY));
    assert_eq!(
        s.service.account_link(LinkKind::Claim).unwrap(),
        "https://publikhq.com/claim/old"
    );
    assert_eq!(
        s.service.account_link(LinkKind::AddCredit).unwrap(),
        "https://publikhq.com/wallet/add"
    );
    assert_eq!(
        s.service.account_link(LinkKind::TopUp),
        Err(LinkError::NoLink)
    );
    s.service.status().note_error(&AiError::NeedsCredit {
        message: "m".into(),
        top_up_url: Some("https://publikhq.com/claim/abc".into()),
    });
    assert_eq!(
        s.service.account_link(LinkKind::TopUp).unwrap(),
        "https://publikhq.com/claim/abc"
    );
    assert_eq!(
        LinkError::CouldNotOpen.to_string(),
        "Redpen could not open your browser."
    );
}

#[tokio::test]
async fn disconnect_through_the_service_forgets_the_status() {
    let s = service(
        Some(ChatChoice::Publik),
        vec![
            wallet_entry(),
            FakeEntry::reply(&revoke_request(), 200, json!({})),
        ],
        None,
    );
    kept_credential(s.dir.path(), Some(KEY));
    s.service.refresh().await.unwrap();
    assert!(s.service.status().current().is_some());
    s.service.disconnect().await;
    assert!(s.service.status().current().is_none());
    assert!(s.service.active_credential().is_none());
    assert_eq!(
        s.service.credential().unwrap().install_id,
        KEPT_ID,
        "the install id stays"
    );
}

#[test]
fn set_providers_saves_only_what_it_is_given() {
    let s = service(None, Vec::new(), None);
    let settings = s
        .service
        .set_providers(ProvidersPatch {
            chat: Some(ChatChoice::Local),
            rewrite_quality: Some(RewriteQuality::Best),
            search: None,
        })
        .unwrap();
    assert_eq!(settings.providers.chat, Some(ChatChoice::Local));
    assert_eq!(settings.providers.rewrite_quality, RewriteQuality::Best);
    assert_eq!(settings.providers.search, None);
    let settings = s
        .service
        .set_providers(ProvidersPatch {
            search: Some(SearchChoice::Publik),
            ..ProvidersPatch::default()
        })
        .unwrap();
    assert_eq!(settings.providers.chat, Some(ChatChoice::Local));
    assert_eq!(settings.providers.search, Some(SearchChoice::Publik));
    let on_disk = Settings::load(&s.dir.path().join("config")).unwrap();
    assert_eq!(on_disk, settings);
    // A settings file from a newer Redpen is never overwritten.
    std::fs::write(
        s.dir.path().join("config").join("settings.json"),
        r#"{"version": 2}"#,
    )
    .unwrap();
    let err = s
        .service
        .set_providers(ProvidersPatch {
            chat: Some(ChatChoice::OwnKey),
            ..ProvidersPatch::default()
        })
        .unwrap_err();
    assert_eq!(err.to_string(), "Settings were written by a newer Redpen.");
    assert_eq!(s.service.settings().providers.chat, Some(ChatChoice::Local));
}

#[tokio::test]
async fn the_service_search_reads_the_status_headers() {
    let q = "\"a phrase\"";
    let mut answer = FakeEntry::reply(
        &search_request(q, 5),
        200,
        json!({"results": [], "charge_micros": 10000}),
    );
    answer.headers = vec![
        ("x-publik-charge-micros".into(), "10000".into()),
        ("x-publik-balance".into(), "40000".into()),
    ];
    let s = service(Some(ChatChoice::Publik), vec![answer], None);
    kept_credential(s.dir.path(), Some(KEY));
    let outcome = s.service.search(q, 5).await.unwrap();
    assert_eq!(outcome.charge_micros, Some(10_000));
    let status = s.service.status().current().unwrap();
    assert_eq!(status.balance_micros, 40_000);
    assert_eq!(status.last_charge_micros, Some(10_000));
}

// ---------------------------------------------------------------------------------------------
// Chat through the AI client with the publik status sink.

#[tokio::test]
async fn charges_from_the_headers_add_up_in_usage_daily() {
    let dir = tempfile::tempdir().unwrap();
    let status = Arc::new(StatusStore::new(dir.path(), Arc::new(FakeClock::at(NOW))));
    let p = key_credential().provider("publik-fast");
    let call = |text: &str| ChatCall {
        feature: Feature::FixGrammar,
        prompt_id: "fix-grammar",
        prompt_version: 1,
        messages: vec![Message::user(text)],
        max_tokens: 256,
        temperature: 0.0,
        timeout: std::time::Duration::from_secs(30),
        cache_input: json!({"text": text}),
        cache: CachePolicy::Disk,
        validation_retry: 0,
    };
    let answer = json!({"choices": [{"message": {"role": "assistant", "content": "x"}}]});
    let mut entries = Vec::new();
    for (text, charge) in [("a", "210"), ("b", "180")] {
        let c = call(text);
        let req = HttpRequest::post_json(
            p.url("/chat/completions"),
            redpen_core::provider::client::chat_body(&p, &c),
        );
        let mut e = FakeEntry::reply(&req, 200, answer.clone());
        e.headers = vec![
            ("x-publik-charge-micros".into(), charge.into()),
            ("x-publik-balance".into(), "3120000".into()),
        ];
        entries.push(e);
    }
    let db = Db::open_in_memory().unwrap();
    let client = AiClient::new(
        Arc::new(FakeTransport::scripted(entries)),
        db.clone(),
        Arc::new(FakeClock::at(NOW)),
        Arc::new(FakeSleeper::new()),
        Arc::new(PublikStatusSink::new(status.clone())),
    );
    client.chat(&p, call("a")).await.unwrap();
    client.chat(&p, call("b")).await.unwrap();
    let usage = db
        .usage(&local_day(NOW, 0), "publik", "fix-grammar")
        .unwrap()
        .unwrap();
    assert_eq!(usage.charge_micros, 390);
    assert_eq!(usage.calls, 2);
    let current = status.current().unwrap();
    assert_eq!(current.last_charge_micros, Some(180));
    assert_eq!(current.balance_micros, 3_120_000);
}
