import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'

// Kept separate from vite.config.ts: the dev-server fs.deny/CSP settings there
// are irrelevant for tests, and the build timestamp must be stable under test.
export default defineConfig({
  plugins: [react()],
  define: { __BUILD_TIME__: JSON.stringify('test') },
  test: {
    environment: 'jsdom',
    setupFiles: ['src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
  },
})
