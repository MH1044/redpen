// Unit tests of the Settings window's state: where it opens, the shortcut recorder, the lists it
// edits, and the AI provider choice with its consent view and first-run card.

import { describe, expect, it } from 'vitest'
import type { ConnectOutcome } from '../ipc'
import {
  NO_PROVIDER,
  accountButton,
  addToBlocklist,
  blocklistEntry,
  hotkeyFromKey,
  providersReduce,
  settingsTarget,
  toggleCategory,
  type ProvidersEvent,
  type ProvidersState,
} from './settings'

function press(
  code: string,
  key: string,
  mods: Partial<Record<'ctrl' | 'alt' | 'shift' | 'meta', boolean>> = {},
) {
  return {
    code,
    key,
    ctrlKey: Boolean(mods.ctrl),
    altKey: Boolean(mods.alt),
    shiftKey: Boolean(mods.shift),
    metaKey: Boolean(mods.meta),
  }
}

function outcome(runPending: boolean): ConnectOutcome {
  return {
    status: {
      provider: 'publik',
      claim_state: 'anonymous',
      balance_micros: runPending ? 50_000 : 0,
      last_charge_micros: null,
      last_request_id: null,
      starter_remaining_micros: 0,
      week_used_micros: 0,
      week_budget_micros: null,
      week_resets_at: null,
      needs_credit: false,
      disconnected: false,
      updated_at_ms: 0,
    },
    run_pending: runPending,
    balance_line: 'line',
    cost_sentence: 'cost',
    claim_url: 'https://publikhq.com/claim/x',
    claim_state: 'anonymous',
  }
}

function run(s: ProvidersState, ...events: ProvidersEvent[]) {
  let state = s
  const effects = []
  for (const e of events) {
    const step = providersReduce(state, e)
    state = step.state
    effects.push(...step.effects)
  }
  return { state, effects }
}

describe('settingsTarget', () => {
  it('opens a known tab, and a section only on Providers', () => {
    expect(settingsTarget({ tab: 'providers', section: 'own_key' })).toEqual({
      tab: 'providers',
      section: 'own_key',
    })
    expect(settingsTarget({ tab: 'privacy', section: 'local' })).toEqual({
      tab: 'privacy',
      section: null,
    })
    expect(settingsTarget({ tab: 'https://example.com' })).toEqual({
      tab: 'general',
      section: null,
    })
    expect(settingsTarget(undefined)).toEqual({ tab: 'general', section: null })
    expect(settingsTarget({ tab: 'providers', section: 'elsewhere' }).section).toBeNull()
  })
})

describe('hotkeyFromKey', () => {
  it('records the physical key with its modifiers, in the core’s order', () => {
    expect(hotkeyFromKey(press('KeyR', 'r', { ctrl: true, alt: true }))).toBe('Ctrl+Alt+R')
    expect(hotkeyFromKey(press('KeyK', 'K', { ctrl: true, shift: true }))).toBe('Ctrl+Shift+K')
    expect(hotkeyFromKey(press('Digit7', '7', { alt: true, meta: true }))).toBe('Alt+Win+7')
    expect(hotkeyFromKey(press('F12', 'F12', { ctrl: true }))).toBe('Ctrl+F12')
    expect(hotkeyFromKey(press('Space', ' ', { ctrl: true }))).toBe('Ctrl+Space')
  })

  it('reads the letter, not the character AltGr types', () => {
    // Polish layout: Ctrl+Alt+C types ć.
    expect(hotkeyFromKey(press('KeyC', 'ć', { ctrl: true, alt: true }))).toBe('Ctrl+Alt+C')
  })

  it('waits while only modifiers are down', () => {
    expect(hotkeyFromKey(press('ControlLeft', 'Control', { ctrl: true }))).toBeNull()
    expect(hotkeyFromKey(press('AltRight', 'Alt', { ctrl: true, alt: true }))).toBeNull()
    expect(hotkeyFromKey(press('ShiftLeft', 'Shift', { shift: true }))).toBeNull()
  })

  it('passes anything else to the core, which says why it is refused', () => {
    expect(hotkeyFromKey(press('Comma', ',', { ctrl: true, alt: true }))).toBe('Ctrl+Alt+,')
    expect(hotkeyFromKey(press('KeyR', 'r'))).toBe('R')
  })
})

describe('the lists Settings edits', () => {
  it('switches a category on or off, keeping the categories’ order', () => {
    expect(toggleCategory([], 'style', false)).toEqual(['style'])
    expect(toggleCategory(['style', 'spelling'], 'grammar', false)).toEqual([
      'spelling',
      'grammar',
      'style',
    ])
    expect(toggleCategory(['style'], 'style', true)).toEqual([])
  })

  it('takes a program name for the blocklist, once', () => {
    expect(blocklistEntry(' notes ')).toBe('notes.exe')
    expect(blocklistEntry('Proton Pass.exe')).toBe('Proton Pass.exe')
    expect(blocklistEntry('')).toBeNull()
    expect(blocklistEntry('C:\\Tools\\notes.exe')).toBeNull()
    expect(addToBlocklist(['keepass.exe'], 'KeePass.exe')).toEqual(['keepass.exe'])
    expect(addToBlocklist(['keepass.exe'], 'notes.exe')).toEqual(['keepass.exe', 'notes.exe'])
  })

  it('names the account button by the claim state, as a link kind', () => {
    expect(accountButton('anonymous')).toEqual({ label: 'Sign in with publik', kind: 'claim' })
    expect(accountButton('claimed')).toEqual({ label: 'Add a plan or pack', kind: 'add_credit' })
  })
})

describe('the AI provider choice', () => {
  it('shows nothing until a provider is chosen, and sends nothing', () => {
    expect(
      providersReduce(NO_PROVIDER, { type: 'loaded', chat: null, publikReady: false }),
    ).toEqual({ state: NO_PROVIDER, effects: [] })
  })

  it('shows the chosen provider’s section, or consent for publik API with no credential', () => {
    expect(
      providersReduce(NO_PROVIDER, { type: 'loaded', chat: 'own_key', publikReady: false }).state,
    ).toEqual({ chosen: 'own_key', pane: 'own_key' })
    expect(
      providersReduce(NO_PROVIDER, { type: 'loaded', chat: 'publik', publikReady: true }).state,
    ).toEqual({ chosen: 'publik', pane: 'publik' })
    expect(
      providersReduce(NO_PROVIDER, { type: 'loaded', chat: 'publik', publikReady: false }).state,
    ).toEqual({ chosen: 'publik', pane: 'consent' })
  })

  it('choosing publik API with no credential saves nothing until Continue', () => {
    const consent = providersReduce(NO_PROVIDER, {
      type: 'choose',
      choice: 'publik',
      publikReady: false,
    })
    expect(consent).toEqual({ state: { chosen: 'publik', pane: 'consent' }, effects: [] })
    const pressed = providersReduce(consent.state, { type: 'connect' })
    expect(pressed.effects).toEqual([{ call: 'publik_connect' }])
    // A second press while connecting sends nothing more.
    expect(providersReduce(pressed.state, { type: 'connect' }).effects).toEqual([])
  })

  it('at $0.00 shows the first-run card; with a balance, the publik section', () => {
    const zero = run(
      NO_PROVIDER,
      { type: 'choose', choice: 'publik', publikReady: false },
      { type: 'connect' },
      { type: 'connected', outcome: outcome(false) },
    )
    expect(zero.state).toEqual({ chosen: 'publik', pane: 'first-run', card: outcome(false) })
    const funded = run(
      NO_PROVIDER,
      { type: 'choose', choice: 'publik', publikReady: false },
      { type: 'connect' },
      { type: 'connected', outcome: outcome(true) },
    )
    expect(funded.state).toEqual({ chosen: 'publik', pane: 'publik' })
  })

  it('keeps the consent view with the message when the set-up fails', () => {
    const error = { kind: 'ProvisionFailed', message: 'publik API is unreachable right now.' }
    const failed = run(
      NO_PROVIDER,
      { type: 'choose', choice: 'publik', publikReady: false },
      { type: 'connect' },
      { type: 'connect-failed', error },
    )
    expect(failed.state).toEqual({ chosen: 'publik', pane: 'consent', error })
  })

  it('saves the own key and local choices at once; local also asks Ollama', () => {
    expect(
      providersReduce(NO_PROVIDER, { type: 'choose', choice: 'own_key', publikReady: false }),
    ).toEqual({
      state: { chosen: 'own_key', pane: 'own_key' },
      effects: [{ call: 'set_providers', chat: 'own_key' }],
    })
    expect(
      providersReduce(NO_PROVIDER, { type: 'choose', choice: 'local', publikReady: false }).effects,
    ).toEqual([{ call: 'set_providers', chat: 'local' }, { call: 'check_local' }])
  })

  it('a section the popup asked for is chosen as it opens', () => {
    expect(
      providersReduce(NO_PROVIDER, {
        type: 'loaded',
        chat: null,
        publikReady: false,
        section: 'local',
      }).effects,
    ).toEqual([{ call: 'set_providers', chat: 'local' }, { call: 'check_local' }])
  })

  it('a choice made before the settings arrived stands', () => {
    const chosen = providersReduce(NO_PROVIDER, {
      type: 'choose',
      choice: 'publik',
      publikReady: false,
    }).state
    expect(
      providersReduce(chosen, { type: 'loaded', chat: null, publikReady: false }).state,
    ).toEqual(chosen)
  })

  it('Disconnect brings back the consent view for publik API', () => {
    expect(
      providersReduce({ chosen: 'publik', pane: 'publik' }, { type: 'disconnected' }).state,
    ).toEqual({ chosen: 'publik', pane: 'consent' })
  })
})
