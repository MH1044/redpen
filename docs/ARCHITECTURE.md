# Redpen architecture

Redpen is a tray app for Windows and Linux. A Rust core does the checking, rewriting, plagiarism
matching and provider calls; thin platform code captures and replaces text; a small React UI draws
the popup and Settings. This file is
rewritten at milestones, not patched.

## 1. Stack

Every licence is verified at the module that adds the dependency and recorded in `THIRD_PARTY.md`.

| Component | Choice | Licence (verify at) | Why |
|---|---|---|---|
| Shell | Tauri 2, tray, `tauri-plugin-global-shortcut` | MIT OR Apache-2.0 (00) | small resident app; one global hotkey |
| Core language | Rust, stable toolchain pinned in `rust-toolchain.toml` | MIT OR Apache-2.0 | one language for core and capture |
| UI | React 19, TypeScript, Vite, Vitest | MIT (00) | popup, settings |
| Checking | `harper-core` | Apache-2.0 (01) | fast, local, private; English |
| Text utilities | `unicode-segmentation`, `similar`, `regex`, `sha2` | MIT OR Apache-2.0; Apache-2.0 for `similar` (00, 01, 04) | UAX #29 words, Myers diff, redaction, hashes |
| HTTP | `reqwest` with rustls, `tokio` | MIT OR Apache-2.0; MIT (02) | one transport for every provider |
| Validation, language | `jsonschema`, `whatlang` | MIT (04) | every AI answer checked; language keeping |
| Storage | `rusqlite` with bundled SQLite | MIT; public domain (02) | answer cache, usage, sent log |
| Windows APIs | `windows` crate: UI Automation, SendInput, clipboard, tokens | MIT OR Apache-2.0 (05) | secure-field checks and the clipboard route |
| Linux | `x11rb`, `arboard`, an AT-SPI crate, a desktop-portal crate | MIT OR Apache-2.0 for the first two; the others verified at 07 | capture on Linux |
| Start with the OS | `tauri-plugin-autostart` | MIT OR Apache-2.0 (10) | opt-in autostart |
| Licence gate | `cargo-deny` | MIT OR Apache-2.0 (00) | fails on GPL, AGPL, SSPL, BUSL |

Clean-room: GSpot (AGPL-3.0) is never opened; its README and listing may be read. Harper
(Apache-2.0) may be studied.

## 2. Repository layout

```text
Cargo.toml                    workspace: crates/*, app/src-tauri
rust-toolchain.toml, deny.toml, .gitattributes, .gitignore
scripts/
  verify.sh, verify.ps1        the one check CI runs (frontend build first)
  publik-proof.sh, .ps1        maintainer only: one real Publik call (--models, --probe-json,
                              --search: one real web search, one cent)
  gates.sh, gates.ps1          eval-rewrite and eval-plagiarism (maintainer only, spend money),
                              check-speed
prompts/                      <id>.v<N>.md prompt files (docs/PUBLIK-INTEGRATION.md §6), loaded at build time
  schemas/                    <name>.v<N>.json response schemas
crates/
  redpen-core/                platform-free library
    src/check/                Harper wrapper, spans, categories, dialects
    src/rewrite/              actions, prompts render, parse, clean, guard, language keeping
    src/plagiarism/           sentences, phrase choice, selection, snippet matching, citations (09)
    src/{redact,diff,lang,words}.rs
    src/provider/             AiClient, providers, cache, errors, status sink, tier rule (TierContext),
                              search.rs: SearchProvider, the Brave client (09)
    src/net/                  HttpTransport, HttpRequest/Response, FakeTransport, recordings
    src/flow/                 rewrite, deliver, plagiarism (coordination over traits)
    src/platform/             Capture trait and its fake
    src/store/                settings.json, dictionary.json, secrets.json, redpen.db
    src/clock.rs              Clock and Sleeper traits
  redpen-os/                  Capture implementations: windows.rs, linux/ (no macOS build)
  redpen-cli/                 binary `redpen`; tests/golden.rs, tests/hygiene.rs
app/
  package.json, vite.config.ts, index.html
  src/windows/                Popup.tsx, Settings.tsx, Welcome.tsx, Consent.tsx, Plagiarism.tsx
  src/state/                  reducers (popup, settings), tested with Vitest
  src/ipc.ts                  typed wrappers for the Tauri commands in §5
  src-tauri/
    Cargo.toml, build.rs, tauri.conf.json, capabilities/, publik-app-token.txt
    src/lib.rs, main.rs       wiring: state, windows, tray, hotkeys, commands; `main` first runs
                              the Publik self-test when `PUBLIK_SELF_TEST` is set (module 03)
    src/publik.rs             the official reference module, adapted (`// redpen:` markers)
    src/commands/             one file per command group in §5
    src/bin/publik-proof.rs, redpen-gates.rs
    tests/03_publik.rs
tests/cases/NN-slug/<case>/   golden cases: cmd.json, input.*, recording.json, expected.json
docs/                         PRODUCT, ARCHITECTURE, PUBLIK-INTEGRATION, PUBLIK-CONTRACT, TESTING,
                              ROADMAP, HANDBOOK, modules/, adr/, releases/
```

## 3. Components and data flow

```text
 hotkey ──> app/src-tauri (wiring) ──> redpen-core::flow ──> popup events
                 │  publik.rs                 │
                 ▼                            ├── check (harper-core)
          ProviderResolver                    ├── rewrite (prompts, redact, parse, guard, diff)
                 │                            ├── plagiarism (phrases, snippet matching, citations)
                 │                            ├── provider::AiClient ──> HttpTransport
                 ▼                            ├── provider::SearchProvider ──┘ (reqwest | FakeTransport)
          {base_url, key, model}              ├── platform::Capture ──> redpen-os | fake
                                              └── store (JSON files, redpen.db)
```

**Wiring (`app/src-tauri`)** registers hotkeys, owns windows and the tray, and turns commands into
calls on `redpen-core::flow`. It holds no logic worth testing apart from `publik.rs`, which resolves
the Publik credential, provisions and reads the wallet (docs/PUBLIK-INTEGRATION.md §2).

**Flows (`redpen-core::flow`)** are the coordinating layer, written against traits so tests run them
end to end with fakes. **Rewrite**: hotkey → `foreground` and the refusal checks → modifier keys up
→ `read_selection` → Harper → popup; a button → redact → render the prompt → `AiClient::chat` →
parse, validate, restore, guard, diff → popup; Accept → `deliver` (focus the captured window, check
again, paste). **Plagiarism** (module 09): a Plagiarism check press → the plan (sentences,
phrases, the eight chosen) → `SearchProvider::search` per phrase (cache first) → snippet matching →
the report → popup; nothing else is requested.

The libraries under the flows: **check** wraps `harper-core` (char, byte and UTF-16 spans, four
categories, dialects); **rewrite** holds the action table, prompt loading and rendering, parsing,
cleaning, redaction, language keeping and the guard; **provider** is the one OpenAI-compatible client
(`chat`) with the cache, error mapping, own-key retries and the status sink that `publik.rs` fills
from `x-publik-*` headers; **platform** (`redpen-os`) implements `Capture` per OS.

## 4. Data model

Directories (Tauri's path resolver): **config** `%APPDATA%\Redpen\` on Windows,
`$XDG_CONFIG_HOME/redpen/` on Linux; **data** `%LOCALAPPDATA%\Redpen\`, `$XDG_DATA_HOME/redpen/`. The Publik credential is
where the contract puts it (`%LOCALAPPDATA%\publik\apps\redpen.json` and equivalents), managed only
by `publik.rs`.

| File | Where | Truth or cache |
|---|---|---|
| `settings.json` | config | source of truth |
| `dictionary.json` | config | source of truth |
| `secrets.json` (mode 0600) | config | source of truth for the own key and the own search key |
| `redpen.db` | data | `ai_cache` is a cache; `usage_daily` and `sent_log` are disposable history |
| `status.json` | data | cache of the Publik status (no key), rewritten after every call |
| `logs/redpen-YYYY-MM-DD.log` | data | seven days, no content (§8) |

`settings.json`, complete example (every field has this default unless noted):

```json
{
  "version": 1,
  "hotkeys": {"check": "Ctrl+Alt+R"},
  "check": {"dialect": "british", "disabled_categories": []},
  "providers": {"chat": null, "rewrite_quality": "standard", "search": null,
    "own_key": {"base_url": "https://api.example.com/v1", "chat_model": "example-chat"},
    "local": {"chat_base_url": "http://localhost:11434/v1", "chat_model": null}},
  "privacy": {"redaction": true, "remember_answers_days": 7, "sent_log": false,
              "blocklist": ["1password.exe", "bitwarden.exe", "keepass.exe", "keepassxc.exe",
                            "dashlane.exe", "enpass.exe", "nordpass.exe", "proton pass.exe"]},
  "capture": {"copy_timeout_ms": 400, "paste_restore_delay_ms": 250},
  "plagiarism": {"citation_style": "apa7"},
  "ui": {"start_with_os": false, "update_check": true, "paused": false, "welcomed": false}
}
```

`dialect` defaults from the OS locale (en-GB British, en-US American, en-AU Australian, en-CA
Canadian, anything else American). `chat` is `null` until the user chooses; values `publik` |
`own_key` | `local`. `search` is `null` until the user chooses; values `publik` | `own_key` (module
09). `citation_style` is `apa7` | `mla9` | `chicago18`. `own_key` fields are empty strings by default;
the example shows them filled.
Unknown fields are kept on rewrite; a newer `version` opens read-only with "Settings were written
by a newer Redpen."

`secrets.json`: `{"version": 1, "own_key": "<the user's key>", "search_key": "<the user's Brave
Search API key>"}` (either may be absent).
`dictionary.json` (module 08): `{"version": 1, "entries": [{"term": "Redpen", "added":
"2026-09-29"}, {"term": "Siobhán", "added": "2026-09-29"}]}`.
`status.json`: `{"provider": "publik", "claim_state": "anonymous", "balance_micros": 0,
"last_charge_micros": null, "last_request_id": null, "starter_remaining_micros": 0,
"week_used_micros": 0, "week_budget_micros": null, "week_resets_at": null, "needs_credit": false,
"disconnected": false, "updated_at_ms": 1790000000000}`.

`redpen.db` (SQLite, `PRAGMA user_version = 1`, WAL):

```sql
CREATE TABLE ai_cache (
  key            TEXT PRIMARY KEY,      -- 64 lowercase hex: SHA-256 of the canonical cache input
  provider       TEXT NOT NULL,         -- 'publik' | 'own_key' | 'local'
  model          TEXT NOT NULL,         -- tier name for publik; 'search' for search results
  prompt_id      TEXT NOT NULL,
  prompt_version INTEGER NOT NULL,
  response_json  TEXT NOT NULL,         -- the validated answer object or the parsed search
                                        -- results, never the request
  charge_micros  INTEGER,               -- x-publik-charge-micros; NULL off Publik
  created_at_ms  INTEGER NOT NULL
) STRICT;
CREATE INDEX ai_cache_created ON ai_cache(created_at_ms);

CREATE TABLE usage_daily (
  day           TEXT NOT NULL,          -- local date 'YYYY-MM-DD'
  provider      TEXT NOT NULL,
  feature       TEXT NOT NULL,          -- fix-grammar | clearer | shorter | more-formal | friendlier
                                        -- | custom | plagiarism (one call per search)
  calls         INTEGER NOT NULL DEFAULT 0,
  charge_micros INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, provider, feature)
) STRICT;

CREATE TABLE sent_log (                -- only while Settings → Privacy → "Keep a log" is on
  id        INTEGER PRIMARY KEY,
  at_ms     INTEGER NOT NULL,
  provider  TEXT NOT NULL,
  endpoint  TEXT NOT NULL,             -- '/chat/completions' | '/search'
  feature   TEXT NOT NULL,
  model     TEXT NOT NULL,
  sent_text TEXT NOT NULL,             -- exactly the user content sent, after redaction; the
                                        -- query for a search
  bytes     INTEGER NOT NULL
) STRICT;
CREATE INDEX sent_log_at ON sent_log(at_ms);
```

Rows in `ai_cache` older than `remember_answers_days` and in `sent_log` older than 7 days are
deleted at start and every hour; `remember_answers_days: 0` disables disk caching.

Cache key K: SHA-256 (hex) of the canonical JSON (keys sorted, no whitespace) of
`{"provider", "model", "prompt": id, "prompt_version", "input": {...}}`, with the input fields
listed per feature in docs/PUBLIK-INTEGRATION.md §5 (`variant` included where it applies). The
idempotency key is the first 32 hex of K, or of SHA-256(`<K>:<n>`) for the n-th Try again after an
answer failed validation (same section).

Test formats (golden cases, `recording.json`) are specified in docs/TESTING.md §1.

## 5. Interfaces

Tauri commands (`app/src/ipc.ts` wraps each; errors are serialised `{kind, message, ...}`).

| Command | Arguments | Result | Errors |
|---|---|---|---|
| `session_get` | `session_id` | `Session {id, text, findings, dialect, english}` | `SessionGone` |
| `apply_fix` | `session_id, finding_id, suggestion_index` | `Session` | `SessionGone`, `StaleFinding`, `NoSuchSuggestion` |
| `rewrite` | `session_id, action, instruction?, style?, variant?` | `RewriteOutcome {result_id, text, diff[], warnings[], accept_default, unchanged, provider_line, charge_micros?}` | `RewriteError` (module 04), `AiError` |
| `accept` | `session_id, result_id?` | `()` | `CaptureError` (§8) |
| `copy_result` | `session_id, result_id?` | `()` | `ClipboardBusy` |
| `undo_last_replace` | none | `"undone" \| "on_clipboard"` | `NothingToUndo` |
| `close_session` | `session_id` | `()` | none |
| `provider_status` | none | `ProviderStatus {chat, publik?: PublikStatus}` | none |
| `consent_info` | none | `{title, data_path, disclosure, disclosure_version}` | none |
| `publik_connect` | none | `ConnectOutcome {status, run_pending, cost_sentence, claim_url}` | `ProvisionFailed {message}`, `NoAppToken` |
| `publik_disconnect` | none | `()` | none (best effort) |
| `publik_refresh` | none | `PublikStatus` | `AiError` |
| `open_account_link` | `"claim" \| "add_credit" \| "top_up"` | `()` (opens the system browser) | `NoLink` |
| `set_providers` | `chat?, rewrite_quality?, search?` | `Settings` | `Invalid` |
| `own_key_save` / `own_key_clear` | `{base_url, key, chat_model}` / none | `()` | `InvalidUrl` |
| `provider_test` | `"own_key" \| "local" \| "search_key"` (the last from module 09) | `{ok, message}` | none |
| `local_models` | none | `[{name, parameter_size}]` | `LocalNotRunning` |
| `dictionary_list` / `dictionary_add` / `dictionary_remove` (module 08) | none / `term` / `term` | `[Entry]` | `Invalid` |
| `dictionary_import` / `dictionary_export` (module 08) | `text` / none | `{added, skipped}` / `text` | `Invalid` |
| `settings_get` / `settings_update` | none / `patch` | `Settings` | `HotkeyInUse {hotkey}`, `HotkeyInvalid`, `Invalid {field}` |
| `usage_month` | none | `{total_micros, calls, by_feature[]}` | none |
| `sent_log_list` / `sent_log_clear` / `cache_clear` | none | `[SentEntry]` / `()` / `()` | none |
| `app_pause` | `paused` | `()` | none |
| `plagiarism_check` (module 09) | `session_id` | `PlagiarismReport {sentences[], summary, provider_line, charge_micros?, stopped_after?, stop?}` | `PlagiarismError` (`TooFew`, `TooMany`), `NeedsSearchConsent`, `NeedsSearchProvider` |
| `search_consent_info` / `search_choose` (module 09) | none / `"publik" \| "own_key"` | `SearchConsentInfo {title, data_path, disclosure?, cost_line}` / `ConnectOutcome` or `()` | `ProvisionFailed`, `NoAppToken` |
| `plagiarism_retry_failed` / `copy_citation` (module 09) | `session_id` / `session_id, result_id, style` | `PlagiarismReport` / `()` | as above / `ClipboardBusy` |
| `search_key_save` / `search_key_clear` (module 09) | `{key}` / none | `()` | `Invalid` |

Events from core to UI: `popup://open {session_id}`, `status://publik {PublikStatus}`.

CLI `redpen` (golden tests and the maintainer's scripts; never provisions Publik):
`--version --json`; `chat --provider P [--recording R] --message S`; `check [--dialect D] [--disable C] FILE`; `fix-all FILE`;
`rewrite --action A [--dialect D] [--style S] [--instruction S] [--redact] --provider
fake|local|own-key|publik-env [--recording R] FILE`; `prompt --action A [--dialect D] [--style S]
[--instruction S] [--redact] FILE` (prints the request body exactly as sent, after redaction); `redact FILE`;
`diff OLD NEW`; `plagiarism (--plan-only | --provider fake|publik-env|own-key-env [--recording R])
[--style S] FILE` (module 09; `own-key-env` reads `REDPEN_SEARCH_KEY`).

## 6. Core algorithms

| Algorithm | Where | Reference |
|---|---|---|
| Linting | check | `harper-core` documentation, pinned version |
| Word tokens | words.rs | Unicode UAX #29 word boundaries |
| Word diff | diff.rs | Myers, "An O(ND) Difference Algorithm and Its Variations", 1986 (via `similar`) |
| Language identification | lang.rs | trigram profiles, Cavnar and Trenkle 1994 (via `whatlang`) |
| IBAN check | redact.rs | ISO 13616 with ISO 7064 mod 97-10 |
| Cache keys | provider | FIPS 180-4 (SHA-256) |
| Sentence boundaries | plagiarism | Unicode UAX #29 sentence boundaries |
| Snippet matching | plagiarism | word 3-gram overlap (shingling), Broder, "On the resemblance and containment of documents", 1997 |
| Citations | plagiarism | APA 7, MLA 9 and Chicago 18 web-page formats, checked against each style's own guidance at module 09 |

## 7. Invariants

Each has a test; the module that proves it is in brackets.

1. **Nothing leaves without an explicit action**: a rewrite button, Plagiarism check, a Test
   button, Connect. A scripted session asserts every request the fake transport saw [06, 09].
2. **Only what the data-path sentence says leaves** (docs/PUBLIK-INTEGRATION.md §10) [06].
3. **Capture never reads a secure field**: password fields, secure input, blocklisted processes and
   elevated windows are refused before any synthetic copy [05, 07].
4. **Nothing is typed into a secure field**: the same checks run again on the window in front
   before Accept pastes [05, 07].
5. **The clipboard is restored** after every capture and replace; Redpen's temporary contents are
   marked to stay out of clipboard history [05].
6. **No key in the webview**, logs, errors, the status object or anything a command returns [02, 03].
7. **Prompts are versioned files**; the rendered request is golden-tested; a change is a new version [04].
8. **Every AI answer is validated** before use; an invalid one is discarded, never half-applied [04].
9. **No automatic retry on Publik calls** [03].
10. **The core is platform-free** and never reads the clock, randomness, filesystem or network
    directly; capture and transport sit behind traits with fakes [00].
11. **The fake transport fails on any unrecorded request** [00].
12. **The local provider never contacts a host other than localhost** [02, 03].
13. **A plagiarism check sends only its phrases**: at most 8 searches, each a quoted window of at
    most 12 words that overlaps no redaction match; result pages are never requested [09].

## 8. Errors, logging and privacy

Every user-facing message is spelled out in the module that raises it and tested. The capture
errors, shared by capture and Accept:

| Error | Message |
|---|---|
| `PasswordField` | "Redpen doesn't read or type into password fields." |
| `Blocklisted {app}` | "Redpen is turned off for this app (Settings → Privacy)." |
| `Elevated` | "This window runs as administrator, so Windows doesn't let Redpen read or type into it." |
| `NothingCopied` | "Nothing was copied. Select some text first." |
| `KeysHeld` | "Release the shortcut keys, then try again." |
| `FocusChanged` | "The window you started in is no longer in front, so nothing was typed. Copy the text instead." |
| `WindowGone` | "The window you started in has closed. Copy the text instead." |

**Logging** (`tracing`, local file, seven days): event names, durations, sizes in bytes, error
kinds and HTTP status codes. Never document text, keys, balances, window titles, process names or
URLs with query strings. A test runs a full session with a capturing subscriber and searches the
output for the session's text, the fake key and the process name.

**Every network request the app can make** (mirrored in `SECURITY.md`):

1. Chat calls to the chosen provider: `POST {publik|own key|local}/chat/completions`.
2. Publik housekeeping: `POST /installs` (after consent), `POST /installs/revoke` (Disconnect),
   `GET /wallet` at start and when Settings opens, only while publik API is the chosen AI provider
   and a credential exists; never while own key or local is chosen. With `PUBLIK_SELF_TEST` set
   (the maintainer's built-app check, never in normal use), one `POST /installs` at start, then the
   app exits.
3. Own key Test: `GET {base_url}/models`. Local: `GET http://localhost:11434/api/tags`.
4. Web search (module 09), only when Plagiarism check or the search key's Test is pressed:
   `POST {publik}/search`, or `GET https://api.search.brave.com/res/v1/web/search` with the user's
   own Brave Search API key; at most 8 per check. Opening a result hands its URL to the system
   browser.
5. Update check (module 10): one `GET` of the public releases feed, off in development builds and
   switchable off.

Opening a claim or top-up link hands the URL to the system browser; the app itself sends nothing.

## 9. Decisions

Module 00 writes these ADRs in `docs/adr/`.

| ADR | Decision | Status | Rejected, and why |
|---|---|---|---|
| 0001 Stack | Tauri 2, Rust core with `harper-core`, React UI | Accepted | Electron (too heavy for a resident tray app); browser extension only (not system-wide) |
| 0002 Licence | MIT OR Apache-2.0 | Accepted | GPL (would block permissive reuse of the core) |
| 0003 Capture | the clipboard route with UI Automation checks; UIA TextPattern later | Accepted | UIA TextPattern only (many apps do not implement it); per-app plugins (not system-wide) |
| 0004 Prompts and the guard | versioned prompt files; minimal-edit guard for Fix grammar | Accepted | editing prompts in place (goldens would lie); no guard (models over-edit) |
| 0005 Publik module | reference `publik.rs` copied into `app/src-tauri/src/publik.rs`, reqwest transport, one AI client | Accepted | a separate `redpen-publik` crate (allowed by the contract; not needed with one consumer) |
| 0006 No streaming in v1 | answers are shown after validation | Accepted | streaming partial JSON (shows unvalidated text) |
| 0007 Name and slug | Redpen, slug `redpen` | Maintainer decides at module 03, before listing | none yet |
| 0008 Platforms | Windows and Linux; no macOS build while GSpot (AGPL-3.0, on Publik) covers macOS | Accepted | a macOS build (would remake GSpot) |
| 0009 Plagiarism from snippets | exact-phrase web search, at most 8 phrases, matched against result snippets on this computer; result pages never fetched | Accepted | fetching pages (a request per result, arbitrary HTML); scraping a search engine without a key (fragile, against its terms); an overall originality percentage (implies a precision the method lacks) |

## 10. Platforms

- **Windows 10 (1809) and 11, x64**: primary. WebView2 (preinstalled on 11; the installer
  bootstraps it on 10). CI `windows-latest`. Modules 05 and 06 target it first.
- **macOS**: not built (ADR-0008).
- **Linux X11** (module 07): clipboard route through XTest. **Wayland**: global shortcuts depend on
  the desktop portal and there is no synthetic input; Redpen puts the text on the clipboard and says
  "On Wayland, Redpen can't type into other apps. The text is on your clipboard: press Ctrl+V."
- **AltGr**: on many European layouts Ctrl+Alt+letter types a character (Polish Ctrl+Alt+C is ć,
  German Ctrl+Alt+E is €). The defaults avoid those letters; Settings rejects a hotkey that the
  active layout maps to a character, with "That shortcut types a character on your keyboard layout.
  Choose another."
