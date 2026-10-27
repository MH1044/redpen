# 01: Harper check

## Goal

Any text can be checked on this computer, with no provider and no network: Harper's findings come
back with a user-facing category, a message, suggestions, and spans in characters, bytes and UTF-16
code units, in the chosen English dialect. A fix can be applied and the text re-checked.
`redpen check` exposes it for golden tests; the popup (module 06) uses it.

## Depends on

00.

## Scope

- `Checker` wraps `harper-core`: a curated lint group for the dialect over a plain-English document.
- Each Harper lint becomes a `Finding` with an id, category, Harper's kind name, message, spans and
  suggestions.
- Harper's kinds map to four categories, Spelling, Grammar, Punctuation and Style, by an exhaustive
  `match` with no wildcard arm, so a new Harper kind fails to compile until it is mapped.
- Spans: Harper's character indices converted to byte offsets and UTF-16 offsets.
- Dialects: every value of the pinned version's `Dialect` (expected American, British, Australian,
  Canadian); `dialect_from_locale` maps `en-GB`, `en-US`, `en-AU`, `en-CA`, anything else American.
- Categories can be disabled; their findings are dropped.
- `apply` applies one suggestion of one finding; a finding whose span text no longer matches is
  refused.
- `fix_all` applies the first suggestion of every finding, right to left, then re-checks, at most
  10 rounds.
- Texts over 20,000 characters are refused.
- CLI: `redpen check [--dialect D] [--disable C]... FILE` prints `{"findings": [...]}`;
  `redpen fix-all [--dialect D] FILE` prints `{"text": "..."}`.
- The `check-speed` gate (docs/TESTING.md §4).

## Files

- `crates/redpen-core/src/check/{mod.rs, spans.rs, categories.rs, dialect.rs}`
- `crates/redpen-cli/src/cmd/{check.rs, fix_all.rs}`
- `scripts/gates.sh`, `scripts/gates.ps1` (created here with the `check-speed` gate)
- `THIRD_PARTY.md` (`harper-core`, `unicode-segmentation`)
- Tests: `crates/redpen-core/tests/01_harper_check.rs`; `tests/cases/01-harper-check/`
  (`corpus.jsonl`, `unicode.txt`, `REFERENCE.md`, one case directory per golden)

## Interfaces

```rust
pub enum Dialect { American, British, Australian, Canadian }        // as Harper's enum, verified here
pub enum Category { Spelling, Grammar, Punctuation, Style }
pub struct Spans { pub chars: (usize, usize), pub bytes: (usize, usize), pub utf16: (usize, usize) }
pub enum SuggestionKind { Replace, Remove, InsertAfter }
pub struct Suggestion { pub kind: SuggestionKind, pub text: String }
pub struct Finding { pub id: String, pub category: Category, pub harper_kind: String,
                     pub message: String, pub span: Spans, pub suggestions: Vec<Suggestion> }
pub struct CheckOptions { pub dialect: Dialect, pub disabled: Vec<Category> }
pub enum CheckError { TooLong { chars: usize }, StaleFinding, NoSuchSuggestion }
impl Checker {
    pub fn new(opts: CheckOptions) -> Self;
    pub fn check(&self, text: &str) -> Result<Vec<Finding>, CheckError>;
    pub fn apply(&self, text: &str, f: &Finding, suggestion: usize) -> Result<String, CheckError>;
    pub fn fix_all(&self, text: &str) -> Result<String, CheckError>;
}
pub fn dialect_from_locale(tag: &str) -> Dialect;
```

`Finding.id` = `"{start_char}-{end_char}-{category}-{first 8 hex of SHA-256 of the span text}"`.
Messages:

- `TooLong`: "This selection is too long to check (over 20,000 characters). Select less text."
- `StaleFinding`: "That suggestion no longer fits the text. Check again."
- `NoSuchSuggestion`: "That suggestion is not available any more."

JSON output of a finding:
`{"id": "4-8-spelling-1f2a3b4c", "category": "spelling", "harper_kind": "Spelling", "message":
"…", "span": {"chars": [4, 8], "bytes": [4, 8], "utf16": [4, 8]}, "suggestions": [{"kind":
"replace", "text": "…"}]}`.

## Out of scope

- The personal dictionary in Harper: module 08.
- Non-English detection: module 08.
- Any UI: module 06.

## Design notes

- Harper works in `char` (Unicode scalar value) indices; JavaScript strings are UTF-16, so the UI
  needs the third span form.
- The category mapping table (every Harper kind of the pinned version and its category) is written
  here during implementation and mirrored in docs/HANDBOOK.md.
- `Remove` and `InsertAfter` are applied at the span end and span respectively, per Harper's
  suggestion semantics at the pinned version.
- Further decisions are filled in here during implementation.

## Acceptance tests

Golden cases in `tests/cases/01-harper-check/`; code tests in
`crates/redpen-core/tests/01_harper_check.rs`. Expected Harper findings come from the reference run
recorded in `REFERENCE.md` (Harper's own linter at the pinned version), never from Redpen.

1. For each of at least 30 corpus sentences, `redpen check --dialect american` gives the findings of
   the reference run: same spans, same Harper kind, same first suggestion; categories per the
   mapping table.
2. Text `👍 teh cat` (the thumbs-up is one scalar, 4 bytes, 2 UTF-16 units): the finding on `teh`
   has spans chars [2, 5], bytes [5, 8], utf16 [3, 6].
3. Text `cafe` + U+0301 + ` teh`: the finding on `teh` has chars [6, 9], bytes [7, 10], utf16
   [6, 9].
4. Text with a four-person family emoji (7 scalars joined by ZWJ, 25 bytes, 11 UTF-16 units) then
   ` teh`: chars [8, 11], bytes [26, 29], utf16 [12, 15].
5. Every Harper kind maps to a category: the test enumerates the table in the Design notes and
   checks each against `categories::map`.
6. With `--disable spelling`, the corpus sentence with a misspelling has no Spelling finding and
   keeps its other findings.
7. `--dialect british` and `--dialect american` on the corpus's dialect sentences give the
   reference run's result for each dialect.
8. `dialect_from_locale`: `en-GB` → British, `en-US` → American, `en-AU` → Australian, `en-CA` →
   Canadian, `fr-FR` → American, `` (empty) → American.
9. `apply` with a Replace suggestion replaces exactly the span; checking the result has no finding
   at that span.
10. `apply` with a finding from an earlier version of the text (span text changed) →
    `StaleFinding` with its message.
11. `apply` with suggestion index 5 on a finding with one suggestion → `NoSuchSuggestion` with its
    message.
12. `fix_all(fix_all(x)) == fix_all(x)` for every corpus sentence, and for 256 generated strings
    (proptest, seed 1) mixing ASCII words, corpus errors and the unicode fixture's characters.
13. 20,000 characters → findings; 20,001 → `TooLong` with its message (exit 2 from the CLI).
14. `redpen check` on an empty file → `{"findings": []}`, exit 0.
15. `scripts/gates.sh check-speed` reports the median for a 1,000-word text; the first run records
    it in Design notes and fails above 50 ms.

## Status

Planned
