import { act, renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useProcessExplorer } from './useProcessExplorer'

const invokeMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

const query = {
  search: 'edge',
  sortBy: 'cpu_percent' as const,
  sortDirection: 'desc' as const,
  offset: 0,
  limit: 100,
}

const processPage = {
  captured_at: 1,
  total: 1,
  offset: 0,
  limit: 100,
  items: [{
    pid: 42,
    parent_pid: 1,
    name: 'msedge.exe',
    cpu_percent: 12,
    memory_percent: 3,
    memory_mb: 256,
    virtual_memory_mb: 512,
    gpu_percent: null,
    gpu_memory_mb: null,
    npu_percent: null,
    npu_memory_mb: null,
    cpu_time_secs: 4,
    start_time: 1,
    status: 'Running',
    thread_count: 8,
    handle_count: 50,
    priority: 8,
    io_read_bytes: 0,
    io_write_bytes: 0,
  }],
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>(res => { resolve = res })
  return { promise, resolve }
}

beforeEach(() => {
  vi.clearAllMocks()
  invokeMock.mockResolvedValue(processPage)
})

afterEach(() => vi.useRealTimers())

describe('useProcessExplorer', () => {
  it('loads a server-filtered process page using the public query contract', async () => {
    const { result } = renderHook(() => useProcessExplorer(query))

    await waitFor(() => expect(result.current.page?.total).toBe(1))
    expect(invokeMock).toHaveBeenCalledWith('list_processes', {
      query: {
        search: 'edge',
        sort_by: 'cpu_percent',
        sort_direction: 'desc',
        offset: 0,
        limit: 100,
      },
    })
    expect(result.current.page?.items[0].name).toBe('msedge.exe')
    expect(result.current.error).toBeNull()
  })

  it('exposes backend failures as retryable errors instead of an empty list', async () => {
    invokeMock.mockRejectedValueOnce('process query failed')
    const { result } = renderHook(() => useProcessExplorer(query))

    await waitFor(() => expect(result.current.error).toContain('process query failed'))
    await act(async () => { await result.current.refresh() })
    await waitFor(() => expect(result.current.page?.total).toBe(1))
    expect(result.current.error).toBeNull()
  })

  it('coalesces polling ticks while a slow enumeration is in flight', async () => {
    vi.useFakeTimers()
    const first = deferred<typeof processPage>()
    invokeMock
      .mockReset()
      .mockReturnValueOnce(first.promise)
      .mockResolvedValue(processPage)

    renderHook(() => useProcessExplorer(query))
    await act(async () => { await vi.advanceTimersByTimeAsync(180) })
    expect(invokeMock).toHaveBeenCalledTimes(1)

    await act(async () => { await vi.advanceTimersByTimeAsync(6_000) })
    expect(invokeMock).toHaveBeenCalledTimes(1)

    await act(async () => {
      first.resolve(processPage)
      await first.promise
      await Promise.resolve()
    })
    expect(invokeMock).toHaveBeenCalledTimes(2)
  })
})
