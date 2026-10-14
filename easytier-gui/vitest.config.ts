import { defineConfig } from 'vitest/config'

// Minimal standalone config: unit tests for pure GUI logic only; the app's
// vite config (vue plugins, tailwind, aliases) is intentionally not loaded.
export default defineConfig({
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node',
  },
})
