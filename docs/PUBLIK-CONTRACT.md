# Publik API: the contract every app here implements

This is the general reference. Each project's `docs/PUBLIK-INTEGRATION.md` says which of these
pieces it uses, for which features, with which prompts and at what cost. Every repository that
uses Publik keeps an unchanged copy of this file as `docs/PUBLIK-CONTRACT.md`.

Sources, read on 2026-09-29 and re-checked on 2026-10-16 and 2026-10-26, in order of authority:

1. `https://publikhq.com/developers`, `https://publikhq.com/pricing`, `https://publikhq.com/llms.txt`
2. `https://publikhq.com/skills/setup.md` (list, wire and prove an app end to end), and the skills
   it drives: `https://publikhq.com/skills/publish.md`, `https://publikhq.com/skills/api.md` and,
   for an app that searches the web, `https://publikhq.com/skills/search.md`
3. The official reference modules: `https://publikhq.com/sdk/publik.rs.md` (Rust, Tauri),
   `publik.ts.md` (Node, Electron main), `publik.py.md` (Python), `publik.swift.md` (macOS)
4. Behaviour observed in a production Publik app (marked **observed** below). Its source is
   AGPL-3.0: do not open it while writing a permissively licensed module; this file carries
   everything needed.

**Before implementing any Publik module, fetch 1 to 3 again with `curl` (raw, not a
summarising fetch; the site asks for that) and record in the spec's Design notes anything that
changed.** Publik changes fast: between two readings a week apart the starter balance went from
$0.25 to $0.00, and in the following three weeks the first-run button, the built-app self-test, the
setup skill and the plan refill period all changed, and ten days later web search arrived as a
sixth product.

## 1. What Publik gives an app

- **One gateway**: `https://publikhq.com/api/v1`. Dialects: OpenAI Chat Completions
  (`POST /chat/completions`, streaming), OpenAI Responses (`POST /responses`), Anthropic Messages
  (`POST /messages`, translated). Also `POST /embeddings`, `POST /audio/transcriptions` (files
  under 4 MB), `POST /audio/speech`, `POST /images/generations`, `POST /search` (web search, section
  6a), `GET /models`, `GET /wallet`.
- **Products**, granted to the app token when it is minted: `ai.chat`, `ai.embeddings`,
  `ai.transcribe`, `ai.speech`, `ai.image`, `search`. Wire only the products the token lists.
- **Tiers, never vendor model names**:

  | Tier | Use for | Typical use costs | Notes |
  |---|---|---|---|
  | `publik-fast` | cleanup, classification, short judgements | $0.001 | cheapest |
  | `publik-balanced` | the default (`publik-default` is a synonym) | $0.01 | |
  | `publik-smart` | the hardest reasoning | $0.018 | **only for a computer linked to a Publik account**; fall back to `publik-balanced` otherwise |

  A "typical use" is about 1,500 words sent and 375 back; smaller calls cost proportionally
  less. Each tier also answers to vendor aliases, but code always sends the tier name.
- **Money**: chat is billed at Publik's published tier price; the developer gets **half of the
  markup on their app's chat calls**. Embeddings, transcription, speech and images pass through at
  the provider's price with **no markup and no developer share**. Web search costs **one cent a
  search**, whatever the result count; the developers page names no developer share for it.
- **Balances**: a new install starts at **$0.00**. Linking the computer to a Publik account gives
  **$0.05 of free use, once**, which expires after 30 days. Users then buy a plan (Basic $8 a month,
  about 800 typical uses; Pro $20 a month, about 2,000; each refills monthly, all of it available from
  the first day of the month, and unused plan balance does not carry over) or a pack ($20, $50, $100,
  tops the balance up once and never expires).
  **Design consequence: the very first AI action of a fresh install is likely to return 402.** Every
  app must make linking the computer, the user's own key, and the local model equally visible, and
  must be fully useful before any of them is set up.
- **Privacy**: requests go through Publik's servers to a shared model account; Publik does not
  train on them and keeps no request bodies beyond a 24-hour idempotency fingerprint; the model
  provider's abuse-monitoring retention applies.
- **Listing rule**: an app that needs a service Publik covers (chat, embeddings, transcription,
  speech, images, web search) must run on publik API to be listed; its own-key path stays as the
  fallback.
- **Every app that uses Publik must also offer the user's own key.** In this folder, every app also
  offers a local model where one exists (Ollama for chat and embeddings, whisper.cpp for
  transcription, OS voices for speech). Web search has no local equivalent: the user's own search
  key (for example a Brave Search API key) comes first, then publik API. Apps here never scrape a
  search engine's result pages as a keyless fallback.

## 2. The maintainer's one-time step: list, wire and prove

The app token (`pat_…`) only mints capped keys attributed to one app. Every user builds from
source or installs a release, so it is **public by design and committed** as
`publik-app-token.txt`. For Rust, it sits next to the `Cargo.toml` of the crate that contains
`publik.rs` (the reference compiles it in with `include_str!("../publik-app-token.txt")` from
`src/`): `app/src-tauri/` by default, or `crates/<app>-publik/` if the module lives in its own
crate. For Node and Python it sits next to the module or in any parent folder up to the repository
root (the modules search upwards). `PUBLIK_APP_TOKEN` in the environment overrides it (at build time
for Rust, at run time everywhere).

Getting it needs the maintainer's approval in a browser, so it is never automated. Each app's
Publik module is completed and tested against a fake server; only the live path needs the token.

Once the Publik module is done and a release build works, the maintainer runs Publik's setup skill
once, in an interactive session in the repository, with `https://publikhq.com/submit` open:

1. It reads the repository, derives the listing (`skills/publish.md` steps 1 and 2) and shows one
   checklist: the listing fields, the products this app needs, and the built-app test. Nothing
   changes before the maintainer says yes.
2. One browser approval covers the rest. It lists the app (a listing can install from source before
   there is any release; new releases update it within a day).
3. It mints the app token for the slug (`skills/api.md` step 2: `POST /apps/<slug>/token`) and
   writes the token file without printing it. A `403 claim_first` means the listing must first be
   claimed on Publik ("Claim it with GitHub" on the listing page).
4. It builds the release artifact and runs it once with `PUBLIK_SELF_TEST=1` (section 3). The line
   `publik self-test: ok <slug> <version>` is the proof Publik counts.

The module and the UI already exist, so the skill only adds what is missing. Nothing is listed or
published without the maintainer's yes.

## 3. The module: copy the reference, do not reinvent it

Copy the reference module for the stack **verbatim**, then change only `APP_SLUG`, `APP_VERSION`
and the items below. Keeping it close to the reference makes Publik's future changes easy to merge.

| Stack | Reference | Goes in |
|---|---|---|
| Tauri (Rust) | `publik.rs.md` | `app/src-tauri/src/publik.rs` (or a `crates/<app>-publik` crate) |
| Electron main / Node | `publik.ts.md` | the main process only, never the renderer |
| Python sidecar | `publik.py.md` | `engine/<pkg>/publik.py` |

Adaptations every app here makes, each recorded in its Design notes:

1. **The key never reaches a webview, renderer or browser bundle.** The UI asks the core (Tauri
   command, IPC or JSON-RPC) to make AI calls; it only ever sees the status object.
2. **`device_name`**: the TypeScript and Python modules send the hostname, which often contains a
   person's name. Omit it (as the Rust module does) or send a generic label (`Windows PC`, `Mac`,
   `Linux PC`, `Android phone`); send the hostname only if the user opts in on the setup screen. Name the field in `SECURITY.md`.
3. **Mobile**: the Rust module shells out to the system `curl`, which Android does not have. On
   Android and iOS builds, use the same logic over an in-process HTTP client (`reqwest` with rustls)
   and store the credential in the app's private storage (or the platform keystore). Desktop builds
   may keep `curl`; if they use `reqwest`, the key still never goes on a command line.
4. **One AI client for every provider** (Publik, own key, local), OpenAI-compatible, in the core.

The module's public surface, used by the rest of the app:

- `DISCLOSURE`: shown verbatim, and accepted, before `provision()`.
- `resolve_provider(user_key)`: the resolution order below.
- `provision()`: `POST /installs`; writes the credential **before** returning it.
- `credential_path()`: `%LOCALAPPDATA%\publik\apps\<slug>.json` (Windows),
  `~/Library/Application Support/publik/apps/<slug>.json` (macOS),
  `$XDG_CONFIG_HOME/publik/apps/<slug>.json` or `~/.config/...` (Linux); mode 0600. Fields:
  `key`, `base_url`, `install_id`, `claim_url`, `claim_state` (`anonymous` | `claimed`),
  `add_credit_url`, `cost_sentence`, `balance_micros`.
- `balance_line(micros, claim_state)` and `top_up_url_from_402(body)`.
- `self_test(version)`: the builder check from the reference module. **First thing at startup**,
  before any window opens: if `PUBLIK_SELF_TEST` is set, print `self_test(...)` (one line,
  `publik self-test: ok <slug> <version> (install <8 chars>)` or `publik self-test: failed: <message>`)
  and exit. It provisions for real; the builder who runs it is the consent. Never on in normal use.

## 4. Choosing the provider

In this order, exactly as the reference modules do:

1. **The user's own key**, if set (any OpenAI-compatible base URL, key and model), exactly as before.
2. **`PUBLIK_API_KEY` / `PUBLIK_API_BASE_URL`** environment variables (terminal runs, CI with fakes).
3. **The credential file** at `credential_path()`.
4. **First launch of an AI feature**: show `DISCLOSURE` plus the app's own data-path sentence; on
   yes, `provision()`.
5. If provisioning fails or is declined: offer the user's own key, and the local model.

Settings always shows all three providers: **publik API** (with the balance line and account
link), **Your own key**, **Local model** (Ollama at `http://localhost:11434/v1` or any
OpenAI-compatible server on localhost; the app picks the largest suitable installed model and
never spends Publik balance, which a test proves).

## 5. Provisioning: `POST /installs`

Request (JSON, no auth header):

```json
{
  "app_token": "pat_<slug>_…",
  "app_slug": "<slug>",
  "app_version": "0.3.0",
  "os": "windows",
  "arch": "x86_64",
  "device_name": "Windows PC",
  "install_id": "5f0c8a52-6e0b-4c55-9a3e-2a3f1d9b7c11",
  "disclosure_version": 1,
  "dialects": ["chat_completions"]
}
```

Responses:

- **201**: a new key. Body includes `key` (`pk_live_…`), `key_id`, `base_url`, `install_id`,
  `claim_url`, `claim_state`, `balance_micros`, `wallet {balance_micros, add_credit_url,
  top_up_url, claim_state}`, `disclosure {version, cost, data_path}`.
- **200**: a replay of this `install_id`. `key` may be `null`: keep the key already on disk; if
  there is none, mint once more under a fresh `install_id`.
- **403 `install_revoked`**: mint once more under a fresh `install_id`.
- **429**: show `error.message` (busy setting up computers); offer the own key.
- Anything else: `error.message`, or "publik API could not set up this computer (code). Use your
  own key for now." Unreachable: "publik API is unreachable right now. Use your own key for now."

**Disconnect** (observed): `POST {base_url}/installs/revoke` with the key as bearer, best effort;
then forget the key but keep `install_id`, so reconnecting does not create a second install.

## 6. Making calls

- `Authorization: Bearer <pk_live_…>` (or `x-api-key`). Never a query parameter, never argv.
- `"model"` is a tier name on chat. For `/audio/*`, `/embeddings` and `/images/*`, **call
  `GET /models` before implementation and use exactly the ids it lists for those endpoints**, even if they look
  like vendor names (the tier-only rule is about chat); the docs do not list them. If none is listed,
  the module stops until the maintainer decides, rather than guessing.
- **Images** go into chat as OpenAI content parts,
  `{"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,…"}}`, resized to about
  1568 px on the long side as JPEG with metadata stripped. Probe once before implementation that the chosen
  tier accepts images. Fallback (observed in production): the Gemini `generateContent` dialect at
  `{base_url}/gemini/v1beta/models/publik-vision:generateContent` with `x-goog-api-key`; that
  dialect refuses `systemInstruction`, `tools`, `toolConfig`, `cachedContent` and `safetySettings`.
- **Audio** for transcription is multipart (`file`, `model`), under 4 MB: record 16 kHz mono and
  encode Opus or MP3 (roughly 10 to 30 minutes fit), or split on silence client-side.
- **Everything under 4 MB**, base64 included.
- **Structured output**: ask for JSON in the prompt, and validate every response against the
  feature's JSON Schema. Send `response_format` only if a probe before implementation shows the tier
  honours it. A response that fails validation is discarded and shown as "the AI answer could not be used",
  never half-applied.
- **Idempotency** (observed): send `x-publik-idempotency-key: <32 lowercase hex>`, the first 32
  characters of the response-cache key's SHA-256. A **Try again** after an answer failed validation
  adds the attempt number to the key's input, so it is a new request and not a replay of the
  unusable answer.
- **No automatic retries on Publik calls**: Publik settles usage even when the client gives up, so a
  retry can charge twice. Offer a **Try again** button instead. Own-key and local calls may retry
  with backoff.
- **Cache** every response, keyed by SHA-256 of (provider, model, prompt id and version, canonical
  input). Identical input never spends twice. Caches are per provider. Bumping a prompt's version
  orphans its old entries on purpose. The cache is on disk by default; where storing content would
  break a privacy promise (dictated speech, text captured from other apps) an app keeps it in memory
  only or with a short expiry, and its `docs/PUBLIK-INTEGRATION.md` says which.
- **Prompts are files**: `prompts/<feature>.v<N>.md`, loaded at build time, golden-tested, and
  never edited in place (a change is a new version).

## 6a. Web search: `POST /search`

Only for an app whose token lists `search` (from `skills/search.md`, read on 2026-10-26).

- Request: `POST {base_url}/search` with the same bearer key, JSON `{"q": "...", "count": 10}`.
  `q` is required, up to 400 characters; a phrase in double quotes is searched exactly. `count` 1 to
  20 (default 10). Optional `country` (`US`), `lang` (`en`), `freshness` (`pd`, `pw`, `pm`, `py` or
  `YYYY-MM-DDtoYYYY-MM-DD`), `safesearch` (`off`, `moderate`, `strict`). `GET /search?q=...` takes the
  same fields; apps here use `POST`, so the query never sits in a URL.
- Answer `200`: `{"object": "search.results", "query", "count", "more", "results": [{"url", "title",
  "snippet", "site", "age"}], "charge_micros", "request_id"}`. The charge is one cent a search
  (`charge_micros` 10000), whatever `count` is; `x-publik-charge-micros` says the same.
- Errors: `400 invalid_request` (`error.message` names the field); `402` exactly as section 8;
  **`502` or `504`: the provider failed and nothing was charged**, so the app may say so and offer
  **Try again** (it never retries by itself).
- Each reference module has a `search()` helper; copy it with the module.
- Resolution order: the user's own search key (its Settings field says what it is for, for example
  "Brave Search API key (for plagiarism checking)"; the vendor's name appears only in that field),
  then publik API, then nothing: the feature says it needs one of the two.
- UI: say where results came from ("Searched with publik API"). A feature that sends several
  queries caps them (eight is plenty for a plagiarism check) and shows the total charge.
- Search queries are the user's own words leaving the computer: the app's data-path sentence names
  them, and a search runs only when the user asks for it.
- The skill asks for one line in the app's publik API notes. Here that line goes in the
  `CONTRIBUTING.md` block (section 10).

## 7. Reading the status

Every metered response carries `x-publik-*` headers: the model that served it, the balance after
the call, this week's usage and budget, the reset time, and the call's cost. Names observed in
production (confirm with a `-D` header dump in the proof call):

| Header | Status field |
|---|---|
| `x-publik-balance` | `balance_micros` |
| `x-publik-charge-micros` | `last_charge_micros` |
| `x-publik-request-id` | `last_request_id` |
| `x-publik-claim-state` | `claim_state` |
| `x-publik-starter-remaining` | `starter_remaining_micros` |
| `x-publik-week-used` | `week_used_micros` |
| `x-publik-week-budget` | `week_budget_micros` |
| `x-publik-week-resets-at` | `week_resets_at` |

Keep a status object in memory and in the app's data directory (never containing the key); refresh
it from `GET /wallet` on start (only once the user has chosen publik API) and when Settings opens. Writing it never fails the user's action.

## 8. Errors: stop, say one thing, offer one way out

| Response | What the app does |
|---|---|
| **402** | Show `error.message` and **exactly one link**, `error.top_up_url` (a claim link while anonymous, an add-credit link once claimed), plus "or use your own key or a local model in Settings". Stop the batch. Cache nothing. The rest of the app keeps working. |
| **401** (`invalid_api_key`, `key_revoked`) | Mark disconnected, offer **Reconnect** (provision again). Stop the batch. |
| **403** `install_revoked` | Same as 401. |
| **403** `claim_first` (token minting) | Maintainer step: claim the listing first. |
| **429** | Show the message; no retry loop. |
| **503** | Show "publik API is busy; try again in a moment" with **Try again**. |
| other 4xx/5xx | "publik API <code>: <error.message>", one attempt only. |
| offline | The feature says it needs a connection or a local model; nothing queues silently. |

A success after a 402 clears the needs-credit state.

## 9. The UI copy every app uses

- **Before setup**, the data-path sentence of the app (what it sends, what stays) and `DISCLOSURE`
  verbatim, with **Continue with publik API**, **Use my own key**, **Use a local model**.
- **First-run card** right after provisioning: `balance_line(...)` (for example "$0.00 · link this
  computer for $0.05 of free use"), then `cost_sentence` verbatim, then the primary button **Sign in
  with publik** opening `claim_url`. Once `claim_state` is `claimed`, the button
  reads **Add a plan or pack** and opens `add_credit_url`.
- **Balance line** afterwards from `x-publik-balance`: "$3.12 of publik balance".
- **Before any batch** of more than 10 metered calls: show the number of calls and the tier, and
  after the first call the measured cost per call from `x-publik-charge-micros`.
- **Copy rule**: talk in dollars. A test fails on "publik credits", "in credits", "Gemini credit",
  "ChatGPT credit" or "OpenAI API" in UI strings, docs and README. The test exempts
  `docs/PUBLIK-CONTRACT.md` and itself, which quote the phrases.

## 10. The CONTRIBUTING.md block

Every repository that uses Publik appends this block to `CONTRIBUTING.md` (from the official skill):

```md
## publik API
- Base URL: https://publikhq.com/api/v1 (OpenAI Chat Completions, Responses, Anthropic Messages)
- Models: publik-fast, publik-balanced, publik-smart only; never a vendor model name
- Key: per-install `pk_` from POST /installs, stored at the publik credential path; env PUBLIK_API_KEY / PUBLIK_API_BASE_URL override
- App token: publik-app-token.txt (public by design, committed) or PUBLIK_APP_TOKEN
- 402: show error.message plus error.top_up_url only
- Never print or log a key
- Self-test: PUBLIK_SELF_TEST=1 makes the built app print one `publik self-test:` line and exit
- Docs: https://publikhq.com/llms.txt, https://publikhq.com/developers, https://publikhq.com/skills/setup.md
```

An app whose token lists `search` adds one line to the block, before `Docs`:
`- Web search: POST /search {"q","count"}; one cent a search; the user's own search key, when set, comes first`.

## 11. Tests every Publik module has

All HTTP is faked: no test reaches publikhq.com. One opt-in proof script
(`scripts/publik-proof`), run by the maintainer only, makes a single real call as the skill's step 5
does and prints the reply and the charge line.

1. Resolution order: own key beats env beats credential file; nothing set gives `NeedsConsent`.
2. `provision()` sends the documented body (and no hostname unless opted in); 201 writes the
   credential with mode 0600 before returning; 200 with `key: null` keeps the key on disk; 200 with no
   key on disk mints once more under a fresh id; `install_revoked` mints once more; a second failure
   surfaces the message.
3. The app token is read from the file, overridden by `PUBLIK_APP_TOKEN`; a build without a `pat_`
   token gives "This build has no publik API app token. Use your own key for now." (the real-token
   test is skipped with a message until the maintainer step is done).
4. Header transport; tier model names only on chat (the Gemini fallback's `publik-vision` and the
   `GET /models` ids for audio, embeddings and images are the only other model values); idempotency
   key is 32 lowercase hex.
5. Status parsed from every header in section 7; `GET /wallet` refresh.
6. 402: the message, exactly one URL (the top-up link), one call, needs-credit set, nothing cached,
   the batch stopped, the app still usable. Parametrised over anonymous and claimed.
7. A success after 402 clears needs-credit.
8. 401 and 403 `install_revoked` mark disconnected and offer reconnect.
9. 429, 503 and other errors: one attempt, the documented message.
10. A timeout on Publik is not retried; the own-key path retries with backoff.
11. Caches are per provider; identical input never spends twice; a prompt version bump misses.
12. The local provider never calls publikhq.com, even with a Publik credential on disk.
13. `publik-smart` falls back to `publik-balanced` while `claim_state` is `anonymous`.
14. Every response is validated against its schema; an invalid one is discarded with the documented
    message.
15. Every request is under 4 MB; if the app sends images, they are resized to at most 1568 px and
    carry no EXIF/XMP/IPTC.
16. The key never appears in logs, errors, the status object or anything the UI receives.
17. The copy rule.
18. The self-test: with `PUBLIK_SELF_TEST` set and a fake server, startup prints exactly one line,
    `publik self-test: ok <slug> <version> (install <first 8 characters>)`, and exits before any
    window or engine starts; with a failing server it prints `publik self-test: failed: <message>`
    and exits; without the variable it never runs.
19. Web search (only for an app whose token lists `search`): the own search key beats publik API;
    the body carries `q` and `count` and the key rides only in the header; 200 results are parsed
    with the charge; 402 follows test 6; 502 and 504 show "nothing was charged" with **Try again**
    and no automatic retry; a multi-query feature never sends more than its cap; the UI line says
    "Searched with publik API".
