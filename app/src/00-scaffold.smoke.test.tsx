import { render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

// The Tauri bridge does not exist under jsdom, so the core module is faked: the test proves the
// root renders and reports ready through the one command module 00 defines.
vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => true,
  invoke: vi.fn(async () => undefined),
}))

import { invoke } from '@tauri-apps/api/core'
import App from './App'

describe('App', () => {
  // Module 00 acceptance test 12: the placeholder app root renders without errors and tells the
  // shell it is ready.
  it('renders the app root and reports ready', async () => {
    render(<App />)
    expect(screen.getByRole('heading', { name: 'Redpen' })).toBeInTheDocument()
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('ui_ready'))
  })
})
