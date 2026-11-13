# Module 06 references

Where the expected values of module 06's tests come from. None of them was taken from Redpen's own
output. The tests are:

- `crates/redpen-core/tests/06_session.rs`: the session API with `FakeCapture` and
  `FakeTransport` (bullets 1, 2, 3 and 4 in their core half, 10, 14, 16), and the error paths of
  the session commands;
- `app/src-tauri/tests/06_hotkeys.rs`: the check shortcut, Pause, the Welcome flag, the own key,
  the tray menu and the popup latency check (bullets 12, 13, 15, 17), the errors of
  `settings_update`, `own_key_save`, `own_key_clear`, `provider_test`, `sent_log_clear`,
  `cache_clear` and `open_settings`, `usage_month`, and the shortcut where selections cannot be
  read;
- `app/src/state/06-popup-and-settings-ui.test.ts`: the popup reducer (bullets 3, 4, 11);
- `app/src/06-format.test.ts`: `formatUsd` (bullet 9);
- `app/src/windows/06-views.test.ts`: the popup's views, the consent view, the first-run card, the
  error card, Welcome and Settings (bullets 3 and 11 in their view half, 5 to 8, 15 and 18).

A test named `tNN_…` (Rust) or `tNN: …` (Vitest) covers acceptance bullet NN; `e_…` / `e: …` an
error path the Scope introduces; `s_…` / `s: …` a Scope bullet the numbered list leaves implicit.

Module 06 adds no command-line command, so this folder holds no golden case: its two
`recording.json` files are read by `06_session.rs` only.

## Texts

- **R1** (bullet 1): `I went to teh store yesterday.`, sentence `c01` of
  `tests/cases/01-harper-check/corpus.jsonl`, checked in American English. Its reference finding
  (harper-cli 3.0.0, tests/cases/01-harper-check/REFERENCE.md, golden case `b01-c01`) is
  `10-13-spelling-57a49948`, a Typo on chars 10 to 13 with one suggestion, replace with `the`;
  `57a49948` is the first 8 hex of SHA-256 of `teh` (`hashlib.sha256(b"teh")`). Applying it gives
  `I went to the store yesterday.`, the corpus's `corrected` text, which Accept pastes. A second
  suggestion index (1) does not exist, and the same finding applied again no longer fits the text.
- **R2** (bullet 2): `Please mail ana@example.com today about the invoice.` (the bullet's text).
  Redaction (module 04, emails first) sends `Please mail [EMAIL_1] today about the invoice.`; the
  hand-written Clearer answer is `{"text": "Please email [EMAIL_1] about the invoice today."}`,
  which restores to `Please email ana@example.com about the invoice today.`
- **Try again** (bullet 3, core half): `Their going to the libary.` with Fix grammar in British
  English on publik API, the input whose cache key and idempotency keys
  tests/cases/03-publik/REFERENCE.md computed: K = `748f0ab5…dba3135b4`, first call
  `748f0ab593f7379e156586aeba37f558`, the first Try again after an invalid answer
  `fc0efd139ba7f503da3f335030060003` (SHA-256 of `<K>:1`). The valid answer is
  `They're going to the library.`

## Recordings

Both are hand-written (docs/TESTING.md §1); their only response header is
`x-publik-charge-micros`, with invented values (2100 for Clearer, $0.0021 in
docs/PUBLIK-INTEGRATION.md §8; 210 for Fix grammar, $0.0002).

| Case | Request | `request_sha256` |
|---|---|---|
| `r2-clearer` | the §6.2 Clearer body (`publik-balanced`, British English, the author's own style, `max_tokens` 256, `temperature` 0.4) with the redacted R2 text | `5a49e040357eb2869c7e2a4b7ddda030ada9a2642fd46579a327ce7fd5a3ab6c` |
| `t03-try-again` | the §6.1 Fix grammar body for `Their going to the libary.`, British English, the same request as module 04's `t03-invalid-*` cases; two entries answered in turn: `hello` (not JSON), then the valid answer | `34724b5ae7e80b2eb211db3ba67c27e0e8c9c2ddbbe0bf3f138cca64e92a5a0b` |

The system content of `r2-clearer` is §6.2's example system content unchanged; the user content
is the §6.1 user section with `{text}` filled. `max_tokens` is `clamp(2 * words + 100, 256, 4096)`
= 256 for any text under 78 words. Hashes are module 00's rule: SHA-256 of
`POST /chat/completions\n` plus the canonical body (keys sorted, no whitespace, UTF-8), computed
with Python's `hashlib` over `json.dumps(body, sort_keys=True, separators=(",", ":"),
ensure_ascii=False)`; the same method gives module 04's recorded hashes for `t02-rewrite-clearer`
(`b4a381bd…`) and `t03-invalid-not-json` (`34724b5a…`), which is how it was checked.

The own key and local variants of the R2 request (provider lines, below) are built in the test
from the `r2-clearer` request with only `model` changed, and answered by entries made in the test.

## Provider lines and the footer

The Scope's footer: `<tier or model> · <cost> · <balance line>` on publik API, `Your own key ·
<model>` or `Local · <model>` otherwise. The tests read the outcome's `provider_line` as the part
before the cost: the tier on publik API (`publik-balanced` for Clearer, `publik-fast` for Fix
grammar, docs/PUBLIK-INTEGRATION.md §1), `Your own key · example-chat` (the own key's model in the
settings example of docs/ARCHITECTURE.md §4) and `Local · qwen2.5:14b` (module 02's local model).
The popup's footer is then `publik-fast · $0.0002 · …` for a charge of 210 micro-dollars, the
example of docs/PUBLIK-INTEGRATION.md §9.

## Accept and Copy without a result

`accept` and `copy_result` take `result_id?` (docs/ARCHITECTURE.md §5). In the check view there is
no result, and the check view's Accept and Copy (docs/PRODUCT.md §5) act on the session's text with
its applied fixes: R1's Accept pastes the fixed text with no `result_id`.

## Money

`formatUsd(micros)`, bullet 9: under one cent, four decimals; from one cent up, two; 0 is `$0.00`.
The bullet's own values are the reference: 0 `$0.00`, 210 `$0.0002`, 2,100 `$0.0021`, 9,999
`$0.0100` (0.009999 to four decimals), 3,120,000 `$3.12`. The other values are exact amounts, true
by construction: 100 `$0.0001`, 1,900 `$0.0019`, 2,400 `$0.0024`, 3,800 `$0.0038` (§8's estimates),
10,000 `$0.01`, 80,000 `$0.08` (a full plagiarism check), 410,000 `$0.41` ("This month: $0.41"),
12,000,000 `$12.00`. No value sits halfway between two results, so no rounding rule beyond the
bullet's is assumed. The Design notes give `formatUsd` the same rule as the Rust `balance_line`,
which groups thousands (module 03's Design notes, Money): 1,250,000,000 is `$1,250.00`.

## Popup placement

`place(caret, cursor, work_areas, size)` (Design notes): below the caret rectangle with an 8 px
gap, else at the cursor; flipped above when there is no room below (the same 8 px gap, mirrored);
clamped into the work area that holds the caret, or the cursor when there is no caret. The popup's
left edge is at the caret's (or cursor's) x. Every test uses a 440 × 320 popup.

| Case | Work areas | Caret (x, y, w, h) | Cursor | Expected |
|---|---|---|---|---|
| room below | 0,0 1920×1040 | 600, 400, 2, 20 | 1200, 300 | (600, 428): 400 + 20 + 8 |
| bottom edge (bullet 10) | 0,0 1920×1040 | 600, 1016, 2, 20 | 1200, 300 | (600, 688): below would start at 1044, past 1040 − 320 = 720, so above: 1016 − 8 − 320 |
| no caret (bullet 10) | 0,0 1920×1040 | none | 1200, 300 | (1200, 300) |
| second monitor (bullet 10) | 0,0 1920×1040 and 1920,0 2560×1400 | 2500, 400, 2, 20 | 100, 100 | (2500, 428), inside the second |
| second monitor, right and bottom edges | as above | 4400, 1370, 2, 20 | 100, 100 | (4040, 1042): x clamped to 1920 + 2560 − 440; above: 1370 − 8 − 320 |
| a monitor left of the main one | −1920,0 1920×1040 and 0,0 1920×1040 | −100, 300, 2, 20 | 100, 100 | (−440, 328): x clamped to −1920 + 1920 − 440 |
| cursor near a corner, no caret | as the second-monitor case | none | 4400, 1300 | inside the second monitor (containment only: the spec does not fix where a cursor-placed popup goes when it does not fit) |

1920 × 1040 is a 1920 × 1080 monitor less a 40 px taskbar, the bullet's work area.

## Hotkeys and the keyboard layout

- `HotkeyInUse`: the spec's message with the refused hotkey in place of `Ctrl+Alt+R`,
  "Ctrl+Alt+K is already used by another app. Choose another shortcut.", and the field `hotkey`
  (docs/ARCHITECTURE.md §5, `HotkeyInUse {hotkey}`).
- AltGr: "That shortcut types a character on your keyboard layout. Choose another." The fake
  layout is the Polish programmer's layout, where Ctrl+Alt+C types `ć` (U+0107) and Ctrl+Alt+E
  types `ę` (U+0119) (docs/ARCHITECTURE.md §10), as do A, L, N, O, S, X and Z their Polish
  letters; on the fake US layout no Ctrl+Alt combination types a character, so Ctrl+Alt+C is
  accepted there. Ctrl+Alt+J types nothing on either.
- The fake registrar holds what it was asked to register, and refuses a hotkey it was told another
  app holds; Pause must leave it holding nothing, and resuming must give back the same set.

## Own key address

The spec's message "Use an https:// address (or http://localhost for a server on this computer)."
Accepted: `https://api.example.com/v1` (the settings example), `http://localhost:11434/v1`,
`http://localhost/v1`. Refused: `http://api.example.com/v1` (plain http to another host),
`ftp://api.example.com/v1`, `api.example.com/v1` (no scheme) and
`http://localhost.example.com/v1`, whose host is not `localhost` (module 02's host rule,
`url_host`). `own_key_clear` (docs/ARCHITECTURE.md §5) leaves `secrets.json` with no own key.

## Strings

- Consent view (docs/PUBLIK-INTEGRATION.md §4): title "Choose how Redpen uses AI"; the data-path
  sentence; `DISCLOSURE` as module 03's spec gives it; buttons "Continue with publik API", "Use my
  own key", "Use a local model"; footer "You can change this at any time in Settings → Providers."
  A second, synthetic disclosure checks that the view shows what `consent_info` returns.
- First-run card (§4): `balance_line(0, anonymous)` = "$0.00 · link this computer for $0.05 of
  free use" (module 03), the cost sentence of `tests/cases/03-publik/installs-201.fake.json`,
  "Sign in with publik" ("Add a plan or pack" once claimed), "Your checks keep working. When you
  have linked this computer, press Try again.", Try again, "Use my own key", "Use a local model".
- Errors (§9 and module 03): 402 shows publik's `error.message` (from
  `tests/cases/03-publik/chat-402-anonymous.fake.json`), one link and "or use your own key or a
  local model in Settings"; `Disconnected` "publik API disconnected this computer." with
  Reconnect; `PublikBusy` "publik API is busy; try again in a moment" with Try again; `Offline`
  "No connection. Rewrites need a connection or a local model; checking still works." with no
  button; `InvalidAnswer` "The AI answer could not be used." with Try again.
- Welcome (docs/PRODUCT.md §5): "Redpen checks your writing on this computer. Nothing you type
  leaves it until you ask for a rewrite, or for a check that needs the internet. Password fields
  and apps you block are never read.", the hotkey Ctrl+Alt+R and Close.
- Capture and clipboard messages: docs/ARCHITECTURE.md §8 and module 05's spec. `apply_fix`'s
  `StaleFinding` and `NoSuchSuggestion`: module 01's spec.

## The reducer's fixtures

`app/src/state/06-popup-and-settings-ui.test.ts` takes every type from `reduce` and `initial`, the
two names the Design notes give `app/src/state/popup.ts`. Its session is R1's (module 01's
reference finding); its outcomes are hand-written in the shape of docs/ARCHITECTURE.md §5 and
module 04's goldens (`{equal}`, `{delete}`, `{insert}` diff parts), and each diff joins back into
the original and the result, which is how they were checked:

| Outcome | Text | Provider line, charge |
|---|---|---|
| Fix grammar | `I went to the store yesterday.` | `publik-fast`, 210 |
| Fix grammar, guarded (`accept_default: false`, §6.1 step 5's warning) | `Yesterday, I went to the shop.` | `publik-fast`, 210 |
| Clearer | `Yesterday, I went to the store.` | `publik-balanced`, 2100 |

What the key map does comes from the Design notes; that Enter in the check view accepts the text
with its fixes (no `result_id`) comes from docs/PRODUCT.md §5 ("Accept (Enter)") and the section
on Accept and Copy above. Ctrl+Shift+C is sent as the key `C`, the value a browser reports with
Shift held. "Use my own key" and "Use a local model" give `open_settings` with the section
(docs/PUBLIK-INTEGRATION.md §4: Settings → Providers → Your own key, or → Local).

## The windows' tests

`app/src/windows/06-views.test.ts` renders `Popup`, `Settings` and `Welcome` and drives them only
through what the spec and docs/ARCHITECTURE.md §5 name: the event `popup://open {session_id}`,
the commands by name (answered by a fake `invoke`), and the labels of docs/PRODUCT.md §5 and
docs/PUBLIK-INTEGRATION.md §4 and §9. An argument is read by its snake_case name or its camelCase
form, at the top level or one object down, so the tests do not fix how `app/src/ipc.ts` packs a
command's arguments. The consent view's strings come from `consent_info` (the Scope: "strings
from `consent_info` and `ConnectOutcome`"); the `NeedsConsent` error is module 03's, `{kind,
message}`.

- **The popup's loading line** is the spec's Message, "Rewriting… (publik-balanced)", for Clearer
  with publik API chosen.
- **The result view** (docs/PRODUCT.md §5: the diff with deletions and insertions, warnings above
  it) is answered with the guarded Fix grammar outcome of the reducer's fixtures: its warning is
  shown verbatim, both the deleted and the inserted words are on the page, Enter sends nothing and
  Ctrl+Enter calls `accept` with its `result_id` (bullet 3). Keys are sent from an element inside
  the popup that is not a button.
- **The footer** on publik API is the outcome's tier, `formatUsd(charge_micros)` and the balance
  line `provider_status` returns ("$3.12 of publik balance" for 3,120,000 micro-dollars once
  linked, module 03's `balance_line`): `publik-balanced · $0.0021 · $3.12 of publik balance`.
  Off publik API it is the provider line alone, with no dollar amount after it (no `·` followed
  by `$` and a digit).
- **The 402 card** is answered with module 03's anonymous 402 (`chat-402-anonymous.fake.json`):
  its `error.message`, `top_up_url` and `claim_state`. Its one link carries that address in
  `href` and is labelled "Sign in with publik" while anonymous (§9); pressing it calls
  `open_account_link` with the kind `top_up`, never with an address.
- **The first-run card's** "Sign in with publik" calls `open_account_link` with the kind `claim`.
- **No request**: the commands that make a request leave the computer are `rewrite`,
  `publik_connect`, `provider_test` and `local_models` (docs/ARCHITECTURE.md §8). Welcome, an
  opened check view and Settings opened with no provider chosen call none of them.
  `publik_refresh` is not counted: it sends nothing unless publik API is chosen and set up
  (module 03).
- **Settings**: the four tabs of the Scope, and no Dictionary tab until module 08. With publik
  API chosen, Providers shows "This month: $0.41 across 312 actions" for a `usage_month` of
  410,000 micro-dollars over 312 calls (§9) and the balance line, and never the word "credits"
  (contract §9).

## Settings commands

`settings_update` takes a patch in the shape of `settings.json` (docs/ARCHITECTURE.md §4), such
as `{"hotkeys": {"check": "Ctrl+Alt+K"}}`; a refusal changes neither the registration nor the
file. Its errors are docs/ARCHITECTURE.md §5's: `HotkeyInUse {hotkey}`, `HotkeyInvalid` for a
combination with no key (`Ctrl+Alt+`), and `Invalid {field}` for a value Settings does not know
(the dialect `klingon`; docs/PUBLIK-INTEGRATION.md §6 lists the five). A refusal at start names
the default shortcut, which gives the spec's message word for word. Pause is saved as
`ui.paused`, and Redpen started while paused registers nothing. The Welcome window shows while
`ui.welcomed` is false; its Close sets it, so the next launch shows nothing. A first launch makes
no request: no provider is chosen, so module 03's start-up wallet read sends nothing. The test
fixes a fresh install: no credential, no app token and no `PUBLIK_API_*` variable.

## The tray menu

docs/PRODUCT.md §5 and the Scope list Open Settings, the balance line (disabled, once publik API
is set up), Pause Redpen and Quit. The test fixes those four items, Quit last, Open Settings
above Pause Redpen and the balance item as the only disabled one; it leaves the balance item's
place to the implementation, since module 03 put it first when Quit was the only other item.
The label of the paused state is not in the spec, so the test does not check it.

## Log hygiene (bullet 16)

docs/TESTING.md §5 and docs/ARCHITECTURE.md §8: a full scripted session (capture, check, Clearer,
Copy, Accept, Undo, close) runs under a capturing `tracing` subscriber at TRACE level. The log must
hold something (or the test proves nothing) and must not hold the selection, its redacted form,
the result, `ana@example.com`, the fake key, the fake balance (an `x-publik-balance` of 3120000,
"$3.12", set in the test's own entry, never in a recording), the fake process name or the
clipboard's earlier text.

## The popup latency check (bullet 17)

`redpen-app --measure-popup` prints `{"p95_ms": …}`; the bullet's threshold is 300 ms over 20 runs
on `windows-latest` with `redpen-testwin`. It types into the window in front and replaces the
clipboard, so like module 05's desktop tests it runs only with `REDPEN_DESKTOP_TESTS=1`.

## This month's usage

`usage_month` sums this local month's `usage_daily` rows for publik API, without the Test buttons
(the spec's Design notes). The test's clock reads 2026-12-01T04:30:00Z at UTC−5, which is
23:30 on 30 November locally, so the month is `2026-11` while the UTC date is already in December.
The instant is day 20,788 after 1970-01-01: 56 years to 2026-01-01, 14 of them leap years (1972
to 2024), give 56 × 365 + 14 = 20,454 days, and January to November 2026 add
31 + 28 + 31 + 30 + 31 + 30 + 31 + 31 + 30 + 31 + 30 = 334; in seconds,
20,788 × 86,400 + 4 × 3,600 + 30 × 60 = 1,796,099,400 (checked with `date -u -d @1796099400`).

| Day | Provider | Feature | Calls | Micro-dollars | Counted |
|---|---|---|---|---|---|
| 2026-11-02 | publik | clearer | 2 | 4,200 | yes |
| 2026-11-30 | publik | clearer | 1 | 2,100 | yes |
| 2026-11-30 | publik | fix-grammar | 3 | 630 | yes |
| 2026-10-31 | publik | clearer | 5 | 10,500 | no: last month |
| 2025-11-15 | publik | shorter | 9 | 18,900 | no: last year |
| 2026-12-01 | publik | clearer | 1 | 2,100 | no: the UTC month, not the local one |
| 2026-11-30 | own_key | clearer | 7 | 0 | no: not publik API |
| 2026-11-30 | local | shorter | 4 | 0 | no: not publik API |
| 2026-11-30 | publik | test | 1 | 100 | no: a Test button |

Expected: 6,930 micro-dollars (4,200 + 2,100 + 630) over 6 calls; `clearer` 3 calls and 6,300,
`fix-grammar` 3 calls and 630.
