# 06: Popup and settings UI

## Goal

Redpen becomes a usable product on Windows: select text anywhere, press Ctrl+Alt+R, see Harper's
findings in a popup near the caret, apply fixes, run any rewrite action, read the diff, and Accept,
Copy, Undo or ask for Another version, all from the keyboard. First use explains the privacy model; the first AI
action shows the consent view, the $0.00 first-run card, and every error with one way out.
Settings configure the hotkey, dialect, providers and privacy.

## Depends on

03, 04, 05.

## Scope

- Hotkey `hotkeys.check` (default Ctrl+Alt+R) through `tauri-plugin-global-shortcut`; changing it
  re-registers; a failed registration or a combination that types a character on the active
  keyboard layout (`ToUnicodeEx` with Ctrl and Alt down) is refused with its message.
- The rewrite flow wired: hotkey → `capture_session` → Harper → `popup://open`; actions →
  `rewrite`; Accept → `deliver`; Copy → `set_clipboard_text`; Undo → the undo buffer.
- Popup window: frameless, always on top, focused; placed below the caret rectangle, else at the
  cursor; flipped above when there is no room; clamped to the monitor's work area; Esc closes and
  `close_session` drops the text from memory.
- Views of docs/PRODUCT.md §5: check, result (diff, warnings, footer, **Another version** for the
  five non-grammar actions), loading with Cancel, consent, first-run card, error (one message, at
  most one link or button, **Try again** where the error allows, findings still usable).
- `Consent.tsx` and `FirstRunCard.tsx` are components used in the popup (with a pending action and
  Try again) and in Settings → Providers (no pending action, no Try again); strings from
  `consent_info` and `ConnectOutcome`.
- Keyboard: 1 to 6 run the actions (when focus is not in the instruction field), Enter accepts when
  `accept_default`, Ctrl+Enter accepts always, Ctrl+Shift+C copies, Esc goes back or closes.
- Footer after an AI action: `<tier or model> · <cost> · <balance line>` on Publik, `Your own key ·
  <model>` or `Local · <model>` otherwise.
- Money formatting: under $0.01 four decimals, otherwise two (`$0.0002`, `$0.0021`, `$3.12`,
  `$0.00`).
- Settings window, tabs General, Providers, Privacy, About (docs/PRODUCT.md §5); the Dictionary tab
  is added by module 08.
- Welcome window on first launch only (a `welcomed` flag in `settings.json`).
- Tray menu: Open Settings, the balance line (module 03), Pause Redpen (unregisters every hotkey,
  grey icon), Quit.
- Tauri commands of docs/ARCHITECTURE.md §5 for sessions, rewrites, settings, own key, provider
  test, local models, usage, sent log, cache and pause.
- Privacy session tests R1 and R2 for rewrites, and the log-hygiene test (docs/TESTING.md §5).

## Files

- `app/src/windows/{Popup.tsx, Consent.tsx, FirstRunCard.tsx, ErrorCard.tsx, Settings.tsx,
  Welcome.tsx}`, `app/src/components/{Diff.tsx, Findings.tsx, Footer.tsx}`
- `app/src/state/{popup.ts, settings.ts}`, `app/src/format.ts`, `app/src/ipc.ts`
- `app/src-tauri/src/{hotkeys.rs, windows.rs, commands/session.rs, commands/settings.rs, tray.rs}`
- `crates/redpen-core/src/flow/rewrite.rs` (accept, copy, undo wiring)
- `docs/HANDBOOK.md` (first chapters: checking, rewriting, providers, privacy)
- Tests: `app/src/state/06-popup-and-settings-ui.test.ts`, `app/src/06-format.test.ts`,
  `crates/redpen-core/tests/06_session.rs`, `app/src-tauri/tests/06_hotkeys.rs`

## Interfaces

Tauri commands and events as listed in docs/ARCHITECTURE.md §5 (this module implements the session,
settings, provider, usage, log, cache and pause rows). UI state machine in `app/src/state/popup.ts`:

```ts
type PopupState =
  | { view: "check"; session: Session }
  | { view: "loading"; session: Session; action: Action; providerLine: string }
  | { view: "result"; session: Session; outcome: RewriteOutcome }
  | { view: "consent"; session: Session; pending: Action; info: ConsentInfo }
  | { view: "first-run"; session: Session; pending: Action; card: FirstRunCard }
  | { view: "error"; session: Session; error: UiError };
function reduce(s: PopupState, e: PopupEvent): PopupState;
function formatUsd(micros: number): string;
```

Messages added here: `HotkeyInUse` "Ctrl+Alt+R is already used by another app. Choose another
shortcut."; AltGr "That shortcut types a character on your keyboard layout. Choose another.";
own key URL "Use an https:// address (or http://localhost for a server on this computer)."; empty
findings "No issues found."; loading "Rewriting… (publik-balanced)".

## Out of scope

- The Dictionary tab and style presets in the popup: module 08.

## Design notes

- Decided: the webview receives only session text, findings, outcomes and keyless status;
  `open_account_link` takes a link kind, never a URL (docs/PUBLIK-INTEGRATION.md §2, A1).
- Further decisions are filled in here during implementation.

## Acceptance tests

Vitest in `app/src/**/06-*.test.ts`; core flow tests in `crates/redpen-core/tests/06_session.rs`
with `FakeCapture` and `FakeTransport`; the manual checklist of docs/TESTING.md §8 for the popup.

1. R1: capture, check, apply one fix, Accept → `requests_seen()` is empty and `paste` received the
   fixed text.
2. R2: capture `Please mail ana@example.com today about the invoice.`, press Clearer, Accept → the
   fake saw exactly one request, with keys `model`, `messages`, `max_tokens`, `temperature` only,
   and a user message containing `[EMAIL_1]` and not the address; `paste` received the restored text.
3. Reducer: check → action 2 → loading → outcome → result; an outcome with `accept_default: false`
   makes Enter a no-op and Ctrl+Enter accept; Another version calls `rewrite` with `variant: 1`,
   is absent for Fix grammar; an `InvalidAnswer` error shows Try again, which calls `rewrite` again
   with the same arguments (the session sets the validation retry).
4. Reducer: `NeedsConsent` → consent view with the pending action; "Continue with publik API" →
   `publik_connect`; `run_pending: false` → first-run view; Try again → loading.
5. The consent view renders `consent_info().data_path` and `disclosure` verbatim and the three
   buttons with their exact labels.
6. The first-run card at `balance_micros: 0` shows the balance line, `cost_sentence`, "Sign in with
   publik", "Your checks keep working. When you have linked this computer, press
   Try again.", Try again, and the own-key and local buttons; the findings list is still rendered.
7. The 402 card contains exactly one `<a href>` (the top-up link) and the text "or use your own key
   or a local model in Settings".
8. Errors `Disconnected`, `PublikBusy`, `Offline` render their messages with Reconnect, Try again
   and no button respectively.
9. `formatUsd`: 0 → `$0.00`; 210 → `$0.0002`; 2,100 → `$0.0021`; 9,999 → `$0.0100`;
   3,120,000 → `$3.12`.
10. Popup placement: caret at the bottom edge of a 1920×1040 work area → popup above the caret;
    caret absent → at the cursor; a caret on the second monitor → inside that monitor.
11. Keyboard map: `1`..`6` dispatch the six actions; with focus in the instruction field `1` types.
12. `settings_update` with a hotkey the fake registrar refuses → `HotkeyInUse` naming it; with a
    combination the fake layout maps to `ć` → the AltGr message; the old hotkey stays registered.
13. Pause Redpen leaves the fake registrar with no hotkeys; unpausing registers the same set again.
14. `close_session` then `session_get` → `SessionGone`.
15. The Welcome window opens on the first launch only; no consent view and no request at launch.
16. Log hygiene: a full scripted rewrite session's `tracing` output contains neither the text,
    `ana@example.com`, the fake key, nor the process name.
17. `redpen-app --measure-popup` on `windows-latest` with `redpen-testwin` reports p95 at or under
    300 ms from hotkey to findings over 20 runs.
18. `Consent` rendered in Settings (no pending action) shows the same strings and buttons as in
    the popup and no Try again; after `publik_connect` at `balance_micros: 0` Settings shows the
    first-run card without Try again.

## Status

Planned
