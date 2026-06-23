import { describe, it, expect } from 'vitest'
import { shouldCheck, CHECK_INTERVAL_MS } from './useUpdateCheck'

describe('update check throttle', () => {
  const now = 1_750_000_000_000

  it('checks when never run before', () => {
    expect(shouldCheck(null, now)).toBe(true)
  })

  it('skips inside the 24h window', () => {
    expect(shouldCheck(String(now - 60_000), now)).toBe(false)
    expect(shouldCheck(String(now - CHECK_INTERVAL_MS + 1), now)).toBe(false)
  })

  it('checks once the window has elapsed', () => {
    expect(shouldCheck(String(now - CHECK_INTERVAL_MS), now)).toBe(true)
    expect(shouldCheck(String(now - 2 * CHECK_INTERVAL_MS), now)).toBe(true)
  })

  it('treats corrupted storage as never-run', () => {
    expect(shouldCheck('not-a-number', now)).toBe(true)
  })

  it('treats future timestamps as corrupt', () => {
    expect(shouldCheck(String(now + 60_000), now)).toBe(false)
    expect(shouldCheck(String(now + 10 * 60_000), now)).toBe(true)
  })
})
