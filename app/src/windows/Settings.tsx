import { getVersion } from '@tauri-apps/api/app'
import { useCallback, useEffect, useEffectEvent, useRef, useState } from 'react'
import { formatUsd, usageLine } from '../format'
import {
  SETTINGS_OPEN_EVENT,
  STATUS_EVENT,
  appPause,
  asCommandError,
  cacheClear,
  localModels,
  onEvent,
  openAccountLink,
  ownKeyClear,
  ownKeySave,
  providerStatus,
  providerTest,
  publikConnect,
  publikDisconnect,
  publikRefresh,
  sentLogClear,
  sentLogList,
  setProviders,
  settingsGet,
  settingsUpdate,
  usageMonth,
  type ChatChoice,
  type LocalModel,
  type ProviderStatus,
  type ProviderTest,
  type SentEntry,
  type SettingsPatch,
  type SettingsTab,
  type SettingsView,
  type UsageMonth,
} from '../ipc'
import {
  CATEGORIES,
  DIALECTS,
  NO_PROVIDER,
  TABS,
  accountButton,
  addToBlocklist,
  blocklistEntry,
  hotkeyFromKey,
  providersReduce,
  settingsTarget,
  toggleCategory,
  type ProvidersEffect,
  type ProvidersEvent,
  type ProvidersState,
} from '../state/settings'
import Consent from './Consent'
import FirstRunCard from './FirstRunCard'

declare global {
  interface Window {
    /** Where a new Settings window opens, set by the shell before the page loads. */
    __REDPEN_SETTINGS_TARGET__?: unknown
  }
}

/** Settings → Providers while publik API cannot be reached (docs/PRODUCT.md §5). */
const PUBLIK_OFFLINE = "Can't reach publik API right now."
/** Any Settings → Providers check in progress. */
const CHECKING = 'Checking…'

type Update = (patch: SettingsPatch) => Promise<boolean>

/** The wallet read when Settings opens: in progress, or why it failed. */
interface Wallet {
  checking: boolean
  failure: string | null
}

/**
 * The Settings window (docs/PRODUCT.md §5): General, Providers, Privacy and About. Every change
 * goes through a command, which checks it and saves `settings.json`; the window never sees a key.
 */
export default function Settings() {
  const [target] = useState(() => settingsTarget(window.__REDPEN_SETTINGS_TARGET__))
  const [tab, setTab] = useState<SettingsTab>(target.tab)
  const [settings, setSettings] = useState<SettingsView | null>(null)
  const [status, setStatus] = useState<ProviderStatus | null>(null)
  const [problem, setProblem] = useState<string | null>(null)
  const [providers, setProvidersState] = useState<ProvidersState>(NO_PROVIDER)
  const providersRef = useRef<ProvidersState>(NO_PROVIDER)
  // Reading the wallet when Settings opens (docs/ARCHITECTURE.md §8).
  const [wallet, setWallet] = useState<Wallet>({ checking: false, failure: null })
  const [local, setLocal] = useState<{
    test?: ProviderTest
    models?: LocalModel[]
    busy?: boolean
  }>({})

  // Stable, because the tabs' own effects call them.
  const reload = useCallback(async () => {
    const [s, st] = await Promise.all([
      settingsGet().catch(() => null),
      providerStatus().catch(() => null),
    ])
    if (s) setSettings(s)
    setStatus(st ?? null)
    return { s, st }
  }, [])

  const refreshStatus = useCallback(() => {
    providerStatus()
      .then((st) => setStatus(st ?? null))
      .catch(() => undefined)
  }, [])

  function checkLocal() {
    setLocal({ busy: true })
    void Promise.all([
      providerTest('local').catch((e) => ({ ok: false, message: asCommandError(e).message })),
      localModels().catch(() => [] as LocalModel[]),
    ]).then(([test, models]) => setLocal({ test, models: models ?? [] }))
  }

  /** Runs one effect of Providers' reducer; resolves to the event its answer makes, if any. */
  async function performProviders(effect: ProvidersEffect): Promise<ProvidersEvent | null> {
    switch (effect.call) {
      case 'set_providers':
        try {
          const next = await setProviders({ chat: effect.chat })
          if (next) setSettings((s) => (s ? { ...s, ...next } : s))
          refreshStatus()
        } catch (e) {
          setProblem(asCommandError(e).message)
        }
        return null
      case 'publik_connect':
        try {
          const outcome = await publikConnect()
          void reload()
          return { type: 'connected', outcome }
        } catch (e) {
          return { type: 'connect-failed', error: asCommandError(e) }
        }
      case 'check_local':
        checkLocal()
        return null
    }
  }

  function providersDispatch(event: ProvidersEvent) {
    const { state, effects } = providersReduce(providersRef.current, event)
    providersRef.current = state
    setProvidersState(state)
    for (const effect of effects) {
      void performProviders(effect).then((follow) => {
        if (follow) providersDispatch(follow)
      })
    }
  }

  // The settings and the provider status arrive; the section the popup asked for is a choice.
  const onLoaded = useEffectEvent((s: SettingsView | null, st: ProviderStatus | null) => {
    setStatus(st ?? null)
    if (!s) return
    setSettings(s)
    if (s.providers?.chat === 'publik' && st?.publik) refreshWallet()
    providersDispatch({
      type: 'loaded',
      chat: s.providers?.chat ?? null,
      publikReady: Boolean(st?.publik),
      section: target.section,
    })
  })

  /** `GET /wallet`, which the core sends only while publik API is chosen and set up. */
  function refreshWallet() {
    setWallet({ checking: true, failure: null })
    publikRefresh()
      .then(() => {
        setWallet({ checking: false, failure: null })
        refreshStatus()
      })
      .catch((e) => {
        const err = asCommandError(e)
        const quiet = err.kind === 'NotConfigured' || err.kind === 'NeedsConsent'
        setWallet({
          checking: false,
          failure: quiet ? null : err.kind === 'Offline' ? PUBLIK_OFFLINE : err.message,
        })
      })
  }

  // An open window is told where to go.
  const onOpen = useEffectEvent((payload: unknown) => {
    const t = settingsTarget(payload)
    setTab(t.tab)
    if (!t.section) return
    const section = t.section
    void providerStatus()
      .then((st) => {
        setStatus(st ?? null)
        providersDispatch({ type: 'choose', choice: section, publikReady: Boolean(st?.publik) })
      })
      .catch(() => undefined)
  })

  const onStatus = useEffectEvent(() => refreshStatus())

  useEffect(() => {
    let live = true
    void Promise.all([settingsGet().catch(() => null), providerStatus().catch(() => null)]).then(
      ([s, st]) => {
        if (live) onLoaded(s, st)
      },
    )
    const stops = [
      onEvent<unknown>(SETTINGS_OPEN_EVENT, (payload) => onOpen(payload)),
      onEvent(STATUS_EVENT, () => onStatus()),
    ]
    return () => {
      live = false
      for (const stop of stops) void stop.then((unlisten) => unlisten()).catch(() => undefined)
    }
  }, [])

  const update: Update = useCallback(async (patch) => {
    setProblem(null)
    try {
      const next = await settingsUpdate(patch)
      if (next) setSettings((s) => (s ? { ...s, ...next } : s))
      return true
    } catch (e) {
      setProblem(asCommandError(e).message)
      return false
    }
  }, [])

  const readOnly = Boolean(settings?.read_only)

  return (
    <main className="settings">
      <h1 className="sr-only">Redpen Settings</h1>
      <div className="tabs" role="tablist" aria-label="Settings">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            id={`tab-${t.id}`}
            aria-selected={tab === t.id}
            aria-controls={`panel-${t.id}`}
            onClick={() => {
              setProblem(null)
              setTab(t.id)
            }}
          >
            {t.label}
          </button>
        ))}
      </div>
      <section className="panel" role="tabpanel" id={`panel-${tab}`} aria-labelledby={`tab-${tab}`}>
        {readOnly && <p className="warning">Settings were written by a newer Redpen.</p>}
        {problem && (
          <p className="message" role="alert">
            {problem}
          </p>
        )}
        {tab === 'general' && settings && (
          <fieldset className="plain" disabled={readOnly}>
            <General settings={settings} update={update} reload={reload} />
          </fieldset>
        )}
        {tab === 'providers' && (
          <fieldset className="plain" disabled={readOnly}>
            <Providers
              settings={settings}
              status={status}
              wallet={wallet}
              state={providers}
              local={local}
              dispatch={providersDispatch}
              checkLocal={checkLocal}
              update={update}
              reload={reload}
              refreshStatus={refreshStatus}
            />
          </fieldset>
        )}
        {tab === 'privacy' && settings && (
          <fieldset className="plain" disabled={readOnly}>
            <Privacy settings={settings} update={update} />
          </fieldset>
        )}
        {tab === 'about' && <About />}
      </section>
    </main>
  )
}

// ---------------------------------------------------------------------------------------------
// General.

function General({
  settings,
  update,
  reload,
}: {
  settings: SettingsView
  update: Update
  reload: () => Promise<unknown>
}) {
  const [recording, setRecording] = useState(false)
  const [hotkeyError, setHotkeyError] = useState<string | null>(null)

  useEffect(() => {
    if (!recording) return
    function onKeyDown(e: KeyboardEvent) {
      e.preventDefault()
      e.stopPropagation()
      if (e.key === 'Escape' && !e.ctrlKey && !e.altKey && !e.metaKey) {
        setRecording(false)
        return
      }
      const hotkey = hotkeyFromKey(e)
      if (!hotkey) return
      setRecording(false)
      settingsUpdate({ hotkeys: { check: hotkey } })
        .then(() => {
          setHotkeyError(null)
          return reload()
        })
        .catch((err) => setHotkeyError(asCommandError(err).message))
    }
    window.addEventListener('keydown', onKeyDown, true)
    return () => window.removeEventListener('keydown', onKeyDown, true)
  }, [recording, reload])

  const disabled = settings.check.disabled_categories
  return (
    <>
      <h2>Check shortcut</h2>
      <p>
        <kbd className="hotkey">{settings.hotkeys.check}</kbd>{' '}
        <button
          type="button"
          onClick={() => {
            setHotkeyError(null)
            setRecording(true)
          }}
        >
          {recording ? 'Press the new shortcut… (Esc to cancel)' : 'Change shortcut'}
        </button>
      </p>
      {(hotkeyError ?? settings.hotkey_problem) && (
        <p className="message" role="alert">
          {hotkeyError ?? settings.hotkey_problem}
        </p>
      )}

      <h2>Checking</h2>
      <label className="row">
        Dialect{' '}
        <select
          value={settings.check.dialect}
          onChange={(e) => void update({ check: { dialect: e.target.value } })}
        >
          {DIALECTS.map((d) => (
            <option key={d.id} value={d.id}>
              {d.label}
            </option>
          ))}
        </select>
      </label>
      <fieldset>
        <legend>Check for</legend>
        {CATEGORIES.map((c) => (
          <label key={c.id} className="row">
            <input
              type="checkbox"
              checked={!disabled.includes(c.id)}
              onChange={(e) =>
                void update({
                  check: { disabled_categories: toggleCategory(disabled, c.id, e.target.checked) },
                })
              }
            />{' '}
            {c.label}
          </label>
        ))}
      </fieldset>

      <h2>Redpen</h2>
      <label className="row">
        <input
          type="checkbox"
          checked={settings.ui.paused}
          onChange={(e) =>
            void appPause(e.target.checked)
              .then(() => reload())
              .catch(() => undefined)
          }
        />{' '}
        Pause Redpen (the shortcut does nothing until you turn this off)
      </label>
    </>
  )
}

// ---------------------------------------------------------------------------------------------
// Providers.

const CHOICES: readonly { id: ChatChoice; label: string; detail: string }[] = [
  {
    id: 'publik',
    label: 'publik API',
    detail: 'No key needed; paid in dollars from a publik balance.',
  },
  {
    id: 'own_key',
    label: 'Your own key',
    detail: 'Any service with an OpenAI-compatible chat endpoint; it bills you.',
  },
  { id: 'local', label: 'Local model', detail: 'Ollama on this computer; free.' },
]

function Providers({
  settings,
  status,
  wallet,
  state,
  local,
  dispatch,
  checkLocal,
  update,
  reload,
  refreshStatus,
}: {
  settings: SettingsView | null
  status: ProviderStatus | null
  wallet: Wallet
  state: ProvidersState
  local: { test?: ProviderTest; models?: LocalModel[]; busy?: boolean }
  dispatch: (e: ProvidersEvent) => void
  checkLocal: () => void
  update: Update
  reload: () => Promise<unknown>
  refreshStatus: () => void
}) {
  const publikReady = Boolean(status?.publik)
  return (
    <>
      <fieldset>
        <legend>AI provider for rewrites</legend>
        {CHOICES.map((c) => (
          <label key={c.id} className="choice">
            <input
              type="radio"
              name="chat"
              value={c.id}
              checked={state.chosen === c.id}
              onChange={() => dispatch({ type: 'choose', choice: c.id, publikReady })}
            />{' '}
            {c.label}
            <span className="detail">{c.detail}</span>
          </label>
        ))}
      </fieldset>

      {state.pane === 'consent' && (
        <>
          <Consent
            busy={state.connecting}
            onChoose={(choice) =>
              choice === 'publik'
                ? dispatch({ type: 'connect' })
                : dispatch({ type: 'choose', choice, publikReady })
            }
          />
          {state.error && (
            <p className="message" role="alert">
              {state.error.message}
            </p>
          )}
        </>
      )}
      {state.pane === 'first-run' && state.card && (
        <FirstRunCard
          card={state.card}
          onAccount={(kind) => void openAccountLink(kind).catch(() => undefined)}
          onChoose={(choice) => dispatch({ type: 'choose', choice, publikReady })}
        />
      )}
      {state.pane === 'publik' && settings && (
        <Publik
          settings={settings}
          status={status}
          wallet={wallet}
          update={update}
          onDisconnected={() => {
            dispatch({ type: 'disconnected' })
            refreshStatus()
          }}
        />
      )}
      {state.pane === 'own_key' && settings && <OwnKey settings={settings} reload={reload} />}
      {state.pane === 'local' && settings && (
        <Local settings={settings} local={local} checkLocal={checkLocal} update={update} />
      )}
    </>
  )
}

function Publik({
  settings,
  status,
  wallet,
  update,
  onDisconnected,
}: {
  settings: SettingsView
  status: ProviderStatus | null
  wallet: Wallet
  update: Update
  onDisconnected: () => void
}) {
  const [usage, setUsage] = useState<UsageMonth | null>(null)

  // This month's use, from this computer's own count (nothing is sent).
  useEffect(() => {
    let live = true
    usageMonth()
      .then((u) => {
        if (live && u) setUsage(u)
      })
      .catch(() => undefined)
    return () => {
      live = false
    }
  }, [])

  const claim = status?.publik?.claim_state ?? 'anonymous'
  const account = accountButton(claim)
  return (
    <section className="provider" aria-label="publik API">
      <h2>publik API</h2>
      {status?.balance_line && <p className="balance">{status.balance_line}</p>}
      {usage && <p>{usageLine(usage.total_micros, usage.calls)}</p>}
      {wallet.checking && <p className="hint">{CHECKING}</p>}
      {wallet.failure && (
        <p className="message" role="alert">
          {wallet.failure}
        </p>
      )}
      <div className="buttons">
        <button
          type="button"
          className="primary"
          onClick={() => void openAccountLink(account.kind).catch(() => undefined)}
        >
          {account.label}
        </button>
        <button
          type="button"
          onClick={() =>
            void publikDisconnect()
              .catch(() => undefined)
              .then(onDisconnected)
          }
        >
          Disconnect
        </button>
      </div>
      <fieldset>
        <legend>Rewrite quality</legend>
        {(['standard', 'best'] as const).map((q) => (
          <label key={q} className="row">
            <input
              type="radio"
              name="quality"
              checked={settings.providers.rewrite_quality === q}
              onChange={() => void update({ providers: { rewrite_quality: q } })}
            />{' '}
            {q === 'standard' ? 'Standard' : 'Best (once this computer is linked)'}
          </label>
        ))}
      </fieldset>
      {usage && usage.by_feature.length > 0 && (
        <ul className="usage">
          {usage.by_feature.map((f) => (
            <li key={f.feature}>
              {f.feature}: {f.calls} · {formatUsd(f.charge_micros)}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

function OwnKey({ settings, reload }: { settings: SettingsView; reload: () => Promise<unknown> }) {
  const [baseUrl, setBaseUrl] = useState(settings.providers.own_key.base_url)
  const [model, setModel] = useState(settings.providers.own_key.chat_model)
  const [key, setKey] = useState('')
  const [line, setLine] = useState<{ text: string; ok: boolean } | null>(null)
  const saved = Boolean(settings.own_key_saved)

  const save = () => {
    setLine(null)
    ownKeySave({ base_url: baseUrl, key, chat_model: model })
      .then(() => {
        setKey('')
        setLine({ text: 'Saved.', ok: true })
        return reload()
      })
      .catch((e) => setLine({ text: asCommandError(e).message, ok: false }))
  }
  const test = () => {
    setLine({ text: CHECKING, ok: true })
    providerTest('own_key')
      .then((r) => setLine({ text: r.message, ok: r.ok }))
      .catch((e) => setLine({ text: asCommandError(e).message, ok: false }))
  }
  const clear = () => {
    ownKeyClear()
      .then(() => {
        setLine({ text: 'Your key was removed.', ok: true })
        return reload()
      })
      .catch((e) => setLine({ text: asCommandError(e).message, ok: false }))
  }

  return (
    <section className="provider" aria-label="Your own key">
      <h2>Your own key</h2>
      <label className="field">
        Base URL
        <input value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} spellCheck={false} />
      </label>
      <label className="field">
        Key
        <input
          type="password"
          value={key}
          autoComplete="off"
          placeholder={saved ? 'Saved (type a new key to replace it)' : ''}
          onChange={(e) => setKey(e.target.value)}
        />
      </label>
      <label className="field">
        Chat model
        <input value={model} onChange={(e) => setModel(e.target.value)} spellCheck={false} />
      </label>
      <div className="buttons">
        <button type="button" className="primary" onClick={save}>
          Save
        </button>
        <button type="button" onClick={test} disabled={!saved}>
          Test
        </button>
        {saved && (
          <button type="button" onClick={clear}>
            Remove key
          </button>
        )}
      </div>
      {line && (
        <p className={line.ok ? 'notice' : 'message'} role={line.ok ? 'status' : 'alert'}>
          {line.text}
        </p>
      )}
    </section>
  )
}

function Local({
  settings,
  local,
  checkLocal,
  update,
}: {
  settings: SettingsView
  local: { test?: ProviderTest; models?: LocalModel[]; busy?: boolean }
  checkLocal: () => void
  update: Update
}) {
  const chosenModel = settings.providers.local.chat_model ?? ''
  const models = (local.models ?? []).filter((m) => !m.name.toLowerCase().includes('embed'))
  return (
    <section className="provider" aria-label="Local model">
      <h2>Local model</h2>
      <p className="hint">Server: {settings.providers.local.chat_base_url}</p>
      {local.busy && <p className="hint">{CHECKING}</p>}
      {local.test && <p className={local.test.ok ? 'notice' : 'message'}>{local.test.message}</p>}
      {models.length > 0 && (
        <label className="row">
          Model{' '}
          <select
            value={chosenModel}
            onChange={(e) =>
              void update({ providers: { local: { chat_model: e.target.value || null } } })
            }
          >
            <option value="">Automatic (the largest up to 14B)</option>
            {models.map((m) => (
              <option key={m.name} value={m.name}>
                {m.name} ({m.parameter_size})
              </option>
            ))}
          </select>
        </label>
      )}
      <div className="buttons">
        <button type="button" onClick={checkLocal} disabled={local.busy}>
          Check Ollama
        </button>
      </div>
    </section>
  )
}

// ---------------------------------------------------------------------------------------------
// Privacy.

function Privacy({ settings, update }: { settings: SettingsView; update: Update }) {
  const [typed, setTyped] = useState('')
  const [log, setLog] = useState<SentEntry[]>([])
  const [line, setLine] = useState<string | null>(null)
  const privacy = settings.privacy

  useEffect(() => {
    let live = true
    sentLogList()
      .then((entries) => {
        if (live) setLog(entries ?? [])
      })
      .catch(() => undefined)
    return () => {
      live = false
    }
  }, [privacy.sent_log])

  const add = () => {
    const name = blocklistEntry(typed)
    if (!name) return
    void update({ privacy: { blocklist: addToBlocklist(privacy.blocklist, name) } }).then(
      (ok) => ok && setTyped(''),
    )
  }

  return (
    <>
      <h2>Before a rewrite</h2>
      <label className="row">
        <input
          type="checkbox"
          checked={privacy.redaction}
          onChange={(e) => void update({ privacy: { redaction: e.target.checked } })}
        />{' '}
        Replace emails, phone numbers, IBANs and long numbers with markers before anything is sent
      </label>

      <h2>Apps Redpen never reads</h2>
      <ul className="blocklist">
        {privacy.blocklist.map((name) => (
          <li key={name}>
            {name}{' '}
            <button
              type="button"
              aria-label={`Remove ${name}`}
              onClick={() =>
                void update({
                  privacy: { blocklist: privacy.blocklist.filter((n) => n !== name) },
                })
              }
            >
              Remove
            </button>
          </li>
        ))}
      </ul>
      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault()
          add()
        }}
      >
        <input
          aria-label="Program name"
          placeholder="Program name, for example notes.exe"
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
        />{' '}
        <button type="submit" disabled={!blocklistEntry(typed)}>
          Add
        </button>
      </form>

      <h2>What is kept</h2>
      <label className="row">
        Remember AI answers{' '}
        <select
          value={privacy.remember_answers_days > 0 ? '7' : '0'}
          onChange={(e) =>
            void update({ privacy: { remember_answers_days: Number(e.target.value) } })
          }
        >
          <option value="7">7 days</option>
          <option value="0">Off</option>
        </select>
      </label>
      <div className="buttons">
        <button
          type="button"
          onClick={() =>
            void cacheClear()
              .then(() => setLine('The remembered answers were cleared.'))
              .catch((e) => setLine(asCommandError(e).message))
          }
        >
          Clear now
        </button>
      </div>
      <label className="row">
        <input
          type="checkbox"
          checked={privacy.sent_log}
          onChange={(e) => void update({ privacy: { sent_log: e.target.checked } })}
        />{' '}
        Keep a log of what was sent (kept for seven days)
      </label>
      {log.length > 0 && (
        <>
          <ul className="sent-log">
            {log.map((entry) => (
              <li key={entry.id}>
                <span className="hint">
                  {new Date(entry.at_ms).toLocaleString()} · {entry.provider} · {entry.feature} ·{' '}
                  {entry.model}
                </span>
                <pre>{entry.sent_text}</pre>
              </li>
            ))}
          </ul>
          <div className="buttons">
            <button
              type="button"
              onClick={() =>
                void sentLogClear()
                  .then(() => setLog([]))
                  .catch((e) => setLine(asCommandError(e).message))
              }
            >
              Clear the log
            </button>
          </div>
        </>
      )}
      {line && <p className="notice">{line}</p>}
    </>
  )
}

// ---------------------------------------------------------------------------------------------
// About.

function About() {
  const [version, setVersion] = useState<string | null>(null)
  useEffect(() => {
    let live = true
    getVersion()
      .then((v) => {
        if (live && typeof v === 'string') setVersion(v)
      })
      .catch(() => undefined)
    return () => {
      live = false
    }
  }, [])
  return (
    <>
      <h2>Redpen{version ? ` ${version}` : ''}</h2>
      <p>Fix or rewrite your writing in any app. Free and open source.</p>
      <p>Licence: MIT OR Apache-2.0, at your option.</p>
      <p>
        Third-party licences: <code>THIRD_PARTY.md</code>, which comes with the source, lists every
        library Redpen uses and its licence.
      </p>
    </>
  )
}
