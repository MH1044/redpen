// Typed wrappers for every Tauri command (docs/ARCHITECTURE.md §5). The interface never holds
// a key and never builds a URL of its own: it sends intents and renders what the core returns.
// Each module adds the wrappers for the commands it implements.

import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

/** Every command the shell exposes, in the order of the Rust `generate_handler!` list. */
export const COMMANDS = [
  'ui_ready',
  'consent_info',
  'publik_connect',
  'publik_disconnect',
  'publik_refresh',
  'provider_status',
  'open_account_link',
  'set_providers',
  'session_get',
  'apply_fix',
  'rewrite',
  'accept',
  'copy_result',
  'undo_last_replace',
  'close_session',
  'settings_get',
  'settings_update',
  'own_key_save',
  'own_key_clear',
  'provider_test',
  'local_models',
  'usage_month',
  'sent_log_list',
  'sent_log_clear',
  'cache_clear',
  'app_pause',
  'open_settings',
  'welcome_done',
] as const

export type Command = (typeof COMMANDS)[number]

/** The event the shell sends when the publik status changes; its payload is `PublikStatus | null`. */
export const STATUS_EVENT = 'status://publik'
/** The event that tells the popup to show a session, or why the capture stopped. */
export const POPUP_OPEN_EVENT = 'popup://open'
/** The event that tells an open Settings window which tab (and section) to show. */
export const SETTINGS_OPEN_EVENT = 'settings://open'

/** True inside the Tauri web view; false in a browser and under the test runner. */
export function insideTauri(): boolean {
  return isTauri()
}

/** Tells the shell that the interface has rendered. In `--smoke` mode the app then exits 0. */
export function uiReady(): Promise<void> {
  return invoke<void>('ui_ready')
}

/** Listens for one of the shell's events; resolves to the function that stops listening. */
export function onEvent<T>(name: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(name, (event) => handler(event.payload))
}

// publik API (module 03). None of these types carries a key.

export type ClaimState = 'anonymous' | 'claimed'
export type ChatChoice = 'publik' | 'own_key' | 'local'
export type SearchChoice = 'publik' | 'own_key'
export type RewriteQuality = 'standard' | 'best'
/** The links `openAccountLink` can open: a kind, never a URL. */
export type LinkKind = 'claim' | 'add_credit' | 'top_up'

/** The publik status, as `status.json` holds it (docs/ARCHITECTURE.md §4). */
export interface PublikStatus {
  provider: 'publik'
  claim_state: ClaimState
  balance_micros: number
  last_charge_micros: number | null
  last_request_id: string | null
  starter_remaining_micros: number
  week_used_micros: number
  week_budget_micros: number | null
  week_resets_at: string | null
  needs_credit: boolean
  disconnected: boolean
  updated_at_ms: number
}

/** The consent view's title, data-path sentence and disclosure, shown verbatim. */
export interface ConsentInfo {
  title: string
  data_path: string
  disclosure: string
  disclosure_version: number
}

/** What Continue with publik API returns: the first-run card, and whether the pending action runs. */
export interface ConnectOutcome {
  status: PublikStatus
  run_pending: boolean
  balance_line: string
  cost_sentence: string
  claim_url: string
  claim_state: ClaimState
}

/** The chosen AI provider and, while publik API is set up, its status and balance line. */
export interface ProviderStatus {
  chat: ChatChoice | null
  publik: PublikStatus | null
  balance_line: string | null
}

/** A command's error: `{kind, message}` plus the error's own fields (such as `top_up_url`). */
export interface CommandError {
  kind: string
  message: string
  [field: string]: unknown
}

/** A rejected command as a `CommandError`, whatever was thrown. */
export function asCommandError(e: unknown): CommandError {
  if (e !== null && typeof e === 'object' && typeof (e as CommandError).message === 'string') {
    const err = e as CommandError
    return { ...err, kind: typeof err.kind === 'string' ? err.kind : 'Error' }
  }
  return { kind: 'Error', message: typeof e === 'string' ? e : 'Something went wrong.' }
}

export function consentInfo(): Promise<ConsentInfo> {
  return invoke<ConsentInfo>('consent_info')
}

export function publikConnect(): Promise<ConnectOutcome> {
  return invoke<ConnectOutcome>('publik_connect')
}

export function publikDisconnect(): Promise<void> {
  return invoke<void>('publik_disconnect')
}

/** Settings opening: reads the wallet, only while publik API is chosen and set up. */
export function publikRefresh(): Promise<PublikStatus> {
  return invoke<PublikStatus>('publik_refresh')
}

export function providerStatus(): Promise<ProviderStatus> {
  return invoke<ProviderStatus>('provider_status')
}

/** Opens the claim, add-credit or top-up link in the system browser. */
export function openAccountLink(kind: LinkKind): Promise<void> {
  return invoke<void>('open_account_link', { kind })
}

/** Settings → Providers. A field left out stays as it is; resolves to the saved settings. */
export function setProviders(change: {
  chat?: ChatChoice
  rewrite_quality?: RewriteQuality
  search?: SearchChoice
}): Promise<Settings> {
  return invoke<Settings>('set_providers', change)
}

// The popup's sessions (module 06).

/** The six rewrite actions, one per popup button (keys 1 to 6). */
export type Action = 'fix-grammar' | 'clearer' | 'shorter' | 'more-formal' | 'friendlier' | 'custom'
export type Category = 'spelling' | 'grammar' | 'punctuation' | 'style'
export type Dialect = 'american' | 'british' | 'australian' | 'canadian' | 'indian'
export type ProviderKind = 'publik' | 'own_key' | 'local' | 'fake'

/** One way to resolve a finding. */
export interface Suggestion {
  kind: 'replace' | 'remove' | 'insert_after'
  text: string
}

/** One of Harper's findings; spans are `[start, end)` in characters, UTF-8 bytes and UTF-16 units. */
export interface Finding {
  id: string
  category: Category
  harper_kind: string
  message: string
  span: { chars: [number, number]; bytes: [number, number]; utf16: [number, number] }
  suggestions: Suggestion[]
}

/** What `session_get` returns: the text as it is now, its findings and the dialect. */
export interface Session {
  id: string
  text: string
  findings: Finding[]
  dialect: Dialect
  english: boolean
  /** The loading view's provider line per action, while a provider is set up. */
  provider_lines?: Partial<Record<Action, string>>
  /** Whether the last Accept can still be undone. */
  can_undo?: boolean
}

/** One part of a word diff. */
export type DiffOp = { equal: string } | { insert: string } | { delete: string }

/** A warning above a result: `{kind, message, ...}` from the core, or its message alone. */
export type Warning = string | { kind: string; message: string; [field: string]: unknown }

/** What `rewrite` returns. */
export interface RewriteOutcome {
  result_id: string
  text: string
  diff: DiffOp[]
  warnings: Warning[]
  accept_default: boolean
  unchanged: boolean
  from_cache?: boolean
  provider?: ProviderKind
  /** The tier on publik API, "Your own key · <model>" or "Local · <model>". */
  provider_line: string
  /** What publik API charged, in micro-dollars; absent from the cache and off publik API. */
  charge_micros?: number | null
  /** On publik API, the balance line after the call. */
  balance_line?: string | null
}

/** The `popup://open` payload: a session to show, or why the capture stopped. */
export interface PopupOpen {
  session_id: string | null
  error?: CommandError
  can_undo?: boolean
}

export function sessionGet(sessionId: string): Promise<Session> {
  return invoke<Session>('session_get', { session_id: sessionId })
}

export function applyFix(
  sessionId: string,
  findingId: string,
  suggestionIndex: number,
): Promise<Session> {
  return invoke<Session>('apply_fix', {
    session_id: sessionId,
    finding_id: findingId,
    suggestion_index: suggestionIndex,
  })
}

/** The arguments of one `rewrite` call; Try again sends the same ones again. */
export interface RewriteArgs {
  session_id: string
  action: Action
  instruction?: string
  variant: number
}

export function rewrite(args: RewriteArgs): Promise<RewriteOutcome> {
  return invoke<RewriteOutcome>('rewrite', { ...args })
}

/** Types the result (or, without one, the session's text) into the window it came from. */
export function accept(sessionId: string, resultId?: string): Promise<void> {
  return invoke<void>('accept', withResult(sessionId, resultId))
}

/** Puts the result (or, without one, the session's text) on the clipboard. */
export function copyResult(sessionId: string, resultId?: string): Promise<void> {
  return invoke<void>('copy_result', withResult(sessionId, resultId))
}

function withResult(sessionId: string, resultId?: string): Record<string, string> {
  return resultId === undefined
    ? { session_id: sessionId }
    : { session_id: sessionId, result_id: resultId }
}

export type UndoOutcome = 'undone' | 'on_clipboard'

export function undoLastReplace(): Promise<UndoOutcome> {
  return invoke<UndoOutcome>('undo_last_replace')
}

/** Esc: the session's text is dropped from memory and the popup hides. */
export function closeSession(sessionId: string): Promise<void> {
  return invoke<void>('close_session', { session_id: sessionId })
}

// Settings (module 06).

export type CitationStyle = 'apa7' | 'mla9' | 'chicago18'

/** `settings.json` (docs/ARCHITECTURE.md §4). */
export interface Settings {
  version: number
  hotkeys: { check: string }
  check: { dialect: Dialect; disabled_categories: Category[] }
  providers: {
    chat: ChatChoice | null
    rewrite_quality: RewriteQuality
    search: SearchChoice | null
    own_key: { base_url: string; chat_model: string }
    local: { chat_base_url: string; chat_model: string | null }
  }
  privacy: {
    redaction: boolean
    remember_answers_days: number
    sent_log: boolean
    blocklist: string[]
  }
  capture: { copy_timeout_ms: number; paste_restore_delay_ms: number }
  plagiarism: { citation_style: CitationStyle }
  ui: { start_with_os: boolean; update_check: boolean; paused: boolean; welcomed: boolean }
}

/**
 * What `settings_get` returns: the settings, whether an own key is saved (never the key), whether
 * the file is read-only, and why the check shortcut does nothing when the system refused it.
 */
export type SettingsView = Settings & {
  own_key_saved?: boolean
  read_only?: boolean
  hotkey_problem?: string | null
}

/** A partial `settings.json`, any depth. */
export type SettingsPatch = { [field: string]: unknown }

export function settingsGet(): Promise<SettingsView> {
  return invoke<SettingsView>('settings_get')
}

/** Merges `patch` into the settings; refused with `HotkeyInUse`, `HotkeyInvalid` or `Invalid`. */
export function settingsUpdate(patch: SettingsPatch): Promise<Settings> {
  return invoke<Settings>('settings_update', { patch })
}

/** The own key: the address and model go to `settings.json`, the key to `secrets.json`. */
export function ownKeySave(input: { base_url: string; key: string; chat_model: string }) {
  return invoke<void>('own_key_save', { input })
}

export function ownKeyClear(): Promise<void> {
  return invoke<void>('own_key_clear')
}

export interface ProviderTest {
  ok: boolean
  message: string
}

/** The Test buttons: the own key's model list, or Ollama on this computer. */
export function providerTest(kind: 'own_key' | 'local'): Promise<ProviderTest> {
  return invoke<ProviderTest>('provider_test', { kind })
}

export interface LocalModel {
  name: string
  parameter_size: string
}

export function localModels(): Promise<LocalModel[]> {
  return invoke<LocalModel[]>('local_models')
}

export interface UsageMonth {
  total_micros: number
  calls: number
  by_feature: { feature: string; calls: number; charge_micros: number }[]
}

export function usageMonth(): Promise<UsageMonth> {
  return invoke<UsageMonth>('usage_month')
}

/** One row of the opt-in "what was sent" log. */
export interface SentEntry {
  id: number
  at_ms: number
  provider: string
  endpoint: string
  feature: string
  model: string
  sent_text: string
  bytes: number
}

export function sentLogList(): Promise<SentEntry[]> {
  return invoke<SentEntry[]>('sent_log_list')
}

export function sentLogClear(): Promise<void> {
  return invoke<void>('sent_log_clear')
}

export function cacheClear(): Promise<void> {
  return invoke<void>('cache_clear')
}

/** Pause Redpen (every hotkey let go of) or resume. */
export function appPause(paused: boolean): Promise<void> {
  return invoke<void>('app_pause', { paused })
}

export type SettingsTab = 'general' | 'providers' | 'privacy' | 'about'
export type ProvidersSection = 'publik' | 'own_key' | 'local'

/** Opens Settings at a tab and, on Providers, a section. */
export function openSettings(tab: SettingsTab, section?: ProvidersSection): Promise<void> {
  return invoke<void>('open_settings', section === undefined ? { tab } : { tab, section })
}

/** The welcome window's Close. */
export function welcomeDone(): Promise<void> {
  return invoke<void>('welcome_done')
}
