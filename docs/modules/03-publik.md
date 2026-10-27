# 03: Publik

## Goal

Redpen implements the shared Publik contract (`docs/PUBLIK-CONTRACT.md`) in full, for the
products `ai.chat` and `search`: the official Rust reference module is copied in and adapted, the
provider resolves in the contract's order, a computer can be set up after consent, the status and
balance are tracked from every response, and every Publik error maps to the one message and one way
out the contract prescribes. After this module any later module can make a Publik chat call through `AiClient`, or
a web search through `publik::search` (used by module 09), without touching the money path.

## Depends on

02.

## Scope

- `app/src-tauri/src/publik.rs`: the reference module verbatim, with `APP_SLUG = "redpen"`,
  `APP_VERSION = env!("CARGO_PKG_VERSION")` and adaptations A1 to A8 of docs/PUBLIK-INTEGRATION.md
  §2, each marked `// redpen:`.
- `app/src-tauri/publik-app-token.txt`, committed with one comment line until the maintainer's step;
  `build.rs` embeds the file, `PUBLIK_APP_TOKEN` at build time overrides it; no `pat_` token gives
  `NoAppToken`.
- `RedpenResolver` (implements `redpen_core::provider::ProviderResolver`): the contract's order,
  keyed on the Settings AI provider (docs/PUBLIK-INTEGRATION.md §3).
- `provision()` with the body of contract §5 (`device_name` a generic label, `dialects:
  ["chat_completions"]`, a fresh v4 `install_id` kept in the credential); 201, 200 with and without
  a key, 403 `install_revoked`, 429, other codes and unreachable handled as the contract says; the
  credential written with mode 0600 before `provision()` returns.
- `ConnectOutcome`: after provisioning, `run_pending = balance_micros > 0` and the first-run card
  data (`balance_line`, `cost_sentence`, `claim_url`, `claim_state`).
- Disconnect: `POST {base_url}/installs/revoke` best effort, then forget the key and keep
  `install_id`.
- `PublikStatusSink` (implements `StatusSink`): parses the eight `x-publik-*` headers of contract §7
  into `PublikStatus`, writes `status.json` (no key), returns the charge for `usage_daily`, sets
  `needs_credit` on 402 and clears it on the next success.
- `GET /wallet` at start and on `publik_refresh` (Settings opening), only while publik API is the
  chosen AI provider and a credential exists; with own key or local chosen, never, even with a
  credential file on disk.
- Publik errors mapped to `AiError` with the messages below; no retries (module 02 enforces it).
- `redpen_core::provider::tier`: `ClaimState`, `RewriteQuality`, `TierContext {quality,
  claim_state}` and `publik_tier(feature, &TierContext)`, the tier table of
  docs/PUBLIK-INTEGRATION.md §1 with `publik-smart` falling back to `publik-balanced` while
  anonymous. It lives in the core because `redpen-core` cannot depend on `app/src-tauri`;
  `publik.rs` builds the `TierContext` from Settings and `PublikStatus`.
- `x-publik-idempotency-key`: the first 32 hex characters of the cache key K, or of
  SHA-256(`<K>:<n>`) for the n-th Try again after an answer failed validation
  (docs/PUBLIK-INTEGRATION.md §5); `AiClient` takes `n` in `ChatCall.validation_retry` (0 by
  default).
- Tauri commands `consent_info`, `publik_connect`, `publik_disconnect`, `publik_refresh`,
  `provider_status`, `open_account_link`, `set_providers` (docs/ARCHITECTURE.md §5), returning only
  keyless types.
- `search`: the reference module's `search()` helper (contract §6a), adapted like the rest (A3: over
  `HttpTransport`; the key only in the `Authorization` header), returning `SearchOutcome {results,
  charge_micros}` from the body's `charge_micros` (or `x-publik-charge-micros`); 402 maps to
  `SearchError::NeedsCredit`, 502 and 504 to `SearchError::ProviderFailed` ("nothing was charged"),
  the rest as for chat; never retried. The status sink reads the headers of a search response like
  any other. `SearchResult`, `SearchOutcome` and `SearchError` live in
  `redpen_core::provider::search` so module 09 can fake them.
- Tray: a disabled menu item with the balance line once a credential exists.
- The copy-rule test (docs/TESTING.md §5).
- `scripts/publik-proof.sh`, `.ps1` and `src/bin/publik-proof.rs` (maintainer only): shows
  `DISCLOSURE` and asks y/N before provisioning; one `publik-fast` call ("Reply with the word ok.");
  prints the reply, the charge line and the names of the `x-publik-*` headers received, never the
  key or balance; `--models` prints the model ids of `GET /models`; `--probe-json` reports whether
  `response_format: {"type": "json_object"}` is honoured; `--search` makes one web search (one cent,
  after its own y/N) for `"largest coral reef system"` and prints the first result's title and site
  and the charge line.
- The built-app self-test (contract §3, test 18): the reference's `self_test`, called by the first
  lines of `main` in `app/src-tauri/src/main.rs`. When `PUBLIK_SELF_TEST` is set, it provisions for
  real over the production transport (the maintainer who runs it is the consent), prints exactly one
  line, `publik self-test: ok redpen <version> (install <first 8 characters of install_id>)` or
  `publik self-test: failed: <message>`, and exits before Tauri builds any window, the tray or a
  hotkey. Without the variable it never runs. Publik's setup skill runs the release build this way
  (docs/PUBLIK-INTEGRATION.md §2).

## Files

- `app/src-tauri/src/main.rs` (the self-test hook as the first lines of `main`)
- `app/src-tauri/src/{publik.rs, resolver.rs, status.rs, commands/publik.rs, tray.rs}`
- `app/src-tauri/publik-app-token.txt`, `app/src-tauri/build.rs`
- `app/src-tauri/src/bin/publik-proof.rs`, `scripts/publik-proof.{sh,ps1}`
- `crates/redpen-core/src/provider/{resolver.rs, publik_errors.rs, tier.rs, search.rs}` (trait,
  error variants, tier rule, the search types)
- `SECURITY.md` (`device_name`, the Publik requests), `CONTRIBUTING.md` (the publik block)
- Tests: `app/src-tauri/tests/03_publik.rs`, `crates/redpen-core/tests/03_publik.rs`,
  `tests/cases/03-publik/*.fake.json` (hand-written responses with invented header values)

## Interfaces

```rust
// publik.rs: the reference's public surface (its own names win if they differ; record the mapping)
pub const APP_SLUG: &str; pub const APP_VERSION: &str; pub const DISCLOSURE: &str;
pub fn credential_path() -> PathBuf;
pub fn resolve_provider(user_key: Option<&OwnKey>) -> Resolution;   // OwnKey | Env | File | NeedsConsent
pub async fn provision(t: &dyn HttpTransport) -> Result<Credential, ProvisionError>;
pub fn balance_line(micros: i64, claim_state: ClaimState) -> String;
pub fn top_up_url_from_402(body: &[u8]) -> Option<String>;
pub async fn self_test(t: &dyn HttpTransport, app_version: &str) -> String;   // the one line
// Redpen additions
pub struct ConnectOutcome { pub status: PublikStatus, pub run_pending: bool,
                            pub cost_sentence: String, pub claim_url: String }
pub fn map_publik_error(status: u16, body: &[u8]) -> AiError;
pub async fn search(t: &dyn HttpTransport, cred: &Credential, q: &str, count: u8)
    -> Result<SearchOutcome, SearchError>;                         // the reference's helper, adapted
// lib.rs: the startup hook, Tauri-free so a test can drive it with a fake transport;
// None unless the flag is set; runs `self_test` on a current-thread runtime
pub fn startup_self_test(flag: Option<OsString>, t: &dyn HttpTransport) -> Option<String>;
// main.rs: the first lines of `main`, before any window, the tray or a hotkey exists
fn main() {
    if let Some(line) = redpen_app::startup_self_test(std::env::var_os("PUBLIK_SELF_TEST"),
                                                      &redpen_app::production_transport()) {
        println!("{line}");
        return;
    }
    redpen_app::run();
}
// redpen-core::provider (resolver.rs, tier.rs)
pub trait ProviderResolver: Send + Sync { fn chat(&self) -> Result<ResolvedProvider, AiError>; }
pub enum ClaimState { Anonymous, Claimed }
pub enum RewriteQuality { Standard, Best }
pub struct TierContext { pub quality: RewriteQuality, pub claim_state: ClaimState }
pub fn publik_tier(f: Feature, ctx: &TierContext) -> &'static str;
pub fn idempotency_key(cache_key: &str, validation_retry: u32) -> String;   // 32 lowercase hex
```

Publik `AiError` variants and messages (docs/PUBLIK-INTEGRATION.md §9):

| Variant | Message and way out |
|---|---|
| `NeedsConsent` | the consent view (no message) |
| `NeedsCredit {message, top_up_url}` | `error.message`, one link, "or use your own key or a local model in Settings" |
| `Disconnected` | "publik API disconnected this computer." with **Reconnect** |
| `PublikRateLimited {message}` | `error.message` |
| `PublikBusy` | "publik API is busy; try again in a moment" with **Try again** |
| `PublikHttp {code, message}` | "publik API 500: The model returned an error." |
| `PublikTimeout {seconds}` | "publik API did not answer within 30 seconds. Try again." |
| `NoAppToken` | "This build has no publik API app token. Use your own key for now." |
| `ProvisionFailed {message}` | `error.message`, or "publik API could not set up this computer (500). Use your own key for now.", or "publik API is unreachable right now. Use your own key for now." |

**The app token.** The live Publik path needs the app token, which the maintainer obtains by
running Publik's setup skill once this module is done and a release build works, after the name
check of ADR-0007 (docs/PUBLIK-INTEGRATION.md §2: it lists the app from docs/PRODUCT.md §9, mints
the token with products `ai.chat` and `search`, and runs the release build with
`PUBLIK_SELF_TEST=1`). Until then
this module is completed and tested against the fake server, and modules 04 onwards proceed on
fakes; once the token is added, the skipped real-token test runs and the proof run is recorded.
Module 10 refuses to release without a token.

## Out of scope

- Rendering the consent view, first-run card and error cards: module 06 (this module provides the
  strings and commands).
- Running Publik's setup skill (listing, minting the token, the built-app run): the maintainer's
  step (docs/PUBLIK-INTEGRATION.md §2).

## Design notes

- Before implementation: fetch `https://publikhq.com/developers`, `/pricing`, `/llms.txt`,
  `/skills/setup.md`, `/skills/api.md`, `/skills/search.md` and `/sdk/publik.rs.md` with `curl` (raw) and record here
  anything that differs from the contract or docs/PUBLIK-INTEGRATION.md.
- Decided: the reference is copied, not rewritten (contract §3); reqwest replaces `curl` (A3).
- Until the maintainer has added the app token, the module is complete and tested against the fake
  server, and the live Publik path waits (above).
- To check on the maintainer's first real run: whether `x-publik-balance` is in micros or dollars;
  that a 402 repeated with the same idempotency key after linking succeeds; which header names
  arrive; that the setup skill's run of the release build prints `publik self-test: ok redpen
  <version>`.
- Further decisions are filled in here during implementation.

## Acceptance tests

Code tests in `app/src-tauri/tests/03_publik.rs` (Tauri-free functions) and
`crates/redpen-core/tests/03_publik.rs`; hand-written responses in `tests/cases/03-publik/*.fake.json`.
All HTTP faked.
Contract tests (docs/PUBLIK-CONTRACT.md §11) are numbered C1 to C19:

- C1 own key beats `PUBLIK_API_KEY` beats the credential file; nothing set → `NeedsConsent`.
- C2 `provision()` sends exactly the §5 body with `device_name: "Windows PC"` on Windows and no
  hostname anywhere; 201 writes the credential (mode 0600 on Unix) before returning; 200 with
  `key: null` keeps the key on disk; 200 with no key on disk mints once more under a fresh id;
  `install_revoked` mints once more; a second failure surfaces `error.message`.
- C3 the token is read from the file, overridden by `PUBLIK_APP_TOKEN`; the placeholder file gives
  `NoAppToken` with its message; the real-token test is skipped with "Publik app token not yet
  added (docs/PUBLIK-INTEGRATION.md §2)".
- C4 bearer header only; `model` is a tier name on every request (Redpen sends no audio,
  embeddings or images, so there is no other model value); the idempotency key matches
  `^[0-9a-f]{32}$`: `idempotency_key(K, 0)` is K's first 32 characters and `idempotency_key(K, 1)`
  is the first 32 of SHA-256(`K:1`) (reference: `sha256sum` of that string).
- C5 every header of contract §7 parsed into `PublikStatus`; `publik_refresh` reads `GET /wallet`.
- C6 402 (anonymous and claimed bodies): `NeedsCredit` with the message and exactly one URL, the
  `top_up_url`; one request; `needs_credit` set; no `ai_cache` row.
- C7 a 200 after a 402 clears `needs_credit`.
- C8 401 `invalid_api_key`, 401 `key_revoked` and 403 `install_revoked` → `Disconnected`.
- C9 429, 503 and 500 → one request each and their messages.
- C10 a Publik timeout → one request; an own-key timeout → retried (module 02 test 5).
- C11 per-provider caches, identical input never spends twice, a prompt version bump misses.
- C12 with a credential file present and the AI provider set to `local`, no request goes to
  `publikhq.com`.
- C13 `publik_tier` with `TierContext`: (Clearer, Best, Anonymous) → `publik-balanced`; (Clearer,
  Best, Claimed) → `publik-smart`; (FixGrammar, Best, Claimed) → `publik-fast`; (Clearer, Standard,
  Claimed) → `publik-balanced`.
- C14 schema validation of answers: covered by module 04 R5; here, a 200 without `choices` →
  `InvalidAnswer`.
- C15 every request built here is under 4 MB; no request body contains `image_url`, since Redpen
  sends no images.
- C16 the key `pk_live_a` never appears in logs, errors, `status.json`, `PublikStatus` or any
  command result (a capturing subscriber over C1 to C13).
- C17 the copy rule passes on the tree, and fails on a planted UI string.
- C18 the built-app self-test: test 11 below.
- C19 web search at the transport level: `search(…, "\"largest coral reef system\"", 5)` sends
  exactly `POST /search` with body `{"q":"\"largest coral reef system\"","count":5}` and the key
  only in `Authorization`; a 200 fake body gives two `SearchResult`s and `charge_micros` 10000; 402 →
  `NeedsCredit` with exactly one URL; 502 and 504 → `ProviderFailed`; one request in every case (the
  feature-level cap and own-key order are module 09's tests).

Redpen's own:

1. R11: two answers with `x-publik-charge-micros: 210` and `180` for `fix-grammar` → that day's
   `usage_daily.charge_micros == 390`.
2. R12: starting the app with no credential makes no request; with a credential and publik API
   chosen it makes exactly one, `GET /wallet`; with a credential on disk but own key or local chosen
   it makes none; `publik_connect` is the only path to `POST /installs` in normal use.
3. R13: a 201 with `balance_micros: 0` → `run_pending: false`, the card's `balance_line` equals
   `balance_line(0, Anonymous)`, `cost_sentence` is the body's verbatim; with `balance_micros:
   50000` → `run_pending: true`.
4. `publik_disconnect` sends `POST /installs/revoke` with the bearer key, forgets the key, keeps
   `install_id`; a failed revoke still forgets the key.
5. `open_account_link("top_up")` with no pending 402 → `NoLink`; the UI can never pass a URL.
6. The tray menu shows `balance_line` after a response with `x-publik-balance`, and no item before
   a credential exists.
7. `consent_info()` returns the data-path sentence of docs/PUBLIK-INTEGRATION.md §4 character for
   character and `DISCLOSURE` unchanged.
8. `publik-proof` without a token prints the `NoAppToken` message and exits 2 (it makes no request).
9. The proof's `run` function over the fake transport prints the reply, the charge line and the
   received `x-publik-*` header names, and its output contains neither `pk_live_a` nor the balance
   figure; `--models` prints the ids of a fake `/models` body; `--probe-json` prints "honoured" for a
   valid JSON answer and "not honoured" otherwise.
10. R7b: with K = `22f53c07ac6b84e42b6fea37d9f1ce05a6c42780307b5932ccf2031f7ef66dd1` (module 00
    test 2): `idempotency_key(K, 0)` = `22f53c07ac6b84e42b6fea37d9f1ce05`, `(K, 1)` =
    `b6058582879f1920e30934ad9402c4db`, `(K, 2)` = `84491dd9d4f881c8ec68ba4c829a264a` (reference:
    `sha256sum` of `K:1` and `K:2`). A first call carries `(K, 0)`; after an invalid answer, Try
    again carries `(K, 1)`, then `(K, 2)`; after a 503, Try again carries `(K, 0)` again; a valid
    answer on retry 1 is cached under K.
11. C18, the built-app self-test (contract test 18): `startup_self_test` with the flag set, the
    fake `PUBLIK_APP_TOKEN` of C3, and a fake server answering `POST /installs` with 201 and
    `install_id` `5f0c8a52-6e0b-4c55-9a3e-2a3f1d9b7c11` returns exactly `publik self-test: ok
    redpen 0.1.0 (install 5f0c8a52)` and the credential is written; with the fake answering 500 it
    returns `publik self-test: failed: publik API could not set up this computer (500). Use your
    own key for now.`; with the flag unset it returns `None` and the fake saw no request. The built
    `redpen-app`, started with `PUBLIK_SELF_TEST=1` and the placeholder token file, prints exactly
    one line, `publik self-test: failed: This build has no publik API app token. Use your own key
    for now.`, exits within 5 s, and creates no window, tray icon or hotkey (the `--smoke` `ready`
    event never fires).

## Status

Planned
