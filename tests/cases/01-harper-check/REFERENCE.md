# Reference run for module 01 (Harper check)

The expected Harper findings of every golden case in this folder, and of `corpus.jsonl`, come
from Harper's own command-line linter at the version Redpen pins, never from Redpen.

## Version and command

- `harper-cli 3.0.0`, built from the Harper repository at tag `v3.0.0`, commit
  `dd536c3009d8f4f8e8e28841fe3115dc944d67bb` (the source of `harper-core = "=3.0.0"`):

  ```sh
  git clone --depth 1 --branch v3.0.0 https://github.com/automattic/harper target/harper-ref
  cargo build --release -p harper-cli --manifest-path target/harper-ref/Cargo.toml
  ```

- Each text is written, as UTF-8 with no trailing newline, to its own `.txt` file and run with

  ```sh
  harper-cli lint --format json --dialect <us|gb|au|ca|in> \
    --user-dict-path <a file that does not exist> --file-dict-path <an empty folder> <text>.txt
  ```

  so no personal Harper dictionary can change the result. `--dialect` maps `american` to `us`,
  `british` to `gb`, `australian` to `au`, `canadian` to `ca` and `indian` to `in`.

## From the reference output to the expected files

- **Findings** are the reference's lints for the text (overlapping lints already removed, as
  `harper-cli` does by default), ordered by start, then end (in characters), then kind name.
- **`harper_kind`** is the reference's `kind` (Harper's display name, such as `Word Choice`).
- **`category`** is the spec's category table (Design notes) applied to that kind.
- **`span.chars`** is the reference's `char_start` and `char_end`; `span.bytes` and
  `span.utf16` are the same boundaries counted in UTF-8 bytes and UTF-16 code units of the text
  (true by the encodings' definitions).
- **`id`** is `{start_char}-{end_char}-{category}-{first 8 hex of SHA-256 of the span text}`, with
  SHA-256 per FIPS 180-4 over the span text's UTF-8 bytes.
- **Suggestions** map from the reference's printed form: `Replace with: “x”` to
  `{"kind": "replace", "text": "x"}`, `Insert “x”` to `{"kind": "insert_after", "text": "x"}` and
  `Remove error` to `{"kind": "remove", "text": ""}`, in the reference's order.
- **fix-all** results apply the first suggestion of every reference finding, right to left
  (Harper's semantics: replace the span, insert after its end, delete the span), and run the
  reference again on the result, until it reports no finding with a suggestion (at most 10
  rounds). Every intermediate text and its reference result is listed below.
- **`corpus.jsonl`**: one JSON object per sentence: `id`, `text`, the `dialect` in which its
  known error is an error, the known `error` (span text), its `category`, `harper_kind`,
  `chars`, first `suggestion` and the sentence with that suggestion applied (`corrected`);
  `reference` holds every reference finding per dialect run, and `fixed` the fix-all result per
  dialect. `use` marks the dialect sentences (`dialect`) and the sentence with two errors
  (`disable`).

## Golden cases

| Case | Acceptance bullet | Command |
|---|---|---|
| `b01-c01` | 1 | `redpen check --dialect american input.txt` |
| `b01-c02` | 1 | `redpen check --dialect american input.txt` |
| `b01-c03` | 1 | `redpen check --dialect american input.txt` |
| `b01-c04` | 1 | `redpen check --dialect american input.txt` |
| `b01-c05` | 1 | `redpen check --dialect american input.txt` |
| `b01-c06` | 1 | `redpen check --dialect american input.txt` |
| `b01-c07` | 1 | `redpen check --dialect american input.txt` |
| `b01-c08` | 1 | `redpen check --dialect american input.txt` |
| `b01-c09` | 1 | `redpen check --dialect american input.txt` |
| `b01-c10` | 1 | `redpen check --dialect american input.txt` |
| `b01-c11` | 1 | `redpen check --dialect american input.txt` |
| `b01-c12` | 1 | `redpen check --dialect american input.txt` |
| `b01-c13` | 1 | `redpen check --dialect american input.txt` |
| `b01-c14` | 1 | `redpen check --dialect american input.txt` |
| `b01-c15` | 1 | `redpen check --dialect american input.txt` |
| `b01-c16` | 1 | `redpen check --dialect american input.txt` |
| `b01-c17` | 1 | `redpen check --dialect american input.txt` |
| `b01-c18` | 1 | `redpen check --dialect american input.txt` |
| `b01-c19` | 1 | `redpen check --dialect american input.txt` |
| `b01-c20` | 1 | `redpen check --dialect american input.txt` |
| `b01-c21` | 1 | `redpen check --dialect american input.txt` |
| `b01-c22` | 1 | `redpen check --dialect american input.txt` |
| `b01-c23` | 1 | `redpen check --dialect american input.txt` |
| `b01-c24` | 1 | `redpen check --dialect american input.txt` |
| `b01-c25` | 1 | `redpen check --dialect american input.txt` |
| `b01-c26` | 1 | `redpen check --dialect american input.txt` |
| `b01-c27` | 1 | `redpen check --dialect american input.txt` |
| `b01-c28` | 1 | `redpen check --dialect american input.txt` |
| `b01-c29` | 1 | `redpen check --dialect american input.txt` |
| `b01-c30` | 1 | `redpen check --dialect american input.txt` |
| `b01-c31` | 1 | `redpen check --dialect american input.txt` |
| `b01-c32` | 1 | `redpen check --dialect american input.txt` |
| `b01-c33` | 1 | `redpen check --dialect american input.txt` |
| `b01-c34` | 1 | `redpen check --dialect american input.txt` |
| `b01-c35` | 1 | `redpen check --dialect american input.txt` |
| `b01-c36` | 1 | `redpen check --dialect american input.txt` |
| `b01-c37` | 1 | `redpen check --dialect american input.txt` |
| `b01-c38` | 1 | `redpen check --dialect american input.txt` |
| `b01-c39` | 1 | `redpen check --dialect american input.txt` |
| `b01-c40` | 1 | `redpen check --dialect american input.txt` |
| `b01-c41` | 1 | `redpen check --dialect american input.txt` |
| `b01-c42` | 1 | `redpen check --dialect american input.txt` |
| `b01-c43` | 1 | `redpen check --dialect american input.txt` |
| `b07-d01-american` | 7 | `redpen check --dialect american input.txt` |
| `b07-d01-british` | 7 | `redpen check --dialect british input.txt` |
| `b07-d02-american` | 7 | `redpen check --dialect american input.txt` |
| `b07-d02-british` | 7 | `redpen check --dialect british input.txt` |
| `b07-d03-american` | 7 | `redpen check --dialect american input.txt` |
| `b07-d03-british` | 7 | `redpen check --dialect british input.txt` |
| `b07-d04-american` | 7 | `redpen check --dialect american input.txt` |
| `b07-d04-british` | 7 | `redpen check --dialect british input.txt` |
| `b06-m01-enabled` | 6 | `redpen check --dialect american input.txt` |
| `b06-m01-disable-spelling` | 6 | `redpen check --dialect american --disable spelling input.txt` |
| `b06-m01-disable-spelling-and-grammar` | 6 | `redpen check --dialect american --disable spelling --disable grammar input.txt` |
| `b02-thumbs-up` | 2 | `redpen check --dialect american input.txt` |
| `b03-combining-acute` | 3 | `redpen check --dialect american input.txt` |
| `b04-family-zwj` | 4 | `redpen check --dialect american input.txt` |
| `dialect-australian-d01` | Scope: every dialect | `redpen check --dialect australian input.txt` |
| `dialect-canadian-d01` | Scope: every dialect | `redpen check --dialect canadian input.txt` |
| `dialect-indian-c42` | Scope: every dialect | `redpen check --dialect indian input.txt` |
| `dialect-indian-d04` | Scope: every dialect | `redpen check --dialect indian input.txt` |
| `fix-all-m01` | Scope: `fix-all` | `redpen fix-all --dialect american input.txt` |
| `fix-all-c23` | Scope: `fix-all` | `redpen fix-all --dialect american input.txt` |
| `fix-all-d02-british` | Scope: `fix-all` | `redpen fix-all --dialect british input.txt` |
| `b13-too-long` | 13 | `redpen check --dialect american input.txt` |
| `b14-empty` | 14 | `redpen check input.txt` |

`b13-too-long` expects exit 2 and the error as `{"error": {"kind": "TooLong", "message": "…",
"chars": 20001}}`, the `{kind, message, ...}` form of errors in docs/ARCHITECTURE.md §5.

The code tests in `crates/redpen-core/tests/01_harper_check.rs` read `corpus.jsonl` and
`unicode.txt` and cover bullets 1 to 14 again through the library; bullet 15 is checked there
through the gate scripts and the spec's Design notes.

## Reference findings per sentence

Each entry: Harper kind, character span, span text, message, and the first suggestion as Harper
prints it.

### c01, american

`I went to teh store yesterday.`

- Typo [10, 13] `teh`: Did you mean the definite article? First suggestion: Replace with: “the”

### c02, american

`She will recieve the letter tomorrow.`

- Spelling [9, 16] `recieve`: Did you mean to spell `recieve` this way? First suggestion: Replace with: “receive” (3 suggestions: Replace with: “receive”; Replace with: “relieve”; Replace with: “recipe”)

### c03, american

`The weather is definately improving.`

- Spelling [15, 25] `definately`: Did you mean to spell `definately` this way? First suggestion: Replace with: “definitely” (3 suggestions: Replace with: “definitely”; Replace with: “defiantly”; Replace with: “delicately”)

### c04, american

`Please seperate the papers into piles.`

- Spelling [7, 15] `seperate`: Did you mean to spell `seperate` this way? First suggestion: Replace with: “separate” (3 suggestions: Replace with: “separate”; Replace with: “sewerage”; Replace with: “separated”)

### c05, american

`This affects everyone, irregardless of age.`

- Spelling [23, 35] `irregardless`: Did you mean `regardless`? First suggestion: Replace with: “regardless”

### c06, american

`My freind lives near the river.`

- Spelling [3, 9] `freind`: Did you mean to spell `freind` this way? First suggestion: Replace with: “friend” (3 suggestions: Replace with: “friend”; Replace with: “fend”; Replace with: “fiend”)

### c07, american

`This sentence has a mispelled word.`

- Spelling [20, 29] `mispelled`: Did you mean to spell `mispelled` this way? First suggestion: Replace with: “misspelled” (3 suggestions: Replace with: “misspelled”; Replace with: “dispelled”; Replace with: “miscalled”)

### c08, american

`Each and everyone of them agreed.`

- BoundaryError [0, 17] `Each and everyone`: Use `each and every one` for referring to a group of people or things. First suggestion: Replace with: “Each and every one”

### c09, american

`Alot of people came to the show.`

- BoundaryError [0, 4] `Alot`: `a lot` should be written as two words. First suggestion: Replace with: “A lot”

### c10, american

`She don't like coffee.`

- Agreement [4, 9] `don't`: The form of the verb must agree in grammatical number with the pronoun. First suggestion: Replace with: “doesn't”

### c11, american

`The cat licked it's paw.`

- Agreement [15, 19] `it's`: Use the possessive pronoun `its` (without an apostrophe) to show ownership. The word `it's` (with an apostrophe) is a contraction of 'it is' or 'it has' and should not be used to indicate possession. First suggestion: Replace with: “its”

### c12, american

`Their going to the park later.`

- Grammar [0, 5] `Their`: Did you mean `they're`? First suggestion: Replace with: “They're” (2 suggestions: Replace with: “They're”; Replace with: “They're ”)

### c13, american

`I have went there before.`

- Grammar [7, 11] `went`: Use the past participle `gone` instead of `went` when using compound tenses or passive voice. First suggestion: Replace with: “gone”

### c14, american

`The book was wrote last year.`

- Grammar [13, 18] `wrote`: Use the past participle `written` instead of `wrote` when using compound tenses or passive voice. First suggestion: Replace with: “written”

### c15, american

`He did it on accident.`

- Usage [10, 12] `on`: Did you mean `by accident`? First suggestion: Replace with: “by”

### c16, american

`This is suppose to be easy.`

- Usage [8, 18] `suppose to`: Did you mean `supposed to`? First suggestion: Replace with: “supposed to”

### c17, american

`We need to discuss about the budget.`

- Usage [11, 24] `discuss about`: `About` is redundant First suggestion: Replace with: “discuss”

### c18, american

`For all intensive purposes, the work is done.`

- Nonstandard [0, 26] `For all intensive purposes`: The correct form is 'for all intents and purposes'. First suggestion: Replace with: “For all intents and purposes” (2 suggestions: Replace with: “For all intents and purposes”; Replace with: “To all intents and purposes”)

### c19, american

`The submarine went bellow the surface.`

- Malapropism [19, 25] `bellow`: `Bellow` means to shout. You probably mean `below`. First suggestion: Replace with: “below”

### c20, american

`He was the one who eluded to the problem.`

- Malapropism [19, 28] `eluded to`: Did you mean `alluded to`? First suggestion: Replace with: “alluded to”

### c21, american

`The captain decided to change tact quickly.`

- Eggcorn [30, 34] `tact`: A change in direction or approach is a change of `tack`. Not `tact` (or `tacks` or `tacts`). First suggestion: Replace with: “tack”

### c22, american

`We waited with baited breath.`

- Eggcorn [15, 28] `baited breath`: Did you mean `bated breath`? First suggestion: Replace with: “bated breath”

### c23, american

`The the dog barked loudly.`

- Repetition [0, 7] `The the`: Did you mean to repeat this word? First suggestion: Replace with: “The”

### c24, american

`The results were very very good.`

- Repetition [17, 26] `very very`: Did you mean to repeat this word? First suggestion: Replace with: “very”

### c25, american

`This is an useful tool.`

- Miscellaneous [8, 10] `an`: Incorrect indefinite article. First suggestion: Replace with: “a”

### c26, american

`He is a honest worker.`

- Miscellaneous [6, 7] `a`: Incorrect indefinite article. First suggestion: Replace with: “an”

### c27, american

`The app works well and no longer I need a manual.`

- Word Order [23, 34] `no longer I`: Consider moving the pronoun to before `no longer`. First suggestion: Replace with: “I no longer”

### c28, american

`Its a beautiful day outside.`

- Punctuation [0, 3] `Its`: Use `it's` (short for `it has` or `it is`) here, not the possessive `its`. First suggestion: Replace with: “It's” (2 suggestions: Replace with: “It's”; Replace with: “It has”)

### c29, american

`We had a really good time,thanks to you.`

- Punctuation [25, 26] `,`: Use a space after a comma. First suggestion: Insert “ ”

### c30, american

`We bought bread , milk and eggs.`

- Punctuation [15, 16] ` `: Don't use a space before a comma. First suggestion: Remove error

### c31, american

`i think the plan will work.`

- Capitalization [0, 1] `i`: The first-person singular subject pronoun must be capitalized. First suggestion: Replace with: “I”

### c32, american

`It is a  long road.`

- Formatting [7, 9] `  `: There are 2 spaces where there should be only one. First suggestion: Replace with: “ ”

### c33, american

`Where are you going ?`

- Formatting [19, 20] ` `: Unnecessary space at the end of the sentence. First suggestion: Remove error

### c34, american

`This puzzle game is very addicting.`

- Style [25, 34] `addicting`: When used as an adjective, `addictive` is the traditional and more f form. First suggestion: Replace with: “addictive”

### c35, american

`I could of done it better.`

- Word Choice [2, 10] `could of`: Use `have` rather than `of` here. First suggestion: Replace with: “could have”

### c36, american

`The new policy will effect everyone.`

- Word Choice [20, 26] `effect`: Use `affect` for the verb meaning to influence; `effect` usually names the result. First suggestion: Replace with: “affect”

### c37, american

`She is very unique in her approach.`

- Word Choice [7, 18] `very unique`: `Unique` is absolute, so consider using `unique` alone or a more precise adjective such as `special`, `rare`, or `unusual`. First suggestion: Replace with: “very special” (4 suggestions: Replace with: “very special”; Replace with: “very rare”; Replace with: “very unusual”; Replace with: “unique”)

### c38, american

`He is more taller than his brother.`

- Redundancy [6, 17] `more taller`: Using `more` and the comparative form of the adjective together is redundant. First suggestion: Replace with: “taller”

### c39, american

`The meeting is at 3 PM in the afternoon.`

- Redundancy [19, 39] ` PM in the afternoon`: The time period is redundant because it repeats what's already specified by the AM/PM indicator. First suggestion: Replace with: “ PM” (2 suggestions: Replace with: “ PM”; Replace with: “ in the afternoon”)

### c40, american

`Please write your name in the box, and then sign it, and then date it, and then hand it to the clerk at the front desk who will check it and stamp it and file it in the cabinet next to the window before the office closes for the day.`

- Readability [0, 233] `Please write your name in the box, and then sign it, and then date it, and then hand it to the clerk at the front desk who will check it and stamp it and file it in the cabinet next to the window before the office closes for the day.`: This sentence is 50 words long. First suggestion: (no suggestion)

### c41, american

`I cant go to the store today.`

- Enhancement [2, 6] `cant`: `Cant` is secret language or jargon. If that's not what you mean you should use `can't` here. First suggestion: Replace with: “can't”

### c42, american

`These are all the results till date.`

- Regionalism [26, 30] `till`: Outside Indian English, prefer `to date`. First suggestion: Replace with: “to”

### c43, american

`He threw the ball out of the window.`

- Regionalism [21, 24] ` of`: If this is the idiom about abandoning a plan, it's more usual to leave out `of` in American English First suggestion: Remove error

### d01, american

`The colour of the sky changed at dusk.`

- Spelling [4, 10] `colour`: Did you mean to spell `colour` this way? First suggestion: Replace with: “color” (3 suggestions: Replace with: “color”; Replace with: “cloud”; Replace with: “collar”)

### d01, british

`The colour of the sky changed at dusk.`

- no findings

### d02, american

`The color of the sky changed at dusk.`

- no findings

### d02, british

`The color of the sky changed at dusk.`

- Spelling [4, 9] `color`: Did you mean to spell `color` this way? First suggestion: Replace with: “colour” (3 suggestions: Replace with: “colour”; Replace with: “colon”; Replace with: “clog”)

### d03, american

`I think a new job is on the cards.`

- Regionalism [21, 23] `on`: Use `in the cards` instead of `on the cards` in American English. First suggestion: Replace with: “in”

### d03, british

`I think a new job is on the cards.`

- no findings

### d04, american

`Please have a look at the report.`

- Regionalism [7, 11] `have`: American English prefers `take a look` over `have a look`. First suggestion: Replace with: “take”

### d04, british

`Please have a look at the report.`

- no findings

### m01, american (misspelling plus a grammar error)

`She will recieve an book tomorrow.`

- Spelling [9, 16] `recieve`: Did you mean to spell `recieve` this way? First suggestion: Replace with: “receive” (3 suggestions: Replace with: “receive”; Replace with: “relieve”; Replace with: “recipe”)
- Miscellaneous [17, 19] `an`: Incorrect indefinite article. First suggestion: Replace with: “a”

### d01, australian

`The colour of the sky changed at dusk.`

- no findings

### d01, canadian

`The colour of the sky changed at dusk.`

- no findings

### c42, indian

`These are all the results till date.`

- no findings

### d04, indian

`Please have a look at the report.`

- Regionalism [7, 11] `have`: Indian English prefers `take a look` over `have a look`. First suggestion: Replace with: “take”

## Unicode spans (bullets 2 to 4), American

### `b02-thumbs-up`

thumbs-up (U+1F44D: one scalar, 4 bytes, 2 UTF-16 units), then ` teh cat`. Code points: U+1F44D U+0020 U+0074 U+0065 U+0068 U+0020 U+0063 U+0061 U+0074.

- Typo [2, 5] `teh`: Did you mean the definite article? First suggestion: Replace with: “the”. Bytes [5, 8], UTF-16 [3, 6].

### `b03-combining-acute`

`cafe`, U+0301 COMBINING ACUTE ACCENT, then ` teh`. Code points: U+0063 U+0061 U+0066 U+0065 U+0301 U+0020 U+0074 U+0065 U+0068.

- Typo [6, 9] `teh`: Did you mean the definite article? First suggestion: Replace with: “the”. Bytes [7, 10], UTF-16 [6, 9].

### `b04-family-zwj`

family emoji (U+1F468 U+200D U+1F469 U+200D U+1F467 U+200D U+1F466: 7 scalars, 25 bytes, 11 UTF-16 units), then ` teh`. Code points: U+1F468 U+200D U+1F469 U+200D U+1F467 U+200D U+1F466 U+0020 U+0074 U+0065 U+0068.

- Typo [8, 11] `teh`: Did you mean the definite article? First suggestion: Replace with: “the”. Bytes [26, 29], UTF-16 [12, 15].

### Each `unicode.txt` sample in a sentence

`<sample> I went to teh store with <sample> yesterday.`, American; the code test checks that
`teh` is flagged and that every finding's byte and UTF-16 spans match its character span.

- line 1: Typo [12, 15], first suggestion Replace with: “the”
- line 2: Typo [18, 21], first suggestion Replace with: “the”
- line 3: Typo [15, 18], first suggestion Replace with: “the”
- line 4: Typo [13, 16], first suggestion Replace with: “the”
- line 5: Capitalization [0, 4], first suggestion Replace with: “Cafe”; Typo [16, 19], first suggestion Replace with: “the”
- line 6: Capitalization [0, 1], first suggestion Replace with: “N”; Typo [13, 16], first suggestion Replace with: “the”
- line 7: Capitalization [0, 1], first suggestion Replace with: “A”; Typo [13, 16], first suggestion Replace with: “the”
- line 8: Typo [14, 17], first suggestion Replace with: “the”
- line 9: Typo [13, 16], first suggestion Replace with: “the”
- line 10: Typo [14, 17], first suggestion Replace with: “the”
- line 11: Typo [12, 15], first suggestion Replace with: “the”
- line 12: Typo [12, 15], first suggestion Replace with: “the”
- line 13: Typo [12, 15], first suggestion Replace with: “the”
- line 14: Capitalization [0, 1], first suggestion Replace with: “é”; Typo [12, 15], first suggestion Replace with: “the”; Spelling [27, 28], first suggestion Replace with: “I”
- line 15: Typo [12, 15], first suggestion Replace with: “the”

## fix-all

### `fix-all-m01` (m01, american)

- start: `She will recieve an book tomorrow.`
- after round 1: `She will receive a book tomorrow.`, reference: no findings
- result: `She will receive a book tomorrow.`

### `fix-all-c23` (c23, american)

- start: `The the dog barked loudly.`
- after round 1: `The dog barked loudly.`, reference: no findings
- result: `The dog barked loudly.`

### `fix-all-d02-british` (d02, british)

- start: `The color of the sky changed at dusk.`
- after round 1: `The colour of the sky changed at dusk.`, reference: no findings
- result: `The colour of the sky changed at dusk.`

### The corpus

Each sentence's `fixed` value in `corpus.jsonl`, reached the same way: every round's text, and
the reference's result on the last one (no finding with a suggestion).

- c01, american: round 1 `I went to the store yesterday.`; reference: no findings
- c02, american: round 1 `She will receive the letter tomorrow.`; reference: no findings
- c03, american: round 1 `The weather is definitely improving.`; reference: no findings
- c04, american: round 1 `Please separate the papers into piles.`; reference: no findings
- c05, american: round 1 `This affects everyone, regardless of age.`; reference: no findings
- c06, american: round 1 `My friend lives near the river.`; reference: no findings
- c07, american: round 1 `This sentence has a misspelled word.`; reference: no findings
- c09, american: round 1 `A lot of people came to the show.`; reference: no findings
- c10, american: round 1 `She doesn't like coffee.`; reference: no findings
- c11, american: round 1 `The cat licked its paw.`; reference: no findings
- c12, american: round 1 `They're going to the park later.`; reference: no findings
- c13, american: round 1 `I have gone there before.`; reference: no findings
- c14, american: round 1 `The book was written last year.`; reference: no findings
- c15, american: round 1 `He did it by accident.`; reference: no findings
- c16, american: round 1 `This is supposed to be easy.`; reference: no findings
- c17, american: round 1 `We need to discuss the budget.`; reference: no findings
- c18, american: round 1 `For all intents and purposes, the work is done.`; reference: no findings
- c19, american: round 1 `The submarine went below the surface.`; reference: no findings
- c20, american: round 1 `He was the one who alluded to the problem.`; reference: no findings
- c21, american: round 1 `The captain decided to change tack quickly.`; reference: no findings
- c22, american: round 1 `We waited with bated breath.`; reference: no findings
- c23, american: round 1 `The dog barked loudly.`; reference: no findings
- c24, american: round 1 `The results were very good.`, round 2 `The results were excellent.`; reference: no findings
- c25, american: round 1 `This is a useful tool.`; reference: no findings
- c26, american: round 1 `He is an honest worker.`; reference: no findings
- c27, american: round 1 `The app works well and I no longer need a manual.`; reference: no findings
- c28, american: round 1 `It's a beautiful day outside.`; reference: no findings
- c29, american: round 1 `We had a really good time, thanks to you.`; reference: no findings
- c30, american: round 1 `We bought bread, milk and eggs.`, round 2 `We bought bread, milk, and eggs.`; reference: no findings
- c31, american: round 1 `I think the plan will work.`; reference: no findings
- c32, american: round 1 `It is a long road.`; reference: no findings
- c33, american: round 1 `Where are you going?`; reference: no findings
- c34, american: round 1 `This puzzle game is very addictive.`; reference: no findings
- c35, american: round 1 `I could have done it better.`; reference: no findings
- c36, american: round 1 `The new policy will affect everyone.`; reference: no findings
- c37, american: round 1 `She is very special in her approach.`; reference: no findings
- c38, american: round 1 `He is taller than his brother.`; reference: no findings
- c39, american: round 1 `The meeting is at 3 PM.`; reference: no findings
- c40, american: no round (nothing to apply); reference: Readability [0, 233] with no suggestion
- c41, american: round 1 `I can't go to the store today.`; reference: no findings
- c42, american: round 1 `These are all the results to date.`; reference: no findings
- c43, american: round 1 `He threw the ball out the window.`; reference: no findings
- d01, american: round 1 `The color of the sky changed at dusk.`; reference: no findings
- d01, british: no round (nothing to apply); reference: no findings
- d02, american: no round (nothing to apply); reference: no findings
- d02, british: round 1 `The colour of the sky changed at dusk.`; reference: no findings
- d03, american: round 1 `I think a new job is in the cards.`; reference: no findings
- d03, british: no round (nothing to apply); reference: no findings
- d04, american: round 1 `Please take a look at the report.`; reference: no findings
- d04, british: no round (nothing to apply); reference: no findings
- m01, american: round 1 `She will receive a book tomorrow.`; reference: no findings

The reference never settles on c08 (american): its rules correct each other in a cycle, so
`fixed` is `null` there and only `fix_all(fix_all(x)) == fix_all(x)` is checked.

- `Each and everyone of them agreed.`: BoundaryError [0, 17] `Each and everyone`, Replace with: “Each and every one”
- `Each and every one of them agreed.`: Miscellaneous [9, 18] `every one`, Replace with: “everyone”
- `Each and everyone of them agreed.`: BoundaryError [0, 17] `Each and everyone`, Replace with: “Each and every one”

## Length limit (bullet 13)

- 20,000 characters: `I went to teh store yesterday. ` repeated 645 times (19,995 characters), then
  `Done.`. Reference: 645 findings (645 Typo).
- The same text ending `Déjà.` instead of `Done.` (20,000 characters, 20,002 UTF-8 bytes).
  Reference: 646 findings (645 Typo, 1 Spelling).
- 20,001 characters: the first text followed by `!` (`b13-too-long`); refused before any check,
  so it has no reference run.

## Empty text (bullet 14)

The reference on an empty file: no findings.

## unicode.txt

One sample per line, written for this repository (CC0): emoji with ZWJ sequences and modifiers,
combining marks, CJK and Hangul, and astral characters.

| Line | Code points |
|---|---|
| 1 | U+1F44D |
| 2 | U+1F468 U+200D U+1F469 U+200D U+1F467 U+200D U+1F466 |
| 3 | U+1F3F3 U+FE0F U+200D U+1F308 |
| 4 | U+1F44B U+1F3FD |
| 5 | U+0063 U+0061 U+0066 U+0065 U+0301 |
| 6 | U+006E U+0303 |
| 7 | U+0061 U+0308 |
| 8 | U+65E5 U+672C U+8A9E |
| 9 | U+4E2D U+6587 |
| 10 | U+D55C U+AD6D U+C5B4 |
| 11 | U+1D49C |
| 12 | U+1F600 |
| 13 | U+20BB7 |
| 14 | U+00E9 |
| 15 | U+03A9 |
