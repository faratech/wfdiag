import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { renderHook, act, waitFor } from '@testing-library/react'
import { useMonitoring } from './useMonitoring'

const invokeMock = vi.fn()
const listenMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

beforeEach(() => {
  vi.clearAllMocks()
  listenMock.mockResolvedValue(vi.fn())
})

describe('useMonitoring lease ownership', () => {
  it('auto-starts after React StrictMode replays mount effects', async () => {
    let nextLease = 0
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'start_monitoring') return Promise.resolve(++nextLease)
      if (cmd === 'stop_monitoring') return Promise.resolve()
      return Promise.resolve(null)
    })
    const { result } = renderHook(
      () => useMonitoring({ autoStart: true, componentName: 'strict-test' }),
      { reactStrictMode: true }
    )

    await waitFor(() => expect(result.current.isActive).toBe(true))
    expect(invokeMock.mock.calls.filter(call => call[0] === 'start_monitoring')).toHaveLength(1)
  })

  it('prevents duplicate starts while the first start is still in flight', async () => {
    const start = deferred<number>()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'start_monitoring') return start.promise
      if (cmd === 'stop_monitoring') return Promise.resolve()
      return Promise.resolve(null)
    })

    const { result } = renderHook(() => useMonitoring({ componentName: 'test' }))

    let first!: Promise<void>
    let second!: Promise<void>
    act(() => {
      first = result.current.start()
      second = result.current.start()
    })

    await waitFor(() => {
      expect(invokeMock.mock.calls.filter(c => c[0] === 'start_monitoring')).toHaveLength(1)
    })

    await act(async () => {
      start.resolve(1)
      await Promise.all([first, second])
    })
  })

  it('does not let a stale start completion clear a newer in-flight start', async () => {
    const firstStart = deferred<number>()
    const secondStart = deferred<number>()
    const starts = [firstStart, secondStart]
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'start_monitoring') return starts.shift()!.promise
      if (cmd === 'stop_monitoring') return Promise.resolve()
      return Promise.resolve(null)
    })

    const { result } = renderHook(() => useMonitoring({ componentName: 'test' }))

    let first!: Promise<void>
    act(() => {
      first = result.current.start()
    })
    await waitFor(() => {
      expect(invokeMock.mock.calls.filter(c => c[0] === 'start_monitoring')).toHaveLength(1)
    })

    await act(async () => {
      await result.current.stop()
    })

    let second!: Promise<void>
    let third!: Promise<void>
    act(() => {
      second = result.current.start()
    })
    await waitFor(() => {
      expect(invokeMock.mock.calls.filter(c => c[0] === 'start_monitoring')).toHaveLength(2)
    })

    await act(async () => {
      firstStart.resolve(1)
      await first
    })
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('stop_monitoring', { leaseId: 1 })
    })

    act(() => {
      third = result.current.start()
    })
    expect(invokeMock.mock.calls.filter(c => c[0] === 'start_monitoring')).toHaveLength(2)

    await act(async () => {
      secondStart.resolve(2)
      await Promise.all([second, third])
    })
  })

  it('does not reactivate after stop while listener registration is pending', async () => {
    const listener = deferred<() => void>()
    const unlisten = vi.fn()
    listenMock.mockReturnValue(listener.promise)
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'start_monitoring') return Promise.resolve(7)
      if (cmd === 'stop_monitoring') return Promise.resolve()
      return Promise.resolve(null)
    })

    const { result } = renderHook(() => useMonitoring({ componentName: 'test' }))

    let starting!: Promise<void>
    act(() => {
      starting = result.current.start()
    })
    await waitFor(() => expect(listenMock).toHaveBeenCalledWith('system-stats', expect.any(Function)))

    await act(async () => {
      await result.current.stop()
      listener.resolve(unlisten)
      await starting
    })

    expect(unlisten).toHaveBeenCalledOnce()
    expect(result.current.isActive).toBe(false)
    expect(invokeMock).toHaveBeenCalledWith('stop_monitoring', { leaseId: 7 })
  })

  it('never sends an unscoped stop when stop calls overlap', async () => {
    const stopping = deferred<void>()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'start_monitoring') return Promise.resolve(9)
      if (cmd === 'stop_monitoring') return stopping.promise
      return Promise.resolve(null)
    })

    const { result } = renderHook(() => useMonitoring({ componentName: 'test' }))
    await act(async () => { await result.current.start() })

    let first!: Promise<void>
    let second!: Promise<void>
    act(() => {
      first = result.current.stop()
      second = result.current.stop()
    })

    expect(invokeMock.mock.calls.filter(call => call[0] === 'stop_monitoring')).toEqual([
      ['stop_monitoring', { leaseId: 9 }],
    ])

    await act(async () => {
      stopping.resolve()
      await Promise.all([first, second])
    })
  })

  it('stops only the lease owned by the unmounted hook instance', async () => {
    let nextLease = 0
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'start_monitoring') return Promise.resolve(++nextLease)
      if (cmd === 'stop_monitoring') return Promise.resolve()
      return Promise.resolve(null)
    })

    const first = renderHook(() => useMonitoring({ componentName: 'first' }))
    const second = renderHook(() => useMonitoring({ componentName: 'second' }))

    await act(async () => {
      await first.result.current.start()
      await second.result.current.start()
    })

    first.unmount()

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('stop_monitoring', { leaseId: 1 })
    })
    expect(invokeMock).not.toHaveBeenCalledWith('stop_monitoring', { leaseId: 2 })
    expect(second.result.current.isActive).toBe(true)
  })
})

describe('useMonitoring visibility handling', () => {
  afterEach(() => {
    Object.defineProperty(document, 'hidden', { value: false, configurable: true })
  })

  it('stops monitoring when the window/app is hidden', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'start_monitoring') return Promise.resolve(1)
      if (cmd === 'stop_monitoring') return Promise.resolve()
      return Promise.resolve(null)
    })

    const { result } = renderHook(() => useMonitoring({ componentName: 'test' }))

    await act(async () => {
      await result.current.start()
    })
    expect(result.current.isActive).toBe(true)

    Object.defineProperty(document, 'hidden', { value: true, configurable: true })
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'))
    })

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('stop_monitoring', { leaseId: 1 })
    })
    expect(result.current.isActive).toBe(false)
  })

  it('does nothing on hide when monitoring was never started', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'stop_monitoring') return Promise.resolve()
      return Promise.resolve(null)
    })

    renderHook(() => useMonitoring({ componentName: 'test' }))

    Object.defineProperty(document, 'hidden', { value: true, configurable: true })
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'))
    })

    expect(invokeMock.mock.calls.some(c => c[0] === 'stop_monitoring')).toBe(false)
  })
})
