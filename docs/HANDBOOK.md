# Redpen handbook

The user guide. It grows with the app: each module adds the pages for what you can now do, and
every example in it has been run.

## Version 0.1.0

Building from source (see `README.md`) gives:

- a tray icon with one menu item, **Quit** (and, once publik API is set up on this computer, a
  greyed-out line above it with the balance, below); no window is shown yet;
- the `redpen` command-line tool, which checks a text on this computer (below), sends one
  message to an AI provider when you ask it to (below), rewrites a text with one of the six
  actions (below), answers `redpen --version --json` with
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

The rewrite actions (below) go through one client that speaks to any OpenAI-compatible provider.
`redpen chat` shows whether a provider answers. It sends exactly one message, only when you run it,
and prints the answer:

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

## Rewriting a text

The popup's six rewrite buttons run on the engine below; until the popup arrives, `redpen rewrite`
runs it on a text file. Nothing is sent until you run it, and then exactly one request.

```sh
redpen rewrite --action fix-grammar --dialect british --redact --provider own-key note.txt
```

**The actions.** `fix-grammar` fixes spelling, grammar and punctuation and changes nothing else;
`clearer`, `shorter`, `more-formal` and `friendlier` rewrite the text in that direction, keeping
its facts, names, numbers, formatting and language; `custom` follows the instruction you type with
`--instruction "..."` (at most 300 characters), and may change the language when you ask it to.
`--dialect` chooses the spelling (as for checking). `--provider` is `local`, `own-key` (as for
`redpen chat`) or `publik-env`, which uses the publik API key in `PUBLIK_API_KEY`. On publik API,
Fix grammar goes to `publik-fast` and the other five to `publik-balanced`; with Settings →
Providers → "Rewrite quality" set to Best on a linked computer, the app sends those five to
`publik-smart`.

**Private details.** With `--redact` (in the app, Settings → Privacy, on by default) email
addresses, phone numbers, IBANs, card numbers and other runs of 8 or more digits are replaced by
markers before anything is sent, and put back in the answer. `redpen redact` shows what would be
replaced; dates and times are left alone:

```sh
redpen redact contact.txt
```

```json
{"text":"Call [PHONE_1] or write to [EMAIL_1] before 2026-09-29 at 10.30.","entities":[{"marker":"[PHONE_1]","kind":"phone","value":"020 7946 0958"},{"marker":"[EMAIL_1]","kind":"email","value":"ana@example.com"}]}
```

An answer that drops a marker, or adds one that was not sent, is not used. `redpen prompt` takes
the same options as `rewrite` (without `--provider`) and prints the exact request a rewrite would
send to publik API, after redaction, without sending it.

**The result.** For `note.txt` holding `Their going to send the report tomorow, please forward it
to ana@example.com when it arrive.`, Fix grammar prints one line, shown here spread out:

```json
{"text": "They're going to send the report tomorrow. Please forward it to ana@example.com when it arrives.",
 "diff": [{"delete": "Their"}, {"insert": "They're"}, {"equal": " going to send the report "},
          {"delete": "tomorow, please"}, {"insert": "tomorrow. Please"},
          {"equal": " forward it to ana@example.com when it "},
          {"delete": "arrive"}, {"insert": "arrives"}, {"equal": "."}],
 "warnings": [], "accept_default": true, "unchanged": false, "from_cache": false,
 "charge_micros": null}
```

The diff runs word by word; `redpen diff old.txt new.txt` prints the diff of two files, for `the
cat sat` and `the dog sat` `[{"equal":"the "},{"delete":"cat"},{"insert":"dog"},{"equal":" sat"}]`.
`charge_micros` is what publik API charged for the call in millionths of a dollar (`null` on other
providers and from the cache). `unchanged` means the answer is your text as it was: the popup will
say "No changes needed." The answer is cleaned before you see it: a line such as "Here is the
corrected text:", quotes around the whole answer, and spaces or line breaks at its edges are
removed, and your own leading and trailing whitespace is put back, so the result fits where the
selection was. In the app, the same request is answered from the answer cache rather than sent
again; **Another version** asks for a new one.

**Warnings.** A result can come with a warning, and Enter will then not accept it in the popup (the
Accept button still does):

| Warning | When |
|---|---|
| This changes more than a grammar fix should (6 of 10 words). Check it before accepting. | Fix grammar changed more than 35% of your words, and at least 4 |
| The answer is in French, but your text is in English. | both texts have 40 characters or more and their languages are recognised with confidence; not for Custom when you asked for another language |
| This is not shorter than your text. | Shorter's answer has as many words as your text, or more (Enter still accepts it) |

**Refusals.** These stop a rewrite before anything is sent, or after an answer that cannot be
used. The command line prints them as `{"error":{"kind":"...","message":"..."}}` and exits with
code 3, as provider errors do:

| Kind | Message |
|---|---|
| `Empty` | Select some text first. |
| `TooLong` | Select at most 1,500 words for a rewrite (this selection has 1,812). |
| `InstructionTooLong` | Keep the instruction under 300 characters. |
| `InstructionMissing` | Type an instruction for Custom first. |
| `RedactionCollision` | This text already contains a marker like [EMAIL_1], so redaction can't run safely. Turn off redaction in Settings → Privacy, or edit the text. |
| `PlaceholderDropped` | The AI answer dropped [PHONE_1], so it was not used. Try again, or turn off redaction for this text. |
| `PlaceholderInvented` | The AI answer added a marker that was not in your text, so it was not used. |
| `InvalidAnswer` | The AI answer could not be used. |

After an answer that could not be used, **Try again** in the popup asks publik API for a fresh
answer rather than a repeat of the unusable one, at the usual price; an unusable answer is never
kept.

## publik API

publik API is one of the three ways Redpen will rewrite text: it needs no key of your own and is
paid from a publik balance, in dollars. Your own key and a local model stay just as available,
and checking never needs any of them. The screens that set it up arrive with the popup and
Settings; underneath them, this is how it works.

**Nothing at launch.** Redpen sends nothing to publik API when it starts, unless publik API is
your chosen AI provider and this computer is already set up; then it reads your balance once
(`GET /wallet`), and again when Settings opens. With your own key or a local model chosen it
never contacts publik, even if this computer was set up before. The choice is asked at the first
AI action, never at launch, on a screen titled **Choose how Redpen uses AI**:

> Checking is done on this computer and needs no AI. Only the text you select and the action you
> choose go through publik's servers, and only when you press a rewrite button.
>
> Requests go through publik's servers to a shared AI model account. publik does not train on
> them and keeps no request bodies beyond a 24-hour fingerprint that stops a request being
> charged twice. The AI provider's own abuse monitoring applies.

**Setting up this computer.** **Continue with publik API** sends one request, `POST /installs`,
with Redpen's name and version, `windows` or `linux`, the processor type, the label `Windows PC`
or `Linux PC` (never your computer's name), a random install id, and which disclosure you saw.
The key it gets back is kept at `%LOCALAPPDATA%\publik\apps\redpen.json` on Windows and
`~/.config/publik/apps/redpen.json` on Linux, readable only by you. It travels only in the
`Authorization` header of publik API requests and is never shown, logged, written to the status
file or handed to Redpen's window. publik API then becomes your AI provider.

**Your balance.** A new computer starts at $0.00, so the action you pressed is not sent yet;
linking the computer to a publik account (**Sign in with publik**) gives $0.05 of free use once,
and after that a plan or a pack adds more. The balance line reads
`$0.00 · link this computer for $0.05 of free use` until then, and afterwards, for example,
`$3.12 of publik balance` (below one cent, four decimals: `$0.0021 of publik balance`), rounded
down. The tray menu shows it, greyed out, above **Quit**, and updates after every publik API
answer. What each answer cost is added to a count per day and action on this computer.

**Disconnect** asks publik API to revoke this computer's key, then forgets the key even when
publik does not answer. The install id stays, so reconnecting does not count as a new computer.

**When something goes wrong.** A publik API request is never sent again by itself: publik may
still charge a call it has started, so a failure offers **Try again** instead.

| When | Message |
|---|---|
| the balance does not cover the request | publik's own message, with one link (**Sign in with publik** while the computer is not linked, **Add a plan or pack** after), then "or use your own key or a local model in Settings" |
| publik API no longer accepts this computer's key | publik API disconnected this computer. (with **Reconnect**) |
| too many requests | publik's own message |
| publik API is busy | publik API is busy; try again in a moment |
| any other error | publik API 500: The model returned an error. (the code and publik's message) |
| no answer within 30 seconds | publik API did not answer within 30 seconds. Try again. |
| no connection | No connection. Rewrites need a connection or a local model; checking still works. |
| the build has no app token | This build has no publik API app token. Use your own key for now. |
| setting up fails | publik's own message, or "publik API could not set up this computer (500). Use your own key for now.", or "publik API is unreachable right now. Use your own key for now." |

**Builds from source.** A build can set up publik API only with the project's app token, which
the maintainer adds once (`docs/PUBLIK-INTEGRATION.md` §2). Until then every build says "This
build has no publik API app token. Use your own key for now." and your own key and a local model
work as usual.

**Checking a build.** With `PUBLIK_SELF_TEST` set, the app prints one line and exits before it
opens anything. With a token it sets up this computer for real and reads the balance; without
one, it sends nothing:

```sh
PUBLIK_SELF_TEST=1 cargo run -p redpen-app
```

```text
publik self-test: failed: This build has no publik API app token. Use your own key for now.
```

A build with the token prints `publik self-test: ok redpen 0.1.0 (install 5f0c8a52)`, the last
part being the start of this computer's install id. Without the variable the check never runs.

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

In the order of `docs/ROADMAP.md`: the popup and Settings (which put the shortcut on top of the
capture above, the rewrite buttons on top of the engine above, and the publik API screens on top
of the set-up above), Linux, the personal dictionary and style presets, the plagiarism check, and
installers.
