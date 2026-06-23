import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, act, fireEvent, waitFor } from '@testing-library/react'
import { ScanReportPanel } from './ScanReportPanel'

const invokeMock = vi.fn()
type Handler = (event: { payload: unknown }) => void
let eventHandlers: Map<string, Handler>

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, handler: Handler) => {
    eventHandlers.set(name, handler)
    return Promise.resolve(vi.fn())
  },
}))
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({
  writeText: vi.fn().mockResolvedValue(undefined),
}))

let contextValue: Record<string, unknown>
vi.mock('../../contexts/AppContext', () => ({
  useAppContext: () => contextValue,
}))
vi.mock('../../contexts/ToastContext', () => ({
  useToast: () => ({ showSuccess: vi.fn(), showError: vi.fn() }),
}))

function makeContext(overrides: Record<string, unknown> = {}) {
  return {
    results: { os_info: { success: true, output: '{}', error: null, duration_ms: 1 } },
    settings: { openAiApiKey: 'sk-test', aiEnabled: true },
    pendingScanReport: false,
    setPendingScanReport: vi.fn(),
    ...overrides,
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
  eventHandlers = new Map()
  invokeMock.mockReset()
  contextValue = makeContext()
})

describe('ScanReportPanel', () => {
  it('prompts to run a scan when there are no results', () => {
    contextValue = makeContext({ results: {} })
    render(<ScanReportPanel />)
    expect(screen.getByText(/Run a scan, then generate/)).toBeInTheDocument()
  })

  it('generates a streamed report from delta events', async () => {
    invokeMock.mockResolvedValue({ reportId: 'r1', cached: false, provider: 'openai' })
    render(<ScanReportPanel />)
    fireEvent.click(screen.getByRole('button', { name: /Explain this scan/ }))

    expect(invokeMock).toHaveBeenCalledWith('ai_generate_report', {
      previousScanId: null,
      apiKey: 'sk-test',
      forceRefresh: false,
    })
    await waitFor(() => expect(eventHandlers.size).toBe(3))

    act(() => {
      eventHandlers.get('ai-report://delta')?.({ payload: { reportId: 'r1', text: '## Health summary\nAll good.' } })
      eventHandlers.get('ai-report://done')?.({ payload: { reportId: 'r1', finishReason: 'stop', provider: 'openai' } })
    })
    expect(screen.getByText('Health summary')).toBeInTheDocument()
    expect(screen.getByText('All good.')).toBeInTheDocument()
    // Finished: copy + regenerate become available
    expect(screen.getByTitle('Copy report')).toBeInTheDocument()
    expect(screen.getByTitle('Regenerate')).toBeInTheDocument()
  })

  it('keeps report events that arrive before the generate ack resolves', async () => {
    const ack = deferred<{ reportId: string; cached: boolean; provider: string }>()
    invokeMock.mockReturnValue(ack.promise)
    render(<ScanReportPanel />)
    fireEvent.click(screen.getByRole('button', { name: /Explain this scan/ }))
    await waitFor(() => expect(eventHandlers.size).toBe(3))

    act(() => {
      eventHandlers.get('ai-report://delta')?.({ payload: { reportId: 'r1', text: '## Health summary\nFast verdict.' } })
      eventHandlers.get('ai-report://done')?.({ payload: { reportId: 'r1', finishReason: 'stop', provider: 'openai' } })
    })

    await act(async () => {
      ack.resolve({ reportId: 'r1', cached: false, provider: 'openai' })
      await ack.promise
    })

    expect(screen.getByText('Health summary')).toBeInTheDocument()
    expect(screen.getByText('Fast verdict.')).toBeInTheDocument()
    expect(screen.getByTitle('Copy report')).toBeInTheDocument()
  })

  it('prevents duplicate report generation before the first ack resolves', async () => {
    const ack = deferred<{ reportId: string; cached: boolean; provider: string }>()
    invokeMock.mockReturnValue(ack.promise)
    render(<ScanReportPanel />)

    const generate = screen.getByRole('button', { name: /Explain this scan/ })
    fireEvent.click(generate)
    fireEvent.click(generate)

    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(1))

    await act(async () => {
      ack.resolve({ reportId: 'r1', cached: false, provider: 'openai' })
      await ack.promise
    })
  })

  it('renders a cached report immediately without events', async () => {
    invokeMock.mockResolvedValue({
      reportId: 'r1', cached: true, provider: 'openai', report: '## Health summary\nCached verdict.',
    })
    render(<ScanReportPanel />)
    fireEvent.click(screen.getByRole('button', { name: /Explain this scan/ }))
    await waitFor(() => expect(screen.getByText('Cached verdict.')).toBeInTheDocument())
    expect(screen.getByText('Cached verdict.').closest('.scan-report-content')).toBeTruthy()
  })

  it('bypasses cache when regenerating a report', async () => {
    invokeMock
      .mockResolvedValueOnce({
        reportId: 'r1', cached: true, provider: 'openai', report: '## Health summary\nCached verdict.',
      })
      .mockResolvedValueOnce({
        reportId: 'r2', cached: false, provider: 'openai',
      })

    render(<ScanReportPanel />)
    fireEvent.click(screen.getByRole('button', { name: /Explain this scan/ }))
    await waitFor(() => expect(screen.getByTitle('Regenerate')).toBeInTheDocument())

    fireEvent.click(screen.getByTitle('Regenerate'))

    await waitFor(() => {
      expect(invokeMock).toHaveBeenLastCalledWith('ai_generate_report', {
        previousScanId: null,
        apiKey: 'sk-test',
        forceRefresh: true,
      })
    })
  })

  it('shows backend errors with a retry action', async () => {
    invokeMock.mockRejectedValue('No scan data yet')
    render(<ScanReportPanel />)
    fireEvent.click(screen.getByRole('button', { name: /Explain this scan/ }))
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('No scan data yet'))
    expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument()
  })

  it('auto-generates and clears the deep-link flag', async () => {
    const setPendingScanReport = vi.fn()
    contextValue = makeContext({ pendingScanReport: true, setPendingScanReport })
    invokeMock.mockResolvedValue({ reportId: 'r1', cached: false, provider: 'openai' })
    render(<ScanReportPanel />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('ai_generate_report', expect.anything()))
    expect(setPendingScanReport).toHaveBeenCalledWith(false)
  })

  it('does not generate when AI insights are disabled', async () => {
    const setPendingScanReport = vi.fn()
    contextValue = makeContext({
      settings: { openAiApiKey: 'sk-test', aiEnabled: false },
      pendingScanReport: true,
      setPendingScanReport,
    })
    render(<ScanReportPanel />)

    expect(screen.getByText(/AI insights are disabled/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Explain this scan/ })).not.toBeInTheDocument()
    await waitFor(() => expect(setPendingScanReport).toHaveBeenCalledWith(false))
    expect(invokeMock).not.toHaveBeenCalledWith('ai_generate_report', expect.anything())
  })
})
