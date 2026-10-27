// Typed wrappers for every Tauri command (docs/ARCHITECTURE.md §5). The interface never holds
// a key and never builds a URL of its own: it sends intents and renders what the core returns.
// Each module adds the wrappers for the commands it implements.

import { invoke, isTauri } from '@tauri-apps/api/core'

/** Every command the shell exposes, in the order of the Rust `generate_handler!` list. */
export const COMMANDS = ['ui_ready'] as const

/** True inside the Tauri web view; false in a browser and under the test runner. */
export function insideTauri(): boolean {
  return isTauri()
}

/** Tells the shell that the interface has rendered. In `--smoke` mode the app then exits 0. */
export function uiReady(): Promise<void> {
  return invoke<void>('ui_ready')
}
