import { useEffect, useEffectEvent, useRef, useState } from 'react'
import Diff from '../components/Diff'
import Findings from '../components/Findings'
import Footer from '../components/Footer'
import {
  POPUP_OPEN_EVENT,
  STATUS_EVENT,
  accept,
  applyFix,
  asCommandError,
  closeSession,
  copyResult,
  onEvent,
  openAccountLink,
  openSettings,
  providerStatus,
  publikConnect,
  rewrite,
  sessionGet,
  undoLastReplace,
  type CommandError,
  type PopupOpen,
  type ProviderStatus,
  type ProvidersSection,
} from '../ipc'
import {
  ACTIONS,
  ACTION_LABELS,
  initial,
  reduce,
  wayOut,
  type Effect,
  type PopupEvent,
  type PopupState,
} from '../state/popup'
import {
  COPIED,
  NO_CHANGES,
  UNDO_ON_CLIPBOARD,
  fromPublik,
  loadingLine,
  providerLineFor,
  warningText,
} from '../state/popup-view'
import Consent from './Consent'
import ErrorCard from './ErrorCard'
import FirstRunCard from './FirstRunCard'

/** Elements that act on Enter themselves. */
const INTERACTIVE = 'button, a, input, select, textarea'

/**
 * The popup (docs/PRODUCT.md §5): opened by the check shortcut with `popup://open`, it shows the
 * session's text and Harper's findings, runs the six actions, and shows the result, the consent
 * view, the first-run card or an error. Its state is the pure reducer of `state/popup.ts`; this
 * component renders it and runs the commands the reducer asks for.
 */
export default function Popup() {
  const [state, setState] = useState<PopupState | null>(null)
  const [refusal, setRefusal] = useState<CommandError | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [canUndo, setCanUndo] = useState(false)
  const [status, setStatus] = useState<ProviderStatus | null>(null)
  const [instruction, setInstruction] = useState('')
  const [connecting, setConnecting] = useState(false)
  const current = useRef<PopupState | null>(null)
  // Each rewrite call takes a number; an answer is used only while its number is the latest, so a
  // cancelled call's late answer is dropped.
  const calls = useRef(0)

  function show(next: PopupState | null) {
    current.current = next
    setState(next)
  }

  function refreshStatus() {
    providerStatus()
      .then((s) => setStatus(s ?? null))
      .catch(() => undefined)
  }

  /** Runs one effect the reducer asked for; resolves to the event its answer makes, if any. */
  async function perform(effect: Effect): Promise<PopupEvent | null> {
    switch (effect.call) {
      case 'rewrite': {
        calls.current += 1
        const mine = calls.current
        setNotice(null)
        try {
          const outcome = await rewrite(effect.args)
          return mine === calls.current ? { type: 'rewrite-done', outcome } : null
        } catch (e) {
          return mine === calls.current
            ? { type: 'rewrite-failed', error: asCommandError(e) }
            : null
        }
      }
      case 'accept':
        try {
          await accept(effect.args.session_id, effect.args.result_id)
          show(null)
          return null
        } catch (e) {
          return { type: 'command-failed', error: asCommandError(e) }
        }
      case 'copy_result':
        try {
          await copyResult(effect.args.session_id, effect.args.result_id)
          setNotice(COPIED)
          return null
        } catch (e) {
          return { type: 'command-failed', error: asCommandError(e) }
        }
      case 'publik_connect':
        setConnecting(true)
        try {
          const outcome = await publikConnect()
          refreshStatus()
          return { type: 'connected', outcome }
        } catch (e) {
          return { type: 'connect-failed', error: asCommandError(e) }
        } finally {
          setConnecting(false)
        }
      case 'open_settings':
        await openSettings(effect.tab, effect.section).catch(() => undefined)
        return null
      case 'open_account_link':
        await openAccountLink(effect.kind).catch((e) => setNotice(asCommandError(e).message))
        return null
      case 'close_session':
        calls.current += 1
        show(null)
        await closeSession(effect.session_id).catch(() => undefined)
        return null
    }
  }

  /** Feeds `event` to the reducer and runs its effects; true when anything happened. */
  function dispatch(event: PopupEvent): boolean {
    const before = current.current
    if (!before) return false
    const { state: next, effects } = reduce(before, event)
    if (next !== before) show(next)
    for (const effect of effects) {
      void perform(effect).then((follow) => {
        if (follow) dispatch(follow)
      })
    }
    return next !== before || effects.length > 0
  }

  // The shortcut opens a session (or says why the capture stopped).
  const onOpen = useEffectEvent((payload: PopupOpen | null) => {
    calls.current += 1
    setNotice(null)
    setInstruction('')
    setCanUndo(Boolean(payload?.can_undo))
    if (!payload?.session_id) {
      show(null)
      setRefusal(payload?.error ?? null)
      return
    }
    setRefusal(null)
    refreshStatus()
    sessionGet(payload.session_id)
      .then((session) => show(initial(session)))
      .catch((e) => {
        show(null)
        setRefusal(asCommandError(e))
      })
  })

  const onStatus = useEffectEvent(() => refreshStatus())

  // The keyboard (the reducer's key map); Enter on a focused button or link stays that control's.
  const onKeyDown = useEffectEvent((e: KeyboardEvent) => {
    const target = e.target instanceof Element ? e.target : null
    if (!current.current) {
      if (e.key === 'Escape') void closeSession('').catch(() => undefined)
      return
    }
    const inInstruction = Boolean(target?.closest('[data-instruction]'))
    if (e.key === 'Enter' && !e.ctrlKey && !e.metaKey && target?.closest(INTERACTIVE)) return
    const handled = dispatch({
      type: 'key',
      key: e.key,
      ctrl: e.ctrlKey || e.metaKey,
      shift: e.shiftKey,
      inInstruction,
    })
    if (handled) e.preventDefault()
  })

  useEffect(() => {
    const stops: Array<Promise<() => void>> = [
      onEvent<PopupOpen>(POPUP_OPEN_EVENT, (payload) => onOpen(payload)),
      onEvent(STATUS_EVENT, () => onStatus()),
    ]
    const keys = (e: KeyboardEvent) => onKeyDown(e)
    window.addEventListener('keydown', keys)
    return () => {
      window.removeEventListener('keydown', keys)
      for (const stop of stops) void stop.then((unlisten) => unlisten()).catch(() => undefined)
    }
  }, [])

  function fix(findingId: string, index: number) {
    const s = current.current
    if (!s) return
    setNotice(null)
    applyFix(s.session.id, findingId, index)
      .then((session) => dispatch({ type: 'fix-applied', session }))
      .catch((e) => setNotice(asCommandError(e).message))
  }

  function undo() {
    setNotice(null)
    undoLastReplace()
      .then((outcome) => {
        setCanUndo(false)
        if (outcome === 'on_clipboard') {
          show(null)
          setRefusal(null)
          setNotice(UNDO_ON_CLIPBOARD)
        } else {
          show(null)
        }
      })
      .catch((e) => {
        setCanUndo(false)
        setNotice(asCommandError(e).message)
      })
  }

  function settings(section?: ProvidersSection) {
    void openSettings('providers', section).catch(() => undefined)
  }

  function runCustom() {
    dispatch({ type: 'action', action: 'custom', instruction })
  }

  const close = () => {
    if (current.current) dispatch({ type: 'close' })
    else void closeSession('').catch(() => undefined)
  }

  const undoButton = canUndo && (
    <button type="button" onClick={undo}>
      Undo last replace
    </button>
  )

  if (!state) {
    return (
      <main className="popup">
        <h1 className="sr-only">Redpen</h1>
        {refusal && (
          <p className="message" role="alert">
            {refusal.message}
          </p>
        )}
        {notice && <p className="notice">{notice}</p>}
        <div className="buttons bar">
          {undoButton}
          <button type="button" onClick={close}>
            Close
          </button>
        </div>
      </main>
    )
  }

  const session = state.session
  const onFix = fix
  const chat = status?.chat ?? null
  const balanceLine = status?.balance_line ?? null

  return (
    <main className="popup">
      <h1 className="sr-only">Redpen</h1>

      {state.view === 'check' && (
        <>
          <Findings session={session} onFix={onFix} />
          <div className="actions" role="group" aria-label="Rewrite">
            {ACTIONS.map((action, i) => (
              <button
                key={action}
                type="button"
                onClick={() =>
                  action === 'custom' ? runCustom() : dispatch({ type: 'action', action })
                }
              >
                <kbd>{i + 1}</kbd> {ACTION_LABELS[action]}
              </button>
            ))}
          </div>
          {state.customOpen && (
            <form
              className="instruction"
              onSubmit={(e) => {
                e.preventDefault()
                runCustom()
              }}
            >
              <input
                data-instruction=""
                aria-label="Instruction"
                placeholder="What should change? For example: make it a question"
                maxLength={300}
                value={instruction}
                autoFocus
                onChange={(e) => setInstruction(e.target.value)}
              />
              <button type="submit" disabled={!instruction.trim()}>
                Rewrite
              </button>
            </form>
          )}
          <div className="buttons bar">
            <button
              type="button"
              className="primary"
              onClick={() => dispatch({ type: 'accept', always: true })}
            >
              Accept
            </button>
            <button type="button" onClick={() => dispatch({ type: 'copy' })}>
              Copy
            </button>
            {undoButton}
            <button type="button" onClick={close}>
              Close
            </button>
          </div>
        </>
      )}

      {state.view === 'loading' && (
        <>
          {state.banner && (
            <FirstRunCard
              card={state.banner}
              banner
              onAccount={() => dispatch({ type: 'account' })}
            />
          )}
          <p className="loading" role="status">
            {loadingLine(state.providerLine || providerLineFor(chat, state.action))}
          </p>
          <div className="buttons bar">
            <button type="button" onClick={() => dispatch({ type: 'cancel' })}>
              Cancel
            </button>
          </div>
        </>
      )}

      {state.view === 'result' && (
        <>
          {state.banner && (
            <FirstRunCard
              card={state.banner}
              banner
              onAccount={() => dispatch({ type: 'account' })}
            />
          )}
          {state.outcome.warnings.map((w, i) => (
            <p key={i} className="warning" role="alert">
              {warningText(w)}
            </p>
          ))}
          {state.outcome.unchanged && <p className="notice">{NO_CHANGES}</p>}
          <Diff ops={state.outcome.diff} />
          <Footer
            outcome={state.outcome}
            publik={fromPublik(state.outcome, chat)}
            balanceLine={balanceLine ?? state.outcome.balance_line ?? null}
          />
          <div className="buttons bar">
            <button
              type="button"
              className="primary"
              onClick={() => dispatch({ type: 'accept', always: true })}
            >
              Accept
            </button>
            <button type="button" onClick={() => dispatch({ type: 'copy' })}>
              Copy
            </button>
            {state.retry.action !== 'fix-grammar' && (
              <button type="button" onClick={() => dispatch({ type: 'another-version' })}>
                Another version
              </button>
            )}
            <button type="button" onClick={() => dispatch({ type: 'back' })}>
              Back
            </button>
          </div>
        </>
      )}

      {state.view === 'consent' && (
        <>
          <Findings session={session} onFix={onFix} />
          <Consent
            info={state.info}
            busy={connecting}
            onChoose={(choice) => dispatch({ type: 'consent', choice })}
            onTryAgain={() => dispatch({ type: 'try-again' })}
          />
        </>
      )}

      {state.view === 'first-run' && (
        <>
          <Findings session={session} onFix={onFix} />
          <FirstRunCard
            card={state.card}
            onAccount={() => dispatch({ type: 'account' })}
            onChoose={(choice) => dispatch({ type: 'consent', choice })}
            onTryAgain={() => dispatch({ type: 'try-again' })}
          />
        </>
      )}

      {state.view === 'error' && (
        <>
          <ErrorCard
            error={state.error}
            way={wayOut(state.error, Boolean(state.retry))}
            onTryAgain={() => dispatch({ type: 'try-again' })}
            onReconnect={() => dispatch({ type: 'consent', choice: 'publik' })}
            onTopUp={() => dispatch({ type: 'account' })}
            onCopy={() => dispatch({ type: 'copy' })}
            onSettings={settings}
          />
          <Findings session={session} onFix={onFix} />
          <div className="buttons bar">
            <button type="button" onClick={() => dispatch({ type: 'back' })}>
              Back
            </button>
          </div>
        </>
      )}

      {notice && <p className="notice">{notice}</p>}
    </main>
  )
}
