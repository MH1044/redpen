# Redpen handbook

The user guide. It grows with the app: each module adds the pages for what you can now do, and
every example in it has been run.

## Version 0.1.0

Building from source (see `README.md`) gives:

- a tray icon whose menu holds **Open Settings**; once publik API is set up on this computer, a
  greyed-out line with the balance (below); **Pause Redpen**; and **Quit**;
- on Windows, the check shortcut, Ctrl+Alt+R: select text in any app, press it, and the popup
  opens next to the text with Harper's findings and the six rewrite actions (below). On Linux the
  shortcut is registered but says "Redpen can't read a selection on this system yet. Checking
  selections works on Windows." until capture arrives there;
- a welcome window the first time Redpen starts, and a Settings window (below);
- the `redpen` command-line tool, which checks a text on this computer (below), sends one
  message to an AI provider when you ask it to (below), rewrites a text with one of the six
  actions (below), answers `redpen --version --json` with
  `{"name":"redpen","version":"0.1.0"}`, and refuses an unknown argument with
  `error: unknown argument <arg>. Run redpen --help.`

## The first start

The first time Redpen starts, a small window says: "Redpen checks your writing on this computer.
Nothing you type leaves it until you ask for a rewrite, or for a check that needs the internet.
Password fields and apps you block are never read." It shows the check shortcut and a **Close**
button, and it never appears again. It asks for no choice: nothing about AI is asked until you
press a rewrite button, and Redpen sends nothing when it starts.

## Checking a selection

Select text in any Windows app and press **Ctrl+Alt+R**. Redpen reads the selection through the
clipboard (and puts your clipboard back; see "Reading and replacing a selection on Windows"
below), checks it with Harper on this computer, and opens the popup under the text cursor, or at
the mouse when the app shows no text cursor; above it when there is no room below, and always
inside the screen the text is on.

The popup shows your text (twelve lines, then it scrolls) with each finding underlined, and the
list of findings: the category (Spelling, Grammar, Punctuation or Style), Harper's message and
one button per fix. A fix changes the text in the popup and checks it again; nothing is typed into
your app yet. A text with nothing to fix says "No issues found."

| Key or button | What it does |
|---|---|
| **Accept** (Enter) | types the text, with the fixes you applied, over your selection |
| **Copy** (Ctrl+Shift+C) | puts it on the clipboard instead |
| 1 to 6 | the six rewrite actions (below) |
| **Close** (Esc) | closes the popup; the text is dropped from Redpen's memory |
| **Undo last replace** | for two minutes after an Accept, the next popup offers to take it back |

Enter on a focused button presses that button. When the popup can't read the selection it says
why, with the messages of "Reading and replacing a selection on Windows" below, and Esc closes it.
Press the shortcut again with the popup in front and nothing happens; press it in another app and
the popup moves to the new selection.

**Changing the shortcut.** Settings → General → **Change shortcut**, then press the new
combination (Esc cancels). It needs Ctrl, Alt or the Windows key, with Shift if you like, and a
letter, a digit, F1 to F24 or Space. Redpen refuses a combination another app holds ("Ctrl+Alt+K
is already used by another app. Choose another shortcut.") and, on keyboard layouts where Ctrl+Alt
is AltGr (Polish Ctrl+Alt+C types ć, German Ctrl+Alt+E types €), one that types a character
("That shortcut types a character on your keyboard layout. Choose another."); the old shortcut
keeps working. When another app already held Ctrl+Alt+R as Redpen started, Settings → General
says so until you choose another shortcut, or set the same one again once it is free.

**Pause Redpen** (in the tray menu, or Settings → General) lets go of the shortcut and greys the
tray icon until you turn it off; Redpen remembers it across restarts.

## Rewriting a selection

The six buttons under the findings, or keys 1 to 6, rewrite the text (with any fixes you applied):
**Fix grammar**, **Clearer**, **Shorter**, **More formal**, **Friendlier** and **Custom**, which
asks for an instruction first (at most 300 characters; while you type it, digits are just
digits). Nothing is sent until you press one, and then one request to the provider you chose
(below). While it runs the popup says, for example, "Rewriting… (publik-balanced)", with
**Cancel**, which stops waiting (the request is not sent again).

The result shows the changes word by word, deletions struck through and insertions highlighted,
with any warning above it (the table in "Rewriting a text" below). Under it:

- the footer: on publik API the tier, what the call cost and your balance, for example
  `publik-fast · $0.0002 · $3.12 of publik balance` (an answer Redpen remembered costs `$0.00`);
  with your own key `Your own key · example-chat`; with a local model `Local · qwen2.5:14b`;
- **Accept** (Enter, unless a warning says to check the result first; Ctrl+Enter always),
  **Copy** (Ctrl+Shift+C), **Another version**, which asks for a different rewrite (not for Fix
  grammar), and **Back** (Esc) to the findings.

**When something goes wrong** the popup shows one message and at most one way out, and your
findings stay usable under it:

| After | The way out |
|---|---|
| an answer that could not be used, publik API busy, no answer in time, your provider's own error | **Try again** |
| publik API disconnected this computer | **Reconnect** |
| a balance too low (the message is publik's own) | one link, **Sign in with publik** (or **Add a plan or pack** once linked), then "or use your own key or a local model in Settings" |
| a key refused, an unknown model, Ollama not running or without a model | **Open Settings**, at the right section |
| Accept could not type into your app, or another app had the clipboard | **Copy** |
| no connection, too many requests, a selection over 1,500 words | none: the message says what to do |

## Choosing how Redpen uses AI

Checking never needs AI. The first time you press a rewrite button, and only then, the popup
asks **Choose how Redpen uses AI**, with the text quoted under "publik API" below, and three
buttons: **Continue with publik API**, **Use my own key** and **Use a local model** ("You can
change this at any time in Settings → Providers."). The action you pressed waits.

- **Continue with publik API** sets up this computer (one request). A new computer starts at
  $0.00, so the action is not sent: the popup shows the balance line, what a rewrite costs,
  **Sign in with publik** (which opens publik in your browser), "Your checks keep working. When
  you have linked this computer, press Try again.", **Try again**, and "Or: Use my own key · Use a
  local model". With a balance already there, the action runs at once.
- **Use my own key** and **Use a local model** open Settings → Providers there; when it is set up,
  press **Try again** in the popup.

**Settings → Providers** holds the same choice:

- **publik API**: the balance line, "This month: $0.41 across 312 actions" (what this computer
  spent through publik API this month), **Sign in with publik** or **Add a plan or pack**,
  **Disconnect**, and Rewrite quality, Standard or Best (Best uses `publik-smart` once the
  computer is linked). Choosing it on a computer that is not set up shows the same consent text,
  then the same card without Try again: the next rewrite uses the new set-up. Opening Settings
  reads the balance from publik API, only while it is your chosen provider; meanwhile it says
  "Checking…", and "Can't reach publik API right now." when it can't.
- **Your own key**: the base URL (it must start with `https://`, or be `http://localhost` for a
  server on this computer: "Use an https:// address (or http://localhost for a server on this
  computer)."), the key and the chat model, then **Save**; **Test** lists the provider's models
  ("Key works: 12 models available.") and **Remove key** forgets it. The key goes to
  `secrets.json` and never comes back to the window.
- **Local model**: whether Ollama runs on this computer and which model Redpen will use ("Ollama
  is running. Redpen will use qwen2.5:14b."), with **Check Ollama**; you can pick another of its
  models instead of the automatic choice.

## Privacy

What leaves this computer, and when:

- **Checking**: nothing, ever.
- **A rewrite**: when you press its button, the selected text (after the markers below), the
  action's instructions and your dialect go to the provider you chose; never the rest of the
  document, the window's title, the app's name or your clipboard.
- **Setting up publik API**, **Test** and **Check Ollama**: one request each, when you press them
  (Ollama's stays on this computer).

Settings → Privacy:

- **Replace emails, phone numbers, IBANs and long numbers with markers before anything is sent**
  (on by default); the answer gets them back.
- **Apps Redpen never reads**: the password managers by default; add a program by its name (for
  example `notes.exe`) or remove one. Redpen refuses to read or type in those apps.
- **Remember AI answers**: 7 days or Off, and **Clear now**. A remembered answer costs nothing.
- **Keep a log of what was sent** (off by default): each request's time, provider, action, model
  and exactly the text sent, kept for seven days, with **Clear the log**.

Settings → About shows the version and the licence (MIT OR Apache-2.0); `THIRD_PARTY.md`, in the
source, lists every library Redpen uses and its licence.

## Checking a text on this computer

Redpen checks English with Harper, a grammar checker that runs on this computer: no account, no
key, no connection, and nothing leaves the computer. The command-line tool runs the same check on
a file. Save a text in a UTF-8 file, then:

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
is a spelling finding with `color` first. In the app, Settings → General → Dialect chooses it,
and the categories below can be switched off there.

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
- In the popup, a fix whose text has changed since the check is refused with "That suggestion no
  longer fits the text. Check again.", and a fix that is no longer offered with "That suggestion
  is not available any more."
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

The popup's six rewrite buttons run on the engine below; `redpen rewrite` runs it on a text file.
Nothing is sent until you run it, and then exactly one request.

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
providers and from the cache). `unchanged` means the answer is your text as it was: the popup
says "No changes needed." The answer is cleaned before you see it: a line such as "Here is the
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

publik API is one of the three ways Redpen rewrites text: it needs no key of your own and is
paid from a publik balance, in dollars. Your own key and a local model stay just as available,
and checking never needs any of them. The screens that set it up are in "Choosing how Redpen
uses AI" above; underneath them, this is how it works.

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
down. The tray menu shows it, greyed out, under **Open Settings**, and updates after every
publik API answer. What each answer cost is added to a count per day and action on this computer.

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

Underneath the shortcut and the popup, this is how Redpen reads the text you selected in any
Windows app and types a result back. It goes through the
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

In the order of `docs/ROADMAP.md`: Linux, the personal dictionary and style presets (with
Settings → Dictionary), the plagiarism check, and installers (with starting with the computer and
the update check).
