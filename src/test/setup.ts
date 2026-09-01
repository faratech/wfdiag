import '@testing-library/jest-dom/vitest'
import { afterEach } from 'vitest'
import { cleanup } from '@testing-library/react'

// Node 26 exposes an experimental `globalThis.localStorage` accessor that
// resolves to `undefined` unless the process is given --localstorage-file.
// That shadows jsdom's Storage implementation in Vitest, so install the
// minimal browser contract explicitly for the test VM. Production browsers
// are unaffected.
const localStorageValues = new Map<string, string>()
const testLocalStorage: Storage = {
  get length() {
    return localStorageValues.size
  },
  clear() {
    localStorageValues.clear()
  },
  getItem(key) {
    return localStorageValues.get(String(key)) ?? null
  },
  key(index) {
    return [...localStorageValues.keys()][index] ?? null
  },
  removeItem(key) {
    localStorageValues.delete(String(key))
  },
  setItem(key, value) {
    localStorageValues.set(String(key), String(value))
  },
}
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: testLocalStorage,
})

// vitest globals are disabled, so Testing Library's automatic cleanup hook
// never registers itself — without this, DOM accumulates across tests
afterEach(() => cleanup())
