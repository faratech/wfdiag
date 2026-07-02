import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, act } from '@testing-library/react'
import { Titlebar } from './Titlebar'

const isMaximizedMock = vi.fn()
const onResizedMock = vi.fn()

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    isMaximized: isMaximizedMock,
    onResized: onResizedMock,
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    close: vi.fn(),
  }),
}))

function deferred<T>() {
  let resolve!: (v: T) => void
  const promise = new Promise<T>(res => { resolve = res })
  return { promise, resolve }
}

beforeEach(() => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  isMaximizedMock.mockResolvedValue(false)
})

afterEach(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
})

describe('Titlebar onResized listener cleanup', () => {
  it('tears down the resize listener even if unmount happens before onResized() resolves', async () => {
    const resizePending = deferred<() => void>()
    const unlisten = vi.fn()
    onResizedMock.mockReturnValue(resizePending.promise)

    const { unmount } = render(<Titlebar />)
    // Unmount before onResized() has a chance to resolve, simulating React
    // StrictMode's dev double-invoke (or a real fast mount/unmount cycle).
    unmount()

    await act(async () => {
      resizePending.resolve(unlisten)
      await resizePending.promise
    })

    expect(unlisten).toHaveBeenCalledTimes(1)
  })

  it('registers the resize listener normally when it resolves before unmount', async () => {
    const unlisten = vi.fn()
    onResizedMock.mockResolvedValue(unlisten)

    const { unmount } = render(<Titlebar />)

    await act(async () => {
      await Promise.resolve()
    })

    unmount()
    expect(unlisten).toHaveBeenCalledTimes(1)
  })
})
