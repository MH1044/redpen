# 00: Scaffold

## Goal

A fresh clone builds and `verify` passes on Windows and Linux. The tray app starts and quits. The
golden harness, the fake transport that fails on unrecorded requests, the injected clock, the
hygiene test, CI and the documents exist, so module 01 can start with the checks already
enforced.

## Depends on

Nothing.

## Scope

- Cargo workspace: `crates/redpen-core` (lib), `crates/redpen-os` (lib, empty), `crates/redpen-cli`
  (binary `redpen`), `app/src-tauri` (package `redpen-app`). `rust-toolchain.toml` pins stable.
- React app in `app/`: Vite, TypeScript `strict`, ESLint, Prettier, Vitest, one component test.
- `redpen --version --json` prints `{"name":"redpen","version":"0.1.0"}`.
- `scripts/verify.sh` and `scripts/verify.ps1` run the steps of docs/TESTING.md §1 in order and stop
  at the first failure; the frontend builds before any `cargo` step.
- `deny.toml`: licences allowed are MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception,
  BSD-2-Clause, BSD-3-Clause, ISC, Zlib, MPL-2.0, Unicode-3.0, Unicode-DFS-2016, BSL-1.0, CC0-1.0,
  Unlicense; everything else fails.
- The hygiene test (docs/TESTING.md §5) in `crates/redpen-cli/tests/hygiene.rs`, run by `verify`
  and CI; `.gitignore` (template plus a Rust/Node block); `.gitattributes` (LF, binaries marked).
- Golden harness `crates/redpen-cli/tests/golden.rs`: every `tests/cases/*/*/cmd.json`, run with
  the case directory as working directory, stdout parsed as JSON and compared with `expected.json`;
  `exit` checked; `GOLDEN_FILTER` (prefix match on `NN-slug/case`) and `UPDATE_GOLDEN=1` (writes
  only a missing `expected.json`, or one named by the filter).
- `redpen_core::net`: `HttpTransport`, `HttpRequest`, `HttpResponse`, `Body`, `TransportError`,
  canonical request hashing, and `FakeTransport` reading `recording.json`
  (docs/TESTING.md §1), keeping every request it saw.
- `redpen_core::clock`: `Clock` and `Sleeper` traits with `SystemClock`, `TokioSleeper`,
  `FakeClock` and `FakeSleeper`.
- `crates/redpen-cli/tests/recordings.rs`: the recordings check of docs/TESTING.md §5 (only
  `x-publik-charge-micros` among `x-publik-*` headers, a synthetic value on recorded entries, body
  keys `choices` for chat, `results` and `charge_micros` for a publik search, or `web` for a Brave
  search); the recorder behind `UPDATE_RECORDINGS=1` writes entries that pass it.
- `redpen-app`: a tray icon with **Quit**, no window shown; `--smoke` exits 0 after the hidden UI
  window reports `ready`.
- CI `.github/workflows/ci.yml` (`verify` on `windows-latest` and `ubuntu-latest`, Ubuntu with
  Tauri's Linux prerequisites) and `release.yml`, a stub that fails without
  `docs/releases/<version>.md`.
- Documents: README, CONTRIBUTING (with the publik block of docs/PUBLIK-INTEGRATION.md §12),
  SECURITY (the network list of docs/ARCHITECTURE.md §8), RELEASING, THIRD_PARTY, LICENSE-MIT and
  LICENSE-APACHE, docs/HANDBOOK.md stub, and ADRs 0001 to 0009 as listed in
  docs/ARCHITECTURE.md §9.

## Files

- `Cargo.toml`, `rust-toolchain.toml`, `deny.toml`, `.gitignore`, `.gitattributes`
- `crates/redpen-core/src/{lib.rs, net/mod.rs, net/fake.rs, net/hash.rs, clock.rs}`
- `crates/redpen-cli/src/main.rs`, `crates/redpen-cli/tests/{golden.rs, hygiene.rs, recordings.rs}`
- `app/` (Vite project), `app/src-tauri/{Cargo.toml, build.rs, tauri.conf.json, src/main.rs, src/lib.rs}`
- `scripts/{verify.sh, verify.ps1}`
- `.github/workflows/{ci.yml, release.yml}`
- `tests/cases/00-scaffold/version/{cmd.json, expected.json}`
- the documents above, `docs/adr/{README.md, template.md, 0001-…md … 0009-…md}`

## Interfaces

```rust
#[async_trait::async_trait]
pub trait HttpTransport: Send + Sync {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, TransportError>;
}
pub struct HttpRequest { pub method: Method, pub url: String, pub headers: Vec<(String, HeaderValue)>,
                         pub body: Body, pub timeout: std::time::Duration }
pub enum Body { Empty, Json(serde_json::Value) }
pub struct HttpResponse { pub status: u16, pub headers: Vec<(String, String)>, pub body: Vec<u8> }
pub enum TransportError { Timeout, Offline, Tls(String), Unrecorded { hash: String, path: String } }
pub enum HeaderValue { Plain(String), Secret(secrecy::SecretString) } // Debug prints "***" for Secret
pub fn request_hash(req: &HttpRequest) -> String;                     // 64 lowercase hex
impl FakeTransport { pub fn from_file(path: &Path) -> Result<Self, FakeError>;
                     pub fn scripted(entries: Vec<FakeEntry>) -> Self;
                     pub fn requests_seen(&self) -> Vec<HttpRequest>; }
pub trait Clock: Send + Sync { fn now_ms(&self) -> i64; }
#[async_trait::async_trait] pub trait Sleeper: Send + Sync { async fn sleep(&self, d: Duration); }
```

`request_hash`: SHA-256 of `METHOD + " " + path + "\n" + canonical`, where `path` is the URL path
after the provider's base path (`/api/v1` for Publik, `/v1` for OpenAI-compatible servers,
`/res/v1` for Brave Search) followed by the query string, if any, with its parameters sorted by
name and percent-encoded as sent, and `canonical` is the JSON body with keys sorted, no whitespace,
UTF-8 (not `\u`-escaped), or empty for a request without a body. Headers never count.

## Out of scope

- Any product code (checking, providers, capture): modules 01 onwards.
- Publik: module 03. The token file does not exist yet.

## Design notes

- Tauri needs `frontendDist` to exist before `cargo` compiles `redpen-app`: `verify` builds the
  frontend first.
- The fake transport is the only HTTP in tests, for the whole project.
- Decided during implementation:
  - The toolchain is pinned to the exact stable release (`1.98.1`, with rustfmt and clippy as
    components), so CI and a fresh clone compile with the same compiler and the same lints.
  - `request_path` takes the path after the last `v1` segment, which covers `/api/v1`, `/v1`
    and `/res/v1` alike, and the whole path when there is none (Ollama's `/api/tags`); query
    parameters are sorted by name with a stable sort, so repeated names keep their order.
  - `canonical_json` sorts object keys itself at every level rather than relying on serde_json's
    default map, because a `preserve_order` feature switched on by any crate in the workspace
    would silently change every hash. Module 02's cache key uses the same function.
  - A recording's `headers` is a JSON object in the file and an ordered list in memory; the fake
    answers with names lower-cased.
  - The recorder (`net::recording::Recorder`) wraps any transport and keeps a scrubbed entry per
    exchange; it lives in `crates/redpen-core/src/net/recording.rs` with the recording types and
    the validation that `crates/redpen-cli/tests/recordings.rs` runs, so the two can never
    disagree.
  - `TransportError::Unrecorded { hash, path }` carries the request path; the body file goes
    under `target/unrecorded/` of the workspace (`CARGO_TARGET_DIR` when set), found by walking
    up from the current directory, so a crate test and the `redpen` binary run from a case
    directory write to the same folder. `net::fake::unrecorded_path(hash)` names the file.
  - The hygiene test's secret shapes include the `pat_<slug>_…` app-token shape, with
    `app/src-tauri/publik-app-token.txt` as the one exempt file, so "the one allowed token" is
    enforced rather than stated. The file floor is 41, half the count at module 00.
  - The golden harness registers one test per case (libtest-mimic, `harness = false`); `exit`
    defaults to 0, and stdout is compared as JSON whatever the exit code.
  - The hidden window is declared in `tauri.conf.json` (`visible: false`); the tray reuses the
    icon embedded at build time from `bundle.icon`; `--smoke` exits from the `ui_ready`
    command, with a 20 s watchdog thread that exits 1; closing a window never ends the app
    (a tray app), only Quit or the smoke does. Tauri 2.12 with the `tray-icon` feature,
    tauri-build 2.7, resolved by `Cargo.lock`.
  - The icons are placeholders written by `scripts/make-icons.mjs` (Node's standard library
    only), because Tauri's build needs an `.ico` on Windows and a `.png` on Linux at compile
    time; the packaging module replaces them. Icon file names carry no `@`, which the
    hygiene email rule would read as an address.
  - The launch smoke is a CI step after `verify` on both runners (docs/TESTING.md §1 and §6):
    on Ubuntu under `dbus-run-session` and `xvfb-run`, with WebKit's DMA-BUF renderer and
    compositing mode disabled because the hosted runner has no GPU.
  - This plan has no fixture generator: the corpora are hand-written and large data is generated
    inside the test that uses it (docs/TESTING.md §2). The only generated expected file is the one
    `UPDATE_GOLDEN=1` writes, as pretty-printed JSON with LF line endings on both platforms.
  - The version 0.1.0 lives in `Cargo.toml`, `app/src-tauri/tauri.conf.json` and
    `app/package.json` (RELEASING.md). `app/src-tauri/gen/schemas/` is generated by the build
    and ignored. The app identifier `org.redpen.app` is a placeholder (ADR-0001).
  - `scripts/verify.sh` runs `npm ci` only when `app/node_modules` is missing or older than
    the lockfile; CI always starts without it, so CI runs every step.

## Acceptance tests

Golden cases in `tests/cases/00-scaffold/`; code tests in `crates/redpen-core/tests/00_scaffold.rs`,
`crates/redpen-cli/tests/hygiene.rs`, `app/src/00-scaffold.smoke.test.tsx`.

1. `redpen --version --json` → stdout `{"name":"redpen","version":"0.1.0"}`, exit 0 (golden
   `version`).
2. `request_hash` of `POST https://publikhq.com/api/v1/chat/completions` with body
   `{"b":1,"a":2}` → `82cab4cdacc60d06a2ccbd424c8fb469aa308bfc7df538fd03a95348e2eeef93`; with body
   `{"model":"publik-fast","messages":[{"role":"user","content":"hi"}]}` →
   `22f53c07ac6b84e42b6fea37d9f1ce05a6c42780307b5932ccf2031f7ef66dd1` (reference: `sha256sum` of the
   canonical string, recorded when this spec was written).
3. The same request with keys reordered, with extra whitespace in the source JSON, or with a
   different `Authorization` header → the same hash; changing `"a":2` to `"a":3` → a different hash.
4. `FakeTransport::from_file` with one entry answers a matching request with that entry's status,
   headers and body.
5. An unrecorded request → `TransportError::Unrecorded`, the message "Unrecorded request <hash> to
   /chat/completions; body written to target/unrecorded/<hash>.json. Add a hand-written entry, or
   ask the maintainer to record it.", and that file exists.
6. `requests_seen()` returns three sent requests in order.
7. `format(HeaderValue::Secret)` with `{:?}` prints `***`, never the value.
8. `FakeClock` returns the set time and advances by a given amount; `FakeSleeper` records a 500 ms
   sleep and returns without waiting.
9. The hygiene test passes on the clean tree and finds more than N files; a home path, an email
   at a real-looking domain, a 40-character fake `sk-` key and a control character in an
   untracked temp file give exactly four failures; the file is then deleted.
10. A test reads `deny.toml` and asserts that none of GPL-2.0, GPL-3.0, AGPL-3.0, LGPL-3.0,
    SSPL-1.0 or BUSL-1.1 is allowed; `cargo deny check licenses` passes.
11. `redpen-app --smoke` exits 0 within 20 s (CI `windows-latest`; Ubuntu under `xvfb-run`).
12. The Vitest smoke test renders the placeholder app root without errors.
13. A test asserts that every document listed in Scope exists, that `docs/adr/` holds ADRs 0001 to
    0009 each with a `Status:` line, and that `CONTRIBUTING.md` contains the `## publik API` block.
14. The recordings check passes on the tree; a planted untracked `recording.json` with
    `"x-publik-balance": "3120000"` fails with "Recordings keep only x-publik-charge-micros with a
    synthetic value; remove x-publik-balance from <file>."; one with a body key `id` fails; the
    recorder, run over the fake transport with a response carrying `x-publik-balance`,
    `x-publik-request-id`, `x-publik-week-used`, `id` and `usage`, writes an entry with only
    `x-publik-charge-micros: "100"` and `choices`.

## Status

Done (2026-10-27)
