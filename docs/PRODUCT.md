# Redpen: product

## 1. Summary

Redpen lets you fix or rewrite your writing in any app on Windows and Linux: select text and press a
hotkey for instant local grammar checks, AI rewrites on request, and a plagiarism check with
citations. It replaces Grammarly and QuillBot for checking and rewriting, without sending what you
type unless you ask.

## 2. Why this exists

**The paid products.**

| Product | Price | Source |
|---|---|---|
| Grammarly Pro | $30 a month, or $144 a year ($12 a month) | [Grammarly support](https://support.grammarly.com/hc/en-us/articles/115000090011-How-much-does-Grammarly-Pro-cost) |
| QuillBot Premium | $19.95 a month, or $99.95 a year | [QuillBot](https://quillbot.com/blog/frequently-asked-questions/how-much-is-quillbot-premium/) |

Both route what you write through their servers, all the time, for a subscription.

**The open source alternatives.** Harper (Apache-2.0) and self-hosted LanguageTool are private but
rule-based: they miss run-on sentences, agreement across clauses and awkward phrasing, and they
live inside editors and browsers rather than every app.

**On Publik.** GSpot (AGPL-3.0, listed and publik verified, re-checked 26 October 2026) is a
Grammarly-style assistant for every **Mac** app: Harper checks in place, AI rewrites, tone,
translation and a plagiarism check, through publik API or the user's own key, with no local-model
option and no Windows or Linux build. Redpen therefore ships for **Windows and Linux only** and does
not remake GSpot on macOS. WordCraft (a Word clone with AI rewrite) works only inside its own
documents. Dictation is a different matter too: Voicebox (MIT) and VoiceStudio (AGPL-3.0) already do
system-wide dictation on Windows with optional local cleanup (re-checked 16 October 2026), so
Redpen does not duplicate it.

**The gap.** Nothing is free, private and smart at once, system-wide, on Windows or Linux. Redpen's design:
the always-on part is local and free (Harper, on this computer), and a model is only called on an
explicit action (a rewrite button), through the user's choice of the publik API, their own key, or
a local model. A paragraph rewrite costs about $0.002 and a typical month of rewrites about $0.28
on Publik (docs/PUBLIK-INTEGRATION.md §8).

## 3. Who it is for

- **The office writer on Windows**: fixes and re-tones email, chat and documents in Outlook,
  Teams, Word and the browser, without a subscription or uploading everything typed.
- **The non-native English professional**: wants Fix grammar with minimal edits in their chosen
  dialect, and More formal for work, with private details redacted.
- **The student on a Windows or Linux laptop**: checks an essay for sentences that already appear
  on the web and copies a citation for each source before handing it in.

## 4. v1 features

**F1 Capture a selection anywhere** (module 05; Linux in 07). Given text selected in
Notepad, Word, Outlook, Teams, Slack, VS Code or a browser, when the user presses Ctrl+Alt+R, then
the popup opens with that text within 300 ms (p95) and whatever was on the clipboard before is back
within 1 s. Given the focus is in a password field, a blocklisted app or an elevated window, then no
copy is attempted and the popup shows the refusal message (docs/ARCHITECTURE.md §8).

**F2 Local checks** (modules 01, 06). Given a captured text, the popup lists Harper's findings
with category (Spelling, Grammar, Punctuation, Style), message and one-click fixes, with no
provider set up and offline. Applying a fix re-checks the text. Dialect (British, American,
Australian, Canadian) and categories are set in Settings. Applying every suggestion twice gives the
same text as applying them once.

**F3 Rewrite actions** (modules 04, 06). Fix grammar, Clearer, Shorter, More formal, Friendlier
and Custom (a typed instruction). The result shows as a word-level diff. Fix grammar warns when it
changes more than a grammar fix should, and then Enter does not accept. A result in another
language warns, unless Custom asked for it. A selection over 1,500 words: "Select at most 1,500
words for a rewrite (this selection has 1,812)."

**F4 Accept, Copy, Undo, Another version, Try again** (modules 05, 06). Accept replaces the
selection in the original window; Copy puts the result on the clipboard; Undo after Accept sends one
Ctrl+Z to that window, or, if it is gone, puts the original on the clipboard and says so; Another
version, after a success, asks for a different rewrite (not for Fix grammar) and the footer shows the
last call's cost on Publik; Try again appears only after an error and never replays an answer that
failed validation.

**F5 Three providers** (modules 02, 03, 06). Settings → Providers shows publik API (balance and
account button), Your own key (base URL, key, chat model, Test), and Local (Ollama state and
model). Given Local is chosen, no request leaves localhost.

**F6 publik API setup** (modules 03, 06). The consent screen appears on the first AI action, never
at launch, with the strings of docs/PUBLIK-INTEGRATION.md §4. At $0.00 the pending action is not
sent and the first-run card offers linking, own key and local model equally. A 402 shows the
message and exactly one link. The tray shows the balance line.

**F7 Safety** (modules 05 and 07). Password fields, secure input, blocklisted apps (password managers
by default) and elevated windows are refused before capture and before insertion.

**F8 Redaction** (module 04). On by default: emails, phone numbers, IBANs, card-shaped and other
long digit runs become markers before a chat call and are restored after; an answer that drops or
invents a marker is refused with its message.

**F9 What is kept** (modules 02, 06). AI answers are cached for at most 7 days so repeating an
action costs nothing (Settings → Privacy: 7 days or Off, and Clear now). An opt-in "what was sent"
log keeps seven days.

**F10 Personal dictionary** (module 08). Words that Harper stops flagging. A word can be added from
a spelling finding or in Settings → Dictionary, removed there, and the list imported and exported
as text.

**F11 Style presets and languages** (module 08). Rewrite style Plain English, Academic, Business or
the author's own; rewrites keep the text's language; checks run for English only, and a
non-English selection says "Checks are available for English only. Rewrites work in your text's
language."

**F12 Linux** (module 07). The same features on Linux X11, with Wayland's limits stated in the UI.
No macOS build: GSpot covers macOS (§2).

**F13 Install and update** (module 10). A per-user Windows installer and a Linux AppImage and
`.deb`, opt-in start with the OS, an update check against the public releases feed that can be
turned off, and Pause Redpen in the tray.

**F14 Plagiarism check** (module 09). Given a selection of 40 to 1,500 words, when the user presses
Plagiarism check (key 7), then Redpen searches at most eight short quoted phrases spread across the
text, through publik API web search (one cent a search, at most $0.08 a check) or the user's own
Brave Search API key, and shows which sentences were found on the web or have similar text, with
each source's title, site and date, Open, Copy link and Copy citation (APA 7, MLA 9 or Chicago).
Phrases never include an email, phone number, IBAN or long number; result pages are never opened by
Redpen; a local model cannot search, so a local-only setup says what the check needs and sends
nothing.

## 5. Screens and flows

**Welcome** (first launch only). Purpose: explain in three sentences and show the hotkey. Text:
"Redpen checks your writing on this computer. Nothing you type leaves it until you ask for a rewrite, or
for a check that needs the internet. Password fields and apps you block are never read." Then the hotkey (Ctrl+Alt+R) and
**Close**. No consent here. States: none beyond this.

**Tray menu.** Open Settings; the balance line when Publik is set up (disabled item); Pause Redpen
(hotkeys off, icon greyed); Quit.

**Popup** (near the caret, else the cursor; focused; Esc closes).
- Check view: the text (read-only, 12 lines then scroll) with findings underlined; the findings
  list with fixes; the six action buttons (keys 1 to 6) and Plagiarism check (key 7, module 09);
  Accept (Enter), Copy, Close. Empty: "No
  issues found." Loading: none (checks are instant).
- Result view: the diff (deletions struck through, insertions highlighted), warnings above it,
  Accept, Copy, Another version, Back; footer with provider line and cost. Loading: "Rewriting… (publik-
  balanced)" with Cancel (which stops waiting; the call is not retried).
- Plagiarism view (module 09): the text with checked sentences marked, the summary line, the
  sources of the selected sentence with Open, Copy link and Copy citation, the footer "Searched with
  publik API · 8 searches · $0.08". Loading: "Searching 3 of 8… (publik API)" with Cancel.
- Consent view and first-run card: docs/PUBLIK-INTEGRATION.md §4.
- Error view: the message, one link or button, and the findings still usable. Offline: "No
  connection. Rewrites need a connection or a local model; checking still works."

**Settings** (a normal window, tabs).
- General: the check hotkey (press to record; conflicts shown inline), dialect, check categories,
  default rewrite style (module 08), start with the OS, update check.
- Providers: AI provider choice; publik API (balance line, "This month: $0.41 across 312 actions",
  **Sign in with publik** or **Add a plan or pack** button, Disconnect, Rewrite quality Standard
  or Best); Your own key; Local; and, from module 09, Web search (for plagiarism checks): publik
  API or "Brave Search API key (for plagiarism checking)" with Test.
  Loading: "Checking…". Error: the provider's message. Offline: "Can't reach publik API right now."
- Privacy: redaction, blocklist (add by picking a running app), remember AI answers (7 days or Off),
  Clear now, the sent log (toggle, list, Clear).
- Dictionary (module 08): the words, add, remove, import and export.
- About: version, licence, third-party licences.

**First-run flow.** (1) Install and launch: tray icon and Welcome. (2) Select text anywhere, press
Ctrl+Alt+R: findings, no setup. (3) Press Clearer: the consent view. (4a) publik API: the computer
is set up; at $0.00 the first-run card; **Sign in with publik** opens the browser; back in the
popup, Try again gives the result. (4b) Own key: Settings → Providers opens; save and Test; Try again. (4c) Local: Settings
shows whether Ollama runs and which model it picked; Try again.

## 6. Non-goals

- Always-on underlining while typing in every app; Harper's extension and editor plugins do that.
- A document editor, AI detection or "humanizing" text.
- A macOS build (GSpot covers macOS, §2).
- Opening or scraping web pages for the plagiarism check; it works from search result snippets.
- Generating text from nothing ("write me an email"); Custom edits a selection.

## 7. Success measures

| Measure | Target | How it is checked |
|---|---|---|
| Time to first useful result | a check within 60 s of first launch, no account, key or download | manual checklist |
| Popup latency | ≤ 300 ms p95 from hotkey to findings for 300 words | Windows integration test (module 05/06) |
| Check speed | ≤ 50 ms for 1,000 words, release build | core benchmark (module 01) |
| Capture success | 10 of 10 apps in the manual checklist | manual checklist per release |
| Clipboard restored | every run, for text, HTML and image | Windows integration test (module 05) |
| Fix grammar restraint | median edit ratio ≤ 0.15 on the corpus; guard warnings on ≤ 5% of correct fixes | `scripts/gates eval-rewrite` (module 04); thresholds confirmed on the first run |
| Language keeping | ≥ 98% on the multilingual corpus | same gate |
| Cost per active user | about $0.28 a month for a typical user, about $1.06 for a heavy one, plus $0.08 per plagiarism check | `usage_daily` in the maintainer's own use after modules 06 and 09 |
| Privacy | zero requests in a check-only session; a plagiarism check sends only its phrases | tests R1 and R14 |

## 8. Later

UI Automation TextPattern capture without the clipboard; streaming results; saved custom actions; a
local grammar model; Windows on Arm; a translated UI; a macOS build if GSpot stops being maintained.

## 9. Listing draft

| Field | Value |
|---|---|
| Name | Redpen |
| Summary | Fix, rewrite or plagiarism-check text in any app on Windows and Linux. (70 characters) |
| Category | Productivity |
| alternativeTo | Grammarly, QuillBot |
| Platforms | Windows, Linux |
| Licence | MIT OR Apache-2.0 |
| Publik API products | ai.chat, search |

Publik lists an app that needs a service it covers (here AI chat and web search) only if the app
runs on publik API. Redpen does: publik API is its default provider, with the own key and the local model as equal
alternatives (docs/PUBLIK-INTEGRATION.md §2).
