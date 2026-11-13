# 04: Rewrite engine

## Goal

Given a selected text and an action, Redpen builds the exact request of
docs/PUBLIK-INTEGRATION.md §6.1 to §6.3, sends it through `AiClient`, and returns a validated,
restored, cleaned result with a word-level diff and any warnings, or a clear refusal. Nothing
invalid is ever returned as a result. `redpen rewrite`, `prompt`, `redact` and `diff` expose it for
golden tests; the popup (06) uses it.

## Depends on

01 (dialects), 02, 03 (tiers, Publik errors).

## Scope

- Prompt files `prompts/{fix-grammar,clearer,shorter,more-formal,friendlier,custom}.v1.md` and
  schemas `prompts/schemas/{rewrite-result,custom-result}.v1.json`, with exactly the text of
  docs/PUBLIK-INTEGRATION.md §6 (each file complete on disk).
- `build.rs` embeds every `prompts/**` file; `prompts::load` parses the front matter (`id`,
  `version`, `tier`, `temperature`, `max_tokens`, `schema`) and the `## system` and `## user`
  sections; the file name must match `id` and `version`.
- `prompts::render` fills `{name}` placeholders; a placeholder with no value, or a value with no
  placeholder, is an error; `</text>` and `</instruction>` inside values are escaped as `<\/…>`
  and unescaped in the answer.
- The action table: prompt, tier by `publik_tier`, temperature, the `max_tokens` formula, schema,
  and validators, for Fix grammar, Clearer, Shorter, More formal, Friendlier and Custom.
- Limits: a selection over 1,500 words, an empty or whitespace-only selection, and a typed
  instruction over 300 characters are refused before any request.
- Cache input `{text, dialect, style, instruction?, variant}` (text after redaction); Fix grammar
  uses `{text, dialect}` only, since it ignores style and has no other version. `variant` counts
  presses of **Another version** after a success.
- Validation retries: the session counts, per cache key, answers that failed validation (schema,
  markers); the next call for that key sets `ChatCall.validation_retry` to that count, so the
  idempotency key becomes SHA-256(`<K>:<n>`) (docs/PUBLIK-INTEGRATION.md §5), for every action
  including Fix grammar. A valid answer is cached under K and resets the count.
- Answer parsing: fence stripping, first-brace fallback, JSON Schema validation (`jsonschema`).
- Redaction (below), marker checks, restoration.
- Output cleaning: preamble line, wrapping quotes, original leading and trailing whitespace.
- "No changes needed." when the result equals the original.
- Language keeping with `whatlang` (both texts at least 40 characters and reliably detected);
  skipped for Custom when `changes_language` is true.
- The minimal-edit guard for Fix grammar and the "not shorter" warning for Shorter.
- Word diff (`similar`, Myers, over UAX #29 word and non-word runs) as `DiffOp` values.
- CLI: `redpen rewrite`, `redpen prompt`, `redpen redact`, `redpen diff`.
- The `eval-rewrite` gate (docs/TESTING.md §4).

Redaction, in this order, each value numbered by first appearance and reused when repeated:

| Kind | Pattern | Extra rule |
|---|---|---|
| EMAIL | `(?i)\b[a-z0-9._%+-]+@[a-z0-9-]+(?:\.[a-z0-9-]+)*\.[a-z]{2,}\b` | none |
| IBAN | `\b[A-Z]{2}[0-9]{2}(?: ?[A-Z0-9]{4}){2,7}(?: ?[A-Z0-9]{1,4})?\b` | 15–34 characters without spaces; ISO 7064 mod 97 equals 1 |
| CARD | `\b\d(?:[ -]?\d){12,18}\b` | 13–19 digits |
| PHONE | `(?:\+\d\|\(\d\|\b\d)[\d ().-]{5,}\d\b` | 7–15 digits; a leading `+` or at least one separator; not an ISO date `\d{4}-\d{2}-\d{2}` or a time `\d{1,2}[:.]\d{2}` |
| NUMBER | `\b\d{8,}\b` | not already inside another entity |

Markers are `[EMAIL_1]`, `[IBAN_1]`, `[CARD_1]`, `[PHONE_1]`, `[NUMBER_1]`. A text that already
contains `\[(EMAIL|PHONE|IBAN|CARD|NUMBER)_\d+\]` is refused.

## Files

- `prompts/*.v1.md`, `prompts/schemas/*.v1.json` (six prompts, two schemas)
- `crates/redpen-core/build.rs`; `crates/redpen-core/src/rewrite/{mod.rs, actions.rs, prompts.rs,
  parse.rs, clean.rs, guard.rs}`; `src/{redact.rs, lang.rs, diff.rs, words.rs}`
- `crates/redpen-cli/src/cmd/{rewrite.rs, prompt.rs, redact.rs, diff.rs}`; `scripts/gates.*`
  (`eval-rewrite`)
- `THIRD_PARTY.md` (`jsonschema`, `whatlang`, `regex`, `similar`)
- Tests: `crates/redpen-core/tests/04_rewrite_engine.rs`; `tests/cases/04-rewrite-engine/`
  (per-action cases, `languages.jsonl`, `redaction.jsonl`, and `fix-grammar.jsonl` for the gate)

## Interfaces

```rust
pub enum Action { FixGrammar, Clearer, Shorter, MoreFormal, Friendlier, Custom }
pub enum Style { Own, PlainEnglish, Academic, Business }             // only Own until module 08
pub struct RewriteRequest { pub action: Action, pub text: String, pub instruction: Option<String>,
    pub dialect: Dialect, pub style: Style, pub redact: bool, pub variant: u32,
    pub validation_retry: u32 }                                   // set by the session, never by the UI
// TierContext {quality, claim_state} and publik_tier come from redpen_core::provider::tier (module 03)
pub enum DiffOp { Equal(String), Insert(String), Delete(String) }
pub enum Warning { Guard { changed: usize, total: usize }, Language { expected: String, got: String }, NotShorter }
pub struct RewriteOutcome { pub text: String, pub diff: Vec<DiffOp>, pub warnings: Vec<Warning>,
    pub accept_default: bool, pub unchanged: bool, pub from_cache: bool, pub charge_micros: Option<u64> }
pub enum RewriteError { Ai(AiError), Empty, TooLong { words: usize }, InstructionTooLong,
    RedactionCollision, PlaceholderDropped { marker: String }, PlaceholderInvented { marker: String } }
pub async fn rewrite(c: &AiClient, p: &ResolvedProvider, ctx: &TierContext,
                     r: RewriteRequest) -> Result<RewriteOutcome, RewriteError>;
pub fn build_request(p: &ResolvedProvider, ctx: &TierContext, r: &RewriteRequest) -> Result<ChatCall, RewriteError>;
pub fn redact(text: &str) -> Result<(String, Vec<Entity>), RewriteError>;
pub fn restore(text: &str, entities: &[Entity]) -> String;
pub fn edit_ratio(old: &str, new: &str) -> (usize, usize);              // (changed, original words)
```

Messages: `Empty` "Select some text first."; `TooLong` "Select at most 1,500 words for a rewrite
(this selection has 1,812)."; `InstructionTooLong` "Keep the instruction under 300 characters.";
`RedactionCollision` "This text already contains a marker like [EMAIL_1], so redaction can't run
safely. Turn off redaction in Settings → Privacy, or edit the text."; `PlaceholderDropped` "The AI
answer dropped [PHONE_1], so it was not used. Try again, or turn off redaction for this text.";
`PlaceholderInvented` "The AI answer added a marker that was not in your text, so it was not used.";
warnings as in docs/PUBLIK-INTEGRATION.md §6.1 and §6.2; `unchanged` shows "No changes needed."

## Out of scope

- The popup and Accept: modules 05 and 06.
- Style presets other than `Own`: module 08. Streaming: not in v1 (ADR-0006).

## Design notes

- Decided: prompts are never edited in place; a change is `v2` and orphans cached answers
  (ADR-0004). The guard thresholds (0.35, 4) are constants in `guard.rs`, confirmed or changed by
  the first `eval-rewrite` run, recorded here.
- Before implementation: fetch the Publik sources and record changes (docs/PUBLIK-INTEGRATION.md §2).
  Fetched raw on 2026-11-13 for module 03; for chat calls nothing changed since then (the base URL,
  the tier names, the 402 body and the idempotency header are as the contract says). The changes
  that were found, and ADR-0010, are in docs/modules/03-publik.md.
- Decided before implementation:
  - **Dependencies** (each checked against `deny.toml` and built): `jsonschema` 0.61 with default
    features off, so no remote `$ref` resolution and no network code; `whatlang` 0.18; `similar` 3;
    `regex` 1 (already a test dependency, now a normal one). `deny.toml` gains `MIT-0` (MIT No
    Attribution, OSI-approved and more permissive than MIT), the licence of `borrow-or-share`, which
    `jsonschema` needs through `referencing` and `fluent-uri`; every copyleft licence is still
    refused.
  - **Prompt files**: the front matter is `key: value` lines between two `---` lines, parsed by
    hand, so no YAML library is needed; `max_tokens` holds the formula as docs/PUBLIK-INTEGRATION.md
    §6 writes it and the action table computes it. A section's text is everything after its
    `## system` or `## user` line up to the next `## ` line or the end, with leading and trailing
    line breaks removed and nothing else changed. That makes the system and user contents exactly
    the §6 example bodies.
  - **The model in a request**: publik API and the fake provider send the tier from `publik_tier`;
    own key and local send their configured model. `redpen prompt` prints the publik API body;
    `redpen rewrite --provider fake` sends that same body, so a hand-written recording holds the §6
    bodies. The CLI's `TierContext` is Standard and anonymous.
  - **Numbers** follow module 02: `temperature` 0 is written `0`, and 0.4 is `0.4`.
  - **Words**: `words(x)` counts the UAX #29 words that contain a letter or a digit; punctuation and
    spaces are not words. The 1,500-word limit, `max_tokens` and the guard all use it, so the §6.1
    example has 15 words and its answer changes 4 (ratio 0.27). Guard tokens compare exactly
    (case-sensitive). The display diff runs over UAX #29 word and non-word runs, with neighbouring
    operations of the same kind merged (test 15).
  - **Language names** are whatlang's English names (`English`, `French`).
  - **CLI output**, one JSON line each:
    - `redpen rewrite` prints the outcome: `{"text", "diff", "warnings", "accept_default",
      "unchanged", "from_cache", "charge_micros"}`. `diff` is a list of one-key objects, `equal`,
      `insert` or `delete`, as test 15 shows. A warning is `{"kind": "guard" | "language" |
      "not_shorter", "message": "…"}` plus its fields (`changed` and `total`; `expected` and
      `got`). A refusal or AI error prints `{"error": {"kind", "message", …}}` and exits 3, as in
      module 02; usage errors exit 2.
    - `redpen prompt` prints the request body, exactly as sent, after redaction when `--redact` is
      given.
    - `redpen redact FILE` prints `{"text": "<redacted text>", "entities": [{"marker": "[EMAIL_1]",
      "kind": "email", "value": "…"}]}`, entities in order of first appearance.
    - `redpen diff OLD NEW` takes two files and prints the diff list.
- Decided during implementation:
  - **Where things live**: `redpen_core::rewrite` (the engine, `actions`, `prompts`, `parse`,
    `clean`, `guard`), with `redact`, `lang`, `diff` and `words` beside it; `rewrite` re-exports
    `DiffOp`, `edit_ratio`, `word_diff`, `Entity`, `redact` and `restore`, and the crate root
    re-exports `prompts`. `build.rs` writes the list of every `prompts/**` file into `OUT_DIR`
    with `include_str!`, so the app never reads a prompt from disk.
  - **Prompt API**: `prompts::load(file_name, text)` parses one file and checks it (front matter
    keys, a publik tier, the formula, `<id>.v<version>.md`, both sections, only the four known
    placeholders, `{text}` in the user section), its error starting with the file name;
    `prompts::get(file)` returns an embedded prompt; `prompts::render(template, values)` fills one
    template; `prompts::render_prompt(prompt, values)` fills both sections, a value being used if
    either section has its placeholder. Carriage returns are dropped before parsing.
  - **Validation inside the client**: `AiClient::chat_validated(p, call, check)` runs the
    engine's check on the answer before it is cached, so an answer that fails the schema, the
    markers or the cleaning is never stored; a cached answer is checked too and passed over if it
    fails. `chat` is `chat_validated` with a check that accepts everything. A failed check is not
    reported to the status sink, since the provider did answer.
  - **The text as sent** is the selection exactly as captured, its edges included (`t04-whitespace`
    sends `<text>\n  hi there\n\n</text>`); the instruction is trimmed. Cleaning re-attaches the
    original's own leading and trailing whitespace whatever the answer does with it. An answer that
    cleaning leaves empty is `InvalidAnswer`.
  - **The display diff's tokens** are the words and the maximal runs of text between them
    (`words::runs`), so `, ` is one token and §6.1's `tomorow, please` → `tomorrow. Please` is one
    deletion and one insertion. A changed stretch is shown as its deletion, then its insertion.
  - **Redaction**: the patterns run with Unicode classes off (`\d` is `[0-9]`, `\b` an ASCII word
    boundary), so digit counts are exact and a digit run beside an accented letter is still found.
    A later kind never matches inside an earlier one's value; a candidate its extra rule refuses is
    skipped and the search goes on after it. "Not an ISO date or a time" is read as: a phone-shaped
    match is refused when only separators are left once every ISO date and every time (each
    between word boundaries) is taken out, so `2026-09-29 10.30` stays and `01.23.45.67.89` is a
    phone. Custom's instruction is redacted too, first, with one numbering shared with the text;
    the answer must keep the text's markers and may hold the instruction's. With redaction off
    there are no marker checks, so a marker the user typed is plain text.
  - **Closing tags** are turned back in the answer only when they were escaped: `<\/text>` when
    the text or the instruction held `</text>`, and `<\/instruction>` likewise.
  - **The guard and the warnings**: the guard compares the selection with the cleaned, restored
    result, in whole numbers (`changed * 100 > 35 * max(1, total)` and `changed >= 4`). Language
    and guard warnings set `accept_default` to false; "not shorter" does not. An `unchanged`
    outcome has no warnings. Language keeping uses `whatlang::detect` and its `is_reliable()`, on
    texts of at least 40 characters after trimming.
  - **Refusal order**: empty, then over 1,500 words, then Custom's instruction (missing or over
    300 characters, counted after trimming), then the marker collision. A Custom request with no
    instruction is `InstructionMissing`, "Type an instruction for Custom first.", a refusal the
    Interfaces did not list.
  - **Validation retries**: `RewriteError::is_invalid_answer` is true for `InvalidAnswer`,
    `PlaceholderDropped` and `PlaceholderInvented`. `ValidationRetries` keeps the count per cache
    key for as long as its owner (the popup) keeps it: its `rewrite` sets `validation_retry`,
    adds one after an unusable answer, resets on a result and leaves the count after any other
    error, so a Try again after a 503 sends the same key again.
  - **The command line**: `--provider publik-env` reads `PUBLIK_API_KEY` and
    `PUBLIK_API_BASE_URL` (publik's base URL when unset) and never provisions; `redpen chat` on it
    sends `publik-fast`. `rewrite` reads the charge from `x-publik-charge-micros` on the fake
    provider too, so a recording with that header shows it. `--style` accepts only `own` until
    module 08. `diff` drops one final line ending from each file; `rewrite`, `prompt` and `redact`
    read their file exactly.
  - **The `eval-rewrite` gate** runs `redpen eval-rewrite` (a debug build, so the question comes at
    once) on this module's `fix-grammar.jsonl` and `languages.jsonl`. It prints the calls, the
    tiers and the estimated cost and asks `[y/N]` before setting up any provider, so a declined run
    needs no key and sends nothing. Fix grammar runs once per corpus sentence, at most 100.
    Module 01's `corpus.jsonl` has only 48 sentences, several of them style findings or with more
    than one error, so the gate has its own corpus: `fix-grammar.jsonl`, 100 sentences written for
    this repository (CC0, 79 American and 21 British English, no digits, so nothing is redacted),
    each with one error that changing, or for one sentence removing, a single word fixes, and the
    corrected sentence. Each has 8 to 14 words (median 11), so a correct fix scores at most 0.125
    and the median of correct fixes is about 0.09, under the 0.15 limit with room for the gate to
    see over-editing. The gate thus makes TESTING.md §4's 100 + 200 = 300 calls, about $0.44, and
    the full check runs it scaled down: each sentence answered with its own correction changes one
    word and never warns.
    The language actions are Clearer, Shorter, More formal, Friendlier and Custom ("Make it sound
    more confident."). "Guard warnings on correct fixes" is the share of Fix grammar answers with a
    guard warning, since every corpus sentence needs a one-word fix. A provider error other than an
    unusable answer (a 401, a 402, no connection) stops the run, so it does not go on spending:
    nothing more is sent, standard error says `eval-rewrite: stopped after N calls: <the error>`,
    and the report says `"pass": false`. An unusable answer is counted in `errors` and the run goes
    on. A corpus line that is not JSON, or has no `text`, exits 2 before the question.
  - **`whatlang` and short English**: whatlang 0.18 calls some correct English paragraphs
    unreliable (for example "Thanks for the quick reply. Friday at ten in the morning works well
    for me, and I will bring the report."), so no language warning is given for them; this errs
    towards no warning.

## Acceptance tests

Golden cases in `tests/cases/04-rewrite-engine/`, each with `cmd.json`, `input.txt`, a
hand-written `recording.json` and `expected.json`; code tests in
`crates/redpen-core/tests/04_rewrite_engine.rs`. Expected results come from docs/PUBLIK-INTEGRATION.md
§6 and this spec. The contract's tests apply by reference (module 03); rows R3 to R9 of
docs/PUBLIK-INTEGRATION.md §11 are tests 1 to 7 here, R10 is test 9 and R7b is test 16.

1. R3: `redpen prompt --action fix-grammar --dialect british --redact input.txt`, where input.txt
   is the §6.1 example text with `ana@example.com` in place of `[EMAIL_1]`, prints exactly the §6.1
   example body; `--action clearer --dialect british` and `--action custom --dialect british
   --instruction "translate to French"` on their example texts print exactly the §6.2 and §6.3
   bodies.
2. R4: for each action, a recorded answer gives the expected text and diff (Fix grammar: the §6.1
   answer restored with `ana@example.com` in place of `[EMAIL_1]`).
3. R5: answers `{"txt": "x"}`, `{"text": "x", "extra": 1}`, `{"text": 5}` and `hello` →
   `InvalidAnswer` ("The AI answer could not be used."), no `ai_cache` row.
4. R6: content "```json\n{\"text\": \"Fine.\"}\n```" → `Fine.`; "Sure! Here is the text:\n{...}"
   via the brace fallback → the text; `"“Fine.”"` for an unquoted original → `Fine.`; original
   `"  hi there\n"` keeps two leading spaces and the trailing newline.
5. R7: guard fires for 6 of 10 words changed (warning text with "6 of 10", `accept_default:
   false`); not for 4 of 15; not for 3 of 5.
6. R8: English original and a French answer (both over 40 characters) → `Language {expected:
   "English", got: "French"}`; Custom with `changes_language: true` → no warning; a 20-character
   answer → no check.
7. R9: `redaction.jsonl` round trips exactly (`restore(redact(x)) == x`), including a repeated
   email (one marker), two emails (two markers), an email next to a phone number, the IBAN, the
   card and a 9-digit order number; `2026-09-29 at 10.30` is not redacted; an answer missing
   `[PHONE_1]` → `PlaceholderDropped` with its message; an answer containing `[EMAIL_2]` that was
   not sent → `PlaceholderInvented`.
8. A text containing `[EMAIL_1]` literally with redaction on → `RedactionCollision`, no request.
9. R10: tier per action via `publik_tier` in the request's `model`; `temperature` 0 for Fix grammar
   and 0.4 for the others; `max_tokens` 256 for a 20-word text, 300 for 100 words and 3,100 for
   1,500 words (Custom: 256, 350 and 3,150).
10. 1,500 words → a request; 1,501 → `TooLong` naming 1,501, no request; whitespace only →
    `Empty`; a 301-character instruction → `InstructionTooLong`.
11. A prompt file with an unknown placeholder `{colour}`, or a missing `## user` section, fails the
    prompt test with the file name.
12. A value containing `</text>` is sent as `<\/text>` and restored in the result.
13. An answer equal to the original → `unchanged: true`, empty warnings.
14. Shorter with an answer longer than the original → `NotShorter` warning.
15. `redpen diff` of `the cat sat` and `the dog sat` → `[{"equal":"the "},{"delete":"cat"},
    {"insert":"dog"},{"equal":" sat"}]`; `edit_ratio` → (1, 3).
16. `variant` 1 changes the cache key for Clearer; Fix grammar has no variant. After an invalid
    answer, Try again for Fix grammar (and for Clearer) keeps the cache key K and sends the
    idempotency key of SHA-256(`<K>:1`), a second invalid answer then `<K>:2`; a valid answer is
    cached under K; after a 503 the Try again sends K's own key (docs/PUBLIK-INTEGRATION.md R7b).
17. `scripts/gates.sh eval-rewrite` prints the number of calls and tier and waits for y/N (tested
    with `n`: exits 0 and sends nothing).

## Status

Done (2026-11-13)
