// Typed wrappers for every Tauri command (docs/ARCHITECTURE.md §5). The interface never holds
// a key and never builds a URL of its own: it sends intents and renders what the core returns.
// Each module adds the wrappers for the commands it implements.

import { invoke, isTauri } from '@tauri-apps/api/core'

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
] as const

/** The event the shell sends when the publik status changes; its payload is `PublikStatus | null`. */
export const STATUS_EVENT = 'status://publik'

/** True inside the Tauri web view; false in a browser and under the test runner. */
export function insideTauri(): boolean {
  return isTauri()
}

/** Tells the shell that the interface has rendered. In `--smoke` mode the app then exits 0. */
export function uiReady(): Promise<void> {
  return invoke<void>('ui_ready')
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
}): Promise<unknown> {
  return invoke<unknown>('set_providers', change)
}
