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
function reduce(s: PopupState, e: PopupEvent): { state: PopupState; effects: Effect[] }; // Design notes
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
- Decided before implementation, so the interface, the shell and their tests agree:
  - **Dependency**: `tauri-plugin-global-shortcut` 2.4 (Apache-2.0 OR MIT, checked against
    `deny.toml` with Tauri 2.12), registered from Rust only; no npm package is added.
  - **The popup reducer is pure** and returns the effects to run, so Vitest needs no Tauri:
    `reduce(s: PopupState, e: PopupEvent): { state: PopupState; effects: Effect[] }`, in
    `app/src/state/popup.ts`, with `initial(session: Session): PopupState` (the check view).
    ```ts
    type Action = 'fix-grammar' | 'clearer' | 'shorter' | 'more-formal' | 'friendlier' | 'custom'
    type PopupEvent =
      | { type: 'action'; action: Action; instruction?: string } // a button, or keys 1 to 6
      | { type: 'key'; key: string; ctrl: boolean; shift: boolean; inInstruction: boolean }
      | { type: 'rewrite-done'; outcome: RewriteOutcome }
      | { type: 'rewrite-failed'; error: UiError }
      | { type: 'fix-applied'; session: Session }
      | { type: 'consent'; choice: 'publik' | 'own_key' | 'local' }
      | { type: 'connected'; outcome: ConnectOutcome }
      | { type: 'connect-failed'; error: UiError }
      | { type: 'try-again' } | { type: 'another-version' } | { type: 'back' }
      | { type: 'accept'; always: boolean } | { type: 'copy' } | { type: 'cancel' } | { type: 'close' }
    type Effect =
      | { call: 'rewrite'; args: { session_id: string; action: Action; instruction?: string; variant: number } }
      | { call: 'accept'; args: { session_id: string; result_id?: string } }
      | { call: 'copy_result'; args: { session_id: string; result_id?: string } }
      | { call: 'publik_connect' }
      | { call: 'open_settings'; tab: 'providers'; section?: 'own_key' | 'local' }
      | { call: 'open_account_link'; kind: 'claim' | 'add_credit' | 'top_up' }
      | { call: 'close_session'; session_id: string }
    type UiError = { kind: string; message: string; [field: string]: unknown }
    ```
    `key` events map to actions: `1` to `6` (only when `inInstruction` is false), Enter (accepts
    only when the result's `accept_default` is true), Ctrl+Enter (accepts always), Ctrl+Shift+C
    (copies), Escape (back from the result, consent, first-run or error view; closes from the
    check view, with `close_session`). `variant` starts at 0 for an action and goes up by one with
    each **Another version**; Try again after an error repeats the last `rewrite` effect unchanged
    (the core counts validation retries). `NeedsConsent` as an error opens the consent view with
    the pending action; `consent` with `publik` gives `publik_connect`; `connected` with
    `run_pending: false` gives the first-run view, and with `true` the pending `rewrite` effect.
  - **Money**: `formatUsd(micros)` in `app/src/format.ts`, under one cent four decimals
    (`$0.0021`), otherwise two (`$3.12`), 0 as `$0.00`, the same rule as the Rust `balance_line`.
  - **Hotkeys** (`app/src-tauri/src/hotkeys.rs`): `trait HotkeyRegistrar { fn register(&self, hotkey:
    &str) -> Result<(), String>; fn unregister(&self, hotkey: &str); fn registered(&self) ->
    Vec<String>; }` and `trait KeyboardLayout { fn types_character(&self, hotkey: &str) ->
    Option<char>; }` (on Windows `ToUnicodeEx` with Ctrl and Alt down), with fakes for the tests;
    `Hotkeys::change(&mut self, new: &str, registrar, layout) -> Result<(), HotkeyError>` keeps
    the old hotkey registered when the new one is refused; `pause` unregisters every hotkey and
    `resume` registers the same set again.
  - **Popup placement** is a pure function in the core, `redpen_core::platform::placement::place(
    caret: Option<Rect>, cursor: Point, work_areas: &[Rect], size: (u32, u32)) -> Point`: below
    the caret (8 px gap), else at the cursor; flipped above when there is no room below; clamped
    into the work area that holds the caret or the cursor.
  - **The session API** the shell's commands call, in `redpen_core::flow::session`: `Sessions`
    holds captured texts in memory by id (`open`, `get`, `close`), with `apply_fix`, `rewrite`,
    `accept`, `copy_result` and `undo`, written against `Capture`, `AiClient`, `ProviderResolver`,
    `Clock` and `Sleeper`; `close` drops the text. `crates/redpen-core/tests/06_session.rs` drives
    it with `FakeCapture` and `FakeTransport`.
  - **The popup latency check**: `redpen-app --measure-popup` starts `redpen-testwin`, runs 20
    captures from the hotkey handler to findings ready, prints `{"p95_ms": …}` and exits; it runs
    in CI's `windows-integration` job.
- Publik sources fetched raw on 2026-11-13 (`/llms.txt`, `/developers`, `/pricing`,
  `/skills/setup.md`, `/skills/api.md`, `/skills/search.md`, `/skills/publish.md`,
  `/sdk/publik.rs.md`): nothing changed for this module since module 03's reading. The 402 body,
  `top_up_url` ("show the message and that one link"), the $0.05 of free use and "Sign in with
  publik" are as the contract says; `/llms.txt` now also lists `/skills/telemetry.md`, usage
  counting on its own, which Redpen does not send (ADR-0010).
- Decided during implementation:
  - **The session API.** Session and result ids are counters (`s1`, `r1`), so nothing reads
    randomness. `Session` also carries `provider_lines` (the loading line per action on the
    provider the resolver gives now: the tier on publik API, Rewrite quality included, `Your own
    key · <model>`, `Local · <model>`; empty while none is set up) and `can_undo`. `rewrite`
    returns the outcome with `result_id`, `provider`, `provider_line`, `from_cache`,
    `charge_micros` and, on publik API, `balance_line`. A text closed while its call runs drops the
    answer. `SessionGone` says "This text is no longer open in Redpen. Select it and press the
    shortcut again."; `ResultGone` "That result is no longer available. Run the action again."
    Every other error is the wrapped error as its own type serialises it; `NeedsConsent` also
    carries the consent view's strings (`consent`) and `NeedsCredit` the claim state, so the
    popup needs no second call to offer its one way out.
  - **The charge** of a publik API call is read from `x-publik-charge-micros` by the chat client
    whatever status sink it has, so the footer shows it in every set-up.
  - **One popup at a time.** A press of the shortcut closes the text shown before; a press while
    the popup has the focus, or while the last press is still capturing, does nothing. The popup
    window is made hidden at start (460 × 440 logical pixels, frameless, always on top, out of the
    taskbar), so a press only moves, fills and shows it; `popup://open` carries `{session_id,
    can_undo}`, or `{session_id: null, error, can_undo}` when the capture stopped. Placement
    works in physical pixels at the scale of the monitor under the caret (or the mouse).
  - **Accept, Undo and the popup.** The popup hides while Accept types and while Undo runs, and
    comes back with the message when either stops. A successful Accept closes the text. Undo is
    **Undo last replace**, offered by the next popup (on a text or a refusal) while the last Accept
    is still undoable; when the window could not take the text back and the original went on the
    clipboard, the popup comes back to say "The window you started in couldn't take the text back,
    so your original text is on the clipboard. Press Ctrl+V to paste it."
  - **On Linux** the shortcut is registered but says "Redpen can't read a selection on this system
    yet. Checking selections works on Windows." until module 07 brings capture there.
  - **Shortcut grammar.** Modifiers `Ctrl`, `Alt`, `Shift` and `Win` in any order and case, at
    least one of Ctrl, Alt and Win, then one key: `A` to `Z`, `0` to `9`, `F1` to `F24` or `Space`.
    It is stored in the canonical spelling `Ctrl+Alt+Shift+Win+<key>`. Anything else is
    `HotkeyInvalid`, "Use Ctrl, Alt or the Windows key, with Shift if you like, and one letter,
    digit or F1 to F24."; the AltGr refusal is `HotkeyInvalid` too, with the spec's message and the
    character (`character`). The new shortcut is registered before the old one is let go of. A
    shortcut the system refused at start is reported in Settings → General (`hotkey_problem`)
    until a change registers one; setting the same shortcut again asks the system again. Settings'
    recorder reads the physical key, so on an AltGr layout it records the letter, not the
    character it types.
  - **Pause** is a check item in the tray, ticked while paused, and a tick box in Settings →
    General; it is saved as `ui.paused`, and Redpen started while paused registers nothing. The
    paused icon is the normal one in grey: each pixel's luminance (ITU-R BT.601 weights, whole
    numbers) lifted halfway to white, alpha kept. The balance item, when there is one, sits after
    Open Settings.
  - **Welcome** is marked as shown (`ui.welcomed`) as soon as it opens, so a Quit before Close
    does not bring it back. Its Close is `welcome_done`; the launch smoke never opens it.
  - **`settings_update`** takes a partial `settings.json` of any depth. Each leaf is checked on its
    own, so a refusal names its field by its dotted path (`Invalid {field}`, "That value can't be
    used for check.dialect."); `remember_answers_days` is at most 7, the copy timeout 100 to
    5,000 ms and the paste delay 50 to 5,000 ms. The shortcut and the pause state change first and
    `settings.json` is written last; a failed write puts the shortcut back. A file written by a
    newer Redpen is read-only: Settings shows "Settings were written by a newer Redpen." above
    disabled controls, and every change, Pause and the own key are refused (`Invalid`) with that
    message, the file and the shortcut left as they are. `settings_get` adds `own_key_saved`
    (never the key), `read_only` and `hotkey_problem`.
  - **Own key.** `own_key_save` takes `{input: {base_url, key, chat_model}}`; an empty key keeps
    the saved one (with none saved it is `Invalid {field: own_key}`), and an empty model is
    `Invalid {field: providers.own_key.chat_model}`. The address must be `https://`, or `http://`
    on this computer (the host rule of module 02).
  - **Refusals of the settings commands**, each `Invalid` with its message: a `secrets.json` that
    cannot be written is "Your key could not be saved." (`own_key_save`) or "Your key could not be
    removed." (`own_key_clear`); a `redpen.db` that fails is "The log could not be cleared."
    (`sent_log_clear`) or "The remembered answers could not be cleared." (`cache_clear`); a
    Settings window that cannot be made or shown is "Settings could not be opened."
    (`open_settings`). `provider_test` for a kind with nothing to test (anything but `own_key` and
    `local` until module 09 adds `search_key`) answers `{ok: false, message: "There is nothing to
    test for that provider."}` and sends nothing.
  - **`open_settings(tab, section?)`** opens Settings at a tab of General, Providers, Privacy and
    About (General for any other name) and, on Providers, at `publik`, `own_key` or `local`. A new
    window learns its target from a script the shell writes from those fixed names only; an open
    one from the `settings://open` event. Opening at a section counts as choosing that provider,
    since the popup's consent view asked for it.
  - **`usage_month`** sums this month's `usage_daily` rows for publik API (local month), leaving
    out the Test buttons.
  - **The popup latency check** times `Sessions::open` (the refusals, the key wait, the copy, the
    clipboard restored and Harper) from the moment the helper window is in front, on a release
    build, through the same call the shortcut makes. It exits 0 at or under 300 ms, 1 above and 2
    when it could not measure; the test runs only with `REDPEN_DESKTOP_TESTS=1`, since it types
    into the window in front. CI's `windows-integration` job runs it after the launch smoke.
  - **The interface's windows.** Every window loads the same page and shows the screen its label
    names; the window module is loaded on demand, so a page outside the shell never evaluates it.
    The three windows get `core:default` only: no global-shortcut, window or shell permission.
  - **The reducer's own additions** to the events above: `command-failed` (Accept or Copy
    stopped: the error view, whose one button for `FocusChanged`, `WindowGone` and
    `ClipboardBusy` is **Copy**, of the result Accept was for) and `account` (the account link by its kind: `claim`, or
    `add_credit` once linked, on the first-run card and its banner; `top_up` on a 402). The states
    carry `retry` (the last `rewrite` arguments), `banner` (the card above a result when publik
    API was set up with a balance) and, on the check view, `customOpen`. Custom without an
    instruction opens its field, and its instruction is trimmed; the actions also run from the
    result and error views; Escape from loading stops waiting, as Cancel does, and in the
    instruction field closes the field first. `consent` with `publik` on the error view is
    **Reconnect**; on the first-run card it does nothing (the computer is set up). The popup's
    consent view has **Try again**, for after the own key or local model is set up in Settings.
    Answers that arrive after Cancel are dropped.
  - **One way out per error** (`wayOut`): Try again after `InvalidAnswer`, `PlaceholderDropped`,
    `PlaceholderInvented`, `PublikBusy`, `PublikTimeout`, `Timeout` and `ProviderError`; Reconnect
    after `Disconnected`; the one link after `NeedsCredit` (labelled by the claim state); Copy
    after `FocusChanged`, `WindowGone` and `ClipboardBusy`; **Open Settings** (at the section that holds the cure)
    after `NotConfigured`, `NoAppToken`, `ProvisionFailed`, `OwnKeyRefused`, `ModelUnknown`,
    `LocalNotRunning`, `LocalNoModel` and `LocalNotLocal`; nothing otherwise (`Offline`, a 429,
    another publik API status, a selection too long). The 402's link carries `top_up_url` in its
    `href` for the reader, but pressing it asks the shell for the `top_up` link and the window
    itself never navigates.
  - **Keys.** Enter on a focused button or link is that control's own; Ctrl+Enter always
    accepts; keys typed in the instruction field stay there.
  - **Money.** `formatUsd` rounds to the nearest shown place, half up, as 9,999 → `$0.0100`
    requires; `balance_line` rounds down so a balance is never overstated, and the balance shown
    anywhere is always the core's line. The footer of an answer from the cache shows `$0.00`; its
    balance line is `provider_status`'s, read again on every `status://publik`.
  - **The loading line** is the session's line for the action; a session without lines falls back
    to the chosen provider (`publik-fast` for Fix grammar and `publik-balanced` for the others on
    publik API, "Your own key", "Local"), or "Rewriting…" alone.
  - **The consent view** renders once its strings are there (from the error, else
    `consent_info`), so the choice is never offered without the words it is made on.
  - **Settings → Providers.** Choosing publik API with no credential shows the consent view and
    saves nothing; Continue calls `publik_connect`, which saves the choice. At $0.00 the
    first-run card follows, without Try again and without its popup-only line; with a balance, the
    publik API section. Your own key and Local are saved as soon as they are chosen. Local asks
    Ollama (`provider_test("local")` and `local_models`, both on this computer only) when it is
    chosen, when Settings opens at it, and on **Check Ollama**; never when Settings merely opens.
    The wallet is read when Settings opens, only while publik API is chosen and set up
    ("Checking…" meanwhile; "Can't reach publik API right now." when offline). A model can be
    picked from Ollama's list, or left to the automatic choice.
  - **Settings → General** shows the shortcut, the dialect, the four categories and Pause. The
    default rewrite style arrives with module 08, and start with the OS and the update check with
    module 10, so they are not shown before the features exist.
  - **Settings → Privacy** adds a blocklist entry by its program name (`.exe` added when there is
    no extension; names compare without regard to case). Picking a running app needs a command
    docs/ARCHITECTURE.md §5 does not have yet, so it waits.
  - **The first start of a new build.** In testing, Windows held a freshly built `redpen-app`
    for 2 to 9 s before any of its code ran (the antivirus's check of a file it has never seen),
    and the build relinks it whenever the front end is built. With the checker, the rewrite
    engine and capture linked in (64 MB in a debug build) that passed the 5 s module 03 allows
    the self-test to exit, so module 03's built-app test starts the binary once untimed, held to
    the same single line, and times the start after it.

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

Done (2026-11-14)
