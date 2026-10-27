# Testing Redpen

## 1. Layers and commands

| Layer | Where | Runs in |
|---|---|---|
| Unit tests | beside the code (`#[cfg(test)]`, `*.test.ts`) | `verify` |
| Module tests | `crates/redpen-core/tests/NN_slug.rs`, `crates/redpen-os/tests/NN_slug.rs`, `app/src-tauri/tests/NN_slug.rs`, `app/src/**/NN-slug.*.test.ts` | `verify` |
| Golden cases | `tests/cases/NN-slug/<case>/`, driven by `crates/redpen-cli/tests/golden.rs` through the `redpen` binary | `verify` |
| Hygiene | `crates/redpen-cli/tests/hygiene.rs` | `verify`, CI |
| Windows integration | `crates/redpen-os/tests/05_capture_windows.rs` with the helper window `redpen-testwin` | CI `windows-integration` |
| Launch smoke | `redpen-app --smoke`: starts, builds the tray and hidden windows, waits for the UI's `ready` event, exits 0 within 20 s | CI, a step after `verify` on both runners from module 00 (`windows-integration` from module 05); Ubuntu under `xvfb-run` |
| Gates | `scripts/gates.sh <gate>` / `scripts/gates.ps1 <gate>` | before a release, and by the maintainer when they spend money |
| Manual | §8 | before a release |

Commands:

- Full check, green before every commit: `sh scripts/verify.sh` (Windows: `powershell -File
  scripts/verify.ps1`). In order: `npm ci` and `npm run build` in `app/` (Tauri needs
  `frontendDist`), `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `npm run typecheck`, `npm run lint`, `npm run format:check`, `cargo deny check licenses`,
  `cargo test --workspace`, `npm test`. It stops at the first failure.
- One module's goldens: `GOLDEN_FILTER=04-rewrite-engine cargo test -p redpen-cli --test golden`.
- A new golden case's expected file (never to turn a red test green): `UPDATE_GOLDEN=1
  GOLDEN_FILTER=<module>/<case>`, then read the diff line by line.
- Re-record fake answers with a real provider (maintainer only, synthetic corpus only, spends
  money): `UPDATE_RECORDINGS=1 GOLDEN_FILTER=<module> REDPEN_RECORD_PROVIDER=own-key|publik-env`.
  Entries get `"source": "recorded <provider> <date>"`. The recorder keeps nothing that describes the
  account or the call: of the response headers it writes only `x-publik-charge-micros`, with the
  synthetic value `"100"` (never the real charge, balance, request id, claim state or weekly
  usage), and of the body only `choices[0].message.content` for chat, or the `url`, `title`,
  `snippet`, `site` and `age` of each result for a search (with `charge_micros` 100 on a publik
  search), so response ids, timestamps and usage counts are dropped.

**Golden cases and recordings.** A golden case `tests/cases/NN-slug/<case>/cmd.json`:
`{"args": ["rewrite", "--action", "fix-grammar", "--provider", "fake", "input.txt"], "exit": 0}`;
stdout is compared, parsed, to `expected.json`. Recording `recording.json`:

```json
{
  "format": "redpen-recording/1",
  "entries": [{
    "request_sha256": "<64 hex: SHA-256 of 'POST /chat/completions\n' + canonical body>",
    "source": "hand-written",
    "request": {"model": "publik-fast", "max_tokens": 256, "temperature": 0, "messages": ["…"]},
    "status": 200,
    "headers": {"x-publik-charge-micros": "210"},
    "body": {"choices": [{"message": {"role": "assistant", "content": "{\"text\": \"…\"}"}}]}
  }]
}
```

Headers never enter the hash. `source` is
`hand-written` or `recorded <provider> <date>`. **Recordings carry no status headers**: the only
allowed response header is `x-publik-charge-micros`, with a synthetic value. Tests of the other
`x-publik-*` headers (module 03) use hand-written scripted responses in the test code or in
`tests/cases/03-publik/*.fake.json`, with invented values such as a balance of `3120000`.
`crates/redpen-cli/tests/recordings.rs` (module 00) enforces this on every committed
`recording.json` (§5).

**The fake transport** (module 00) answers from `recording.json` by the SHA-256 of the method, path
and canonical body (module 00). An unrecorded request fails the test with
"Unrecorded request <hash> to <path>; body written to target/unrecorded/<hash>.json. Add a
hand-written entry, or ask the maintainer to record it." It also keeps every request it saw, so
privacy tests can assert on them. Most entries are **hand-written** from the spec (what a correct
or deliberately broken answer looks like); tests never depend on a real model.

## 2. Test data

Hand-written text corpora (CC0, written for this repository at the module that first needs them)
live with that module's cases: `tests/cases/01-harper-check/corpus.jsonl`
(sentences with one known error each, the expected category and correction) and `unicode.txt`
(emoji with ZWJ, combining marks, CJK, astral characters);
`tests/cases/04-rewrite-engine/languages.jsonl` (5 paragraphs each in English, French, German,
Spanish, Portuguese, Italian, Dutch and Polish) and `redaction.jsonl` (`@example.com` addresses,
fictional numbers from the UK and US reserved ranges, the standard example IBAN
`GB82 WEST 1234 5698 7654 32`, the public test card `4111 1111 1111 1111`).

No real documents are committed, except the public-domain quotations of the plagiarism corpus
(§4, module 09), each with its source. Data that would be large or binary (the 70 MB clipboard image of
module 05, the generated strings of module 01) is generated inside the test that uses it.

## 3. References

- Harper itself: its own command-line linter (`harper-cli`) at the pinned `harper-core` version, run
  once on the corpus and recorded with the version in `tests/cases/01-harper-check/REFERENCE.md`;
  Redpen's goldens check the wrapper (spans, categories, fixes) against that run.
- Unicode UAX #29 test data for word tokens; Myers 1986 for diff edit counts.
- ISO 13616 / ISO 7064 mod 97-10 for IBAN validity.
- docs/PUBLIK-CONTRACT.md §11 for the Publik test list; docs/PUBLIK-INTEGRATION.md §6 for every
  prompt, schema and validation rule.

## 4. Quality gates

`scripts/gates.sh <gate>`. Gates that spend money say so, print the number of calls and the tier,
ask y/N, and after the first call print the measured cost per call.

| Gate | Measures | Threshold | Cost |
|---|---|---|---|
| `eval-rewrite` | Fix grammar on 100 corpus sentences: median edit ratio, guard warnings on correct fixes; 40 paragraphs × 5 actions: language keeping | median ≤ 0.15; warnings ≤ 5%; keeping ≥ 98%; the first run confirms or sets them (module 04) | about 100 × $0.0002 + 200 × $0.0021 = $0.44 |
| `check-speed` | median of 10 checks of a 1,000-word corpus text, release build | ≤ 50 ms (module 01) | free |
| `eval-plagiarism` | 10 essays in `tests/cases/09-plagiarism-check/corpus/`, each mixing sentences written for this repository (CC0) with sentences quoted from public-domain works (Project Gutenberg texts of works out of copyright, United States federal government pages), each source named in the corpus README: quoted sentences labelled Found or Similar, original ones not | copied found ≥ 80%; original flagged ≤ 10% (module 09; the first run confirms or sets them) | at most 10 × 8 searches = $0.80 on publik API |

Scaled-down version in `verify`: the guard and edit ratio over the recorded corpus answers.

## 5. Hygiene test

The source-hygiene specification, ported to Rust in `crates/redpen-cli/tests/hygiene.rs` (it walks
up from `CARGO_MANIFEST_DIR` to the repository root, because a root `tests/` folder in a Cargo
virtual workspace is never compiled).

- **Files**: `git ls-files -z --cached --others --exclude-standard`, skipping paths missing on disk,
  so ignored files are never scanned. Without git, walk the tree skipping every path `.gitignore` ignores, plus `.git`, `node_modules`,
  `target`, `dist`, `build`, `out`, `coverage`, `release`, `.venv`, `__pycache__`, and the root
  `models/` and `eval-data/` folders. Text by extension only: `.rs .ts .tsx .mts .cts .js .jsx .mjs
  .cjs .py .toml .json .jsonc .yml .yaml .md .html .css .txt .sh .ps1 .xml .svg .csv .tsv .lock`,
  plus `LICENSE*` and `COPYING*`. Lockfiles and `THIRD_PARTY.md` are exempt from the email rule
  only.
- **Tests**, exactly five, run in `verify` and CI: (1) the walk found more than N files (N set at
  module 00 to about half the count); (2) no control characters except tab, LF, CR; (3) no
  home-directory paths, by the pattern in the
  template (described, never illustrated); (4) no email addresses except GitHub noreply identities,
  `noreply@anthropic.com` or `noreply@github.com`, and the documentation domains (`example`,
  `invalid`, `localhost`, `test`); (5) no secret-shaped strings (`pk_live_`/`pk_test_` with 16+
  characters, `pbt_`, `sk-` with 20+, Google, GitHub and Slack token shapes, private key headers),
  the one allowed token being the `pat_` token in `app/src-tauri/publik-app-token.txt`.
- Every failure says what to do instead ("use a path relative to the repository root, or an
  environment variable"; "a real address in a public repository cannot be taken back"; "write the
  character as an escape sequence"; "remove the key and rotate it").
- Proven at module 00: a home path, an email at a real-looking domain, a 40-character fake `sk-`
  key and a control character in an untracked temp file give exactly four failures; the file is
  then deleted.

**Recordings check**, module 00: `crates/redpen-cli/tests/recordings.rs` parses every
`tests/cases/**/recording.json` and fails when an entry's `format` is not `redpen-recording/1`, when
its `headers` contain any `x-publik-*` name other than `x-publik-charge-micros`, when an entry
marked `recorded …` has a charge other than `"100"` (header or body), or when a body has keys
other than `choices` (chat), `results` and `charge_micros` (publik search) or `web` (Brave search).
The message says "Recordings keep only x-publik-charge-micros
with a synthetic value; remove <header> from <file>."

**Copy rule** (contract §9), module 03: a test scans `app/src`, `README.md` and `docs/` (except
`docs/PUBLIK-CONTRACT.md` and the test file itself) for the phrases the contract forbids, and fails
on any.

**Log hygiene**, module 06 onwards: a full scripted session runs with a capturing `tracing`
subscriber; the output must not contain the session's text, the fake key, the fake balance or the
fake process name.

## 6. CI matrix

| Job | Runner | From module | What |
|---|---|---|---|
| `verify` | `windows-latest`, `ubuntu-latest` | 00 | `scripts/verify`, then the launch smoke (`redpen-app --smoke`; Ubuntu under `xvfb-run`); Ubuntu installs Tauri's Linux prerequisites |
| `windows-integration` | `windows-latest` | 05 | capture and clipboard tests with `redpen-testwin`; launch smoke |
| `release` | `windows-latest`, `ubuntu-latest` | 10 | on a tag: verify, build installers, draft the release with `docs/releases/<version>.md`, publish when green |

If synthetic input does not reach windows on the hosted runner, module 05 records it, keeps the
test runnable locally with `REDPEN_DESKTOP_TESTS=1`, and moves those checks to the manual list.

## 7. Test sources

A module's tests are `tests/cases/NN-slug/`, the code test files its spec names, and the
hand-written corpora of §2. Expected values come from the spec, the references above, or the
corpus's construction, never from running the implementation.

## 8. Manual checklist

Run on Windows 11 before each release (and the Linux X11 and Wayland lists from module 07). There is
no macOS list: Redpen has no macOS build (docs/ARCHITECTURE.md ADR-0008).

**Apps**: Notepad, Word, Outlook (classic and new), Teams, Slack, Chrome, Edge, Firefox, VS Code,
Windows Terminal. In each: Ctrl+Alt+R captures and Accept replaces; the clipboard holds what it
held before.

**Refusals**: a browser password field, the Windows credential prompt, 1Password or Bitwarden, an
administrator Command Prompt: each refused with its message, at capture and at Accept.

**Popup**: findings and fixes; each action; diff colours in light and dark mode; keyboard only
(1–7, Enter, Esc, Ctrl+Shift+C); positioning on a second monitor and at screen edges; consent view;
first-run card at $0.00; 402 with one link; offline message.

**Settings**: hotkey conflicts; AltGr rejection on a Polish layout; own key Test; Ollama detection;
blocklist picker; Clear now; sent log shows the redacted text that was sent.

**Plagiarism check** (module 09): a paragraph copied from a Wikipedia article is found, with that
article as a source; an original paragraph shows nothing found; Copy citation in each style pastes
the expected text; Open opens the browser; the footer shows the searches and the charge; with only
Local chosen the check explains what it needs and the sent log stays empty.

**Formats not preserved on the clipboard** (named in the HANDBOOK): enhanced metafiles, palettes,
delayed-render formats.
