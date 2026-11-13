import type { MouseEvent } from 'react'
import type { CommandError, ProvidersSection } from '../ipc'
import type { WayOut } from '../state/popup'

/** After the 402's link (docs/PUBLIK-INTEGRATION.md §9). */
const OR_OWN_KEY = 'or use your own key or a local model in Settings'

/**
 * An error: its message, and at most one way out (docs/PRODUCT.md §5, docs/PUBLIK-INTEGRATION.md
 * §9). A 402 has exactly one link, to the top-up address the error carries; pressing it asks the
 * shell to open the `top_up` link, so the window itself never navigates anywhere.
 */
export default function ErrorCard({
  error,
  way,
  onTryAgain,
  onReconnect,
  onTopUp,
  onCopy,
  onSettings,
}: {
  error: CommandError
  way: WayOut
  onTryAgain: () => void
  onReconnect: () => void
  onTopUp: () => void
  onCopy: () => void
  onSettings: (section?: ProvidersSection) => void
}) {
  const openTopUp = (e: MouseEvent) => {
    e.preventDefault()
    onTopUp()
  }
  return (
    <section className="error-card" role="alert">
      <p className="message">{error.message}</p>
      {way?.kind === 'top-up' && (
        <p>
          <a href={String(error.top_up_url)} onClick={openTopUp} onAuxClick={openTopUp}>
            {way.label}
          </a>
          , {OR_OWN_KEY}.
        </p>
      )}
      {way?.kind === 'try-again' && (
        <button type="button" className="primary" onClick={onTryAgain}>
          Try again
        </button>
      )}
      {way?.kind === 'reconnect' && (
        <button type="button" className="primary" onClick={onReconnect}>
          Reconnect
        </button>
      )}
      {way?.kind === 'copy' && (
        <button type="button" className="primary" onClick={onCopy}>
          Copy
        </button>
      )}
      {way?.kind === 'settings' && (
        <button type="button" onClick={() => onSettings(way.section)}>
          Open Settings
        </button>
      )}
    </section>
  )
}
