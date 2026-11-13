import { useEffect, useState } from 'react'
import { consentInfo, type ConsentInfo } from '../ipc'

/** The consent view's footer (docs/PUBLIK-INTEGRATION.md §4). */
const FOOTER = 'You can change this at any time in Settings → Providers.'

/**
 * Choose how Redpen uses AI (docs/PUBLIK-INTEGRATION.md §4): the title, the data-path sentence
 * and the disclosure exactly as `consent_info` returns them, then **Continue with publik API**,
 * **Use my own key** and **Use a local model**. In the popup it has a pending action and **Try
 * again** (for after the own key or local model is set up); in Settings neither. Declining sends
 * nothing.
 */
export default function Consent({
  info,
  onChoose,
  onTryAgain,
  busy = false,
}: {
  info?: ConsentInfo | null
  onChoose: (choice: 'publik' | 'own_key' | 'local') => void
  onTryAgain?: () => void
  busy?: boolean
}) {
  const [loaded, setLoaded] = useState<ConsentInfo | null>(null)

  useEffect(() => {
    if (info) return
    let live = true
    consentInfo()
      .then((i) => {
        if (live && i) setLoaded(i)
      })
      .catch(() => undefined)
    return () => {
      live = false
    }
  }, [info])

  const shown = info ?? loaded
  // The choice is offered only with the strings it is made on.
  if (!shown) return null
  return (
    <section className="consent" aria-label={shown.title}>
      <h2>{shown.title}</h2>
      <p>{shown.data_path}</p>
      <p className="disclosure">{shown.disclosure}</p>
      <div className="buttons">
        <button
          type="button"
          className="primary"
          disabled={busy}
          onClick={() => onChoose('publik')}
        >
          Continue with publik API
        </button>
        <button type="button" onClick={() => onChoose('own_key')}>
          Use my own key
        </button>
        <button type="button" onClick={() => onChoose('local')}>
          Use a local model
        </button>
      </div>
      <p className="hint">{FOOTER}</p>
      {onTryAgain && (
        <div className="buttons">
          <button type="button" onClick={onTryAgain}>
            Try again
          </button>
        </div>
      )}
    </section>
  )
}
