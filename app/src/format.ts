// Money as the interface shows it (docs/modules/06-popup-and-settings-ui.md): dollars, never
// anything else (docs/PUBLIK-CONTRACT.md §9).

/** Micro-dollars in a dollar. */
const DOLLAR = 1_000_000
/** Micro-dollars in a cent. */
const CENT = 10_000
/** Micro-dollars in a ten-thousandth of a dollar, the last place shown below one cent. */
const BASIS = 100

/**
 * An amount of micro-dollars in dollars, with the thresholds of the core's `balance_line`: under
 * one cent four decimals (`$0.0021`), otherwise two (`$3.12`, `$1,250.00`), and 0 as `$0.00`.
 * Rounded to the nearest shown place (half up), so 9,999 reads `$0.0100`; the balance itself is
 * always the core's line, which rounds down so it is never overstated. Integer arithmetic only.
 */
export function formatUsd(micros: number): string {
  const m = Number.isFinite(micros) ? Math.max(0, Math.trunc(micros)) : 0
  if (m === 0) return '$0.00'
  if (m < CENT) {
    const units = Math.floor((m + BASIS / 2) / BASIS)
    return `$${Math.floor(units / 10_000)}.${String(units % 10_000).padStart(4, '0')}`
  }
  const cents = Math.floor((m + CENT / 2) / CENT)
  const dollars = Math.floor(cents / (DOLLAR / CENT))
  return `$${groupThousands(dollars)}.${String(cents % 100).padStart(2, '0')}`
}

/** `1250` as `1,250`. */
export function groupThousands(n: number): string {
  const digits = String(Math.max(0, Math.trunc(n)))
  let out = ''
  for (let i = 0; i < digits.length; i += 1) {
    if (i > 0 && (digits.length - i) % 3 === 0) out += ','
    out += digits[i]
  }
  return out
}

/** "This month: $0.41 across 312 actions", from `usage_month` (docs/PUBLIK-INTEGRATION.md §9). */
export function usageLine(totalMicros: number, calls: number): string {
  const n = Math.max(0, Math.trunc(calls))
  return `This month: ${formatUsd(totalMicros)} across ${groupThousands(n)} ${n === 1 ? 'action' : 'actions'}`
}
