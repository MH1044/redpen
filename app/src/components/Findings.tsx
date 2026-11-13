import type { Session } from '../ipc'
import {
  CATEGORY_LABELS,
  NO_ISSUES,
  segments,
  spanText,
  suggestionLabel,
} from '../state/popup-view'

/**
 * The captured text, read-only, with Harper's findings underlined, then the findings with their
 * fixes. Without `onFix` the fixes are not offered (a result is on screen).
 */
export default function Findings({
  session,
  onFix,
  showText = true,
}: {
  session: Session
  onFix?: (findingId: string, suggestionIndex: number) => void
  showText?: boolean
}) {
  return (
    <section className="findings" aria-label="Findings">
      {showText && (
        <p className="captured" tabIndex={-1}>
          {segments(session.text, session.findings).map((s, i) =>
            s.findingId ? (
              <mark key={i} className="finding-span">
                {s.text}
              </mark>
            ) : (
              <span key={i}>{s.text}</span>
            ),
          )}
        </p>
      )}
      {session.findings.length === 0 ? (
        <p className="no-issues">{NO_ISSUES}</p>
      ) : (
        <ul className="finding-list">
          {session.findings.map((f) => {
            const covered = spanText(session.text, f)
            return (
              <li key={f.id}>
                <span className={`category category-${f.category}`}>
                  {CATEGORY_LABELS[f.category] ?? f.category}
                </span>{' '}
                <span className="message">{f.message}</span>
                {onFix && f.suggestions.length > 0 && (
                  <span className="fixes">
                    {f.suggestions.map((s, i) => (
                      <button key={i} type="button" className="fix" onClick={() => onFix(f.id, i)}>
                        {suggestionLabel(s, covered)}
                      </button>
                    ))}
                  </span>
                )}
              </li>
            )
          })}
        </ul>
      )}
    </section>
  )
}
