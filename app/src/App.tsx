import { useEffect } from 'react'
import { insideTauri, uiReady } from './ipc'

// The placeholder app root. The popup, Settings, welcome and consent windows arrive with
// module 06; until then the one hidden window only tells the shell that the interface loaded.
export default function App() {
  useEffect(() => {
    if (insideTauri()) {
      void uiReady()
    }
  }, [])

  return (
    <main>
      <h1>Redpen</h1>
      <p>Redpen runs in the tray. Select text in any app and press the check shortcut.</p>
    </main>
  )
}
