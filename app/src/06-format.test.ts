// Module 06 tests of the money format (docs/modules/06-popup-and-settings-ui.md, Acceptance
// tests, bullet 9). `formatUsd(micros)` in app/src/format.ts: under one cent four decimals,
// otherwise two, 0 as `$0.00`, the same rule as the Rust `balance_line`.
//
// Expected values: the bullet's own five, then exact amounts that are true by construction
// (tests/cases/06-popup-and-settings-ui/REFERENCE.md, Money). None sits halfway between two
// results, so no rounding rule beyond the bullet's is assumed.

import { describe, expect, it } from 'vitest'
import { formatUsd } from './format'

describe('formatUsd', () => {
  // Bullet 9: 0 → $0.00; 210 → $0.0002; 2,100 → $0.0021; 9,999 → $0.0100; 3,120,000 → $3.12.
  it.each([
    [0, '$0.00'],
    [210, '$0.0002'],
    [2_100, '$0.0021'],
    [9_999, '$0.0100'],
    [3_120_000, '$3.12'],
  ])('t09: %i micro-dollars read %s', (micros, expected) => {
    expect(formatUsd(micros)).toBe(expected)
  })

  // Under one cent, four decimals: the estimates of docs/PUBLIK-INTEGRATION.md §8.
  it.each([
    [100, '$0.0001'],
    [1_900, '$0.0019'],
    [2_400, '$0.0024'],
    [3_800, '$0.0038'],
  ])('s: %i micro-dollars, under one cent, read %s', (micros, expected) => {
    expect(formatUsd(micros)).toBe(expected)
  })

  // From one cent up, two decimals: one cent, a full plagiarism check, "This month: $0.41", and
  // a whole number of dollars.
  it.each([
    [10_000, '$0.01'],
    [80_000, '$0.08'],
    [410_000, '$0.41'],
    [12_000_000, '$12.00'],
  ])('s: %i micro-dollars, one cent or more, read %s', (micros, expected) => {
    expect(formatUsd(micros)).toBe(expected)
  })

  // The same rule as `balance_line`, which groups thousands (docs/modules/03-publik.md, Design
  // notes, Money: `$1,250.00`).
  it('s: thousands are grouped as in the balance line', () => {
    expect(formatUsd(1_250_000_000)).toBe('$1,250.00')
  })
})
