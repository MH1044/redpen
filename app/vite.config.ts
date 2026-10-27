import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

// The front end is built into app/dist, which the Tauri crate embeds (app/src-tauri/tauri.conf.json,
// frontendDist). Build it before any cargo step: scripts/verify does so.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
  },
})
