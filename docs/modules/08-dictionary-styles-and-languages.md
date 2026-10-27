# 08: Dictionary, styles and languages

## Goal

The user has a personal dictionary: Harper stops flagging its words, a spelling finding can be
added with one click, and the list can be managed in Settings and imported and exported as text.
Rewrites can follow a style preset (Plain English, Academic, Business). Text in other languages is
handled honestly: checks say they are English only, and rewrites keep the language.

## Depends on

01, 04, 06.

## Scope

- The personal dictionary store `dictionary.json` (docs/ARCHITECTURE.md §4): list, add, remove,
  saved atomically like `settings.json`; the Settings → Dictionary tab with add, remove, import and
  export. An entry is one word (letters, digits, apostrophes and hyphens), since Harper checks
  words.
- `Checker` takes the dictionary terms and merges them into Harper's dictionary (the pinned
  `harper-core` API for a user dictionary, recorded in Design notes); changing the dictionary
  rebuilds the checker.
- **Add to dictionary** on every Spelling finding in the popup: adds the span text as a term, then
  re-checks.
- Import and export as UTF-8 text: one word per line; blank lines and lines starting with `#`
  ignored; export sorted by term (case-insensitive), in the same format; import reports
  `{added, skipped}` with duplicates and invalid lines (more than one word) skipped.
- Style presets for Clearer, Shorter, More formal, Friendlier and Custom: the `{style}` values of
  docs/PUBLIK-INTEGRATION.md §6 (Own, Plain English, Academic, Business); a default in Settings →
  General and a selector in the popup's result view; Fix grammar ignores style. The prompt files do
  not change (the value of a placeholder does), so they stay v1; the style is part of the cache key.
- Language of the captured text by `whatlang` (at least 40 characters, reliable): non-English → the
  findings list is replaced by "Checks are available for English only. Rewrites work in your text's
  language." and `{dialect_name}` becomes `standard <Language>` (for example `standard French`).

## Files

- `crates/redpen-core/src/check/dictionary.rs`, `src/store/dictionary.rs` (store, import, export),
  `src/rewrite/styles.rs`, `src/lang.rs`
- `app/src/windows/settings/Dictionary.tsx` (list, add, remove, import, export),
  `app/src/components/StyleSelect.tsx`, `app/src/components/Findings.tsx` (Add to dictionary),
  `app/src-tauri/src/commands/dictionary.rs`
- `docs/HANDBOOK.md` (the chapter on the dictionary, presets and languages)
- Tests: `crates/redpen-core/tests/08_dictionary_styles_languages.rs`,
  `app/src/08-dictionary.test.ts`, `tests/cases/08-dictionary-styles-and-languages/`

## Interfaces

```rust
impl Checker { pub fn with_words(opts: CheckOptions, words: &[String]) -> Self; }
impl Dictionary {
    pub fn list(&self) -> &[Entry];
    pub fn add(&mut self, term: &str) -> Result<(), DictError>;
    pub fn remove(&mut self, term: &str) -> bool;
    pub fn import(&mut self, text: &str) -> ImportReport;          // {added, skipped}
    pub fn export(&self) -> String;
}
pub fn style_line(s: Style) -> &'static str;
pub fn dialect_name(d: Dialect, detected: Option<Lang>) -> String; // "British English" | "standard French"
```

Messages: "Checks are available for English only. Rewrites work in your text's language.";
dictionary "That word is already in your dictionary.", "A dictionary entry needs a word." and "A
dictionary entry is one word."; import "Added 3 words, skipped 2."; Tauri commands
`dictionary_list`, `dictionary_add`, `dictionary_remove`, `dictionary_import`, `dictionary_export`
(docs/ARCHITECTURE.md §5).

## Out of scope

- Grammar checking in languages other than English (Harper is English only at the time of writing;
  re-check before this module is implemented).
- A translated UI.

## Design notes

- Decided: presets are placeholder values, not new prompt versions, so a preset never orphans the
  cache of the others.
- Further decisions are filled in here during implementation.

## Acceptance tests

Code tests in `crates/redpen-core/tests/08_dictionary_styles_languages.rs`; goldens (`redpen check`
with a dictionary file, `redpen prompt` with `--style`) in
`tests/cases/08-dictionary-styles-and-languages/`; Vitest in `app/src/08-dictionary.test.ts`.

1. `Zorbleflax is here.` has a Spelling finding on `Zorbleflax`; with `Zorbleflax` in the
   dictionary it has none.
2. Add to dictionary on that finding → the dictionary holds `Zorbleflax` and the re-check has no
   finding at that span.
3. Importing `Redpen`, `Siobhán`, `Kubernetes`, `Redpen` (duplicate), `red pen` (two words,
   invalid), a blank line and `# note` → added 3, skipped 2, "Added 3 words, skipped 2."
4. `export` of that dictionary is exactly `Kubernetes\nRedpen\nSiobhán\n`; importing it into an
   empty dictionary gives the same entries.
5. `redpen prompt --action clearer --style plain-english` → the system message contains
   `Style: Plain English: short sentences, everyday words, active voice, no jargon.`; the same for
   Academic and Business; Fix grammar's request is identical whatever the style.
6. The cache key of Clearer differs between Own and Business for the same text.
7. A 200-character French paragraph → the English-only message instead of findings; the Clearer
   request contains `Use standard French spelling.`; an English answer to it → the language warning.
8. The style selector in the result view re-runs the action with the chosen preset (reducer test).
9. `dictionary_add` of `Redpen` twice → "That word is already in your dictionary."; an empty term →
   "A dictionary entry needs a word."; `red pen` → "A dictionary entry is one word."; after
   `dictionary_remove` of `Zorbleflax`, test 1's sentence has its Spelling finding again;
   `dictionary.json` round trips with an unknown field kept.

## Status

Planned
