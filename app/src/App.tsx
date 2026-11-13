import { useEffect, useState } from 'react'
import { insideTauri, uiReady } from './ipc'
import { windowLabel } from './window-label'
import Popup from './windows/Popup'
import Settings from './windows/Settings'
import Welcome from './windows/Welcome'

/**
 * The app root. Every window loads the same page and shows the screen its label names (the
 * shell's `popup`, `settings` and `welcome` windows); outside the shell it shows a short
 * placeholder. It tells the shell once it has rendered, which ends a `--smoke` run.
 */
export default function App() {
  // `undefined` until the label is known, `null` outside the shell.
  const [label, setLabel] = useState<string | null | undefined>(undefined)

  useEffect(() => {
    let live = true
    void windowLabel().then((l) => {
      if (live) setLabel(l)
    })
    if (insideTauri()) {
      void uiReady().catch(() => undefined)
    }
    return () => {
      live = false
    }
  }, [])

  switch (label) {
    case 'popup':
      return <Popup />
    case 'settings':
      return <Settings />
    case 'welcome':
      return <Welcome />
    default:
      return (
        <main>
          <h1>Redpen</h1>
          {label === null && (
            <p>Redpen runs in the tray. Select text in any app and press the check shortcut.</p>
          )}
        </main>
      )
  }
}
