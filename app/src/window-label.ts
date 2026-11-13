/**
 * This window's label (`popup`, `settings`, `welcome`), or `null` outside the shell. The window
 * module is loaded on demand, so a page outside the shell (a browser, a test) never evaluates it.
 */
export async function windowLabel(): Promise<string | null> {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    return getCurrentWindow().label
  } catch {
    return null
  }
}
