import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { renderHook, act, waitFor } from '@testing-library/react'

// The module-level scan lock in useScanner is shared across hook instances,
// so each test reloads the module via vi.resetModules() + dynamic import to
// start from a clean lock.

const invokeMock = vi.fn()
const listenMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}))

interface MockContext {
  settings: { autoSave: boolean; maxConcurrentTasks: number; quickScanTasks?: string[] }
  [key: string]: unknown
}

let contextValue: MockContext

vi.mock('../contexts/AppContext', () => ({
  useAppContext: () => contextValue,
}))

function makeContext(overrides: Partial<MockContext> = {}): MockContext {
  return {
    availableTasks: [{ id: 'os_info', name: 'OS Info', admin_required: false }],
    systemInfo: { is_admin: true },
    sessionId: null,
    setSessionId: vi.fn(),
    results: {},
    setResults: vi.fn(),
    isRunning: false,
    setIsRunning: vi.fn(),
    setCurrentProgress: vi.fn(),
    setCurrentTaskName: vi.fn(),
    setDiagnosticsError: vi.fn(),
    setScanStartTime: vi.fn(),
    setScanEndTime: vi.fn(),
    setTaskStatuses: vi.fn(),
    setIssues: vi.fn(),
    settings: { autoSave: false, maxConcurrentTasks: 2 },
    searchQuery: '',
    setFilteredResults: vi.fn(),
    ...overrides,
  }
}

function deferred<T>() {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

async function loadHook() {
  vi.resetModules()
  const mod = await import('./useScanner')
  return mod.useScanner
}

function startCalls() {
  return invokeMock.mock.calls.filter(c => c[0] === 'start_diagnostics')
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.useRealTimers()
  contextValue = makeContext()
  listenMock.mockResolvedValue(vi.fn())
})

afterEach(() => {
  vi.useRealTimers()
})

describe('useScanner scan lock', () => {
  it('ignores a second scan started while one is running', async () => {
    const parallel = deferred<Array<[string, unknown]>>()
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-1'
      if (cmd === 'run_diagnostics_parallel') return parallel.promise
      return undefined
    })

    const useScanner = await loadHook()
    const { result } = renderHook(() => useScanner())

    let p1!: Promise<void>
    let p2!: Promise<void>
    act(() => {
      p1 = result.current.runQuickScan()
      p2 = result.current.runQuickScan()
    })

    parallel.resolve([])
    await act(async () => {
      await Promise.all([p1, p2])
    })

    expect(startCalls()).toHaveLength(1)
  })

  it('releases the lock after a completed scan', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-1'
      if (cmd === 'run_diagnostics_parallel') return []
      return undefined
    })

    const useScanner = await loadHook()
    const { result } = renderHook(() => useScanner())

    await act(async () => {
      await result.current.runQuickScan()
    })
    await act(async () => {
      await result.current.runQuickScan()
    })

    expect(startCalls()).toHaveLength(2)
  })

  it('releases the lock when the scan fails', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') throw new Error('backend down')
      return undefined
    })

    const useScanner = await loadHook()
    const { result } = renderHook(() => useScanner())

    await act(async () => {
      await result.current.runQuickScan()
    })

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-2'
      if (cmd === 'run_diagnostics_parallel') return []
      return undefined
    })

    await act(async () => {
      await result.current.runQuickScan()
    })

    expect(startCalls()).toHaveLength(2)
  })

  it('allows a new scan after stopScan (deadlock regression)', async () => {
    const parallel = deferred<Array<[string, unknown]>>()
    const setIsRunning = vi.fn()
    contextValue = makeContext({ setIsRunning })
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-1'
      if (cmd === 'run_diagnostics_parallel') return parallel.promise
      return undefined
    })

    const useScanner = await loadHook()
    const { result } = renderHook(() => useScanner())

    let p1!: Promise<void>
    act(() => {
      p1 = result.current.runQuickScan()
    })

    // Wait for the scan to reach the parallel-run stage before stopping
    await waitFor(() => {
      expect(invokeMock.mock.calls.some(c => c[0] === 'run_diagnostics_parallel')).toBe(true)
    })

    act(() => {
      result.current.stopScan()
    })

    // stopScan requests backend cancellation for the active session
    expect(invokeMock).toHaveBeenCalledWith('cancel_diagnostics', { sessionId: 'session-1' })
    // UI should stay in the running/stopping state until the backend invoke drains
    expect(setIsRunning).not.toHaveBeenCalledWith(false)

    // Backend resolves (quickly, since queued tasks are skipped); the scan's
    // finally must release the lock even though the session was abandoned
    parallel.resolve([])
    await act(async () => {
      await p1
    })

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-2'
      if (cmd === 'run_diagnostics_parallel') return []
      return undefined
    })

    await act(async () => {
      await result.current.runQuickScan()
    })

    expect(startCalls()).toHaveLength(2)
  })

  it('still commits results and clears isRunning after the owning instance unmounts mid-scan', async () => {
    const parallel = deferred<Array<[string, unknown]>>()
    const setIsRunning = vi.fn()
    const setResults = vi.fn()
    contextValue = makeContext({ setIsRunning, setResults })
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-1'
      if (cmd === 'run_diagnostics_parallel') return parallel.promise
      return undefined
    })

    const useScanner = await loadHook()
    const { result, unmount } = renderHook(() => useScanner())

    let scan!: Promise<void>
    act(() => {
      scan = result.current.runQuickScan()
    })

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(c => c[0] === 'run_diagnostics_parallel')).toBe(true)
    })

    // Simulate a tab switch: the component that started the scan unmounts,
    // but the scan itself (a plain promise chain, not tied to React) keeps
    // running in the background exactly like the real backend call does.
    unmount()

    parallel.resolve([['os_info', { success: true, output: '{}', error: null, duration_ms: 1 }]])
    await act(async () => {
      await scan
    })

    // Results must still land in AppContext, and isRunning must still be
    // cleared — neither should be silently dropped just because the
    // initiating component is gone.
    expect(setResults).toHaveBeenCalledWith({
      os_info: { success: true, output: '{}', error: null, duration_ms: 1 },
    })
    expect(setIsRunning).toHaveBeenCalledWith(false)
  })

  it('normalizes maxConcurrentTasks 0 before invoking the backend', async () => {
    contextValue = makeContext({ settings: { autoSave: false, maxConcurrentTasks: 0 } })
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-1'
      if (cmd === 'run_diagnostics_parallel') return []
      return undefined
    })

    const useScanner = await loadHook()
    const { result } = renderHook(() => useScanner())

    await act(async () => {
      await result.current.runQuickScan()
    })

    expect(invokeMock).toHaveBeenCalledWith('run_diagnostics_parallel', expect.objectContaining({
      maxConcurrent: 5,
    }))
  })

  it('lets any hook instance cancel the active scan', async () => {
    const parallel = deferred<Array<[string, unknown]>>()
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-1'
      if (cmd === 'run_diagnostics_parallel') return parallel.promise
      return undefined
    })

    const useScanner = await loadHook()
    const starter = renderHook(() => useScanner())
    const stopper = renderHook(() => useScanner())

    let scan!: Promise<void>
    act(() => {
      scan = starter.result.current.runQuickScan()
    })

    await waitFor(() => {
      expect(invokeMock.mock.calls.some(c => c[0] === 'run_diagnostics_parallel')).toBe(true)
    })

    act(() => {
      stopper.result.current.stopScan()
    })

    expect(invokeMock).toHaveBeenCalledWith('cancel_diagnostics', { sessionId: 'session-1' })

    parallel.resolve([])
    await act(async () => {
      await scan
    })
  })

  it('cancels a pending auto-save when stopped and still releases the lock', async () => {
    contextValue = makeContext({ settings: { autoSave: true, maxConcurrentTasks: 2 } })
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-1'
      if (cmd === 'run_diagnostics_parallel') {
        return [['os_info', { success: true, output: '{}', duration_ms: 1 }]]
      }
      if (cmd === 'save_current_scan') return 'saved-id'
      return undefined
    })

    const useScanner = await loadHook()
    const { result } = renderHook(() => useScanner())

    let p1!: Promise<void>
    act(() => {
      p1 = result.current.runQuickScan()
    })

    // Let the scan complete and enter the awaited 500ms auto-save delay
    await waitFor(() => {
      expect(invokeMock.mock.calls.some(c => c[0] === 'run_diagnostics_parallel')).toBe(true)
    })
    await act(async () => {
      await Promise.resolve()
    })

    // stopScan resolves the pending delay early; the awaited runDiagnostics
    // body unwinds without waiting out the timer and without saving
    act(() => {
      result.current.stopScan()
    })
    await act(async () => {
      await p1
    })

    expect(invokeMock.mock.calls.some(c => c[0] === 'save_current_scan')).toBe(false)
    expect(invokeMock.mock.calls.some(c => c[0] === 'cancel_diagnostics')).toBe(false)

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-2'
      if (cmd === 'run_diagnostics_parallel') return []
      if (cmd === 'save_current_scan') return 'saved-id'
      return undefined
    })

    await act(async () => {
      await result.current.runQuickScan()
    })

    expect(startCalls()).toHaveLength(2)
  })

  it('clears stale issues when a new scan starts and when results are cleared', async () => {
    const setIssues = vi.fn()
    contextValue = makeContext({ setIssues })
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-1'
      if (cmd === 'run_diagnostics_parallel') return []
      return undefined
    })

    const useScanner = await loadHook()
    const { result } = renderHook(() => useScanner())

    await act(async () => {
      await result.current.runQuickScan()
    })

    expect(setIssues).toHaveBeenCalledWith([])

    act(() => {
      result.current.clearResults()
    })

    expect(setIssues).toHaveBeenCalledTimes(2)
    expect(setIssues).toHaveBeenLastCalledWith([])
  })

  it('auto-saves a non-admin full scan with a Full Scan tag', async () => {
    contextValue = makeContext({
      systemInfo: { is_admin: false },
      availableTasks: [
        { id: 'os_info', name: 'OS Info', admin_required: false },
        { id: 'dism_health', name: 'DISM', admin_required: true },
      ],
      settings: { autoSave: true, maxConcurrentTasks: 2 },
    })
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-1'
      if (cmd === 'run_diagnostics_parallel') {
        return [['os_info', { success: true, output: '{}', duration_ms: 1 }]]
      }
      if (cmd === 'save_current_scan') return 'saved-id'
      return undefined
    })

    const useScanner = await loadHook()
    const { result } = renderHook(() => useScanner())

    await act(async () => {
      await result.current.runFullScan()
    })

    expect(invokeMock).toHaveBeenCalledWith('save_current_scan', expect.objectContaining({
      tags: ['Full Scan'],
    }))
    expect(invokeMock).toHaveBeenCalledWith('start_diagnostics', expect.objectContaining({
      scanKind: 'full',
    }))
  })

  it('rolls back an incomplete replacement scan and reports failure to tracked callers', async () => {
    const previous = { os_info: { success: true, output: '{"before":true}', duration_ms: 1 } }
    const setResults = vi.fn()
    const setSessionId = vi.fn()
    contextValue = makeContext({
      sessionId: 'quick-session',
      results: previous,
      setResults,
      setSessionId,
      availableTasks: [
        { id: 'os_info', name: 'OS Info', admin_required: false },
        { id: 'logical_disk', name: 'Logical disk', admin_required: false },
      ],
    })
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'full-session'
      if (cmd === 'run_diagnostics_parallel') {
        return [['os_info', { success: true, output: '{}', duration_ms: 1 }]]
      }
      return undefined
    })

    const useScanner = await loadHook()
    const { result } = renderHook(() => useScanner())
    let outcome: unknown
    await act(async () => {
      outcome = await result.current.runFullScanTracked()
    })

    expect(outcome).toBe('failed')
    expect(invokeMock).toHaveBeenCalledWith('cancel_diagnostics', { sessionId: 'full-session' })
    expect(setResults).toHaveBeenLastCalledWith(previous)
    expect(setSessionId).toHaveBeenLastCalledWith('quick-session')
  })

  it('reruns one diagnostic in the active session without discarding other results', async () => {
    const previousResult = { success: true, output: '{"before":true}', error: null, duration_ms: 3 }
    const replacementResult = { success: true, output: '{"after":true}', error: null, duration_ms: 4 }
    const setResults = vi.fn()
    const setIssues = vi.fn()
    contextValue = makeContext({
      sessionId: 'session-existing',
      results: { previous_task: previousResult },
      setResults,
      setIssues,
    })
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'run_diagnostic_task') return replacementResult
      if (cmd === 'detect_issues') return []
      return undefined
    })

    const useScanner = await loadHook()
    const { result } = renderHook(() => useScanner())

    await act(async () => {
      await result.current.rerunDiagnostic('os_info')
    })

    expect(startCalls()).toHaveLength(0)
    expect(invokeMock).toHaveBeenCalledWith('run_diagnostic_task', { taskId: 'os_info' })
    const merge = setResults.mock.calls[0]?.[0] as (
      previous: Record<string, unknown>
    ) => Record<string, unknown>
    expect(merge({ previous_task: previousResult })).toEqual({
      previous_task: previousResult,
      os_info: replacementResult,
    })
    expect(invokeMock).toHaveBeenCalledWith('detect_issues')
    expect(setIssues).toHaveBeenCalledWith([])
  })
})

describe('useScanner quick-scan detection coverage', () => {
  const quickTaskIds = () =>
    (startCalls()[0]?.[1] as { taskIds: string[] } | undefined)?.taskIds ?? []

  it('always includes the issue-detection source tasks in a quick scan', async () => {
    const detectionIds = [
      'event_codes_critical', 'services', 'performance',
      'startup_command', 'hosts_file', 'firewall_status',
    ]
    contextValue = makeContext({
      availableTasks: [
        { id: 'os_info', name: 'OS Info', admin_required: false },
        ...detectionIds.map(id => ({ id, name: id, admin_required: false })),
      ],
    })
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-1'
      if (cmd === 'run_diagnostics_parallel') return []
      return undefined
    })

    const useScanner = await loadHook()
    const { result } = renderHook(() => useScanner())
    await act(async () => {
      await result.current.runQuickScan()
    })

    for (const id of detectionIds) {
      expect(quickTaskIds()).toContain(id)
    }
    expect(startCalls()[0]?.[1]).toEqual(expect.objectContaining({ scanKind: 'quick' }))
  })

  it('unions detection sources into a customised quick-scan list', async () => {
    contextValue = makeContext({
      settings: { autoSave: false, maxConcurrentTasks: 2, quickScanTasks: ['os_info'] },
      availableTasks: [
        { id: 'os_info', name: 'OS Info', admin_required: false },
        { id: 'defender_status', name: 'Defender', admin_required: false },
      ],
    })
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_diagnostics') return 'session-1'
      if (cmd === 'run_diagnostics_parallel') return []
      return undefined
    })

    const useScanner = await loadHook()
    const { result } = renderHook(() => useScanner())
    await act(async () => {
      await result.current.runQuickScan()
    })

    // 'defender_status' is a detection source, so it runs even though the user's
    // custom quick list only named 'os_info'.
    expect(quickTaskIds()).toContain('defender_status')
  })
})
