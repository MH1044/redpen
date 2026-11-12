# 01: Harper check

## Goal

Any text can be checked on this computer, with no provider and no network: Harper's findings come
back with a user-facing category, a message, suggestions, and spans in characters, bytes and UTF-16
code units, in the chosen English dialect. A fix can be applied and the text re-checked.
`redpen check` exposes it for golden tests; the popup (module 06) uses it.

## Depends on

00.

## Scope

- `Checker` wraps `harper-core` (pinned `=3.0.0`): a curated lint group for the dialect over a
  plain-English document, built exactly as Harper's own linter builds it for a `.txt` file
  (Design notes), so its findings equal the reference run's.
- Each Harper lint becomes a `Finding` with an id, category, Harper's kind name, message, spans and
  suggestions.
- Harper's kinds map to four categories, Spelling, Grammar, Punctuation and Style, by an exhaustive
  `match` with no wildcard arm, so a new Harper kind fails to compile until it is mapped.
- Spans: Harper's character indices converted to byte offsets and UTF-16 offsets.
- Dialects: every value of the pinned version's `Dialect`: American, British, Australian, Canadian
  and Indian in harper-core 3.0.0; `dialect_from_locale` maps `en-GB`, `en-US`, `en-AU`, `en-CA`,
  `en-IN`, anything else American.
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
- `crates/redpen-cli/src/cmd/{mod.rs, check.rs, fix_all.rs, check_speed.rs}`
- `scripts/gates.sh`, `scripts/gates.ps1` (created here with the `check-speed` gate)
- `THIRD_PARTY.md` (`harper-core`, `unicode-segmentation`)
- Tests: `crates/redpen-core/tests/01_harper_check.rs`; `tests/cases/01-harper-check/`
  (`corpus.jsonl`, `unicode.txt`, `REFERENCE.md`, one case directory per golden);
  `crates/redpen-cli/tests/gates.rs` (the gate scripts' refusals)

## Interfaces

```rust
pub enum Dialect { American, British, Australian, Canadian, Indian } // as Harper 3.0.0's enum
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
- Decided before implementation, from harper-core 3.0.0 (Apache-2.0, published 2026-10-26; its
  dependency tree passes `deny.toml`):
  - **The pinned version** is `harper-core = "=3.0.0"`. Its `Dialect` has five values (American,
    Canadian, Australian, British, Indian), so Redpen offers all five; `--dialect` takes
    `american`, `british`, `australian`, `canadian`, `indian`.
  - **How the checker is built**, the same way Harper's own linter (`harper-cli lint`) builds it for
    a `.txt` file, so the reference run and Redpen agree: the curated dictionary
    (`FstDictionary::curated()`), a plain-English document (`Document::new_plain_english`),
    `LintGroup::new_curated(dictionary, dialect)`, `organized_lints`, then overlapping lints
    removed with `remove_overlaps_map`. Findings are ordered by start, then end (in characters),
    then Harper kind name.
  - **`harper_kind`** is the kind's display name in Harper (`Spelling`, `Word Choice`, `Word Order`).
  - **Suggestions** map from Harper's `ReplaceWith` (`replace`), `InsertAfter` (`insert_after`) and
    `Remove` (`remove`) and are applied with Harper's own semantics: replace the span, insert after
    its end, or delete the span.
  - **The category table**, every `LintKind` of 3.0.0, mirrored in docs/HANDBOOK.md:

    | Category | Harper kinds |
    |---|---|
    | Spelling | Spelling, Typo, BoundaryError |
    | Grammar | Agreement, Grammar, WordOrder, Usage, Nonstandard, Malapropism, Eggcorn, Repetition, Miscellaneous |
    | Punctuation | Punctuation, Capitalization, Formatting |
    | Style | Style, WordChoice, Redundancy, Readability, Enhancement, Regionalism |

    Miscellaneous holds corrections such as a/an, so it counts as Grammar; Capitalization and
    Formatting sit with Punctuation as the mechanics of the text.
  - **The reference run**: `harper-cli` is not published on crates.io, so it is built from the Harper
    repository at tag `v3.0.0` (commit `dd536c3009d8f4f8e8e28841fe3115dc944d67bb`):
    `git clone --depth 1 --branch v3.0.0 https://github.com/automattic/harper target/harper-ref`,
    then `cargo build --release -p harper-cli --manifest-path target/harper-ref/Cargo.toml`
    (`target/` is ignored by git). Each corpus sentence is written to a `.txt` file and run with
    `harper-cli lint --format json --dialect <us|gb|au|ca|in> --user-dict-path <a file that does not exist>
    --file-dict-path <an empty folder> <file>.txt`, so no personal Harper dictionary can change the
    result. `REFERENCE.md` records the version, the commit, that command with placeholder paths, and
    for every sentence each finding's kind, character span, message and first suggestion. Harper
    prints suggestions as `Replace with: “…”`, `Insert “…”` and `Remove error`.
- Decided during implementation:
  - **The dictionary** is a `MergedDictionary` whose one child is `FstDictionary::curated()`, as
    `harper-cli` builds it (with no user or file dictionary, which the reference run has none of);
    module 08 adds the personal dictionary as a second child. Merging one child changes no lookup
    and no suggestion order (both sort by edit distance, then word). `harper-core` keeps its
    default features (`thesaurus`, `concurrent`), as `harper-cli` uses it.
  - **One `LintGroup` per `Checker`**, built once and kept behind a mutex, because
    `organized_lints` takes `&mut self`. Its cache is keyed by each chunk's characters and the rule
    configuration, so a reused checker gives the same findings as a fresh one; the code tests run
    many texts through one checker and match the reference. A check that panics poisons the mutex;
    the next check rebuilds the group.
  - **Input is taken as it is.** `redpen check` passes the file's text to Harper unchanged (no line
    ending or byte-order mark removed), as `harper-cli` reads a `.txt` file, and refuses a file
    that is not UTF-8.
  - **The limit counts chars** (Unicode scalar values, Harper's unit): 20,000 pass, 20,001 do not.
    `check`, `apply` and `fix_all` all refuse a longer text. If the fixes of a `fix_all` round
    would push the text over the limit, that round is not kept: `fix_all` returns the text the
    round started from, the last one within the limit, so its result can always be checked again
    and `fix_all` of it returns it unchanged. A 20,000-character text whose only lengthening fix
    is `cant` to `can't` therefore comes back as it is.
  - **Disabled categories are dropped after `remove_overlaps_map`**, so the findings left are
    exactly those of a check with nothing disabled, minus the disabled ones. A finding hidden by
    an overlapping finding of a disabled category is not brought back.
  - **Order**: a stable sort by start, end, then Harper kind name, over `organized_lints`' rule-name
    order, so equal keys keep the order Harper gives them.
  - **Staleness**: `apply` compares the SHA-256 prefix at the end of the finding's id with the hash
    of the text now under the span (and refuses a span that no longer fits). Same characters at the
    same place means the finding still fits, whatever changed elsewhere. Staleness is checked
    before the suggestion index. A suggestion is converted back to Harper's `Suggestion` and
    applied with Harper's own `Suggestion::apply`.
  - **`fix_all` rounds** apply first suggestions right to left by span; findings without a
    suggestion are left alone, and a span reaching into one already applied in the round is
    skipped. Rounds stop when nothing is left to apply, after 10 rounds, when a round would pass
    the limit (above), or when a round gives a text an earlier round gave (fixes that undo each
    other). In the last case the repeated text is returned, so `fix_all` of the result returns it
    again.
  - **`dialect_from_locale`** reads BCP 47 and POSIX tags alike: case does not matter, `_`
    separates like `-`, an encoding or modifier (`en_GB.UTF-8`, `@euro`) is ignored, and a script
    subtag (`en-Latn-AU`) is skipped. The language must be English, so `fr-CA` and `hi-IN` give
    American.
  - **`CheckError` serialises** as `{"kind", "message"}` plus `chars` for `TooLong`, the shape of
    docs/ARCHITECTURE.md §5. The command line prints a refusal as `{"error": {...}}` on standard
    output, so a golden case can compare it, repeats the message on standard error, and exits 2.
    Usage errors (an unknown dialect or category, a missing or unreadable file) go to standard
    error only, also with exit 2. Without `--dialect` the command line checks American English: the
    operating system's locale would make goldens depend on the machine. `fix-all` takes no
    `--disable`, as specified, and fixes every category.
  - **Harper's kind names**: `harper_kind` comes from `LintKind`'s `Display`. Redpen never parses
    a kind name back, because in 3.0.0 `LintKind::from_string_key` has no arm for `WordOrder`.
  - **The `check-speed` gate** is `scripts/gates.sh check-speed` (and `gates.ps1`), which runs
    `redpen check-speed` from a release build: the command line also serves the maintainer's
    scripts. The text joins the corpus's `text` fields with spaces, repeated, cut just after the
    1,000th UAX #29 word (`unicode-segmentation`, the word count Redpen uses elsewhere). One
    untimed check loads the dictionary; each of the 10 runs then builds a fresh checker, untimed,
    and times one check, so Harper's chunk cache never serves a run. That is the wait for a new
    selection. The gate also reports, for information, the median of checking the same text again
    on one checker (as the popup does after a fix), where the cache serves unchanged chunks.
  - **First check-speed runs** (release build, Windows 11 x64, 2026-11-12; a 1,000-word text of
    5,173 characters with 137 findings): check-speed median **71.3 ms** for a new text (runs 69.1
    to 78.1 ms), **above the 50 ms limit, so the gate fails**; checking the same text again 23.8 ms.
    The very first run, on a busy machine, gave 85.8 ms. Profiling shows the
    time is inside Harper: about 6 ms to build the lint group (untimed here), 1.6 ms to parse the
    document, and about 74 ms in `organized_lints` with an empty cache against 27 ms with the
    text's chunks cached. Nothing in the wrapper can shorten it without changing findings. The
    corpus has an error in nearly every sentence, so the 137 findings make the text slower than
    ordinary prose. Whether to keep 50 ms for a new text, gate the re-check figure, or time a
    less error-dense text is the maintainer's decision; the gate stays at 50 ms on the new-text
    figure until then.
  - **Dependencies.** `harper-core` 3.0.0 brings `harper-brill`, `harper-pos-utils` (with the Burn
    machine-learning framework for its chunker) and `harper-thesaurus`, about 340 crates in all
    (`Cargo.lock` grew from 444 to 785 packages), every one passing `deny.toml`. A cold build takes
    a few minutes longer. `THIRD_PARTY.md` lists Harper's compiled-in tagger, chunker model,
    thesaurus and dictionary under Models. `proptest` (tests only) generates the fixed-seed strings
    of acceptance test 12.

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
   Canadian, `en-IN` → Indian, `fr-FR` → American, `` (empty) → American.
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

Done (2026-11-12)
