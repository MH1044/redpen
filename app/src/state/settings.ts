// The Settings window's state (docs/PRODUCT.md §5): where it opens, the shortcut recorder, and the
// AI provider choice of Settings → Providers with its consent view and first-run card. Pure, so
// it is tested without Tauri.

import type {
  Category,
  ChatChoice,
  CommandError,
  ConnectOutcome,
  Dialect,
  ProvidersSection,
  SettingsTab,
} from '../ipc'

/** The default check shortcut (docs/ARCHITECTURE.md §4). */
export const DEFAULT_HOTKEY = 'Ctrl+Alt+R'

/** Settings' tabs; the Dictionary tab arrives with the personal dictionary. */
export const TABS: readonly { id: SettingsTab; label: string }[] = [
  { id: 'general', label: 'General' },
  { id: 'providers', label: 'Providers' },
  { id: 'privacy', label: 'Privacy' },
  { id: 'about', label: 'About' },
]

const SECTIONS: readonly ProvidersSection[] = ['publik', 'own_key', 'local']

/** Where Settings opens: a tab and, on Providers, a section. */
export interface SettingsTarget {
  tab: SettingsTab
  section: ProvidersSection | null
}

/**
 * The target the shell gave (`window.__REDPEN_SETTINGS_TARGET__` or a `settings://open` event),
 * checked: General for anything unknown, a section only on Providers.
 */
export function settingsTarget(value: unknown): SettingsTarget {
  const v = (value ?? {}) as { tab?: unknown; section?: unknown }
  const tab = TABS.find((t) => t.id === v.tab)?.id ?? 'general'
  const section = tab === 'providers' ? (SECTIONS.find((s) => s === v.section) ?? null) : null
  return { tab, section }
}

/** The five dialects Harper offers, as Settings names them. */
export const DIALECTS: readonly { id: Dialect; label: string }[] = [
  { id: 'british', label: 'British English' },
  { id: 'american', label: 'American English' },
  { id: 'australian', label: 'Australian English' },
  { id: 'canadian', label: 'Canadian English' },
  { id: 'indian', label: 'Indian English' },
]

/** The four check categories. */
export const CATEGORIES: readonly { id: Category; label: string }[] = [
  { id: 'spelling', label: 'Spelling' },
  { id: 'grammar', label: 'Grammar' },
  { id: 'punctuation', label: 'Punctuation' },
  { id: 'style', label: 'Style' },
]

/** `disabled` with `category` switched on (removed) or off (added), in the categories' order. */
export function toggleCategory(
  disabled: readonly Category[],
  category: Category,
  on: boolean,
): Category[] {
  const next = new Set(disabled)
  if (on) next.delete(category)
  else next.add(category)
  return CATEGORIES.map((c) => c.id).filter((c) => next.has(c))
}

/** A program name for the blocklist: trimmed, `.exe` added on Windows-style names without one. */
export function blocklistEntry(typed: string): string | null {
  const name = typed.trim()
  if (!name || /[\\/]/.test(name)) return null
  return /\.[a-z0-9]+$/i.test(name) ? name : `${name}.exe`
}

/** `list` with `name` added once (names compare without regard to case). */
export function addToBlocklist(list: readonly string[], name: string): string[] {
  const lower = name.toLowerCase()
  return list.some((n) => n.toLowerCase() === lower) ? [...list] : [...list, name]
}

/** The keys of a key press the shortcut recorder reads. */
export interface KeyPress {
  code: string
  key: string
  ctrlKey: boolean
  altKey: boolean
  shiftKey: boolean
  metaKey: boolean
}

const MODIFIER_CODES = /^(Control|Alt|Shift|Meta|OS)(Left|Right)?$/

/**
 * The shortcut a key press makes, in the core's spelling (`Ctrl+Alt+Shift+Win+<key>`), read from
 * the physical key so AltGr layouts record the letter, not the character it types. `null` while
 * only modifiers are down. The core decides whether the combination can be used.
 */
export function hotkeyFromKey(e: KeyPress): string | null {
  if (MODIFIER_CODES.test(e.code) || ['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) {
    return null
  }
  let key: string
  const letter = /^Key([A-Z])$/.exec(e.code)
  const digit = /^Digit([0-9])$/.exec(e.code)
  if (letter) key = letter[1]
  else if (digit) key = digit[1]
  else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(e.code)) key = e.code
  else if (e.code === 'Space') key = 'Space'
  else key = e.key.length === 1 ? e.key.toUpperCase() : e.key
  const parts: string[] = []
  if (e.ctrlKey) parts.push('Ctrl')
  if (e.altKey) parts.push('Alt')
  if (e.shiftKey) parts.push('Shift')
  if (e.metaKey) parts.push('Win')
  parts.push(key)
  return parts.join('+')
}

/** The account button in Settings → Providers: by the claim state, a link kind, never a URL. */
export function accountButton(claimState: 'anonymous' | 'claimed'): {
  label: string
  kind: 'claim' | 'add_credit'
} {
  return claimState === 'claimed'
    ? { label: 'Add a plan or pack', kind: 'add_credit' }
    : { label: 'Sign in with publik', kind: 'claim' }
}

// Settings → Providers: the AI provider choice.

/**
 * What Providers shows under the choice: nothing (none chosen), the consent view (publik API
 * chosen here with no credential), the first-run card (just set up at $0.00), or the chosen
 * provider's section.
 */
export type ProvidersPane = 'none' | 'consent' | 'first-run' | 'publik' | 'own_key' | 'local'

export interface ProvidersState {
  /** The provider selected in the window (saved, or about to be after consent). */
  chosen: ChatChoice | null
  pane: ProvidersPane
  card?: ConnectOutcome
  error?: CommandError
  connecting?: boolean
}

export type ProvidersEvent =
  /** Settings loaded: the saved choice, whether publik API is set up, the section asked for. */
  | {
      type: 'loaded'
      chat: ChatChoice | null
      publikReady: boolean
      section?: ProvidersSection | null
    }
  /** A provider picked in the window (or a section asked for by the popup). */
  | { type: 'choose'; choice: ChatChoice; publikReady: boolean }
  /** Continue with publik API on the consent view. */
  | { type: 'connect' }
  | { type: 'connected'; outcome: ConnectOutcome }
  | { type: 'connect-failed'; error: CommandError }
  /** Disconnect: the key is forgotten, so publik API needs consent again. */
  | { type: 'disconnected' }

export type ProvidersEffect =
  | { call: 'set_providers'; chat: ChatChoice }
  | { call: 'publik_connect' }
  /** Ask Ollama on this computer what it runs (the Local section's check). */
  | { call: 'check_local' }

export const NO_PROVIDER: ProvidersState = { chosen: null, pane: 'none' }

/**
 * Choosing a provider: publik API with no credential shows the consent view and saves nothing
 * (Continue does, through `publik_connect`); otherwise the choice is saved, and Local also asks
 * Ollama what it runs.
 */
function choose(
  choice: ChatChoice,
  publikReady: boolean,
): {
  state: ProvidersState
  effects: ProvidersEffect[]
} {
  if (choice === 'publik' && !publikReady) {
    return { state: { chosen: 'publik', pane: 'consent' }, effects: [] }
  }
  const effects: ProvidersEffect[] = [{ call: 'set_providers', chat: choice }]
  if (choice === 'local') effects.push({ call: 'check_local' })
  return { state: { chosen: choice, pane: choice }, effects }
}

export function providersReduce(
  state: ProvidersState,
  event: ProvidersEvent,
): { state: ProvidersState; effects: ProvidersEffect[] } {
  switch (event.type) {
    case 'loaded': {
      // A section asked for (from the popup's consent view) is a choice the user made there.
      if (event.section) return choose(event.section, event.publikReady)
      // A choice made in the window before the settings arrived stands.
      if (state.chosen !== null) return { state, effects: [] }
      const chosen = event.chat
      if (!chosen) return { state: NO_PROVIDER, effects: [] }
      if (chosen === 'publik' && !event.publikReady) {
        return { state: { chosen, pane: 'consent' }, effects: [] }
      }
      return { state: { chosen, pane: chosen }, effects: [] }
    }
    case 'choose':
      if (event.choice === state.chosen && state.pane !== 'none') {
        return { state, effects: [] }
      }
      return choose(event.choice, event.publikReady)
    case 'connect':
      if (state.pane !== 'consent' || state.connecting) return { state, effects: [] }
      return {
        state: { chosen: 'publik', pane: 'consent', connecting: true },
        effects: [{ call: 'publik_connect' }],
      }
    case 'connected':
      // At $0.00 the card; with a balance publik API is ready at once. No Try again here: the
      // next rewrite uses the new set-up.
      return event.outcome.run_pending
        ? { state: { chosen: 'publik', pane: 'publik' }, effects: [] }
        : { state: { chosen: 'publik', pane: 'first-run', card: event.outcome }, effects: [] }
    case 'connect-failed':
      return { state: { chosen: 'publik', pane: 'consent', error: event.error }, effects: [] }
    case 'disconnected':
      return {
        state: { chosen: state.chosen, pane: state.chosen === 'publik' ? 'consent' : state.pane },
        effects: [],
      }
  }
}
