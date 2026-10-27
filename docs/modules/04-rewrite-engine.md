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
  (per-action cases, `languages.jsonl`, `redaction.jsonl`)

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
- Further decisions are filled in here during implementation.

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

Planned
