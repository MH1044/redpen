# Contributing to Redpen

Redpen is pre-alpha and is built one module at a time, in the order of `docs/ROADMAP.md`, each
from its specification in `docs/modules/`. Read `docs/ARCHITECTURE.md` and `docs/TESTING.md`
first; for anything that makes an AI call or a web search, also `docs/PUBLIK-INTEGRATION.md` and
`docs/PUBLIK-CONTRACT.md`.

## The full check

`sh scripts/verify.sh` (PowerShell: `powershell -File scripts/verify.ps1`) is what continuous
integration runs, minus the launch smoke. It must be green before every commit, with no warnings.
Windows PowerShell 5.1 has no `&&`: run the steps separately and read `$LASTEXITCODE`.

## The clean-room rule

Redpen is permissively licensed (ADR-0002). Never have the source of a GPL, AGPL, SSPL, BUSL or
source-available project open, in an editor, a browser tab or an assistant's context, while
writing the corresponding part of Redpen. `docs/PRODUCT.md` names GSpot (AGPL-3.0): its README
and its listing may be read, its source never. Documentation, papers, issue trackers and
file-format specifications are fine. Permissive prior art (MIT, BSD, ISC, Apache-2.0, MPL-2.0,
CC0) may be studied and ported with attribution in `THIRD_PARTY.md`; Harper (Apache-2.0) may be
studied.

## Dependencies

- Permissive licences only. `deny.toml` fails the build on any Rust dependency, transitive ones
  included, outside its allow list; it runs in the full check. Front-end packages are checked
  when they are added and listed with their licence in `THIRD_PARTY.md`.
- Every new dependency needs a one-line justification in the commit body and a row in
  `THIRD_PARTY.md`. Prefer boring, widely used packages with more than one maintainer. A dependency
  with one maintainer gets a decision record in `docs/adr/` that names the exit.
- Models and datasets are dependencies too: each has a row in `THIRD_PARTY.md` with licence and
  source. Non-commercial or research-only weights are never shipped, and weights are never
  committed. Public datasets under CC BY and similar licences are downloaded by checksum and never
  committed; committed fixtures are generated, or CC0 or public domain with the source recorded.

## Determinism

- Core code never reads the clock, randomness, the filesystem or the network directly. Time
  arrives as the `Clock` and `Sleeper` traits of `crates/redpen-core/src/clock.rs`, HTTP as the
  `HttpTransport` trait of `crates/redpen-core/src/net/`, and tests pass fakes.
- No test reaches the network. Every HTTP call goes through the fake transport, which answers from
  a `recording.json` and fails on any request it has no entry for. A test that needs a real model
  file skips with a clear message when the file is absent.
- Every random fixture has a fixed seed. A fixture that is compared byte for byte is built with
  integer maths or a pure-software maths library, never the platform's `sin` or `exp`, so it is
  identical on Windows and Linux.
- Floats are compared with a stated tolerance, never with `==`.
- Golden files are the specification, not a cache. Expected values come from a reference (a
  published algorithm, a reference implementation run once and recorded with its version, or a
  fixture whose truth is known by construction), never from running the code. Write a new case's
  expected file with `UPDATE_GOLDEN=1 GOLDEN_FILTER=<module>/<case>`, only for a new case or a
  change the spec and commit body describe, and read the diff line by line.
- Prompts are files under `prompts/`, versioned, and never edited in place: a change is a new
  version (ADR-0004).

## Tests and commits

- Every feature and every user-facing error message has at least one test. `docs/TESTING.md`
  says where each kind of test lives.
- Nothing that identifies a person, an account or a machine goes into a file or a commit message:
  no home-directory paths, no email address except a GitHub noreply one, no keys or tokens, no
  hostnames, no personal data in fixtures. The source-hygiene test
  (`crates/redpen-cli/tests/hygiene.rs`) fails the build on any of them.
- One logical change per commit, revertable on its own, with a conventional subject (`feat`,
  `fix`, `test`, `docs`, `chore`, `refactor`, `perf`, `ci`) and a body that says what changed and
  why, in product terms.
- Talk in dollars. Interface text, the documentation and this file never say "credits".

## publik API
- Base URL: https://publikhq.com/api/v1 (OpenAI Chat Completions, Responses, Anthropic Messages)
- Models: publik-fast, publik-balanced, publik-smart only; never a vendor model name
- Key: per-install `pk_` from POST /installs, stored at the publik credential path; env PUBLIK_API_KEY / PUBLIK_API_BASE_URL override
- App token: publik-app-token.txt (public by design, committed) or PUBLIK_APP_TOKEN
- 402: show error.message plus error.top_up_url only
- Never print or log a key
- Self-test: PUBLIK_SELF_TEST=1 makes the built app print one `publik self-test:` line and exit
- Docs: https://publikhq.com/llms.txt, https://publikhq.com/developers, https://publikhq.com/skills/setup.md
- Web search: POST /search {"q","count"}; one cent a search; the user's own search key, when set, comes first
- Redpen products: ai.chat, search
- Redpen tiers: publik-fast for Fix grammar; publik-balanced for Clearer, Shorter, More formal, Friendlier and Custom; publik-smart for those five only when Rewrite quality is Best and the computer is linked
- The module lives at app/src-tauri/src/publik.rs; every change to the reference is marked `// redpen:`
- No retries on Publik calls; Try again is a button. Rewrite answers and search results are cached on disk for at most 7 days
- Plagiarism check: at most 8 searches per check, quoted phrases of at most 12 words, snippets only; result pages are never fetched
- Chat always sends a tier. Idempotency key: first 32 hex of the cache key, or of SHA-256(<key>:<n>) after n invalid answers
