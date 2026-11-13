// Module 06 tests of the windows (docs/modules/06-popup-and-settings-ui.md, Acceptance tests,
// bullets 5 to 8, 15 and 18, and the views of bullets 3 and 11). A test named `tNN: …` covers
// bullet NN, `s: …` a Scope bullet the numbered list leaves implicit, `e: …` an error path.
//
// The windows are rendered as the app renders them and driven only through what the spec names:
// the event `popup://open {session_id}` and the commands of docs/ARCHITECTURE.md §5, answered
// here by a fake `invoke`, and the labels of docs/PRODUCT.md §5 and docs/PUBLIK-INTEGRATION.md
// §4 and §9. Tauri's event and window modules are faked the same way, so no test needs the shell.
// An argument is read by its snake_case name or its camelCase form, at the top level or one
// object down, so the tests do not fix how `app/src/ipc.ts` packs a command's arguments.
//
// Expected strings: docs/PUBLIK-INTEGRATION.md §4 (consent view, first-run card) and §9 (errors,
// the footer), docs/modules/02-provider-layer.md and 03-publik.md (error messages, DISCLOSURE,
// balance_line), docs/PRODUCT.md §5 (Welcome, the popup, Settings), the spec's Messages and
// Design notes (the read-only settings banner, the wallet read when Settings opens), and the
// hand-written publik answers of tests/cases/03-publik/*.fake.json. The session is module 01's
// reference finding for corpus sentence c01. None of them was taken from Redpen's own output
// (tests/cases/06-popup-and-settings-ui/REFERENCE.md).

import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { createElement } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ---------------------------------------------------------------------------------------------
// Tauri, faked: `invoke` answers from `fake.answers`; `listen` keeps its handlers so a test can
// send an event; every window method resolves to a no-op.

type Args = Record<string, unknown>
type Answer = (args: Args) => unknown

const fake = vi.hoisted(() => {
  const handlers = new Map<string, Array<(event: unknown) => void>>()
  const answers: Record<string, (args: Record<string, unknown>) => unknown> = {}
  let eventId = 0
  async function listen(name: string, handler: (event: unknown) => void) {
    handlers.set(name, [...(handlers.get(name) ?? []), handler])
    return () => {
      handlers.set(
        name,
        (handlers.get(name) ?? []).filter((h) => h !== handler),
      )
    }
  }
  function send(name: string, payload: unknown) {
    for (const handler of handlers.get(name) ?? []) {
      eventId += 1
      handler({ event: name, id: eventId, payload })
    }
  }
  function windowStub(label: string) {
    const methods: Record<string | symbol, unknown> = { label, listen, once: listen }
    return new Proxy(methods, {
      get(target, prop) {
        if (prop === 'then' || typeof prop === 'symbol') return undefined
        if (!(prop in target)) target[prop] = vi.fn(async () => () => undefined)
        return target[prop]
      },
    })
  }
  return { handlers, answers, listen, send, windowStub }
})

vi.mock('@tauri-apps/api/core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@tauri-apps/api/core')>()),
  isTauri: () => true,
  invoke: vi.fn(async (command: string, args?: Record<string, unknown>) => {
    const answer = fake.answers[command]
    return answer ? answer(args ?? {}) : undefined
  }),
}))

vi.mock('@tauri-apps/api/event', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@tauri-apps/api/event')>()),
  listen: vi.fn(fake.listen),
  once: vi.fn(fake.listen),
  emit: vi.fn(async () => undefined),
  emitTo: vi.fn(async () => undefined),
}))

vi.mock('@tauri-apps/api/window', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@tauri-apps/api/window')>()),
  getCurrentWindow: () => fake.windowStub('popup'),
}))

vi.mock('@tauri-apps/api/webviewWindow', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@tauri-apps/api/webviewWindow')>()),
  getCurrentWebviewWindow: () => fake.windowStub('popup'),
}))

vi.mock('@tauri-apps/api/webview', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@tauri-apps/api/webview')>()),
  getCurrentWebview: () => fake.windowStub('popup'),
}))

import { invoke } from '@tauri-apps/api/core'
import Popup from './Popup'
import Settings from './Settings'
import Welcome from './Welcome'

// ---------------------------------------------------------------------------------------------
// Expected strings.

const SESSION_ID = 'session-06'
const TEXT = 'I went to teh store yesterday.'
const FINDING_MESSAGE = 'Did you mean the definite article?'

// docs/PUBLIK-INTEGRATION.md §4 and docs/modules/03-publik.md (DISCLOSURE).
const TITLE = 'Choose how Redpen uses AI'
const DATA_PATH =
  "Checking is done on this computer and needs no AI. Only the text you select and the action you choose go through publik's servers, and only when you press a rewrite button."
const DISCLOSURE =
  "Requests go through publik's servers to a shared AI model account. publik does not train on them and keeps no request bodies beyond a 24-hour fingerprint that stops a request being charged twice. The AI provider's own abuse monitoring applies."
const CONSENT_FOOTER = 'You can change this at any time in Settings → Providers.'
const CONTINUE = 'Continue with publik API'
const OWN_KEY = 'Use my own key'
const LOCAL = 'Use a local model'

// The first-run card (§4) at $0.00 while anonymous, from tests/cases/03-publik/installs-201.fake.json.
const ZERO_LINE = '$0.00 · link this computer for $0.05 of free use'
const COST_SENTENCE =
  "A typical rewrite costs less than one cent, paid from this computer's publik balance."
const CLAIM_URL = 'https://publikhq.com/claim/fake-claim-token'
const SIGN_IN = 'Sign in with publik'
const KEEP_WORKING =
  'Your checks keep working. When you have linked this computer, press Try again.'
const TRY_AGAIN = 'Try again'

// Errors (§9, modules 02 and 03).
const MSG_402 =
  'This computer has $0.00 of publik balance. Link it to a publik account for $0.05 of free use.'
const OR_OWN_KEY = 'or use your own key or a local model in Settings'
const MSG_DISCONNECTED = 'publik API disconnected this computer.'
const MSG_BUSY = 'publik API is busy; try again in a moment'
const MSG_OFFLINE =
  'No connection. Rewrites need a connection or a local model; checking still works.'
const MSG_INVALID = 'The AI answer could not be used.'

// docs/PRODUCT.md §5, Settings → Providers: loading and offline.
const CHECKING = 'Checking…'
const PUBLIK_OFFLINE = "Can't reach publik API right now."
// The spec's Design notes, `settings_update`: a settings.json written by a newer Redpen.
const NEWER = 'Settings were written by a newer Redpen.'

// docs/PRODUCT.md §5.
const WELCOME_SENTENCES = [
  'Redpen checks your writing on this computer.',
  'Nothing you type leaves it until you ask for a rewrite, or for a check that needs the internet.',
  'Password fields and apps you block are never read.',
]

// ---------------------------------------------------------------------------------------------
// Fixtures: what the core returns, in the shapes of docs/ARCHITECTURE.md §4 and §5.

const SESSION = {
  id: SESSION_ID,
  text: TEXT,
  findings: [
    {
      id: '10-13-spelling-57a49948',
      category: 'spelling',
      harper_kind: 'Typo',
      message: FINDING_MESSAGE,
      span: { chars: [10, 13], bytes: [10, 13], utf16: [10, 13] },
      suggestions: [{ kind: 'replace', text: 'the' }],
    },
  ],
  dialect: 'american',
  english: true,
}

/** Clearer on publik API at $0.0021; the diff joins back into both texts. */
const CLEARER = {
  result_id: 'result-clearer',
  text: 'Yesterday, I went to the store.',
  diff: [
    { insert: 'Yesterday, ' },
    { equal: 'I went to ' },
    { delete: 'teh' },
    { insert: 'the' },
    { equal: ' store' },
    { delete: ' yesterday' },
    { equal: '.' },
  ],
  warnings: [],
  accept_default: true,
  unchanged: false,
  provider_line: 'publik-balanced',
  charge_micros: 2100,
}

/** Fix grammar on publik API at $0.0002. */
const FIX_GRAMMAR = {
  result_id: 'result-fix',
  text: 'I went to the store yesterday.',
  diff: [
    { equal: 'I went to ' },
    { delete: 'teh' },
    { insert: 'the' },
    { equal: ' store yesterday.' },
  ],
  warnings: [],
  accept_default: true,
  unchanged: false,
  provider_line: 'publik-fast',
  charge_micros: 210,
}

/** The minimal-edit guard's warning (docs/PUBLIK-INTEGRATION.md §6.1 step 5). */
const GUARD_WARNING =
  'This changes more than a grammar fix should (9 of 14 words). Check it before accepting.'

/** Fix grammar whose answer the guard flagged: Enter no longer accepts it. */
const GUARDED = {
  result_id: 'result-guarded',
  text: 'Yesterday, I went to the shop.',
  diff: [
    { insert: 'Yesterday, ' },
    { equal: 'I went to ' },
    { delete: 'teh store yesterday' },
    { insert: 'the shop' },
    { equal: '.' },
  ],
  warnings: [GUARD_WARNING],
  accept_default: false,
  unchanged: false,
  provider_line: 'publik-fast',
  charge_micros: 210,
}

function publikStatus(balance: number, claimState: 'anonymous' | 'claimed') {
  return {
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
    updated_at_ms: 1_790_000_000_000,
  }
}

/** No AI provider chosen yet: a fresh install. */
const NO_PROVIDER = { chat: null, publik: null, balance_line: null }
/** publik API set up, $3.12, linked (docs/PUBLIK-INTEGRATION.md §9). */
const ON_PUBLIK = {
  chat: 'publik',
  publik: publikStatus(3_120_000, 'claimed'),
  balance_line: '$3.12 of publik balance',
}

const CONSENT_INFO = {
  title: TITLE,
  data_path: DATA_PATH,
  disclosure: DISCLOSURE,
  disclosure_version: 1,
}

/** `/installs` at $0.00 (installs-201.fake.json): the pending action is not sent. */
const AT_ZERO = {
  status: publikStatus(0, 'anonymous'),
  run_pending: false,
  balance_line: ZERO_LINE,
  cost_sentence: COST_SENTENCE,
  claim_url: CLAIM_URL,
  claim_state: 'anonymous',
}

/** Settings as docs/ARCHITECTURE.md §4 gives them. */
const SETTINGS = {
  version: 1,
  hotkeys: { check: 'Ctrl+Alt+R' },
  check: { dialect: 'british', disabled_categories: [] },
  providers: {
    chat: null,
    rewrite_quality: 'standard',
    search: null,
    own_key: { base_url: 'https://api.example.com/v1', chat_model: 'example-chat' },
    local: { chat_base_url: 'http://localhost:11434/v1', chat_model: null },
  },
  privacy: {
    redaction: true,
    remember_answers_days: 7,
    sent_log: false,
    blocklist: ['1password.exe', 'bitwarden.exe', 'keepass.exe', 'keepassxc.exe'],
  },
  capture: { copy_timeout_ms: 400, paste_restore_delay_ms: 250 },
  plagiarism: { citation_style: 'apa7' },
  ui: { start_with_os: false, update_check: true, paused: false, welcomed: false },
}

const NEEDS_CONSENT = { kind: 'NeedsConsent', message: 'Choose how Redpen uses AI.' }
const NEEDS_CREDIT = {
  kind: 'NeedsCredit',
  message: MSG_402,
  top_up_url: CLAIM_URL,
  claim_state: 'anonymous',
}

/**
 * The commands that make a request leave the computer (docs/ARCHITECTURE.md §8). `publik_refresh`
 * is not among them: it sends nothing unless publik API is chosen and set up (module 03).
 */
const SENDING = ['rewrite', 'publik_connect', 'provider_test', 'local_models']

// ---------------------------------------------------------------------------------------------
// Helpers.

function answer(command: string, a: Answer) {
  fake.answers[command] = a
}

function reply(value: unknown): Answer {
  return () => value
}

function refuse(error: unknown): Answer {
  return () => Promise.reject(error)
}

/** Answers in turn, the last one repeating. */
function inTurn(...answers: Answer[]): Answer {
  let n = 0
  return (args) => answers[Math.min(n++, answers.length - 1)](args)
}

const never: Answer = () => new Promise(() => undefined)

/** The arguments of every call of `command`, in order. */
function callsTo(command: string): Args[] {
  return vi
    .mocked(invoke)
    .mock.calls.filter(([c]) => c === command)
    .map(([, a]) => (a ?? {}) as Args)
}

function commandsCalled(): string[] {
  return vi.mocked(invoke).mock.calls.map(([c]) => c)
}

/** An argument by its snake_case name or camelCase form, at the top level or one object down. */
function arg(args: Args, name: string): unknown {
  const camel = name.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase())
  for (const scope of [args, ...Object.values(args).filter(isObject)]) {
    if (name in scope) return scope[name]
    if (camel in scope) return scope[camel]
  }
  return undefined
}

function isObject(value: unknown): value is Args {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

/** What a `rewrite` call asked for: the session, the action and the version (0 when left out). */
function rewriteOf(args: Args) {
  return {
    session_id: arg(args, 'session_id'),
    action: arg(args, 'action'),
    variant: arg(args, 'variant') ?? 0,
  }
}

/** The page's text with runs of white space made one space. */
function pageText(): string {
  return (document.body.textContent ?? '').replace(/\s+/g, ' ')
}

async function seeText(text: string) {
  await waitFor(() => expect(pageText()).toContain(text))
}

function button(name: string | RegExp): HTMLElement {
  return screen.getByRole('button', { name })
}

function noButton(name: string | RegExp) {
  expect(screen.queryByRole('button', { name })).toBeNull()
}

/** A button, or a link styled as one. */
function control(name: string | RegExp): HTMLElement {
  return screen.queryByRole('button', { name }) ?? screen.getByRole('link', { name })
}

function links(): HTMLAnchorElement[] {
  return Array.from(document.querySelectorAll<HTMLAnchorElement>('a[href]'))
}

/** publik API chosen, set up and linked, at $3.12. */
function onPublik() {
  answer('provider_status', reply(ON_PUBLIK))
  answer('publik_refresh', reply(ON_PUBLIK.publik))
}

/** Renders the popup and opens it on SESSION, as the hotkey does. */
async function openPopup() {
  render(createElement(Popup))
  await waitFor(() => expect(fake.handlers.get('popup://open')?.length ?? 0).toBeGreaterThan(0))
  await act(async () => fake.send('popup://open', { session_id: SESSION_ID }))
  await seeText(FINDING_MESSAGE)
}

/** Presses an action button and waits for its `rewrite` call. */
async function press(name: RegExp) {
  const before = callsTo('rewrite').length
  fireEvent.click(button(name))
  await waitFor(() => expect(callsTo('rewrite').length).toBe(before + 1))
}

/** The popup on the consent view, reached by pressing Clearer with no provider chosen. */
async function popupConsent() {
  answer('rewrite', refuse(NEEDS_CONSENT))
  await openPopup()
  await press(/Clearer/)
  await screen.findByRole('button', { name: CONTINUE })
}

/** Settings → Providers with publik API chosen and not set up: the consent component. */
async function settingsConsent() {
  render(createElement(Settings))
  const tab =
    (await screen.findAllByRole('tab', { name: 'Providers' }).catch(() => null))?.[0] ??
    button('Providers')
  fireEvent.click(tab)
  if (!screen.queryByRole('button', { name: CONTINUE })) {
    const choice =
      screen.queryAllByRole('radio', { name: /publik API/ })[0] ??
      screen.getAllByRole('button', { name: /publik API/ })[0]
    fireEvent.click(choice)
  }
  await screen.findByRole('button', { name: CONTINUE })
}

/** Renders Settings and opens its Providers tab. */
async function openProviders() {
  render(createElement(Settings))
  const tab =
    (await screen.findAllByRole('tab', { name: 'Providers' }).catch(() => null))?.[0] ??
    button('Providers')
  fireEvent.click(tab)
}

/** The consent strings and buttons, wherever the component is shown. */
function expectConsent(info: { title: string; data_path: string; disclosure: string }) {
  const text = pageText()
  expect(text).toContain(info.title)
  expect(text).toContain(info.data_path)
  expect(text).toContain(info.disclosure)
  expect(text).toContain(CONSENT_FOOTER)
  expect(button(CONTINUE)).toBeInTheDocument()
  expect(button(OWN_KEY)).toBeInTheDocument()
  expect(button(LOCAL)).toBeInTheDocument()
}

beforeEach(() => {
  for (const command of Object.keys(fake.answers)) delete fake.answers[command]
  fake.handlers.clear()
  vi.mocked(invoke).mockClear()
  answer('session_get', reply(SESSION))
  answer('consent_info', reply(CONSENT_INFO))
  answer('provider_status', reply(NO_PROVIDER))
  answer('settings_get', reply(SETTINGS))
  answer('usage_month', reply({ total_micros: 0, calls: 0, by_feature: [] }))
  answer('sent_log_list', reply([]))
  answer('set_providers', (a) => ({
    ...SETTINGS,
    providers: { ...SETTINGS.providers, chat: arg(a, 'chat') ?? null },
  }))
  answer(
    'local_models',
    refuse({
      kind: 'LocalNotRunning',
      message: "Ollama isn't running. Start it, or choose another provider.",
    }),
  )
  answer('publik_connect', reply(AT_ZERO))
  answer(
    'publik_refresh',
    refuse({ kind: 'NotConfigured', message: 'Choose an AI provider in Settings → Providers.' }),
  )
})

afterEach(() => {
  cleanup()
})

// ---------------------------------------------------------------------------------------------
// The check view.

describe('the popup', () => {
  it('s: opens on the check view: the text, its findings and the six actions', async () => {
    await openPopup()
    expect(pageText()).toContain(TEXT)
    for (const name of [
      /Fix grammar/,
      /Clearer/,
      /Shorter/,
      /More formal/,
      /Friendlier/,
      /Custom/,
    ]) {
      expect(button(name)).toBeInTheDocument()
    }
    for (const command of SENDING) expect(callsTo(command)).toEqual([])
  })

  it('s: a text with no findings says "No issues found."', async () => {
    answer(
      'session_get',
      reply({ ...SESSION, text: 'I went to the store yesterday.', findings: [] }),
    )
    render(createElement(Popup))
    await waitFor(() => expect(fake.handlers.get('popup://open')?.length ?? 0).toBeGreaterThan(0))
    await act(async () => fake.send('popup://open', { session_id: SESSION_ID }))
    await seeText('No issues found.')
  })

  it('s: Escape in the check view closes the session', async () => {
    await openPopup()
    fireEvent.keyDown(button(/Clearer/), { key: 'Escape' })
    await waitFor(() => expect(callsTo('close_session').length).toBe(1))
    expect(arg(callsTo('close_session')[0], 'session_id')).toBe(SESSION_ID)
  })

  it('t11: key 2 runs Clearer', async () => {
    answer('rewrite', never)
    await openPopup()
    fireEvent.keyDown(button(/Clearer/), { key: '2' })
    await waitFor(() => expect(callsTo('rewrite').length).toBe(1))
    expect(rewriteOf(callsTo('rewrite')[0])).toEqual({
      session_id: SESSION_ID,
      action: 'clearer',
      variant: 0,
    })
  })

  it('t11: with focus in the instruction field, 1 types and runs nothing', async () => {
    answer('rewrite', never)
    await openPopup()
    // The Custom instruction field: the one text field the user can type in (the selection is
    // shown read-only).
    const editable = () =>
      screen.queryAllByRole('textbox').filter((el) => !(el as HTMLInputElement).readOnly)
    if (editable().length === 0) fireEvent.click(button(/Custom/))
    await waitFor(() => expect(editable().length).toBeGreaterThan(0))
    const field = editable()[0] as HTMLInputElement
    field.focus()
    const before = callsTo('rewrite').length

    fireEvent.keyDown(field, { key: '1' })
    fireEvent.change(field, { target: { value: '1' } })
    await act(async () => undefined)
    expect(callsTo('rewrite').length).toBe(before)
    expect(field.value).toBe('1')
  })

  it('s: loading shows "Rewriting… (publik-balanced)" with Cancel, which goes back', async () => {
    onPublik()
    answer('rewrite', never)
    await openPopup()
    await press(/Clearer/)
    await seeText('Rewriting… (publik-balanced)')
    fireEvent.click(button('Cancel'))
    await screen.findByRole('button', { name: /Shorter/ })
    expect(callsTo('rewrite').length).toBe(1)
  })
})

// ---------------------------------------------------------------------------------------------
// The result view (bullet 3) and its footer.

describe('the result view', () => {
  it('t03: Another version follows Clearer and asks for variant 1', async () => {
    onPublik()
    answer('rewrite', inTurn(reply(CLEARER), never))
    await openPopup()
    await press(/Clearer/)
    fireEvent.click(await screen.findByRole('button', { name: 'Another version' }))
    await waitFor(() => expect(callsTo('rewrite').length).toBe(2))
    expect(rewriteOf(callsTo('rewrite')[1])).toEqual({
      session_id: SESSION_ID,
      action: 'clearer',
      variant: 1,
    })
  })

  it('t03: Another version is absent after Fix grammar', async () => {
    onPublik()
    answer('rewrite', reply(FIX_GRAMMAR))
    await openPopup()
    await press(/Fix grammar/)
    await screen.findByRole('button', { name: /Back/ })
    noButton('Another version')
  })

  it('t03: a guarded answer shows its warning and diff; Enter does nothing, Ctrl+Enter accepts', async () => {
    onPublik()
    answer('rewrite', reply(GUARDED))
    await openPopup()
    await press(/Fix grammar/)
    await seeText(GUARD_WARNING)
    expect(pageText()).toContain('the shop')
    expect(pageText()).toContain('teh store yesterday')

    // Keys are sent from inside the popup, next to its Accept button.
    const inside = button(/^Accept/).parentElement as HTMLElement
    fireEvent.keyDown(inside, { key: 'Enter' })
    await act(async () => undefined)
    expect(callsTo('accept')).toEqual([])

    fireEvent.keyDown(inside, { key: 'Enter', ctrlKey: true })
    await waitFor(() => expect(callsTo('accept').length).toBe(1))
    expect(arg(callsTo('accept')[0], 'session_id')).toBe(SESSION_ID)
    expect(arg(callsTo('accept')[0], 'result_id')).toBe('result-guarded')
  })

  it('t03: an InvalidAnswer shows its message and Try again, which repeats the call', async () => {
    answer('rewrite', inTurn(refuse({ kind: 'InvalidAnswer', message: MSG_INVALID }), never))
    await openPopup()
    await press(/Clearer/)
    await seeText(MSG_INVALID)
    fireEvent.click(button(TRY_AGAIN))
    await waitFor(() => expect(callsTo('rewrite').length).toBe(2))
    expect(rewriteOf(callsTo('rewrite')[1])).toEqual(rewriteOf(callsTo('rewrite')[0]))
  })

  it('s: the footer on publik API is "<tier> · <cost> · <balance line>"', async () => {
    onPublik()
    answer('rewrite', reply(CLEARER))
    await openPopup()
    await press(/Clearer/)
    await act(async () => fake.send('status://publik', publikStatus(3_120_000, 'claimed')))
    await seeText('publik-balanced · $0.0021 · $3.12 of publik balance')
  })

  it.each([
    ['own_key', 'Your own key · example-chat'],
    ['local', 'Local · qwen2.5:14b'],
  ])('s: the footer on %s is the provider line alone', async (chat, line) => {
    answer('provider_status', reply({ chat, publik: null, balance_line: null }))
    answer('rewrite', reply({ ...CLEARER, provider_line: line, charge_micros: null }))
    await openPopup()
    await press(/Clearer/)
    await seeText(line)
    // No cost and no balance line follow the provider line.
    expect(pageText()).not.toMatch(/·\s*\$\d/)
  })
})

// ---------------------------------------------------------------------------------------------
// Bullets 5 and 6: the consent view and the first-run card in the popup.

describe('consent in the popup', () => {
  it.each([
    ['the module 03 strings', CONSENT_INFO],
    [
      'other strings',
      {
        title: TITLE,
        data_path: 'A data-path sentence written for this test.',
        disclosure: 'A disclosure written for this test, version 2.',
        disclosure_version: 2,
      },
    ],
  ])('t05: renders consent_info verbatim (%s) and the three buttons', async (_, info) => {
    answer('consent_info', reply(info))
    await popupConsent()
    expectConsent(info)
    for (const command of SENDING.filter((c) => c !== 'rewrite'))
      expect(callsTo(command)).toEqual([])
  })

  it('s: "Use my own key" and "Use a local model" open Settings → Providers at their section', async () => {
    await popupConsent()
    fireEvent.click(button(OWN_KEY))
    await waitFor(() => expect(callsTo('open_settings').length).toBe(1))
    expect(arg(callsTo('open_settings')[0], 'tab')).toBe('providers')
    expect(arg(callsTo('open_settings')[0], 'section')).toBe('own_key')
    expect(callsTo('publik_connect')).toEqual([])
  })

  it('t06: at $0.00 the first-run card offers linking, own key and local, with the findings kept', async () => {
    await popupConsent()
    fireEvent.click(button(CONTINUE))
    await seeText(COST_SENTENCE)

    const text = pageText()
    expect(text).toContain(ZERO_LINE)
    expect(text).toContain(KEEP_WORKING)
    expect(text).toContain(FINDING_MESSAGE)
    expect(control(SIGN_IN)).toBeInTheDocument()
    expect(button(TRY_AGAIN)).toBeInTheDocument()
    expect(button(OWN_KEY)).toBeInTheDocument()
    expect(button(LOCAL)).toBeInTheDocument()
    expect(callsTo('publik_connect').length).toBe(1)
    expect(callsTo('rewrite').length).toBe(1)
  })

  it('t06: Sign in with publik opens the claim link by its kind, never by a URL', async () => {
    await popupConsent()
    fireEvent.click(button(CONTINUE))
    await seeText(COST_SENTENCE)
    fireEvent.click(control(SIGN_IN))
    await waitFor(() => expect(callsTo('open_account_link').length).toBe(1))
    expect(arg(callsTo('open_account_link')[0], 'kind')).toBe('claim')
    expect(JSON.stringify(vi.mocked(invoke).mock.calls)).not.toContain('https://')
  })

  it('t04: Try again on the first-run card sends the pending action', async () => {
    await popupConsent()
    answer('rewrite', never)
    fireEvent.click(button(CONTINUE))
    await seeText(COST_SENTENCE)
    fireEvent.click(button(TRY_AGAIN))
    await waitFor(() => expect(callsTo('rewrite').length).toBe(2))
    expect(rewriteOf(callsTo('rewrite')[1])).toEqual({
      session_id: SESSION_ID,
      action: 'clearer',
      variant: 0,
    })
  })
})

// ---------------------------------------------------------------------------------------------
// Bullets 7 and 8: the error card.

describe('the error card', () => {
  it('t07: a 402 shows its message, exactly one link (the top-up link) and the way out', async () => {
    answer(
      'provider_status',
      reply({ chat: 'publik', publik: publikStatus(0, 'anonymous'), balance_line: ZERO_LINE }),
    )
    answer('rewrite', refuse(NEEDS_CREDIT))
    await openPopup()
    await press(/Clearer/)
    await seeText(MSG_402)

    expect(pageText()).toContain(OR_OWN_KEY)
    expect(pageText()).toContain(FINDING_MESSAGE)
    const all = links()
    expect(all).toHaveLength(1)
    expect(all[0].getAttribute('href')).toBe(CLAIM_URL)
    expect(all[0]).toHaveTextContent(SIGN_IN)

    fireEvent.click(all[0])
    await waitFor(() => expect(callsTo('open_account_link').length).toBe(1))
    expect(arg(callsTo('open_account_link')[0], 'kind')).toBe('top_up')
  })

  it('t08: Disconnected shows its message with Reconnect, which connects again', async () => {
    answer('rewrite', refuse({ kind: 'Disconnected', message: MSG_DISCONNECTED }))
    await openPopup()
    await press(/Clearer/)
    await seeText(MSG_DISCONNECTED)
    noButton(TRY_AGAIN)
    fireEvent.click(button('Reconnect'))
    await waitFor(() => expect(callsTo('publik_connect').length).toBe(1))
  })

  it('t08: PublikBusy shows its message with Try again, which repeats the call', async () => {
    answer('rewrite', inTurn(refuse({ kind: 'PublikBusy', message: MSG_BUSY }), never))
    await openPopup()
    await press(/Clearer/)
    await seeText(MSG_BUSY)
    noButton('Reconnect')
    fireEvent.click(button(TRY_AGAIN))
    await waitFor(() => expect(callsTo('rewrite').length).toBe(2))
    expect(rewriteOf(callsTo('rewrite')[1])).toEqual(rewriteOf(callsTo('rewrite')[0]))
  })

  it('t08: Offline shows its message and no button, and the findings stay', async () => {
    answer('rewrite', refuse({ kind: 'Offline', message: MSG_OFFLINE }))
    await openPopup()
    await press(/Clearer/)
    await seeText(MSG_OFFLINE)
    noButton(TRY_AGAIN)
    noButton('Reconnect')
    expect(links()).toHaveLength(0)
    expect(pageText()).toContain(FINDING_MESSAGE)
  })
})

// ---------------------------------------------------------------------------------------------
// Bullet 15: Welcome.

describe('Welcome', () => {
  it('t15: explains the privacy model and the hotkey, with Close, and asks for no consent', async () => {
    render(createElement(Welcome))
    for (const sentence of WELCOME_SENTENCES) await seeText(sentence)
    expect(pageText()).toContain('Ctrl+Alt+R')
    expect(button('Close')).toBeInTheDocument()
    expect(pageText()).not.toContain(TITLE)
    noButton(CONTINUE)
    for (const command of SENDING) expect(callsTo(command)).toEqual([])
  })
})

// ---------------------------------------------------------------------------------------------
// Bullet 18 and the Settings window.

describe('Settings', () => {
  it('s: has the tabs General, Providers, Privacy and About, and no Dictionary tab yet', async () => {
    render(createElement(Settings))
    for (const name of ['General', 'Providers', 'Privacy', 'About']) {
      await waitFor(() =>
        expect(
          screen.queryAllByRole('tab', { name }).length +
            screen.queryAllByRole('button', { name }).length,
        ).toBeGreaterThan(0),
      )
    }
    expect(screen.queryAllByRole('tab', { name: 'Dictionary' })).toHaveLength(0)
    expect(screen.queryAllByRole('button', { name: 'Dictionary' })).toHaveLength(0)
  })

  it('t18: Consent in Settings shows the same strings and buttons as in the popup, and no Try again', async () => {
    await settingsConsent()
    expectConsent(CONSENT_INFO)
    noButton(TRY_AGAIN)
    expect(callsTo('publik_connect')).toEqual([])
  })

  it('t18: after publik_connect at $0.00 Settings shows the first-run card without Try again', async () => {
    await settingsConsent()
    fireEvent.click(button(CONTINUE))
    await seeText(COST_SENTENCE)
    expect(pageText()).toContain(ZERO_LINE)
    expect(control(SIGN_IN)).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: OWN_KEY }).length).toBeGreaterThan(0)
    expect(screen.getAllByRole('button', { name: LOCAL }).length).toBeGreaterThan(0)
    noButton(TRY_AGAIN)
    expect(callsTo('publik_connect').length).toBe(1)
    expect(callsTo('rewrite')).toEqual([])
  })

  it('s: Providers shows this month’s use in dollars', async () => {
    onPublik()
    answer(
      'settings_get',
      reply({ ...SETTINGS, providers: { ...SETTINGS.providers, chat: 'publik' } }),
    )
    answer('usage_month', reply({ total_micros: 410_000, calls: 312, by_feature: [] }))
    render(createElement(Settings))
    const tab =
      (await screen.findAllByRole('tab', { name: 'Providers' }).catch(() => null))?.[0] ??
      button('Providers')
    fireEvent.click(tab)
    await seeText('This month: $0.41 across 312 actions')
    expect(pageText()).toContain('$3.12 of publik balance')
    expect(pageText()).not.toMatch(/credits?\b/i)
  })

  it('s: the wallet is read when Settings opens on publik API, with "Checking…" meanwhile', async () => {
    onPublik()
    answer(
      'settings_get',
      reply({ ...SETTINGS, providers: { ...SETTINGS.providers, chat: 'publik' } }),
    )
    let answerWallet: (status: unknown) => void = () => undefined
    answer('publik_refresh', () => new Promise((resolve) => (answerWallet = resolve)))
    await openProviders()
    await seeText(CHECKING)
    expect(callsTo('publik_refresh').length).toBe(1)

    await act(async () => answerWallet(ON_PUBLIK.publik))
    await waitFor(() => expect(pageText()).not.toContain(CHECKING))
    expect(pageText()).toContain('$3.12 of publik balance')
    expect(pageText()).not.toContain(PUBLIK_OFFLINE)
  })

  it('e: offline, the wallet line is "Can\'t reach publik API right now." in place of the error\'s own message', async () => {
    onPublik()
    answer(
      'settings_get',
      reply({ ...SETTINGS, providers: { ...SETTINGS.providers, chat: 'publik' } }),
    )
    answer('publik_refresh', refuse({ kind: 'Offline', message: MSG_OFFLINE }))
    await openProviders()
    await seeText(PUBLIK_OFFLINE)
    expect(pageText()).not.toContain(MSG_OFFLINE)
    expect(pageText()).not.toContain(CHECKING)
  })

  it('e: settings written by a newer Redpen show the banner and every control is disabled', async () => {
    answer('settings_get', reply({ ...SETTINGS, read_only: true }))
    render(createElement(Settings))
    await seeText(NEWER)
    const dialect = screen.getByRole('combobox')
    expect(dialect).toBeDisabled()
    expect(button('Change shortcut')).toBeDisabled()
    for (const box of screen.getAllByRole('checkbox')) expect(box).toBeDisabled()

    fireEvent.click(screen.getAllByRole('tab', { name: 'Privacy' })[0])
    await seeText(NEWER)
    for (const box of screen.getAllByRole('checkbox')) expect(box).toBeDisabled()
    expect(callsTo('settings_update')).toEqual([])
  })

  it('s: opening Settings sends nothing while no provider is chosen', async () => {
    render(createElement(Settings))
    await waitFor(() => expect(commandsCalled()).toContain('settings_get'))
    for (const command of SENDING) expect(callsTo(command)).toEqual([])
  })
})
