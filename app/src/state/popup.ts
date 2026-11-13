// The popup's state machine (docs/modules/06-popup-and-settings-ui.md, Design notes). `reduce` is
// pure: it returns the next state and the effects to run (Tauri commands), so it is tested
// without Tauri and the component only runs what it is told.

import type {
  Action,
  CommandError,
  ConnectOutcome,
  ConsentInfo,
  LinkKind,
  ProvidersSection,
  RewriteArgs,
  RewriteOutcome,
  Session,
} from '../ipc'

export type { Action, ConnectOutcome, ConsentInfo, RewriteOutcome, Session }

/** An error as a command returns it: `{kind, message}` plus its own fields. */
export type UiError = CommandError

/** The first-run card: what Continue with publik API returned. */
export type FirstRunCard = ConnectOutcome

/** The actions on keys 1 to 6, in that order. */
export const ACTIONS: readonly Action[] = [
  'fix-grammar',
  'clearer',
  'shorter',
  'more-formal',
  'friendlier',
  'custom',
]

/** The buttons' labels. */
export const ACTION_LABELS: Record<Action, string> = {
  'fix-grammar': 'Fix grammar',
  clearer: 'Clearer',
  shorter: 'Shorter',
  'more-formal': 'More formal',
  friendlier: 'Friendlier',
  custom: 'Custom',
}

/**
 * The popup's views. `retry` is the last `rewrite` call's arguments: Try again sends them
 * unchanged (the core counts the validation retries), Another version with `variant` one higher.
 * `banner` is the first-run card shown above a result when publik API was just set up with a
 * balance. `customOpen` shows the instruction field of Custom.
 */
export type PopupState =
  | { view: 'check'; session: Session; customOpen?: boolean }
  | {
      view: 'loading'
      session: Session
      action: Action
      providerLine: string
      retry: RewriteArgs
      banner?: FirstRunCard
    }
  | {
      view: 'result'
      session: Session
      outcome: RewriteOutcome
      retry: RewriteArgs
      banner?: FirstRunCard
    }
  | {
      view: 'consent'
      session: Session
      pending: Action
      info: ConsentInfo | null
      retry: RewriteArgs
    }
  | {
      view: 'first-run'
      session: Session
      pending: Action
      card: FirstRunCard
      retry: RewriteArgs
    }
  | {
      view: 'error'
      session: Session
      error: UiError
      retry?: RewriteArgs
      /** The result an Accept that failed was for, which Copy then copies. */
      resultId?: string
    }

export type PopupEvent =
  | { type: 'action'; action: Action; instruction?: string }
  | { type: 'key'; key: string; ctrl: boolean; shift: boolean; inInstruction: boolean }
  | { type: 'rewrite-done'; outcome: RewriteOutcome }
  | { type: 'rewrite-failed'; error: UiError }
  | { type: 'fix-applied'; session: Session }
  | { type: 'consent'; choice: 'publik' | 'own_key' | 'local' }
  | { type: 'connected'; outcome: ConnectOutcome }
  | { type: 'connect-failed'; error: UiError }
  | { type: 'command-failed'; error: UiError }
  | { type: 'account' }
  | { type: 'try-again' }
  | { type: 'another-version' }
  | { type: 'back' }
  | { type: 'accept'; always: boolean }
  | { type: 'copy' }
  | { type: 'cancel' }
  | { type: 'close' }

export type Effect =
  | { call: 'rewrite'; args: RewriteArgs }
  | { call: 'accept'; args: { session_id: string; result_id?: string } }
  | { call: 'copy_result'; args: { session_id: string; result_id?: string } }
  | { call: 'publik_connect' }
  | { call: 'open_settings'; tab: 'providers'; section?: 'own_key' | 'local' }
  | { call: 'open_account_link'; kind: LinkKind }
  | { call: 'close_session'; session_id: string }

export interface Step {
  state: PopupState
  effects: Effect[]
}

/** The check view of a session just opened. */
export function initial(session: Session): PopupState {
  return { view: 'check', session }
}

/** The one way out an error offers (docs/PUBLIK-INTEGRATION.md §9): a button, a link or none. */
export type WayOut =
  | { kind: 'try-again' }
  | { kind: 'reconnect' }
  | { kind: 'top-up'; label: string }
  | { kind: 'copy' }
  | { kind: 'settings'; section?: ProvidersSection }
  | null

/** Errors after which the same call may well succeed: Try again. */
const RETRYABLE = new Set([
  'InvalidAnswer',
  'PlaceholderDropped',
  'PlaceholderInvented',
  'PublikBusy',
  'PublikTimeout',
  'Timeout',
  'ProviderError',
])

/** Errors whose cure is in Settings → Providers, and the section that holds it. */
const IN_SETTINGS: Record<string, ProvidersSection | null> = {
  NotConfigured: null,
  NoAppToken: 'own_key',
  ProvisionFailed: 'own_key',
  OwnKeyRefused: 'own_key',
  ModelUnknown: 'own_key',
  LocalNotRunning: 'local',
  LocalNoModel: 'local',
  LocalNotLocal: 'local',
}

/**
 * What the error card offers for `error`: Try again only where the error allows and there is a
 * call to repeat; Reconnect after publik API disconnected this computer; the 402's one link,
 * labelled by the claim state; Copy when Accept could not type into the window or the clipboard
 * was busy; Settings where
 * the cure is there; nothing otherwise (offline, a refused request, a selection too long).
 */
export function wayOut(error: UiError, canRetry: boolean): WayOut {
  if (error.kind === 'NeedsCredit') {
    const label = error.claim_state === 'claimed' ? 'Add a plan or pack' : 'Sign in with publik'
    return typeof error.top_up_url === 'string' ? { kind: 'top-up', label } : null
  }
  if (error.kind === 'Disconnected') return { kind: 'reconnect' }
  if (RETRYABLE.has(error.kind)) return canRetry ? { kind: 'try-again' } : null
  // Accept could not type (Copy instead), or the clipboard was busy (Copy again).
  if (['FocusChanged', 'WindowGone', 'ClipboardBusy'].includes(error.kind)) return { kind: 'copy' }
  if (!Object.hasOwn(IN_SETTINGS, error.kind)) return null
  const section = IN_SETTINGS[error.kind]
  return section ? { kind: 'settings', section } : { kind: 'settings' }
}

function stay(state: PopupState): Step {
  return { state, effects: [] }
}

/** The loading view for `retry`, with the effect that runs it. */
function load(state: PopupState, retry: RewriteArgs, banner?: FirstRunCard): Step {
  const providerLine = state.session.provider_lines?.[retry.action] ?? ''
  const loading: PopupState = {
    view: 'loading',
    session: state.session,
    action: retry.action,
    providerLine,
    retry,
  }
  if (banner) loading.banner = banner
  return { state: loading, effects: [{ call: 'rewrite', args: retry }] }
}

function argsFor(session: Session, action: Action, instruction?: string): RewriteArgs {
  const trimmed = instruction?.trim()
  return trimmed
    ? { session_id: session.id, action, instruction: trimmed, variant: 0 }
    : { session_id: session.id, action, variant: 0 }
}

function backToCheck(state: PopupState): Step {
  return stay({ view: 'check', session: state.session })
}

function closeEffect(state: PopupState): Step {
  return { state, effects: [{ call: 'close_session', session_id: state.session.id }] }
}

/** Accept: in the check view the text with its fixes; in the result view the result. */
function acceptIn(state: PopupState, always: boolean): Step {
  if (state.view === 'check') {
    return { state, effects: [{ call: 'accept', args: { session_id: state.session.id } }] }
  }
  if (state.view === 'result' && (always || state.outcome.accept_default)) {
    const args = { session_id: state.session.id, result_id: state.outcome.result_id }
    return { state, effects: [{ call: 'accept', args }] }
  }
  return stay(state)
}

/** Copy: the result in the result view, the result Accept was for after it failed, else the text. */
function copyIn(state: PopupState): Step {
  const session_id = state.session.id
  const resultId =
    state.view === 'result'
      ? state.outcome.result_id
      : state.view === 'error'
        ? state.resultId
        : undefined
  if (state.view === 'loading') return stay(state)
  const args = resultId === undefined ? { session_id } : { session_id, result_id: resultId }
  return { state, effects: [{ call: 'copy_result', args }] }
}

/**
 * The account link: on the first-run card (or its banner) Sign in with publik, or Add a plan or
 * pack once linked; on a 402 the top-up link the error carries. Always a kind, never a URL.
 */
function accountIn(state: PopupState): Step {
  let kind: LinkKind | null = null
  if (state.view === 'first-run') {
    kind = state.card.claim_state === 'claimed' ? 'add_credit' : 'claim'
  } else if ((state.view === 'loading' || state.view === 'result') && state.banner) {
    kind = state.banner.claim_state === 'claimed' ? 'add_credit' : 'claim'
  } else if (state.view === 'error' && state.error.kind === 'NeedsCredit') {
    kind = 'top_up'
  }
  return kind ? { state, effects: [{ call: 'open_account_link', kind }] } : stay(state)
}

/** An action pressed: Custom without an instruction opens its field; the others run at once. */
function runAction(state: PopupState, action: Action, instruction?: string): Step {
  if (state.view !== 'check' && state.view !== 'result' && state.view !== 'error') {
    return stay(state)
  }
  if (action === 'custom' && !instruction?.trim()) {
    return stay({ view: 'check', session: state.session, customOpen: true })
  }
  return load(state, argsFor(state.session, action, instruction))
}

/** The keyboard (Design notes): 1 to 6, Enter, Ctrl+Enter, Ctrl+Shift+C, Escape. */
function onKey(state: PopupState, e: Extract<PopupEvent, { type: 'key' }>): Step {
  // Escape goes back (from loading it stops waiting), and closes from the check view.
  if (e.key === 'Escape') {
    if (state.view !== 'check') return backToCheck(state)
    return state.customOpen ? backToCheck(state) : closeEffect(state)
  }
  // The instruction field keeps its own keys: digits type, Enter runs Custom from the field.
  if (e.inInstruction) return stay(state)
  if (!e.ctrl && /^[1-6]$/.test(e.key)) {
    return runAction(state, ACTIONS[Number(e.key) - 1])
  }
  if (e.key === 'Enter') return acceptIn(state, e.ctrl)
  if (e.ctrl && e.shift && e.key.toLowerCase() === 'c') return copyIn(state)
  return stay(state)
}

export function reduce(state: PopupState, event: PopupEvent): Step {
  switch (event.type) {
    case 'action':
      return runAction(state, event.action, event.instruction)

    case 'key':
      return onKey(state, event)

    case 'rewrite-done':
      if (state.view !== 'loading') return stay(state)
      return stay({
        view: 'result',
        session: state.session,
        outcome: event.outcome,
        retry: state.retry,
        ...(state.banner ? { banner: state.banner } : {}),
      })

    case 'rewrite-failed':
      if (state.view !== 'loading') return stay(state)
      if (event.error.kind === 'NeedsConsent') {
        const consent = event.error.consent as ConsentInfo | undefined
        return stay({
          view: 'consent',
          session: state.session,
          pending: state.retry.action,
          info: consent ?? null,
          retry: state.retry,
        })
      }
      return stay({ view: 'error', session: state.session, error: event.error, retry: state.retry })

    case 'command-failed': {
      if (state.view === 'loading') return stay(state)
      const resultId = state.view === 'result' ? state.outcome.result_id : undefined
      const failed: PopupState = { view: 'error', session: state.session, error: event.error }
      if (resultId !== undefined) failed.resultId = resultId
      return stay(failed)
    }

    case 'fix-applied':
      return stay({ ...state, session: event.session })

    case 'consent':
      if (state.view !== 'consent' && state.view !== 'first-run' && state.view !== 'error') {
        return stay(state)
      }
      if (event.choice !== 'publik') {
        return {
          state,
          effects: [{ call: 'open_settings', tab: 'providers', section: event.choice }],
        }
      }
      // Continue with publik API, or Reconnect; on the first-run card the computer is set up.
      if (state.view === 'first-run') return stay(state)
      return { state, effects: [{ call: 'publik_connect' }] }

    case 'account':
      return accountIn(state)

    case 'connected': {
      if (state.view !== 'consent' && state.view !== 'error') return stay(state)
      if (!state.retry) return backToCheck(state)
      if (event.outcome.run_pending) return load(state, state.retry, event.outcome)
      return stay({
        view: 'first-run',
        session: state.session,
        pending: state.retry.action,
        card: event.outcome,
        retry: state.retry,
      })
    }

    case 'connect-failed':
      if (state.view !== 'consent' && state.view !== 'error' && state.view !== 'first-run') {
        return stay(state)
      }
      return stay({
        view: 'error',
        session: state.session,
        error: event.error,
        ...(state.retry ? { retry: state.retry } : {}),
      })

    case 'try-again':
      if (
        (state.view === 'error' || state.view === 'first-run' || state.view === 'consent') &&
        state.retry
      ) {
        return load(state, state.retry)
      }
      return stay(state)

    case 'another-version':
      if (state.view !== 'result' || state.retry.action === 'fix-grammar') return stay(state)
      return load(state, { ...state.retry, variant: state.retry.variant + 1 })

    case 'back':
    case 'cancel':
      return state.view === 'check' ? stay(state) : backToCheck(state)

    case 'accept':
      return acceptIn(state, event.always)

    case 'copy':
      return copyIn(state)

    case 'close':
      return closeEffect(state)
  }
}
