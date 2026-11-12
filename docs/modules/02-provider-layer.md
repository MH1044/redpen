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
| `Unrecorded {hash, path}` | the fake transport's message (docs/TESTING.md §1); only the fake transport produces it |

## Out of scope

- Publik resolution, provisioning, status headers and its errors: module 03.
- Prompts and answer validation beyond "is there a `content` string": module 04.
- The Settings UI: module 06.

## Design notes

- Cache input canonicalisation is the same function as the fake transport's (module 00).
- Windows file permissions: the per-user `%APPDATA%` ACL is relied on; no DPAPI in v1 (the Publik
  credential is plain JSON by contract, and the two should match).
- Decided before implementation (each dependency checked against `deny.toml` and built on Windows):
  - **Dependencies**: `reqwest` 0.13 with its default TLS (rustls, certificates checked against the
    operating system's store through `rustls-platform-verifier`), `rusqlite` 0.42 with bundled
    SQLite, `tokio`, and `tracing` for the log events of docs/ARCHITECTURE.md §8
    (`tracing-subscriber` in tests only, for the capturing subscriber of test 14).
  - **Files and invariant 10**: the stores never choose a path. The caller passes the config and data
    directories (the app from Tauri's path resolver, tests a temporary directory); `redpen chat`
    uses an in-memory database and no settings file. The `store` module is the only core code that
    touches the filesystem, and only under the directories it was given.
  - **The request body** of a chat call is exactly `{"model", "messages", "max_tokens",
    "temperature"}`, `messages` being `[{"role", "content"}]`. Numbers are written in their
    shortest form: a whole temperature is an integer (`0`) and `0.4` is `0.4`, never the `f32`
    widening `0.4000000059604645`, because recordings match on the body's canonical JSON.
  - **The fake provider**: kind `Fake`, base URL `http://fake.invalid/v1`, chat model `fake-chat`,
    no key. `redpen chat --provider fake --recording R --message S` sends
    `{"model": "fake-chat", "messages": [{"role": "user", "content": S}], "max_tokens": 256,
    "temperature": 0}` to `/chat/completions` with cache policy `Off` and an in-memory database,
    and prints `{"content": "...", "from_cache": false}` on one line. `--provider local` and
    `own-key` send the same body with their own model. An `AiError` prints
    `{"error": {"kind": "<variant>", "message": "<message>"}}` and exits 3 (an unrecorded request
    gives kind `Unrecorded` and the transport's message); usage errors exit 2, as in module 01.
  - **Cache key K**: SHA-256 (64 hex) of module 00's `canonical_json` of `{"provider", "model",
    "prompt": prompt_id, "prompt_version", "input": cache_input}`, `provider` being `publik`,
    `own_key`, `local` or `fake` (the same names as `ai_cache.provider`).
  - **The local day** of `usage_daily`: `Clock` gains `utc_offset_minutes()`; `SystemClock` reads
    the operating system's offset and `FakeClock` returns 0 unless set, so tests see UTC dates.
  - **Timeouts** (docs/PUBLIK-INTEGRATION.md §6): connect 5 s; a chat call 30 s on own key and
    Publik, 120 s on local, carried in `ChatCall.timeout`.
  - **Error mapping**: 401 and 403 → `OwnKeyRefused {code}`; 404 → `ModelUnknown`; 429 →
    `RateLimited`; 5xx and other 4xx → `ProviderError {code}`; `TransportError::Timeout` →
    `Timeout`; `Offline` and `Tls` → `Offline`; `Unrecorded` → `Unrecorded`. On the local
    provider, a refused connection to `/api/tags` → `LocalNotRunning`; `/api/tags` is on the
    base URL's scheme, host and port. `AiError` serialises as `{"kind", "message"}` plus the
    variant's fields, and its `Display` is the message.
- Decided during implementation:
  - **The fake transport answers in turn and can fail.** Entries for the same request answer it
    in order, the last one repeating, so a script can say "timeout, timeout, 200". An entry with
    status 0 and the body `{"transport_error": "timeout" | "offline" | "tls"}` makes the transport
    fail with that `TransportError` instead of answering; `FakeEntry::reply` and
    `FakeEntry::failing` build entries from a request. Such entries are for code tests only: the
    recordings check refuses that body in a `recording.json`.
  - **Privacy settings in the client.** `AiClient::new` starts with answers remembered for 7 days
    and the sent log off, and runs the start-up sweep. `set_privacy` (with
    `PrivacyOptions::from(&PrivacySettings)`), `set_remember_answers_days` and `set_sent_log`
    follow Settings → Privacy and sweep at once. `remember_answers_days` is clamped to 7
    (docs/PUBLIK-INTEGRATION.md §5: at most 7 days). `keep_sweeping` waits an hour through the
    injected `Sleeper`, sweeps, and repeats for as long as the app runs.
  - **Sweeps and expiry.** A row goes when it is strictly older than the limit
    (`created_at_ms < now − days`); between sweeps a lookup ignores rows past the limit, so an
    expired answer is never served. 0 days deletes every `ai_cache` row on the next sweep.
  - **The cache.** `ai_cache.response_json` is `{"content": "…"}`. `Disk` reads `ai_cache`, then
    the memory map (answers kept while the setting was 0); `Memory` reads and writes only the map;
    `Off` neither. A cache hit has no charge and no request id. `cache_clear` empties both.
  - **Usage.** A call counts once when it is sent, however many attempts it takes and whatever
    the outcome; never when the cache answers it or when it is refused before sending
    (`NotConfigured`, `LocalNotLocal`, no Ollama model). Every chat response, each attempt and
    any status, goes through the status sink; a positive charge it returns is added to the day's
    row, and `ChatReply.charge_micros` is the last response's. The day is taken when the call
    starts. `ChatReply.request_id` is the response's `x-publik-request-id`, else `x-request-id`.
  - **The sent log.** One row per sent call, written before the request goes out and only while
    the log is on: `sent_text` is the content of the user messages, joined by a blank line;
    `bytes` is the size of the request body as sent (compact JSON).
  - **An own key needs its parts.** No key (or an empty one), no `http`/`https` base URL, or no
    chat model gives `NotConfigured` and sends nothing. Publik and the fake provider are not
    checked here; module 03 resolves publik API. `ResolvedProvider::from_own_key_settings` builds
    the own key from `settings.json` and `secrets.json` (`NotConfigured` when a part is empty),
    and `from_local_settings` the local provider from `settings.json`.
  - **The local provider.** The localhost check comes before any request, for chat, the model
    list and Ollama's tags alike. With no chat model, Ollama is asked (`GET /api/tags`) and
    `pick_local_model` chooses before the cache lookup, so a cache key always names a real model.
    No connection (or a TLS failure) on a local call gives `LocalNotRunning`, and so does a 404
    from `/api/tags` (something answers, but not Ollama); a timeout gives `Timeout`. The `embed`
    rule ignores case; a model whose `parameter_size` cannot be read is skipped; sizes are read
    in integer arithmetic (`K`, `M`, `B`, `T`), so `14.8B` is exactly 14,800,000,000.
  - **Reading a URL's host** (`net::url_host`): `http` and `https` only; user information
    (`user@host`) or a port that is not a number gives no host; a backslash ends the host as a
    slash does, as browsers read it. Only exactly `localhost`, `127.0.0.1` or `::1` is local, so
    shorthand (`127.1`) and percent-encoded hosts are refused, the safe direction.
  - **The production transport** follows no redirect, so a local server cannot pass a call on and
    a key is never replayed to another host; requests to this computer bypass any proxy, others
    use the system's proxy settings. The key's header value is marked sensitive. reqwest's error
    messages name the URL, so errors are reduced to their kind: a failed connection is `Offline`
    (`Tls` when the handshake or certificate failed), an expired timeout `Timeout`. A header
    value that cannot be sent (a key with a line break) is `Offline`.
  - **Error details.** `GET /models` answering 404 is `ProviderError {code: 404}` (the address is
    wrong, not the model). `RateLimited.message` is the message shown. `Timeout.who` is the
    provider kind, serialised by its database name; the message names publik API "publik API" and
    the fake provider "the fake provider" until module 03 maps Publik's own errors.
  - **The Test button** (`AiClient::provider_test`): own key "Key works: 14 models available."
    ("1 model available." for one); local "Ollama is running. Redpen will use qwen2.5:14b." (the
    chosen model, or Ollama's pick); a failure gives `ok: false` and the error's message.
  - **`settings.json`.** `Settings::load(config_dir)` gives the defaults (American English) when
    there is no file; `Settings::load_for_locale` takes the operating system's locale for that
    case. `save(config_dir)` writes the settings over the file as it is on disk at that moment, so
    unknown fields at any depth are kept even by settings built from defaults, and refuses with
    "Settings were written by a newer Redpen." when the loaded settings or the file on disk carry
    a newer `version`, leaving the file untouched. Keys are written in sorted order. A file that
    is not JSON, or a known field of the wrong type in a version-1 file, is
    `SettingsError::Invalid`; a newer file that does not parse loads read-only with the defaults.
    The messages: "settings.json is not valid: " and the reason (serde_json's, or "the file is
    not a JSON object"); an I/O error is `SettingsError::Io`, "settings.json could not be read or
    written: " and the operating system's error kind (such as "permission denied"), never a path,
    which would name the user's home folder. `RewriteQuality`, `ChatChoice`, `SearchChoice` and
    `CitationStyle` live in `store::settings`; module 03's tier rule can reuse `RewriteQuality`.
  - **`secrets.json`.** No file means no keys, and an empty string counts as no key. A file that
    does not parse, is not an object or holds a key that is not a string is
    `SecretsError::Invalid`, with one fixed message, "secrets.json is damaged. Save your key again
    in Settings → Providers.", because serde_json's messages can quote a value. An I/O error is
    `SecretsError::Io`, "secrets.json could not be read or written: " and the error's kind only,
    never a path. On Unix the temporary file is set to 0600 before the rename, also when a stale
    temporary file is left from an earlier crash.
  - **`redpen.db`.** `Db::open(data_dir)` creates the folder and uses WAL; `Db::open_in_memory`
    serves tests and `redpen chat`; a `user_version` above 1 is refused. Clones of a `Db` share one
    connection. A database failure during a call is logged and never fails the call.
  - **The operating system's offset.** `SystemClock::utc_offset_minutes` asks SQLite's
    `localtime` modifier, which asks the operating system (`localtime_s` on Windows,
    `localtime_r` on Linux) for the current instant, daylight saving included, so no time zone
    database is shipped. `store::local_day` turns the instant and the offset into `YYYY-MM-DD`
    with integer days-to-civil arithmetic.
  - **`redpen chat`** lives in `crates/redpen-cli/src/cmd/chat.rs`, beside the other commands,
    and `--provider` in `providers.rs`. The call is feature `Test`, prompt id `chat` version 1,
    cache input `{"message": S}`, cache `Off`, timeout 30 s (120 s on local), over the system
    clock and real sleeps. An `AiError` prints its full serialised form (`kind`, `message` and
    the variant's fields, `hash` and `path` for `Unrecorded`) under `"error"`, as module 01
    prints a refusal, repeats the message on standard error, and exits 3. `--provider own-key`
    without its three variables is a usage error (exit 2). With `--recording`, any provider
    answers from the file; `fake` without one answers nothing; `local` and `own-key` without one
    use the network.
  - **Log events**: "provider answered" and "provider request failed" (provider, endpoint path,
    attempt, status or error kind, bytes, milliseconds), "chat answered from the cache", "chat
    failed" (the error's kind), "redpen.db swept" (row counts). Never text, a key, an answer or a
    URL.
  - **Dependencies.** `Cargo.lock` grew from 785 to 823 packages, every one passing `deny.toml`;
    `aws-lc-rs` (reqwest's default rustls provider) builds on Windows with the C compiler alone.

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

Done (2026-11-12)
