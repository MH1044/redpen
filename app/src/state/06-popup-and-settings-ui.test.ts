// Module 06 tests of the popup's state machine (docs/modules/06-popup-and-settings-ui.md,
// Acceptance tests, bullets 3, 4 and 11, the reducer half; the core half of 3 and 4 is in
// crates/redpen-core/tests/06_session.rs). A test named `tNN: …` covers bullet NN, `s: …` a Scope
// bullet the numbered list leaves implicit, `e: …` an error path.
//
// `reduce(s, e)` is pure and returns the effects to run (the spec's Design notes), so these tests
// need no Tauri. Every type is taken from `reduce` and `initial` themselves, so the tests name
// nothing the spec does not.
//
// Expected values come from the spec's Design notes (the events, the effects and the key map),
// docs/PRODUCT.md §5 (the check view's Accept on Enter), docs/PUBLIK-INTEGRATION.md §4 and §5
// (consent, the first-run card, `variant`, Try again) and the fixtures below, which are true by
// construction: the session is module 01's reference finding for corpus sentence c01, and each
// outcome's diff joins back into both texts (tests/cases/06-popup-and-settings-ui/REFERENCE.md).

import { describe, expect, it } from 'vitest'
import { initial, reduce } from './popup'

type PopupState = Parameters<typeof reduce>[0]
type PopupEvent = Parameters<typeof reduce>[1]
type Effect = ReturnType<typeof reduce>['effects'][number]
type Session = Extract<PopupState, { view: 'check' }>['session']
type RewriteOutcome = Extract<PopupState, { view: 'result' }>['outcome']
type Action = Extract<PopupEvent, { type: 'action' }>['action']
type UiError = Extract<PopupEvent, { type: 'rewrite-failed' }>['error']
type ConnectOutcome = Extract<PopupEvent, { type: 'connected' }>['outcome']
type View = PopupState['view']

// ---------------------------------------------------------------------------------------------
// Fixtures, in the shapes of docs/ARCHITECTURE.md §4 and §5. They are cast through `unknown`, so
// a field the interface's own types add or name more narrowly never stops them compiling.

/** Corpus sentence c01 checked in American English: module 01's reference finding. */
const SESSION = {
  id: 'session-06',
  text: 'I went to teh store yesterday.',
  findings: [
    {
      id: '10-13-spelling-57a49948',
      category: 'spelling',
      harper_kind: 'Typo',
      message: 'Did you mean the definite article?',
      span: { chars: [10, 13], bytes: [10, 13], utf16: [10, 13] },
      suggestions: [{ kind: 'replace', text: 'the' }],
    },
  ],
  dialect: 'american',
  english: true,
} as unknown as Session

/** The same session after its one fix: the corpus's corrected text, with nothing left to fix. */
const FIXED = {
  ...SESSION,
  text: 'I went to the store yesterday.',
  findings: [],
} as unknown as Session

const ID = SESSION.id

function outcome(fields: Record<string, unknown>): RewriteOutcome {
  return {
    warnings: [],
    accept_default: true,
    unchanged: false,
    ...fields,
  } as unknown as RewriteOutcome
}

/** Fix grammar on publik API: the corrected text, $0.0002. */
const FIX_GRAMMAR = outcome({
  result_id: 'result-fix',
  text: 'I went to the store yesterday.',
  diff: [
    { equal: 'I went to ' },
    { delete: 'teh' },
    { insert: 'the' },
    { equal: ' store yesterday.' },
  ],
  provider_line: 'publik-fast',
  charge_micros: 210,
})

/** Fix grammar whose answer the minimal-edit guard flagged: Enter must not accept it. */
const GUARDED = outcome({
  result_id: 'result-guarded',
  text: 'Yesterday, I went to the shop.',
  diff: [
    { insert: 'Yesterday, ' },
    { equal: 'I went to ' },
    { delete: 'teh store yesterday' },
    { insert: 'the shop' },
    { equal: '.' },
  ],
  warnings: [
    'This changes more than a grammar fix should (9 of 14 words). Check it before accepting.',
  ],
  accept_default: false,
  provider_line: 'publik-fast',
  charge_micros: 210,
})

/** Clearer on publik API, $0.0021. */
const CLEARER = outcome({
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
  provider_line: 'publik-balanced',
  charge_micros: 2100,
})

const NEEDS_CONSENT = { kind: 'NeedsConsent', message: 'Choose how Redpen uses AI.' } as UiError
const INVALID_ANSWER = {
  kind: 'InvalidAnswer',
  message: 'The AI answer could not be used.',
} as UiError
const PROVISION_FAILED = {
  kind: 'ProvisionFailed',
  message: 'publik API is unreachable right now. Use your own key for now.',
} as UiError

/** A publik status as status.json holds it (docs/ARCHITECTURE.md §4). */
function status(balance: number) {
  return {
    provider: 'publik',
    claim_state: 'anonymous',
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

/** `/installs` at $0.00 (tests/cases/03-publik/installs-201.fake.json): the pending action waits. */
const AT_ZERO = {
  status: status(0),
  run_pending: false,
  balance_line: '$0.00 · link this computer for $0.05 of free use',
  cost_sentence:
    "A typical rewrite costs less than one cent, paid from this computer's publik balance.",
  claim_url: 'https://publikhq.com/claim/fake-claim-token',
  claim_state: 'anonymous',
} as unknown as ConnectOutcome

/** `/installs` with a balance above 0: the pending action runs at once. */
const FUNDED = {
  ...AT_ZERO,
  status: status(50_000),
  run_pending: true,
  balance_line: '$0.05 of publik balance',
} as unknown as ConnectOutcome

// ---------------------------------------------------------------------------------------------
// Helpers.

function key(
  name: string,
  modifiers: { ctrl?: boolean; shift?: boolean; inInstruction?: boolean } = {},
): PopupEvent {
  return { type: 'key', key: name, ctrl: false, shift: false, inInstruction: false, ...modifiers }
}

function rewrite(action: Action, variant: number, instruction?: string): Effect {
  const args =
    instruction === undefined
      ? { session_id: ID, action, variant }
      : { session_id: ID, action, instruction, variant }
  return { call: 'rewrite', args } as Effect
}

/** The state narrowed to `view`, failing the test when it is another view. */
function inView<V extends View>(s: PopupState, view: V): Extract<PopupState, { view: V }> {
  expect(s.view).toBe(view)
  return s as Extract<PopupState, { view: V }>
}

/** Runs `events` from `s` and returns the last state and every effect, in order. */
function run(s: PopupState, ...events: PopupEvent[]): { state: PopupState; effects: Effect[] } {
  let state = s
  const effects: Effect[] = []
  for (const event of events) {
    const next = reduce(state, event)
    state = next.state
    effects.push(...next.effects)
  }
  return { state, effects }
}

function calls(effects: Effect[]): string[] {
  return effects.map((e) => e.call)
}

const CHECK = initial(SESSION)

/** The result view of `action`, reached the way the popup reaches it. */
function resultOf(action: Action, o: RewriteOutcome): PopupState {
  return run(CHECK, { type: 'action', action }, { type: 'rewrite-done', outcome: o }).state
}

/** The consent view with Clearer pending. */
function consent(): PopupState {
  return run(
    CHECK,
    { type: 'action', action: 'clearer' },
    { type: 'rewrite-failed', error: NEEDS_CONSENT },
  ).state
}

/** A deep copy of plain JSON data, so freezing it never touches the shared fixtures. */
function copy<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

function deepFreeze<T>(value: T): T {
  if (value !== null && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value)
    for (const inner of Object.values(value)) deepFreeze(inner)
  }
  return value
}

// ---------------------------------------------------------------------------------------------
// Bullet 3: check → action 2 → loading → outcome → result; Enter and Ctrl+Enter; Another version;
// Try again after an invalid answer.

describe('t03: rewriting from the popup', () => {
  it('t03: the popup opens on the check view of its session', () => {
    expect(inView(CHECK, 'check').session).toEqual(SESSION)
  })

  it('t03: action 2 loads Clearer, asking for variant 0, and its outcome shows the result', () => {
    const loading = reduce(CHECK, key('2'))
    expect(inView(loading.state, 'loading').action).toBe('clearer')
    expect(loading.state.session).toEqual(SESSION)
    expect(loading.effects).toEqual([rewrite('clearer', 0)])

    const done = reduce(loading.state, { type: 'rewrite-done', outcome: CLEARER })
    expect(inView(done.state, 'result').outcome).toEqual(CLEARER)
    expect(done.state.session).toEqual(SESSION)
    expect(done.effects).toEqual([])
  })

  it('t03: with accept_default false, Enter does nothing and Ctrl+Enter accepts', () => {
    const result = resultOf('fix-grammar', GUARDED)

    const enter = reduce(result, key('Enter'))
    expect(enter.effects).toEqual([])
    expect(enter.state).toEqual(result)

    const ctrlEnter = reduce(result, key('Enter', { ctrl: true }))
    expect(ctrlEnter.effects).toEqual([
      { call: 'accept', args: { session_id: ID, result_id: 'result-guarded' } },
    ])
  })

  it('t03: with accept_default true, Enter accepts the result', () => {
    const result = resultOf('clearer', CLEARER)
    expect(reduce(result, key('Enter')).effects).toEqual([
      { call: 'accept', args: { session_id: ID, result_id: 'result-clearer' } },
    ])
  })

  it('t03: Another version asks for variant 1, then 2', () => {
    const result = resultOf('clearer', CLEARER)

    const first = reduce(result, { type: 'another-version' })
    inView(first.state, 'loading')
    expect(first.effects).toEqual([rewrite('clearer', 1)])

    const second = run(
      first.state,
      { type: 'rewrite-done', outcome: { ...CLEARER, result_id: 'result-clearer-1' } },
      { type: 'another-version' },
    )
    inView(second.state, 'loading')
    expect(second.effects).toEqual([rewrite('clearer', 2)])
  })

  it.each(['clearer', 'shorter', 'more-formal', 'friendlier'] as Action[])(
    't03: Another version is there after %s',
    (action) => {
      const result = resultOf(action, CLEARER)
      expect(reduce(result, { type: 'another-version' }).effects).toEqual([rewrite(action, 1)])
    },
  )

  it('t03: Another version keeps the Custom instruction', () => {
    const instruction = 'Make it a question.'
    const loading = reduce(CHECK, { type: 'action', action: 'custom', instruction })
    expect(loading.effects).toEqual([rewrite('custom', 0, instruction)])
    const again = run(
      loading.state,
      { type: 'rewrite-done', outcome: CLEARER },
      { type: 'another-version' },
    )
    expect(again.effects).toEqual([rewrite('custom', 1, instruction)])
  })

  it('t03: Another version is absent for Fix grammar', () => {
    const result = resultOf('fix-grammar', FIX_GRAMMAR)
    const pressed = reduce(result, { type: 'another-version' })
    expect(pressed.effects).toEqual([])
    expect(inView(pressed.state, 'result').outcome).toEqual(FIX_GRAMMAR)
  })

  it('t03: an InvalidAnswer shows Try again, which calls rewrite again with the same arguments', () => {
    const loading = reduce(CHECK, { type: 'action', action: 'clearer' })
    const failed = reduce(loading.state, { type: 'rewrite-failed', error: INVALID_ANSWER })
    expect(inView(failed.state, 'error').error).toEqual(INVALID_ANSWER)
    expect(failed.state.session).toEqual(SESSION)
    expect(failed.effects).toEqual([])

    const again = reduce(failed.state, { type: 'try-again' })
    inView(again.state, 'loading')
    expect(again.effects).toEqual(loading.effects)
    expect(again.effects).toEqual([rewrite('clearer', 0)])
  })

  it('t03: Try again after a failed Another version repeats that version', () => {
    const anotherVersion = run(
      resultOf('friendlier', CLEARER),
      { type: 'another-version' },
      { type: 'rewrite-failed', error: INVALID_ANSWER },
      { type: 'try-again' },
    )
    expect(anotherVersion.effects).toEqual([rewrite('friendlier', 1), rewrite('friendlier', 1)])
  })
})

// ---------------------------------------------------------------------------------------------
// Bullet 4: NeedsConsent → consent view → publik_connect → first-run view → Try again.

describe('t04: consent and the first-run card', () => {
  it('t04: NeedsConsent opens the consent view with the pending action', () => {
    const loading = reduce(CHECK, { type: 'action', action: 'clearer' })
    const failed = reduce(loading.state, { type: 'rewrite-failed', error: NEEDS_CONSENT })
    expect(inView(failed.state, 'consent').pending).toBe('clearer')
    expect(failed.state.session).toEqual(SESSION)
    expect(failed.effects).toEqual([])
  })

  it('t04: "Continue with publik API" calls publik_connect and nothing else', () => {
    const pressed = reduce(consent(), { type: 'consent', choice: 'publik' })
    expect(pressed.effects).toEqual([{ call: 'publik_connect' }])
  })

  it('t04: run_pending false gives the first-run view, and Try again loads the pending action', () => {
    const connected = run(
      consent(),
      { type: 'consent', choice: 'publik' },
      { type: 'connected', outcome: AT_ZERO },
    )
    expect(inView(connected.state, 'first-run').pending).toBe('clearer')
    expect(connected.state.session).toEqual(SESSION)
    expect(calls(connected.effects)).toEqual(['publik_connect'])

    const again = reduce(connected.state, { type: 'try-again' })
    expect(inView(again.state, 'loading').action).toBe('clearer')
    expect(again.effects).toEqual([rewrite('clearer', 0)])
  })

  it('t04: run_pending true runs the pending action at once', () => {
    const connected = reduce(consent(), { type: 'connected', outcome: FUNDED })
    expect(inView(connected.state, 'loading').action).toBe('clearer')
    expect(connected.effects).toEqual([rewrite('clearer', 0)])
  })

  it('s: "Use my own key" and "Use a local model" open Settings → Providers there, sending nothing', () => {
    expect(reduce(consent(), { type: 'consent', choice: 'own_key' }).effects).toEqual([
      { call: 'open_settings', tab: 'providers', section: 'own_key' },
    ])
    expect(reduce(consent(), { type: 'consent', choice: 'local' }).effects).toEqual([
      { call: 'open_settings', tab: 'providers', section: 'local' },
    ])
  })

  it('e: a failed connect shows its error, with the findings kept', () => {
    const failed = run(
      consent(),
      { type: 'consent', choice: 'publik' },
      { type: 'connect-failed', error: PROVISION_FAILED },
    )
    expect(inView(failed.state, 'error').error).toEqual(PROVISION_FAILED)
    expect(failed.state.session).toEqual(SESSION)
  })
})

// ---------------------------------------------------------------------------------------------
// Bullet 11 and the rest of the key map.

describe('t11: the keyboard', () => {
  const ACTIONS: [string, Action][] = [
    ['1', 'fix-grammar'],
    ['2', 'clearer'],
    ['3', 'shorter'],
    ['4', 'more-formal'],
    ['5', 'friendlier'],
    ['6', 'custom'],
  ]

  it.each(ACTIONS)('t11: key %s dispatches %s', (name, action) => {
    expect(reduce(CHECK, key(name))).toEqual(reduce(CHECK, { type: 'action', action }))
  })

  it.each(ACTIONS.slice(0, 5))('t11: key %s runs %s at variant 0', (name, action) => {
    const pressed = reduce(CHECK, key(name))
    expect(inView(pressed.state, 'loading').action).toBe(action)
    expect(pressed.effects).toEqual([rewrite(action, 0)])
  })

  it.each(ACTIONS)(
    't11: with focus in the instruction field, %s types and runs nothing',
    (name) => {
      const typed = reduce(CHECK, key(name, { inInstruction: true }))
      expect(typed.effects).toEqual([])
      expect(typed.state).toEqual(CHECK)
    },
  )

  it('s: Ctrl+Shift+C copies the result, or the text in the check view', () => {
    const copy = key('C', { ctrl: true, shift: true })
    expect(reduce(resultOf('clearer', CLEARER), copy).effects).toEqual([
      { call: 'copy_result', args: { session_id: ID, result_id: 'result-clearer' } },
    ])
    expect(reduce(CHECK, copy).effects).toEqual([{ call: 'copy_result', args: { session_id: ID } }])
  })

  it('s: Enter in the check view accepts the text with its fixes', () => {
    expect(reduce(CHECK, key('Enter')).effects).toEqual([
      { call: 'accept', args: { session_id: ID } },
    ])
  })

  it('s: Escape goes back from the result, consent, first-run and error views', () => {
    const firstRun = run(consent(), { type: 'connected', outcome: AT_ZERO }).state
    const error = run(
      CHECK,
      { type: 'action', action: 'clearer' },
      { type: 'rewrite-failed', error: INVALID_ANSWER },
    ).state
    for (const from of [resultOf('clearer', CLEARER), consent(), firstRun, error]) {
      const back = reduce(from, key('Escape'))
      expect(inView(back.state, 'check').session).toEqual(SESSION)
      expect(back.effects).toEqual([])
    }
  })

  it('s: Escape in the check view closes the popup and drops the session', () => {
    expect(reduce(CHECK, key('Escape')).effects).toEqual([
      { call: 'close_session', session_id: ID },
    ])
  })
})

// ---------------------------------------------------------------------------------------------
// The other events of the Design notes.

describe('s: the other popup events', () => {
  it('s: an applied fix replaces the session in the check view', () => {
    const fixed = reduce(CHECK, { type: 'fix-applied', session: FIXED })
    expect(inView(fixed.state, 'check').session).toEqual(FIXED)
    expect(fixed.effects).toEqual([])
  })

  it('s: Cancel stops waiting, back on the check view, and asks for nothing again', () => {
    const cancelled = run(CHECK, { type: 'action', action: 'clearer' }, { type: 'cancel' })
    inView(cancelled.state, 'check')
    expect(cancelled.effects).toEqual([rewrite('clearer', 0)])
  })

  it('s: the Accept button accepts a result Enter would not', () => {
    expect(
      reduce(resultOf('fix-grammar', GUARDED), { type: 'accept', always: true }).effects,
    ).toEqual([{ call: 'accept', args: { session_id: ID, result_id: 'result-guarded' } }])
  })

  it('s: Copy copies the result, Back returns to the check view, Close closes', () => {
    const result = resultOf('clearer', CLEARER)
    expect(reduce(result, { type: 'copy' }).effects).toEqual([
      { call: 'copy_result', args: { session_id: ID, result_id: 'result-clearer' } },
    ])
    const back = reduce(result, { type: 'back' })
    inView(back.state, 'check')
    expect(back.effects).toEqual([])
    expect(reduce(CHECK, { type: 'close' }).effects).toEqual([
      { call: 'close_session', session_id: ID },
    ])
  })

  it('s: reduce never changes the state it is given', () => {
    const events: PopupEvent[] = [
      { type: 'action', action: 'clearer' },
      { type: 'rewrite-failed', error: NEEDS_CONSENT },
      { type: 'consent', choice: 'publik' },
      { type: 'connected', outcome: AT_ZERO },
      { type: 'try-again' },
      { type: 'rewrite-done', outcome: CLEARER },
      { type: 'another-version' },
      { type: 'rewrite-failed', error: INVALID_ANSWER },
      key('Escape'),
      { type: 'fix-applied', session: FIXED },
    ]
    let state: PopupState = deepFreeze(initial(deepFreeze(copy(SESSION))))
    for (const event of events) {
      state = deepFreeze(reduce(state, deepFreeze(copy(event))).state)
    }
    expect(inView(state, 'check').session).toEqual(FIXED)
  })
})
