// Unit tests of the helpers beside `formatUsd` (whose own cases are in 06-format.test.ts).

import { describe, expect, it } from 'vitest'
import { formatUsd, groupThousands, usageLine } from './format'

describe('groupThousands', () => {
  it('puts a comma every three digits', () => {
    expect(groupThousands(0)).toBe('0')
    expect(groupThousands(999)).toBe('999')
    expect(groupThousands(1000)).toBe('1,000')
    expect(groupThousands(1_234_567)).toBe('1,234,567')
  })
})

describe('formatUsd at its edges', () => {
  it('never shows a negative or broken amount', () => {
    expect(formatUsd(-5)).toBe('$0.00')
    expect(formatUsd(Number.NaN)).toBe('$0.00')
  })

  it('rounds to the place it shows', () => {
    expect(formatUsd(49)).toBe('$0.0000')
    expect(formatUsd(50)).toBe('$0.0001')
    expect(formatUsd(14_999)).toBe('$0.01')
    expect(formatUsd(15_000)).toBe('$0.02')
  })
})

describe('usageLine', () => {
  it('is this month’s total and the number of actions (docs/PUBLIK-INTEGRATION.md §9)', () => {
    expect(usageLine(410_000, 312)).toBe('This month: $0.41 across 312 actions')
    expect(usageLine(210, 1)).toBe('This month: $0.0002 across 1 action')
    expect(usageLine(0, 0)).toBe('This month: $0.00 across 0 actions')
    expect(usageLine(12_340_000, 4_521)).toBe('This month: $12.34 across 4,521 actions')
  })
})
