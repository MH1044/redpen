// What the popup's views show, as pure functions of what the core returned, so the wording is
// tested without rendering.

import { formatUsd } from '../format'
import type { Action, Finding, ProviderStatus, RewriteOutcome, Suggestion, Warning } from '../ipc'

/** The loading view's line: "Rewriting… (publik-balanced)", or "Rewriting…" with no provider. */
export function loadingLine(providerLine: string): string {
  const line = providerLine.trim()
  return line ? `Rewriting… (${line})` : 'Rewriting…'
}

/**
 * The provider an action goes to, from the chosen provider, for a session that came without its
 * own lines: the tier on publik API (Fix grammar on `publik-fast`, the others on
 * `publik-balanced`), "Your own key" or "Local". The core's lines, when the session has them, also
 * name the model and Rewrite quality's tier.
 */
export function providerLineFor(chat: ProviderStatus['chat'], action: Action): string {
  switch (chat) {
    case 'publik':
      return action === 'fix-grammar' ? 'publik-fast' : 'publik-balanced'
    case 'own_key':
      return 'Your own key'
    case 'local':
      return 'Local'
    default:
      return ''
  }
}

/** Whether a result came from publik API: the outcome says so, else the chosen provider does. */
export function fromPublik(outcome: RewriteOutcome, chat: ProviderStatus['chat']): boolean {
  return outcome.provider !== undefined ? outcome.provider === 'publik' : chat === 'publik'
}

/**
 * The footer after an AI action (the Scope): `<tier> · <cost> · <balance line>` on publik API,
 * where an answer from the cache cost $0.00; the provider line alone otherwise.
 */
export function footerLine(
  outcome: RewriteOutcome,
  publik: boolean,
  balanceLine: string | null,
): string {
  if (!publik) return outcome.provider_line
  const parts = [outcome.provider_line, formatUsd(outcome.charge_micros ?? 0)]
  if (balanceLine) parts.push(balanceLine)
  return parts.join(' · ')
}

/** A warning's message, whether the core sent the object or the message alone. */
export function warningText(w: Warning): string {
  return typeof w === 'string' ? w : w.message
}

/** "No changes needed." for an answer identical to the text (module 04). */
export const NO_CHANGES = 'No changes needed.'

/** The check view with no findings. */
export const NO_ISSUES = 'No issues found.'

/** After Undo, when the window could not take the text back. */
export const UNDO_ON_CLIPBOARD =
  "The window you started in couldn't take the text back, so your original text is on the clipboard. Press Ctrl+V to paste it."

/** After Copy. */
export const COPIED = 'Copied.'

/** The finding categories as the list names them. */
export const CATEGORY_LABELS: Record<Finding['category'], string> = {
  spelling: 'Spelling',
  grammar: 'Grammar',
  punctuation: 'Punctuation',
  style: 'Style',
}

/** The text a finding covers, by its UTF-16 span (what JavaScript strings count). */
export function spanText(text: string, f: Finding): string {
  const [start, end] = f.span.utf16
  return text.slice(start, end)
}

/** A suggestion's button label: the replacement, "Remove “x”", or "Add “x”". */
export function suggestionLabel(s: Suggestion, covered: string): string {
  switch (s.kind) {
    case 'replace':
      return s.text === '' ? `Remove “${covered}”` : s.text
    case 'remove':
      return `Remove “${covered}”`
    case 'insert_after':
      return `Add “${s.text}”`
  }
}

/** One run of the text: underlined when a finding covers it. */
export interface Segment {
  text: string
  findingId?: string
}

/**
 * The text cut into runs at the findings' UTF-16 spans, in order. Where findings overlap, the
 * earlier one keeps the overlap; spans outside the text are ignored.
 */
export function segments(text: string, findings: Finding[]): Segment[] {
  const spans = findings
    .map((f) => ({ id: f.id, start: f.span.utf16[0], end: f.span.utf16[1] }))
    .filter((s) => s.start >= 0 && s.end <= text.length && s.start < s.end)
    .sort((a, b) => a.start - b.start || a.end - b.end)
  const out: Segment[] = []
  let at = 0
  for (const s of spans) {
    const start = Math.max(s.start, at)
    if (start >= s.end) continue
    if (start > at) out.push({ text: text.slice(at, start) })
    out.push({ text: text.slice(start, s.end), findingId: s.id })
    at = s.end
  }
  if (at < text.length) out.push({ text: text.slice(at) })
  return out
}
