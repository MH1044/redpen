# Module 04 references

Where the expected values of `tests/cases/04-rewrite-engine/` and
`crates/redpen-core/tests/04_rewrite_engine.rs` come from. None of them was taken from Redpen's own
output. Each case directory is named after the acceptance test it covers (`t01-…` is test 1).

## Request bodies (tests 1 and 2)

- `t01-prompt-fix-grammar`, `t01-prompt-clearer` and `t01-prompt-custom` expect the three example
  request bodies of docs/PUBLIK-INTEGRATION.md §6.1, §6.2 and §6.3, copied unchanged. Their inputs
  are the example texts; the Fix grammar input has `ana@example.com` where the example shows
  `[EMAIL_1]`, and runs with `--redact`.
- `t01-prompt-shorter`, `t01-prompt-more-formal` and `t01-prompt-friendlier` expect the §6.2 body
  with the system content built as §6.2 says: Clearer's system text with its first two paragraphs
  replaced by the two paragraphs §6.2 gives for that file. Model, `max_tokens` and `temperature` are
  as for Clearer.
- `t01-prompt-fix-grammar-american` expects the §6.1 body for the text `Their going to the libary.`
  with `{dialect_name}` = `American English`.
- Every system and user content is the prompt section of §6 with its placeholders filled:
  `{dialect_name}` (`British English`), `{style}` (`Keep the author's own style.`, the only style
  until module 08), `{text}` and `{instruction}`. Filling the §6 templates this way gives the three
  §6 example bodies byte for byte, which is how the other bodies were checked.
- `max_tokens` is `clamp(2 * words(text) + 100, 256, 4096)` (Custom: `+ 150`), 256 for every text
  here: the longest, the Clearer example, has 35 words (2 × 35 + 100 = 170).

## Request hashes

Module 00's rule: SHA-256 (FIPS 180-4) of `POST /chat/completions\n` followed by the canonical body
(keys sorted, no whitespace, UTF-8). Computed with Python's `hashlib` over
`json.dumps(body, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`; the same method gives
module 00's reference hash `82cab4cd…eef93` for `{"a":2,"b":1}` and module 02's `97e299ca…11a05` for
the `hi` request. Each recording's `request` field holds the body that was hashed, and the core test
checks every `request_sha256` against it.

| Cases | `request_sha256` |
|---|---|
| `t02-rewrite-fix-grammar` | `2f27bc06a0488d26f4a93c81cdc8e1b92a4e401d3d68d78a54276a218b426276` |
| `t02-rewrite-clearer` | `b4a381bd207fbba3e5d070655c29bd2e84141944731b583f6a8c71eae8cfa36c` |
| `t02-rewrite-shorter` | `52181487d9ae901ea8aa2a8e6ce1b80fa81a615160fe462a3ef856f86cf8f127` |
| `t02-rewrite-more-formal` | `5523e82f3ccf8331cdd6e1987fc0699f3f8029ef9dc425367efb320ad8977497` |
| `t02-rewrite-friendlier` | `8477214fd576292cb0480c9965c42b7676ddf7468f02c02d804c2a1d359e1195` |
| `t02-rewrite-custom` | `c981f4e95fcea04c316b71aa62707abea25e3ffe6a302b5d235cd1617e62e689` |
| `t03-invalid-*` (Fix grammar, `Their going to the libary.`) | `34724b5ae7e80b2eb211db3ba67c27e0e8c9c2ddbbe0bf3f138cca64e92a5a0b` |
| `t04-fence`, `t04-brace-fallback`, `t04-wrapping-quotes`, `t04-preamble-line` (`fine.`) | `84d415d6c635e4147d39b9633c5ba0ad2d0e63083ab934407f86a8f92a819cce` |
| `t04-whitespace`, the text as selected | `02b2911555ed84e364b85c10cfdc0494b852bcff6deda48f5329601bb37e1c76` |
| `t04-whitespace`, the text trimmed | `54a9b6d61613902c87c6e4c53456bc069d1009cafd5b618b59c8a02a23af546d` |
| `t04-quotes-kept` | `b777ecbfc60f476b3b4498574fd0ef27f8bbd50e9575107a47fa55bc1f34a95f` |
| `t05-guard-6-of-10` | `f0fcc653a57d0fe71bf600eafb27044eb2b86f3c554c62ea8a1608611833c3c8` |
| `t05-guard-3-of-5` | `d64ad857e3ebe0a946656080d3bb5c211d13bea5b2d425e298eae6436f5081c5` |
| `t07-restore`, `t07-placeholder-dropped`, `t07-placeholder-invented` | `8bced8e49c609c2d95f4d78593995d6c0a7123b4041b0ebf16a73490ba78f635` |
| `t12-escaped-text-tag` | `30ff0fc29865a846a98ace37f7567a2671361c2c58ace3fd625f00c4a8147642` |

`{text}` is the selection after redaction (§6). Whether its leading and trailing whitespace is sent
is not fixed by §6 or the spec; what is fixed is that the result gets the original's whitespace
back (R6). So `t04-whitespace` records both requests, `<text>\n  hi there\n\n</text>` (the text as
selected) and `<text>\nhi there\n</text>` (trimmed), with the same answer, and its expected result
is `  Hi there\n` either way. Every other case has one request.

## Diffs

The display diff runs over UAX #29 word and non-word runs: the word segments (those containing a
letter or a digit) and the maximal runs of text between them, so `, ` and `. ` are one run each.
Each answer was written so that the longest common subsequence of the two token lists is unique
(one-for-one word substitutions with words found nowhere else in either text, and deletions that
cannot slide), so Myers' algorithm has exactly one shortest edit script; a changed stretch is
written as its deletion followed by its insertion, and neighbouring operations of the same kind are
merged (test 15). Every expected diff was derived by hand and then confirmed with a reference run of
`similar` 3.2.1 (`Algorithm::Myers`) over the segments of `unicode-segmentation` 1.13.3, the UAX #29
implementation the workspace pins.

All golden diffs read the same whether the non-word segments are merged into runs or not, except
`t02-rewrite-fix-grammar`, where `tomorow, please` becomes `tomorrow. Please`. With runs (the
reading these files take) that is one deletion and one insertion:
`{"delete": "tomorow, please"}, {"insert": "tomorrow. Please"}`. Over unmerged segments the space
between them would match, giving `{"delete": "tomorow,"}, {"insert": "tomorrow."}, {"equal": " "},
{"delete": "please"}, {"insert": "Please"}`.

## The guard (test 5)

`changed` sums max(deleted, inserted) word tokens over the hunks of a Myers diff of the word tokens
(case-sensitive); `total` is the original's word count; it warns when changed / total > 0.35 and
changed ≥ 4 (docs/PUBLIK-INTEGRATION.md §6.1).

| Original → answer | changed of total | Ratio | Warning |
|---|---|---|---|
| §6.1 example (with `[EMAIL_1]`) → §6.1 answer | 4 of 15 (Their, tomorow please, arrive) | 0.27 | no |
| `We go to the shop in the van at six.` → `They went to a shop on the bus by six.` | 6 of 10 (We go, the, in, van at) | 0.6 | yes |
| `Me and him goes there.` → `He and I go there.` | 3 of 5 (Me, him goes) | 0.6 | no, fewer than 4 |
| `We go to the shop in the van at six.` → `We went to a shop on the bus at six.` | 4 of 10 | 0.4 | yes |
| `The old man walked his small dog to the park every morning and then sat on a green wooden bench.` → `The young woman took his big dog to the river every evening and then sat on a blue wooden bench.` | 7 of 20 | 0.35, not above | no |
| `the cat sat` → `the dog sat` | 1 of 3 | | no (test 15's `edit_ratio`) |

The same reference run gives these counts. With the email restored (`ana@example.com` is two word
segments, `ana` and `example.com`) the §6.1 example counts 4 of 16, which gives no warning either.

## Language keeping (test 6)

`languages.jsonl` holds five paragraphs in each of English, French, German, Spanish, Portuguese,
Italian, Dutch and Polish, written for this repository (CC0), the same five everyday topics in each
language. A reference run of `whatlang` 0.18.0 (`whatlang::detect`) detects every paragraph as its
own language with `is_reliable()` true; the names are whatlang's English names (`Lang::eng_name`).
So an English paragraph answered by a paragraph in another language warns with that pair, and
answered by another paragraph in its own language does not.

The golden answers in English never warn: each pair is either detected as English on both sides or
not reliably detected, or has a side under 40 characters. `Merci beaucoup, ami.` has 20 characters,
so it is never checked.

## Redaction (test 7)

`redaction.jsonl` was derived by hand from the spec's table (patterns, order, extra rules, markers
numbered by first appearance within their kind and reused for a repeated value) and checked against
an independent model of that table written with Python's `re`. The data is fictional:
`@example.com` and `@example.org` addresses, UK numbers from Ofcom's drama range 020 7946 0000 to
0999, US numbers from the fictional 555-0100 to 555-0199 range, the standard example IBAN
`GB82 WEST 1234 5698 7654 32` and the public test card `4111 1111 1111 1111`.

- IBANs: ISO 13616 with ISO 7064 mod 97-10. `GB82WEST12345698765432` moves its first four
  characters to the end and replaces each letter by its number (A = 10 … Z = 35); the result mod 97
  is 1, so it is an IBAN (22 characters). `GB82 WEST ABCD EFGH IJ` has the IBAN shape and 18
  characters, but mod 97 gives 0, not 1, and it has no other digit run, so nothing is redacted.
- `2026-09-29 at 10.30`: the only PHONE-shaped match is `2026-09-29`, an ISO date, so it is not a
  phone; `10.30` is too short for the pattern and is a time.
- `123456789` and `12345678`: no leading `+` and no separator, so not phones; 8 digits or more, so
  numbers. `1234567` is neither.
- `t07-redact` lists its entities in order of first appearance; in that text the order by position
  and the order of the table are the same.

## Test 16 (R7b)

The Fix grammar call on publik API for `Their going to the libary.` (dialect British) has the cache
key K and the idempotency keys of tests/cases/03-publik/REFERENCE.md: K =
`748f0ab593f7379e156586aeba37f558d87039ad6da30400ee0c179dba3135b4`, first key
`748f0ab593f7379e156586aeba37f558`, after one invalid answer `fc0efd139ba7f503da3f335030060003`,
after two `7ba59aa7d4199e866e157ae016c9b78c` (SHA-256 of `<K>:1` and `<K>:2`, first 32 hex). For
Clearer the test computes K the same way from the request's `cache_input` and the tier it sends.

## Limits (test 10) and the gate (test 17)

- `t10-too-long` holds `w1 w2 … w1812`: 1,812 words, each a letter followed by digits, one UAX #29
  word. The 1,500 and 1,501 word texts of the core test are built the same way.
- `t10-instruction-too-long` passes 301 letters `a`; the core test also uses 300.
- `eval-rewrite` (docs/TESTING.md §4) makes 100 Fix grammar calls on `publik-fast` and 40 paragraphs
  × 5 actions = 200 calls on `publik-balanced` (its cost line prices those 200 at the balanced
  rate), 300 calls in all. Test 17 itself checks only that the gate prints a number of calls and
  both tiers, asks y/N, and on `n` exits 0 with no connection made; the counts of §4 have their own
  test.
- The 100 Fix grammar sentences are `fix-grammar.jsonl`, written for this repository (CC0): each
  has one error (`error`, the word as written) and the sentence with it fixed (`corrected`), which
  differs from `text` in that one word only (99 replaced, one removed), checked word by word with
  Python's `difflib`. The corrections are standard English usage; the 21 sentences marked
  `british` use no word that is spelled differently in American English. No sentence has a digit,
  so nothing in it is redacted. Each has 8 to 14 words (median 11), so `edit_ratio(text,
  corrected)` is `(1, n)` with n at least 8: at most 0.125, under the gate's 0.15 median, and one
  changed word never reaches the guard's 4.
