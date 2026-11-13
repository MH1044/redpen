// Unit tests of the popup component's own paths: a capture that stopped, Undo, an Accept that
// could not type, and an error whose cure is in Settings. Tauri is faked: `invoke` answers from
// `answers`, and `listen` keeps its handlers so a test can send `popup://open`.

import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const fake = vi.hoisted(() => {
  const handlers = new Map<string, Array<(event: unknown) => void>>()
  const answers: Record<string, (args: Record<string, unknown>) => unknown> = {}
  return { handlers, answers }
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
  listen: vi.fn(async (name: string, handler: (event: unknown) => void) => {
    fake.handlers.set(name, [...(fake.handlers.get(name) ?? []), handler])
    return () => undefined
  }),
}))

import { invoke } from '@tauri-apps/api/core'
import { UNDO_ON_CLIPBOARD } from '../state/popup-view'
import Popup from './Popup'

const SESSION = {
  id: 's1',
  text: 'I saw teh cat.',
  findings: [],
  dialect: 'british',
  english: true,
}

const OUTCOME = {
  result_id: 'r1',
  text: 'I saw the cat.',
  diff: [{ equal: 'I saw the cat.' }],
  warnings: [],
  accept_default: true,
  unchanged: false,
  provider_line: 'Local · qwen2.5:14b',
  charge_micros: null,
}

function callsTo(command: string): Record<string, unknown>[] {
  return vi
    .mocked(invoke)
    .mock.calls.filter(([c]) => c === command)
    .map(([, a]) => (a ?? {}) as Record<string, unknown>)
}

async function open(payload: Record<string, unknown>) {
  render(<Popup />)
  await waitFor(() => expect(fake.handlers.get('popup://open')?.length ?? 0).toBeGreaterThan(0))
  await act(async () => {
    for (const h of fake.handlers.get('popup://open') ?? []) h({ payload })
  })
}

beforeEach(() => {
  for (const k of Object.keys(fake.answers)) delete fake.answers[k]
  fake.handlers.clear()
  vi.mocked(invoke).mockClear()
  fake.answers.session_get = () => SESSION
  fake.answers.provider_status = () => ({ chat: 'local', publik: null, balance_line: null })
})

afterEach(() => cleanup())

describe('the popup', () => {
  it('shows why a capture stopped, and Escape hides it', async () => {
    await open({
      session_id: null,
      error: { kind: 'NothingCopied', message: 'Nothing was copied. Select some text first.' },
    })
    await screen.findByText('Nothing was copied. Select some text first.')
    expect(callsTo('session_get')).toEqual([])
    fireEvent.keyDown(window, { key: 'Escape' })
    await waitFor(() => expect(callsTo('close_session')).toEqual([{ session_id: '' }]))
  })

  it('offers Undo after an Accept, and says when the original went on the clipboard', async () => {
    fake.answers.undo_last_replace = () => 'on_clipboard'
    await open({
      session_id: null,
      error: { kind: 'NothingCopied', message: 'Nothing was copied. Select some text first.' },
      can_undo: true,
    })
    fireEvent.click(await screen.findByRole('button', { name: 'Undo last replace' }))
    await screen.findByText(UNDO_ON_CLIPBOARD)
    expect(screen.queryByRole('button', { name: 'Undo last replace' })).toBeNull()
  })

  it('offers no Undo when there is nothing to undo', async () => {
    await open({ session_id: 's1', can_undo: false })
    await screen.findByText('No issues found.')
    expect(screen.queryByRole('button', { name: 'Undo last replace' })).toBeNull()
  })

  it('after an Accept that could not type, Copy copies that result', async () => {
    fake.answers.rewrite = () => OUTCOME
    fake.answers.accept = () =>
      Promise.reject({
        kind: 'FocusChanged',
        message:
          'The window you started in is no longer in front, so nothing was typed. Copy the text instead.',
      })
    await open({ session_id: 's1' })
    fireEvent.click(await screen.findByRole('button', { name: /Clearer/ }))
    fireEvent.click(await screen.findByRole('button', { name: 'Accept' }))
    await screen.findByText(/nothing was typed/)
    fireEvent.click(screen.getByRole('button', { name: 'Copy' }))
    await waitFor(() =>
      expect(callsTo('copy_result')).toEqual([{ session_id: 's1', result_id: 'r1' }]),
    )
    await screen.findByText('Copied.')
  })

  it('an error whose cure is in Settings opens Settings → Providers there', async () => {
    fake.answers.rewrite = () =>
      Promise.reject({
        kind: 'LocalNotRunning',
        message: "Ollama isn't running. Start it, or choose another provider.",
      })
    await open({ session_id: 's1' })
    fireEvent.click(await screen.findByRole('button', { name: /Shorter/ }))
    fireEvent.click(await screen.findByRole('button', { name: 'Open Settings' }))
    await waitFor(() =>
      expect(callsTo('open_settings')).toEqual([{ tab: 'providers', section: 'local' }]),
    )
    expect(screen.queryByRole('button', { name: 'Try again' })).toBeNull()
  })

  it('Custom asks for an instruction, then sends it', async () => {
    fake.answers.rewrite = () => new Promise(() => undefined)
    await open({ session_id: 's1' })
    fireEvent.click(await screen.findByRole('button', { name: /Custom/ }))
    const field = await screen.findByRole('textbox', { name: 'Instruction' })
    // Digits typed into the field are text, not actions.
    fireEvent.keyDown(field, { key: '2' })
    expect(callsTo('rewrite')).toEqual([])
    fireEvent.change(field, { target: { value: 'Make it a question.' } })
    fireEvent.click(screen.getByRole('button', { name: 'Rewrite' }))
    await waitFor(() =>
      expect(callsTo('rewrite')).toEqual([
        { session_id: 's1', action: 'custom', instruction: 'Make it a question.', variant: 0 },
      ]),
    )
  })
})
