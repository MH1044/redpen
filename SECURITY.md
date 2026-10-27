# Security

## Threat model

Redpen is local-first. Checking runs on this computer, the app is fully useful with no account,
key or connection, and text leaves the machine only inside a request the user triggered. The
things worth protecting, and what protects them:

| Asset | Protection |
|---|---|
| The text you select | Read only when you press the hotkey; leaves the machine only in the one request you trigger (a rewrite button, Plagiarism check), after redaction of emails, phone numbers, IBANs, card numbers and long digit runs (invariants 1 and 2 of `docs/ARCHITECTURE.md` §7); the rest of the document, the window title and the process name never leave |
| Passwords and secure fields | Password fields, secure input, blocklisted apps (password managers by default) and windows running as administrator are refused before anything is copied and again before anything is typed (invariants 3 and 4) |
| Your clipboard | Saved before every capture and replace and restored after; Redpen's temporary entry is marked to stay out of clipboard history and cloud sync (invariant 5) |
| The publik API key | Minted per install, stored at the publik credential path with mode 0600; never reaches the web view, a log line, the status file, an error or a command result (invariant 6) |
| Your own keys | `secrets.json` in the per-user config folder, mode 0600 on Linux; never in `settings.json` |
| Money | One user action makes at most one attempt per publik API call, never an automatic retry (invariant 9); identical input is served from the cache; a plagiarism check sends at most 8 searches and shows the total (invariant 13) |
| What you accept | Every AI answer passes a JSON Schema and the feature's checks before it is shown; a failure shows nothing and inserts nothing (invariant 8) |

Untrusted input is treated as such: every AI response and every search result is validated
before use, and settings written by a newer version are opened read-only. The web view has no
filesystem, shell, dialog or HTTP permission of its own (`app/src-tauri/capabilities/default.json`);
it sends intents to commands, and the core decides what happens. The web view never opens a URL
it made up: the only links it can open are the claim, add-credit and top-up URLs the core holds,
the release page, and a search result's URL that you chose to open, each handed to the system
browser.

Out of scope: a compromised operating-system account, which can read any local file, and apps
that ignore synthetic keyboard input.

## Every network call the app can make

This is the complete list (`docs/ARCHITECTURE.md` §8). Nothing else contacts the network, and
tests over the fake transport prove that a check-only session makes no request at all. At version
0.1.0 none of these calls exists yet; each arrives with the module named.

1. **Chat calls** to the chosen provider (module 04): `POST {publik API | your own key | local}/chat/completions`.
2. **Publik housekeeping** (module 03): `POST /installs` after consent, `POST /installs/revoke` on
   Disconnect, `GET /wallet` at start and when Settings opens, only while publik API is the chosen
   AI provider and a credential exists; never while your own key or a local model is chosen. With
   `PUBLIK_SELF_TEST` set (the built-app check, never in normal use), one `POST /installs` at
   start, then the app exits.
3. **Provider tests** (module 02): your own key's Test makes `GET {base_url}/models`; the local
   model's `GET http://localhost:11434/api/tags`. The local provider never contacts a host other
   than localhost (invariant 12).
4. **Web search** (module 09), only when Plagiarism check or the search key's Test is pressed:
   `POST {publik API}/search`, or `GET https://api.search.brave.com/res/v1/web/search` with your
   own Brave Search API key; at most 8 per check, each a quoted phrase of at most 12 words.
   Opening a result hands its URL to the system browser; Redpen never fetches a result page.
5. **Update check** (module 10): one `GET` of the public releases feed, carrying nothing but the
   request and a `User-Agent` of `Redpen/<version>`; off in development builds; switchable off in
   Settings.

Opening a claim or top-up link hands the URL to the system browser; the app itself sends nothing.

What each feature sends, and what never leaves the machine, is listed feature by feature in
`docs/PUBLIK-INTEGRATION.md` §10, and the consent screen says it in one sentence before the first
AI action, never at launch.

The provisioning request carries a `device_name` of `Windows PC` or `Linux PC`, a generic label,
never the hostname.

## Keys

- The publik API key rides only in the `Authorization` header: never a query string, a command
  line, a log line or a file inside the app data directory.
- The app token in `app/src-tauri/publik-app-token.txt` (from module 03) is public by design: it
  can only mint keys capped to this app.
- Keys never appear in error messages, the status object, events, or anything the interface
  receives. The test suite scans every one of them for a fake key, and a secret header's debug
  output is `***`.
- Logs (`logs/redpen-YYYY-MM-DD.log`, seven days) record event names, durations, sizes in bytes,
  error kinds and HTTP status codes. Never document text, keys, balances, window titles, process
  names or URLs with query strings.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting on this repository (the Security tab, "Report a
vulnerability"), so the report stays private until a fix is released. Please do not open a public
issue for a security problem.
