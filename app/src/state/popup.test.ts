// Unit tests of the popup reducer's own decisions beyond the spec's key map: the error card's
// one way out, the account link, Accept failing, Custom's instruction, Try again on the consent
// view, Reconnect, and answers that arrive too late.

import { describe, expect, it } from 'vitest'
import type { ConnectOutcome, RewriteOutcome, Session } from '../ipc'
import { initial, reduce, wayOut, type PopupEvent, type PopupState } from './popup'

const SESSION: Session = {
  id: 's1',
  text: 'I saw teh cat.',
  findings: [],
  dialect: 'british',
  english: true,
  provider_lines: { clearer: 'publik-balanced', 'fix-grammar': 'publik-fast' },
}

const OUTCOME: RewriteOutcome = {
  result_id: 'r1',
  text: 'I saw the cat.',
  diff: [{ equal: 'I saw the cat.' }],
  warnings: [],
  accept_default: true,
  unchanged: false,
  provider_line: 'publik-balanced',
  charge_micros: 2100,
}

function card(balance: number, claimState: 'anonymous' | 'claimed'): ConnectOutcome {
  return {
    status: {
      provider: 'publik',
      claim_state: claimState,
      balance_micros: balance,
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
    run_pending: balance > 0,
    balance_line: 'line',
    cost_sentence: 'cost',
    claim_url: 'https://publikhq.com/claim/x',
    claim_state: claimState,
  }
}

function run(s: PopupState, ...events: PopupEvent[]) {
  let state = s
  const effects = []
  for (const e of events) {
    const step = reduce(state, e)
    state = step.state
    effects.push(...step.effects)
  }
  return { state, effects }
}

const CHECK = initial(SESSION)

describe('wayOut', () => {
  it('offers Try again only where the error allows and there is a call to repeat', () => {
    for (const kind of ['InvalidAnswer', 'PlaceholderDropped', 'PublikBusy', 'PublikTimeout']) {
      expect(wayOut({ kind, message: 'm' }, true)).toEqual({ kind: 'try-again' })
      expect(wayOut({ kind, message: 'm' }, false)).toBeNull()
    }
    for (const kind of ['Offline', 'PublikRateLimited', 'TooLong', 'PublikHttp', 'Unrecorded']) {
      expect(wayOut({ kind, message: 'm' }, true)).toBeNull()
    }
  })

  it('labels the 402 link by the claim state, and needs the address', () => {
    const anonymous = {
      kind: 'NeedsCredit',
      message: 'm',
      top_up_url: 'https://x',
      claim_state: 'anonymous',
    }
    expect(wayOut(anonymous, true)).toEqual({ kind: 'top-up', label: 'Sign in with publik' })
    expect(wayOut({ ...anonymous, claim_state: 'claimed' }, true)).toEqual({
      kind: 'top-up',
      label: 'Add a plan or pack',
    })
    expect(wayOut({ kind: 'NeedsCredit', message: 'm' }, true)).toBeNull()
  })

  it('reconnects, copies instead, or opens Settings where the cure is', () => {
    expect(wayOut({ kind: 'Disconnected', message: 'm' }, false)).toEqual({ kind: 'reconnect' })
    expect(wayOut({ kind: 'FocusChanged', message: 'm' }, false)).toEqual({ kind: 'copy' })
    expect(wayOut({ kind: 'WindowGone', message: 'm' }, false)).toEqual({ kind: 'copy' })
    expect(wayOut({ kind: 'ClipboardBusy', message: 'm' }, false)).toEqual({ kind: 'copy' })
    expect(wayOut({ kind: 'OwnKeyRefused', message: 'm' }, true)).toEqual({
      kind: 'settings',
      section: 'own_key',
    })
    expect(wayOut({ kind: 'LocalNotRunning', message: 'm' }, true)).toEqual({
      kind: 'settings',
      section: 'local',
    })
    expect(wayOut({ kind: 'NotConfigured', message: 'm' }, true)).toEqual({ kind: 'settings' })
  })
})

describe('reduce', () => {
  it('names the loading view after the session’s provider line for the action', () => {
    const loading = reduce(CHECK, { type: 'action', action: 'fix-grammar' }).state
    expect(loading).toMatchObject({ view: 'loading', providerLine: 'publik-fast' })
  })

  it('opens Custom’s field without an instruction and runs it with one, trimmed', () => {
    const open = reduce(CHECK, { type: 'action', action: 'custom', instruction: '  ' })
    expect(open.state).toEqual({ view: 'check', session: SESSION, customOpen: true })
    expect(open.effects).toEqual([])
    const ran = reduce(open.state, { type: 'action', action: 'custom', instruction: ' Shorter. ' })
    expect(ran.effects).toEqual([
      {
        call: 'rewrite',
        args: { session_id: 's1', action: 'custom', instruction: 'Shorter.', variant: 0 },
      },
    ])
    // Escape closes the field before it closes the popup.
    expect(
      reduce(open.state, {
        type: 'key',
        key: 'Escape',
        ctrl: false,
        shift: false,
        inInstruction: true,
      }),
    ).toEqual({
      state: CHECK,
      effects: [],
    })
  })

  it('drops an answer that arrives after Cancel', () => {
    const cancelled = run(CHECK, { type: 'action', action: 'clearer' }, { type: 'cancel' }).state
    expect(reduce(cancelled, { type: 'rewrite-done', outcome: OUTCOME }).state).toBe(cancelled)
    expect(
      reduce(cancelled, { type: 'rewrite-failed', error: { kind: 'Offline', message: 'm' } }).state,
    ).toBe(cancelled)
  })

  it('keeps the result an Accept failed for, so Copy copies it', () => {
    const result = run(
      CHECK,
      { type: 'action', action: 'clearer' },
      { type: 'rewrite-done', outcome: OUTCOME },
    ).state
    const failed = reduce(result, {
      type: 'command-failed',
      error: { kind: 'FocusChanged', message: 'm' },
    })
    expect(failed.state).toMatchObject({ view: 'error', resultId: 'r1' })
    expect(reduce(failed.state, { type: 'copy' }).effects).toEqual([
      { call: 'copy_result', args: { session_id: 's1', result_id: 'r1' } },
    ])
    // A failed Accept is not a rewrite: there is nothing to try again.
    expect(reduce(failed.state, { type: 'try-again' }).effects).toEqual([])
  })

  it('offers Try again on the consent view, for after the own key or local model is set up', () => {
    const consent = run(
      CHECK,
      { type: 'action', action: 'shorter' },
      { type: 'rewrite-failed', error: { kind: 'NeedsConsent', message: 'm' } },
    ).state
    expect(consent).toMatchObject({ view: 'consent', info: null })
    expect(reduce(consent, { type: 'try-again' }).effects).toEqual([
      { call: 'rewrite', args: { session_id: 's1', action: 'shorter', variant: 0 } },
    ])
  })

  it('takes the consent strings the error carries', () => {
    const info = { title: 't', data_path: 'd', disclosure: 'x', disclosure_version: 1 }
    const consent = run(
      CHECK,
      { type: 'action', action: 'clearer' },
      { type: 'rewrite-failed', error: { kind: 'NeedsConsent', message: 'm', consent: info } },
    ).state
    expect(consent).toMatchObject({ view: 'consent', info })
  })

  it('Reconnect sets up publik API again and runs the call that failed', () => {
    const disconnected = run(
      CHECK,
      { type: 'action', action: 'clearer' },
      { type: 'rewrite-failed', error: { kind: 'Disconnected', message: 'm' } },
    ).state
    const pressed = reduce(disconnected, { type: 'consent', choice: 'publik' })
    expect(pressed.effects).toEqual([{ call: 'publik_connect' }])
    const funded = reduce(disconnected, { type: 'connected', outcome: card(50_000, 'claimed') })
    expect(funded.state).toMatchObject({ view: 'loading', action: 'clearer' })
    const zero = reduce(disconnected, { type: 'connected', outcome: card(0, 'anonymous') })
    expect(zero.state).toMatchObject({ view: 'first-run', pending: 'clearer' })
  })

  it('opens the account link by its kind: claim, add credit, or the 402’s top-up', () => {
    const consent = run(
      CHECK,
      { type: 'action', action: 'clearer' },
      { type: 'rewrite-failed', error: { kind: 'NeedsConsent', message: 'm' } },
    ).state
    const anonymous = reduce(consent, { type: 'connected', outcome: card(0, 'anonymous') }).state
    expect(reduce(anonymous, { type: 'account' }).effects).toEqual([
      { call: 'open_account_link', kind: 'claim' },
    ])
    const claimed = reduce(consent, { type: 'connected', outcome: card(0, 'claimed') }).state
    expect(reduce(claimed, { type: 'account' }).effects).toEqual([
      { call: 'open_account_link', kind: 'add_credit' },
    ])
    // On the card, "publik" is the account button, never a second set-up.
    expect(reduce(claimed, { type: 'consent', choice: 'publik' }).effects).toEqual([])
    const banner = run(
      consent,
      { type: 'connected', outcome: card(50_000, 'anonymous') },
      { type: 'rewrite-done', outcome: OUTCOME },
    ).state
    expect(banner).toMatchObject({ view: 'result', banner: { run_pending: true } })
    expect(reduce(banner, { type: 'account' }).effects).toEqual([
      { call: 'open_account_link', kind: 'claim' },
    ])
    const credit = run(
      CHECK,
      { type: 'action', action: 'clearer' },
      {
        type: 'rewrite-failed',
        error: { kind: 'NeedsCredit', message: 'm', top_up_url: 'https://x' },
      },
    ).state
    expect(reduce(credit, { type: 'account' }).effects).toEqual([
      { call: 'open_account_link', kind: 'top_up' },
    ])
    expect(reduce(CHECK, { type: 'account' }).effects).toEqual([])
  })

  it('applies a fix in any view without leaving it', () => {
    const consent = run(
      CHECK,
      { type: 'action', action: 'clearer' },
      { type: 'rewrite-failed', error: { kind: 'NeedsConsent', message: 'm' } },
    ).state
    const fixed = { ...SESSION, text: 'I saw the cat.' }
    const next = reduce(consent, { type: 'fix-applied', session: fixed }).state
    expect(next).toMatchObject({ view: 'consent', session: fixed, pending: 'clearer' })
  })

  it('ignores the action keys while a call runs, and Escape stops waiting', () => {
    const loading = reduce(CHECK, { type: 'action', action: 'clearer' }).state
    const key = (k: string) =>
      ({ type: 'key', key: k, ctrl: false, shift: false, inInstruction: false }) as const
    expect(reduce(loading, key('3'))).toEqual({ state: loading, effects: [] })
    expect(reduce(loading, key('Escape')).state).toEqual(CHECK)
  })
})
