# Redpen handbook

The user guide. It grows with the app: each module adds the pages for what you can now do, and
every example in it has been run.

## Version 0.1.0

Building from source (see `README.md`) gives:

- a tray icon with one menu item, **Quit**; no window is shown yet;
- the `redpen` command-line tool, which checks a text on this computer (below), sends one
  message to an AI provider when you ask it to (below), answers `redpen --version --json` with
  `{"name":"redpen","version":"0.1.0"}`, and refuses an unknown argument with
  `error: unknown argument <arg>. Run redpen --help.`

## Checking a text on this computer

Redpen checks English with Harper, a grammar checker that runs on this computer: no account, no
key, no connection, and nothing leaves the computer. Until the popup arrives, the command-line
tool shows what a check finds. Save a text in a UTF-8 file, then:

```sh
redpen check note.txt
```

(from a source checkout: `cargo run -q -p redpen-cli -- check note.txt`). For a `note.txt` that
holds `I saw teh cat.` it prints one line of JSON, shown here spread out:

```json
{"findings": [{"id": "6-9-spelling-57a49948", "category": "spelling", "harper_kind": "Typo",
  "message": "Did you mean the definite article?",
  "span": {"chars": [6, 9], "bytes": [6, 9], "utf16": [6, 9]},
  "suggestions": [{"kind": "replace", "text": "the"}]}]}
```

Each finding has:

- a **category**, one of four (table below), and Harper's own name for the kind of problem;
- Harper's **message**;
- **suggestions**, each a `replace` (put the text in place of the span), a `remove` (delete the
  span) or an `insert_after` (add the text just after the span);
- the **span** it covers, counted three ways: in characters, in UTF-8 bytes, and in UTF-16 units
  (what JavaScript counts). An emoji such as a thumbs-up is one character, four bytes and two
  UTF-16 units, so the three can differ;
- an **id** made of the span, the category and a short fingerprint of the text under the span.

`redpen fix-all note.txt` applies the first suggestion of every finding, checks the result again,
and repeats until nothing is left to fix (at most 10 rounds). For the note above it prints
`{"text":"I saw the cat."}`. The fixes never take a text past 20,000 characters: a round that
would is left out and the text from before it is printed, so the result can always be checked
again.

**Dialects.** `--dialect` chooses the English the text is checked against: `american` (the
default), `british`, `australian`, `canadian` or `indian`, the five Harper offers. With
`--dialect british`, `The colour of the sky is blue.` has no findings; with `american`, `colour`
is a spelling finding with `color` first. The app will start from your computer's language
setting (English for the United Kingdom, the United States, Australia, Canada or India gives that
dialect; any other setting gives American) and let you change it in Settings.

**Turning a category off.** `--disable spelling` (repeat it for more) drops that category's
findings and keeps the others. `redpen check --disable spelling note.txt` prints
`{"findings":[]}`.

**Categories.** Harper names more than twenty kinds of problem; Redpen groups them into four:

| Category | Harper kinds |
|---|---|
| Spelling | Spelling, Typo, BoundaryError |
| Grammar | Agreement, Grammar, WordOrder, Usage, Nonstandard, Malapropism, Eggcorn, Repetition, Miscellaneous |
| Punctuation | Punctuation, Capitalization, Formatting |
| Style | Style, WordChoice, Redundancy, Readability, Enhancement, Regionalism |

Miscellaneous holds corrections such as "a" or "an", so it counts as Grammar; Capitalization and
Formatting sit with Punctuation as the mechanics of the text.

**Limits and messages.**

- A text over 20,000 characters is not checked: "This selection is too long to check (over
  20,000 characters). Select less text." The command-line tool prints it as
  `{"error":{"kind":"TooLong","message":"…","chars":20001}}` and exits with code 2.
- Applying one fix at a time comes with the popup. A fix whose text has changed since the check
  is refused with "That suggestion no longer fits the text. Check again.", and a fix that is no
  longer offered with "That suggestion is not available any more."
- An empty file has no findings: `{"findings":[]}`.

## Talking to an AI provider

The rewrite actions will go through one client that speaks to any OpenAI-compatible provider. Until
they arrive, `redpen chat` shows whether a provider answers. It sends exactly one message, only when
you run it, and prints the answer:

```sh
redpen chat --provider local --message "Say hello."
```

**A local model** (`--provider local`) runs on this computer, so nothing leaves it. Redpen asks
Ollama at `http://localhost:11434` which models it has, leaves out embedding models (their names
contain `embed`), and uses the largest one up to 15B parameters (the 14B class, which Ollama
reports as, for example, 14.8B); between two of the same size, the first by name. A local address
must be this computer: `localhost`, `127.0.0.1` or `::1`. Any other address is refused before
anything is sent.

**Your own key** (`--provider own-key`) works with any OpenAI-compatible server. The command line
reads the key, the base URL (for example `https://api.example.com/v1`) and the chat model from
`REDPEN_OWN_KEY`, `REDPEN_OWN_BASE_URL` and `REDPEN_OWN_MODEL`; without all three it stops with
`error: --provider own-key needs REDPEN_OWN_KEY, REDPEN_OWN_BASE_URL and REDPEN_OWN_MODEL.` The key
is sent only in the `Authorization` header and is never printed or logged.

The answer is one line, `{"content":"...","from_cache":false}`. When the call fails, the command
prints `{"error":{"kind":"...","message":"..."}}`, repeats the message as `error: ...`, and exits
with code 3. With Ollama stopped, for example:

```json
{"error":{"kind":"LocalNotRunning","message":"Ollama isn't running. Start it, or choose another provider."}}
```

The messages a provider call can end with:

| Kind | Message |
|---|---|
| `NotConfigured` | Choose an AI provider in Settings → Providers. |
| `OwnKeyRefused` | Your key was refused (401). Check it in Settings → Providers. |
| `ModelUnknown` | Your provider does not know the model “example-chat” (404). Check the model name in Settings. |
| `RateLimited` | Your provider says you are sending too many requests (429). Wait a moment and try again. |
| `ProviderError` | Your provider had a problem (502). Try again later. |
| `Timeout` | No answer from your own key's server after 30 seconds. Try again. (120 seconds for a local model) |
| `Offline` | No connection. Rewrites need a connection or a local model; checking still works. |
| `LocalNotRunning` | Ollama isn't running. Start it, or choose another provider. |
| `LocalNoModel` | Ollama has no chat model installed. Install one in Ollama, or choose another provider. |
| `LocalNotLocal` | A local model must run on this computer (localhost). |
| `InvalidAnswer` | The AI answer could not be used. |

When your own key's server or the local model times out, is busy (429) or fails (5xx), Redpen tries
twice more, after half a second and then a second. A refused key or an unknown model is not
retried. publik API calls are never retried, because a call it has started may still be charged.

`--recording recording.json` answers from a file of recorded answers instead of the network (the
format is in `docs/TESTING.md`), which is how the tests run.

**What the app will keep.** When the app uses this client, answers are remembered on this computer
for up to seven days (Settings → Privacy; at 0 days nothing is written to disk, but answers are
still kept in memory until Redpen closes), so the same request never goes out twice; a count of
calls per day and action is kept for the usage line in Settings; and, only if you turn on
Settings → Privacy → "Keep a log", a log of exactly what was sent, deleted after seven days.
Your own key is kept in `secrets.json` in your user's settings folder, readable only by you.

## Coming next

In the order of `docs/ROADMAP.md`: publik API, the rewrite actions, reading and replacing a
selection in any Windows app, the popup and Settings, Linux, the personal dictionary and style
presets, the plagiarism check, and installers.
