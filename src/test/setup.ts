import '@testing-library/jest-dom/vitest'
import { afterEach } from 'vitest'
import { cleanup } from '@testing-library/react'

// vitest globals are disabled, so Testing Library's automatic cleanup hook
// never registers itself — without this, DOM accumulates across tests
afterEach(() => cleanup())
