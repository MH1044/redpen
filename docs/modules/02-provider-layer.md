# 02: Provider layer

## Goal

The rest of Redpen can make a chat call through one OpenAI-compatible client, to the user's own
key, a local model (Ollama or any OpenAI-compatible server on localhost) or the fake, with the
answer cache, usage counting, the settings and secrets files, and errors a user can act on. Module
03 plugs Publik into the same client.

## Depends on

00.

## Scope

- `AiClient::chat`: `POST {base_url}/chat/completions` with `{model, messages, max_tokens,
  temperature}`, not streamed; returns `choices[0].message.content`.
- `ResolvedProvider` with kinds `OwnKey`, `Local`, `Fake` (and `Publik`, filled by module 03);
  the key is a `SecretString`, sent only as `Authorization: Bearer`.
- Own key: base URL and chat model in `settings.json`; the key in
  `secrets.json`, mode 0600 on Linux, in the per-user config folder on Windows.
- `provider_test("own_key")`: `GET {base_url}/models`, "Key works: 14 models available."
- Local: base URL must be `localhost`, `127.0.0.1` or `::1`; Ollama detection by
  `GET /api/tags`; `pick_local_model` (docs/PUBLIK-INTEGRATION.md §3); no request to any other host.
- Retries for own key and local only: two more attempts after 0.5 s and 1 s (injected `Sleeper`) on
  timeout, 429 and 5xx; never on other 4xx. The retry rule is keyed on the provider kind, so a
  `Publik` provider can never retry.
- Answer cache in `redpen.db` `ai_cache` (docs/ARCHITECTURE.md §4): key = SHA-256 of the canonical
  cache input including the provider; `CachePolicy::Disk`, `Memory` (process-lifetime map) or
  `Off`; errors and invalid answers are never cached; rows older than `remember_answers_days`
  deleted at start and hourly; 0 days means `Disk` behaves as `Memory`.
- `usage_daily`: one row per (local day, provider, feature), `calls` incremented per sent call (not
  cache hits), `charge_micros` added from the status sink.
- `sent_log`: written only while Settings → Privacy → "Keep a log" is on; rows older than 7 days
  deleted at start and hourly; `cache_clear` and `sent_log_clear` empty their tables.
- `settings.json`: the full schema with defaults (docs/ARCHITECTURE.md §4), atomic write
  (temporary file then rename), unknown fields preserved, a newer `version` opened read-only.
- Errors: the `AiError` variants below with their messages; the key never appears in any error,
  `Debug` output or log line.
- CLI `--provider local|own-key|fake` (own key from `REDPEN_OWN_KEY`, `REDPEN_OWN_BASE_URL`,
  `REDPEN_OWN_MODEL`) and the diagnostic `redpen chat --provider P [--recording R] --message S`.

## Files

- `crates/redpen-core/src/provider/{mod.rs, client.rs, resolved.rs, errors.rs, cache.rs, local.rs, retry.rs}`
- `crates/redpen-core/src/store/{mod.rs, settings.rs, secrets.rs, db.rs}`
- `crates/redpen-core/src/net/reqwest.rs` (the production `HttpTransport`)
- `crates/redpen-cli/src/providers.rs`
- `THIRD_PARTY.md` (`reqwest`, `tokio`, `rusqlite`, `secrecy`, `async-trait`)
- Tests: `crates/redpen-core/tests/02_provider_layer.rs`, `tests/cases/02-provider-layer/`

## Interfaces

```rust
pub enum ProviderKind { Publik, OwnKey, Local, Fake }
pub struct ResolvedProvider { pub kind: ProviderKind, pub base_url: String,
    pub key: Option<SecretString>, pub chat_model: String }
pub enum Feature { FixGrammar, Clearer, Shorter, MoreFormal, Friendlier, Custom,
                   Test }                                             // kebab-case in the database
pub enum CachePolicy { Disk, Memory, Off }
pub struct ChatCall { pub feature: Feature, pub prompt_id: &'static str, pub prompt_version: u32,
    pub messages: Vec<Message>, pub max_tokens: u32, pub temperature: f32,
    pub timeout: Duration, pub cache_input: serde_json::Value, pub cache: CachePolicy,
    pub validation_retry: u32 }   // n-th Try again after an invalid answer; module 03 uses it for the idempotency key
pub struct ChatReply { pub content: String, pub from_cache: bool,
                       pub charge_micros: Option<u64>, pub request_id: Option<String> }
pub trait StatusSink: Send + Sync {     // module 03 implements it for Publik; default no-op
    fn on_response(&self, kind: ProviderKind, feature: Feature, status: u16,
                   headers: &[(String, String)]) -> Option<u64>;   // returns the charge in micros
}
impl AiClient {
    pub fn new(t: Arc<dyn HttpTransport>, db: Db, clock: Arc<dyn Clock>,
               sleeper: Arc<dyn Sleeper>, sink: Arc<dyn StatusSink>) -> Self;
    pub async fn chat(&self, p: &ResolvedProvider, c: ChatCall) -> Result<ChatReply, AiError>;
    pub fn cache_key(p: &ResolvedProvider, c: &ChatCall) -> String;       // 64 hex
    pub async fn list_models(&self, p: &ResolvedProvider) -> Result<usize, AiError>;
}
pub fn pick_local_model(tags: &serde_json::Value) -> Option<String>;
```

`AiError` (serialised to the UI as `{kind, message}`), this module's variants and messages:

| Variant | Message |
|---|---|
| `NotConfigured` | "Choose an AI provider in Settings → Providers." |
| `OwnKeyRefused {code}` | "Your key was refused (401). Check it in Settings → Providers." |
| `ModelUnknown {model}` | "Your provider does not know the model “example-chat” (404). Check the model name in Settings." |
| `RateLimited {message}` | own key: "Your provider says you are sending too many requests (429). Wait a moment and try again." |
| `ProviderError {code}` | "Your provider had a problem (502). Try again later." |
| `Timeout {who, seconds}` | "No answer from your own key's server after 30 seconds. Try again." / "No answer from the local model after 120 seconds. Try again." |
| `Offline` | "No connection. Rewrites need a connection or a local model; checking still works." |
| `LocalNotRunning` | "Ollama isn't running. Start it, or choose another provider." |
| `LocalNoModel` | "Ollama has no chat model installed. Install one in Ollama, or choose another provider." |
| `LocalNotLocal` | "A local model must run on this computer (localhost)." |
| `InvalidAnswer` | "The AI answer could not be used." |

## Out of scope

- Publik resolution, provisioning, status headers and its errors: module 03.
- Prompts and answer validation beyond "is there a `content` string": module 04.
- The Settings UI: module 06.

## Design notes

- Cache input canonicalisation is the same function as the fake transport's (module 00).
- Windows file permissions: the per-user `%APPDATA%` ACL is relied on; no DPAPI in v1 (the Publik
  credential is plain JSON by contract, and the two should match).
- Further decisions are filled in here during implementation.

## Acceptance tests

Code tests in `crates/redpen-core/tests/02_provider_layer.rs`, all over `FakeTransport::scripted`,
`FakeClock`, `FakeSleeper` and an in-memory SQLite. Contract tests 10, 11, 12 and 16 start here and
are completed in module 03.

1. Own key 200 with `{"choices":[{"message":{"content":"hello"}}]}` → `ChatReply.content ==
   "hello"`; the request carries `Authorization: Bearer` with the key, `model` from settings, and
   the exact `max_tokens` and `temperature` given.
2. A second identical call → `from_cache: true`, no second request; the same call with a different
   provider kind → a request (caches are per provider); `prompt_version` 2 → a request.
3. `CachePolicy::Memory` → no `ai_cache` row; `Off` → a request every time;
   `remember_answers_days: 0` → `Disk` writes no row.
4. A row created 8 days ago (fake clock) is deleted by the start-up sweep with the 7-day setting;
   one created 6 days ago is kept.
5. Own key timeout, timeout, 200 → success after sleeps of exactly 500 ms and 1,000 ms; three
   timeouts → `Timeout` with its message and three requests.
6. Own key 429 then 200 → success after one retry; 503 ×3 → `ProviderError {code: 503}` message.
7. Own key 401 → `OwnKeyRefused` with its message and one request (no retry); 404 → `ModelUnknown`.
8. A 200 whose body is `not json`, or lacks `choices` → `InvalidAnswer` with its message, nothing
   cached.
9. `TransportError::Offline` → `Offline` with its message.
10. A `ResolvedProvider` of kind `Publik` that times out → one request only, no sleep.
11. `pick_local_model` (largest `parameter_size` at or under 15B, embedding models dropped) on
    tags `llama3.2:3b` (3.2B), `qwen2.5:14b` (14.8B), `example-16b` (16.0B), `nomic-embed-text`
    (137M), `mixtral:8x22b` (141B) → `qwen2.5:14b`; on only `nomic-embed-text` → `None`; two
    models of 7.6B → the first by name.
12. Local provider: `/api/tags` connection refused → `LocalNotRunning`; empty tags →
    `LocalNoModel`; base URL `http://192.168.1.5:11434/v1` → `LocalNotLocal`, and no request is
    made to that host.
13. `list_models` on own key with a 14-entry `data` array → "Key works: 14 models available."
14. The fake key `sk-test-a` never appears in the `Display` or `Debug` of any `AiError`, in
    `ResolvedProvider`'s `Debug`, or in the output of a capturing `tracing` subscriber over tests
    1–10 (contract test 16, first half).
15. `secrets.json` is created with mode 0600 on Unix (test `#[cfg(unix)]`); saving then loading
    returns the key.
16. `settings.json` round trip keeps an unknown field `"future": {"x": 1}`; a file with
    `"version": 2` loads read-only and saving is refused with "Settings were written by a newer
    Redpen."
17. Three sent calls and one cache hit for `fix-grammar` → `usage_daily.calls == 3` for that day,
    provider and feature.
18. `sent_log` gets no rows while the log is off; with it on, one row per sent call with the user
    content; rows older than 7 days are deleted; `sent_log_clear` empties it.
19. Golden `tests/cases/02-provider-layer/fake-chat/`: `redpen chat --provider fake --recording
    recording.json --message "hi"` prints `{"content": "hello", "from_cache": false}`; the same with
    no matching entry exits 3 with the unrecorded-request message.

## Status

Planned
