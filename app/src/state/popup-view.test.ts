// Unit tests of what the popup's views say: the loading line, the footer, the underlined runs of
// the text and the fix buttons' labels.

import { describe, expect, it } from 'vitest'
import type { Finding, RewriteOutcome } from '../ipc'
import {
  UNDO_ON_CLIPBOARD,
  footerLine,
  fromPublik,
  loadingLine,
  providerLineFor,
  segments,
  suggestionLabel,
  warningText,
} from './popup-view'

const OUTCOME: RewriteOutcome = {
  result_id: 'r1',
  text: 'x',
  diff: [],
  warnings: [],
  accept_default: true,
  unchanged: false,
  provider_line: 'publik-fast',
  charge_micros: 210,
}

function finding(id: string, start: number, end: number): Finding {
  return {
    id,
    category: 'spelling',
    harper_kind: 'Typo',
    message: 'm',
    span: { chars: [start, end], bytes: [start, end], utf16: [start, end] },
    suggestions: [],
  }
}

describe('the loading line', () => {
  it('names the provider, or nothing without one', () => {
    expect(loadingLine('publik-balanced')).toBe('Rewriting… (publik-balanced)')
    expect(loadingLine('Local · qwen2.5:14b')).toBe('Rewriting… (Local · qwen2.5:14b)')
    expect(loadingLine('  ')).toBe('Rewriting…')
  })

  it('falls back to the chosen provider’s tier', () => {
    expect(providerLineFor('publik', 'fix-grammar')).toBe('publik-fast')
    expect(providerLineFor('publik', 'custom')).toBe('publik-balanced')
    expect(providerLineFor('own_key', 'clearer')).toBe('Your own key')
    expect(providerLineFor('local', 'clearer')).toBe('Local')
    expect(providerLineFor(null, 'clearer')).toBe('')
  })
})

describe('the footer', () => {
  it('is tier, cost and balance on publik API (docs/PUBLIK-INTEGRATION.md §9)', () => {
    expect(footerLine(OUTCOME, true, '$3.12 of publik balance')).toBe(
      'publik-fast · $0.0002 · $3.12 of publik balance',
    )
    expect(footerLine(OUTCOME, true, null)).toBe('publik-fast · $0.0002')
  })

  it('counts an answer from the cache as $0.00', () => {
    expect(footerLine({ ...OUTCOME, charge_micros: null }, true, null)).toBe('publik-fast · $0.00')
  })

  it('is the provider line alone off publik API', () => {
    const own = { ...OUTCOME, provider_line: 'Your own key · example-chat', charge_micros: null }
    expect(footerLine(own, false, '$3.12 of publik balance')).toBe('Your own key · example-chat')
  })

  it('takes the provider from the outcome when it says, else from the choice', () => {
    expect(fromPublik({ ...OUTCOME, provider: 'local' }, 'publik')).toBe(false)
    expect(fromPublik({ ...OUTCOME, provider: 'publik' }, null)).toBe(true)
    expect(fromPublik(OUTCOME, 'publik')).toBe(true)
    expect(fromPublik(OUTCOME, 'own_key')).toBe(false)
  })
})

describe('warnings and notices', () => {
  it('reads a warning object or its message', () => {
    expect(warningText('This is not shorter than your text.')).toBe(
      'This is not shorter than your text.',
    )
    expect(
      warningText({ kind: 'not_shorter', message: 'This is not shorter than your text.' }),
    ).toBe('This is not shorter than your text.')
  })

  it('says where the original went when Undo could not type it back', () => {
    expect(UNDO_ON_CLIPBOARD).toContain('on the clipboard')
    expect(UNDO_ON_CLIPBOARD).toContain('Ctrl+V')
  })
})

describe('the text’s runs', () => {
  it('underlines each finding by its UTF-16 span', () => {
    expect(segments('I saw teh cat.', [finding('a', 6, 9)])).toEqual([
      { text: 'I saw ' },
      { text: 'teh', findingId: 'a' },
      { text: ' cat.' },
    ])
  })

  it('counts an emoji as two units, as JavaScript does', () => {
    // A thumbs-up (two UTF-16 units) then " teh".
    const text = '\u{1F44D} teh'
    expect(segments(text, [finding('a', 3, 6)])).toEqual([
      { text: '\u{1F44D} ' },
      { text: 'teh', findingId: 'a' },
    ])
  })

  it('gives an overlap to the earlier finding and ignores spans outside the text', () => {
    expect(
      segments('abcdef', [finding('b', 2, 5), finding('a', 0, 3), finding('z', 4, 99)]),
    ).toEqual([{ text: 'abc', findingId: 'a' }, { text: 'de', findingId: 'b' }, { text: 'f' }])
    expect(segments('', [])).toEqual([])
  })
})

describe('fix labels', () => {
  it('shows the replacement, a removal or an addition', () => {
    expect(suggestionLabel({ kind: 'replace', text: 'the' }, 'teh')).toBe('the')
    expect(suggestionLabel({ kind: 'replace', text: '' }, 'very')).toBe('Remove “very”')
    expect(suggestionLabel({ kind: 'remove', text: '' }, 'the the')).toBe('Remove “the the”')
    expect(suggestionLabel({ kind: 'insert_after', text: ',' }, 'However')).toBe('Add “,”')
  })
})
