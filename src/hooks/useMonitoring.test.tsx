import { describe, it, expect, vi, beforeEach } from 'vitest'
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
