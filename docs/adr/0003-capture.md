# ADR-0003: Capture: the clipboard route with UI Automation checks

- **Status:** Accepted
- **Date:** 2026-10-27

## Context

Redpen must read the selection in whatever app is in front and later type a result back into it,
system-wide, on Windows first and Linux after. Windows offers UI Automation's TextPattern, which
many apps (Chromium-based ones in particular, and most Electron apps) implement only partly or
not at all, and the clipboard, which every app that can copy and paste supports. Reading a
password field or typing into one is never acceptable, and the user's clipboard must survive
untouched (`docs/ARCHITECTURE.md` §7, invariants 3 to 5).

## Decision

The clipboard route: Redpen sends Ctrl+C to read the selection and Ctrl+V to replace it, after
saving every clipboard format it can and restoring them afterwards, with its temporary entry
marked to stay out of clipboard history and cloud sync. Before any synthetic key, and again before
Accept pastes, UI Automation and the process and token APIs decide whether the focused field is a
password field, the app is blocklisted (password managers by default) or the window runs as
administrator; any of those refuses with its message. UI Automation's TextPattern, which would
avoid the clipboard where it is implemented, comes later as an optimisation, not as the primary
route.

## Consequences

- Every app that can copy and paste works, including the ones with no accessibility tree.
- The clipboard is written and restored on every capture and replace, so the save and restore
  code is load-bearing and is tested format by format on a real Windows runner.
- Known limits, stated in the handbook: windows running as administrator (Windows refuses the
  synthetic keys), apps that ignore synthetic input, and terminals where Ctrl+C interrupts
  rather than copies.
- On Wayland there is no synthetic input at all, so Redpen only reads text the user copied and
  puts results on the clipboard (module 07).

## Alternatives considered

- **UI Automation TextPattern only**: many apps do not implement it, so capture would fail
  exactly in the apps people write in most.
- **Per-app plugins** (an Outlook add-in, a browser extension, a Word add-in): not system-wide,
  and a new integration for every app.
- **A keyboard hook that records what is typed**: reads everything, all the time, which is the
  privacy model Redpen exists to avoid.
