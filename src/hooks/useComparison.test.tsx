import { describe, it, expect, vi, beforeEach } from 'vitest'
import { act, renderHook, waitFor } from '@testing-library/react'
import { useComparison, type ComparisonResult } from './useComparison'

const invokeMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

function scan(id: string): ComparisonResult['current_scan'] {
  return {
    id,
    timestamp: '2026-06-12T10:00:00Z',
    computer_name: 'PC-A',
    task_count: 2,
    success_count: 1,
    failure_count: 1,
    duration_ms: 1200,
    tags: [],
  }
}

function comparisonResult(previousId: string, totalChanges = 1): ComparisonResult {
  return {
    current_scan: scan('current'),
    previous_scan: scan(previousId),
    total_changes: totalChanges,
    new_failures: [],
    new_successes: [],
    status_unchanged: [],
  }
}

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
})

describe('useComparison request ordering', () => {
  it('keeps the newest comparison when an earlier request resolves last', async () => {
    const first = deferred<ComparisonResult>()
    const second = deferred<ComparisonResult>()
    invokeMock
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)

    const { result } = renderHook(() => useComparison())

    let firstRun!: Promise<void>
    act(() => {
      firstRun = result.current.compareScans('current', 'scan-old')
    })
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(1))

    let secondRun!: Promise<void>
    act(() => {
      secondRun = result.current.compareScans('current', 'scan-new')
    })
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(2))

    await act(async () => {
      second.resolve(comparisonResult('scan-new', 2))
      await secondRun
    })

    expect(result.current.comparison?.previous_scan.id).toBe('scan-new')
    expect(result.current.loading).toBe(false)
    expect(result.current.error).toBeNull()

    await act(async () => {
      first.resolve(comparisonResult('scan-old', 1))
      await firstRun
    })

    expect(result.current.comparison?.previous_scan.id).toBe('scan-new')
    expect(result.current.loading).toBe(false)
    expect(result.current.error).toBeNull()
  })

  it('ignores stale comparison errors after a newer request succeeds', async () => {
    const first = deferred<ComparisonResult>()
    const second = deferred<ComparisonResult>()
    invokeMock
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)

    const { result } = renderHook(() => useComparison())

    let firstRun!: Promise<void>
    let secondRun!: Promise<void>
    act(() => {
      firstRun = result.current.compareScans('current', 'scan-old')
    })
    act(() => {
      secondRun = result.current.compareScans('current', 'scan-new')
    })

    await act(async () => {
      second.resolve(comparisonResult('scan-new', 2))
      await secondRun
    })

    await act(async () => {
      first.reject(new Error('old comparison failed'))
      await firstRun
    })

    expect(result.current.comparison?.previous_scan.id).toBe('scan-new')
    expect(result.current.error).toBeNull()
  })

  it('clearComparison invalidates an in-flight comparison', async () => {
    const pending = deferred<ComparisonResult>()
    invokeMock.mockReturnValueOnce(pending.promise)

    const { result } = renderHook(() => useComparison())

    let run!: Promise<void>
    act(() => {
      run = result.current.compareScans('current', 'scan-old')
    })
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(1))

    act(() => {
      result.current.clearComparison()
    })

    expect(result.current.loading).toBe(false)
    expect(result.current.comparison).toBeNull()

    await act(async () => {
      pending.resolve(comparisonResult('scan-old'))
      await run
    })

    expect(result.current.comparison).toBeNull()
    expect(result.current.error).toBeNull()
  })
})
