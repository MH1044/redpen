import { useEffect, useState } from 'react'
import { settingsGet, welcomeDone } from '../ipc'
import { DEFAULT_HOTKEY } from '../state/settings'

/**
 * The welcome window, on the first launch only (docs/PRODUCT.md §5): the privacy model in three
 * sentences and the check shortcut, then Close. It asks for no choice and sends nothing; the
 * consent view waits for the first AI action.
 */
export default function Welcome() {
  const [hotkey, setHotkey] = useState(DEFAULT_HOTKEY)

  useEffect(() => {
    let live = true
    settingsGet()
      .then((s) => {
        if (live && s?.hotkeys?.check) setHotkey(s.hotkeys.check)
      })
      .catch(() => undefined)
    return () => {
      live = false
    }
  }, [])

  return (
    <main className="welcome">
      <h1>Welcome to Redpen</h1>
      <p>
        Redpen checks your writing on this computer. Nothing you type leaves it until you ask for a
        rewrite, or for a check that needs the internet. Password fields and apps you block are
        never read.
      </p>
      <p>
        Select text in any app and press <kbd className="hotkey">{hotkey}</kbd>.
      </p>
      <div className="buttons">
        <button
          type="button"
          className="primary"
          onClick={() => void welcomeDone().catch(() => undefined)}
        >
          Close
        </button>
      </div>
    </main>
  )
}
