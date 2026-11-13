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

## Reading and replacing a selection on Windows

The shortcut and the popup arrive with the next modules; underneath them, this is how Redpen
reads the text you selected in any Windows app and types a result back. It goes through the
clipboard, the one route every app that can copy and paste supports, and leaves your clipboard as
it found it.

**Before any key is sent**, Redpen looks at the window in front and stops, in this order, when:

1. the app is on the blocklist (Settings → Privacy). By default it holds the password managers
   1Password, Bitwarden, KeePass, KeePassXC, Dashlane, Enpass, NordPass and Proton Pass:
   "Redpen is turned off for this app (Settings → Privacy).";
2. the window runs as administrator, which Windows does not let a normal app read or type into:
   "This window runs as administrator, so Windows doesn't let Redpen read or type into it.";
3. the field you are in is a password field: "Redpen doesn't read or type into password fields."

Then it waits for you to let go of Ctrl, Alt, Shift and the Windows key, for up to a second
("Release the shortcut keys, then try again."), sends Ctrl+C, and gives the app 400 ms to copy
(`capture.copy_timeout_ms` in `settings.json`). When nothing arrives: "Nothing was copied. Select
some text first."

**Accept** brings back the window you started in, waits a moment, checks it again (the field may
have become a password field while the rewrite ran), and pastes with Ctrl+V. When it can't:

| When | Message |
|---|---|
| that window has closed | The window you started in has closed. Copy the text instead. |
| it will not come back to the front | The window you started in is no longer in front, so nothing was typed. Copy the text instead. |
| another app has the clipboard open | Another app is using the clipboard. Try again. |
| Windows refuses a call (the number is Windows' own error code) | Windows refused the request (code 5). |

**Undo**, for two minutes after Accept, sends one Ctrl+Z to that window. When the window has
closed, will not come back to the front, or now has a password field in front, the text you had
selected goes on the clipboard instead, so you can paste it yourself. After two minutes, or with
nothing replaced: "There is nothing to undo." **Copy** puts the result on the clipboard as an
ordinary entry.

**Your clipboard.** Before it copies or pastes, Redpen saves every format on the clipboard and
puts each back afterwards, byte for byte. The text it pastes is marked so that clipboard history
(Win+V) and the cloud clipboard leave it out. Not kept:

- pictures held as drawing objects rather than data: bitmaps in their drawing-object form (Windows
  makes them again from the image data, which is kept), metafiles and enhanced metafiles, palettes,
  owner-drawn entries, and the private and drawing-object format ranges some programs use;
- a format the app that copied it no longer provides (apps can put a format on the clipboard as a
  promise and supply it only when asked; one that has closed can't);
- everything except the text, when the clipboard holds more than 64 MB in all (a very large
  image, say); the log records only the size left out.

The Ctrl+C is the app's own copy, so when clipboard history is on, the text you selected appears
in Win+V like anything you copy yourself.

**Limits.**

- Apps running as administrator are refused, as above.
- Some apps ignore keys that another program sends; Redpen can't read or type there.
- Terminals: Windows Terminal copies a selection with Ctrl+C, but the classic console treats
  Ctrl+C as "stop", so Redpen can't read a selection there.
- Text goes on the clipboard with Windows line endings and comes off it with plain ones.

## Coming next

In the order of `docs/ROADMAP.md`: publik API, the rewrite actions, the popup and Settings (which
put the shortcut on top of the capture above), Linux, the personal dictionary and style presets,
the plagiarism check, and installers.
