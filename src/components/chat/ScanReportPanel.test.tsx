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
    expect(screen.getByRole('heading', { name: /Run a scan first/ })).toBeInTheDocument()
  })

  it('generates a streamed report from delta events', async () => {
    invokeMock.mockResolvedValue({ reportId: 'r1', cached: false, provider: 'openai' })
    render(<ScanReportPanel />)
    fireEvent.click(screen.getByRole('button', { name: /Generate report/ }))

    expect(invokeMock).toHaveBeenCalledWith('ai_generate_report', {
      previousScanId: null,
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
    fireEvent.click(screen.getByRole('button', { name: /Generate report/ }))
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

    const generate = screen.getByRole('button', { name: /Generate report/ })
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
    fireEvent.click(screen.getByRole('button', { name: /Generate report/ }))
    await waitFor(() => expect(screen.getByText('Cached verdict.')).toBeInTheDocument())
    expect(screen.getByText('Cached verdict.').closest('.scan-report-content')).toBeTruthy()
  })

  it('does not present a report from an earlier result snapshot as the latest scan', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'ai_generate_report') {
        return Promise.resolve({
          reportId: 'r-old', cached: true, provider: 'openai', report: '## Health summary\nOld verdict.',
        })
      }
      if (command === 'ai_report_cancel') return Promise.resolve()
      return Promise.reject(new Error(`Unexpected command: ${command}`))
    })
    const view = render(<ScanReportPanel />)
    fireEvent.click(screen.getByRole('button', { name: /Generate report/ }))
    await waitFor(() => expect(screen.getByText('Old verdict.')).toBeInTheDocument())

    contextValue = makeContext({
      results: { os_info: { success: true, output: '{"new":true}', error: null, duration_ms: 2 } },
    })
    view.rerender(<ScanReportPanel />)

    expect(screen.queryByText('Old verdict.')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Generate report/ })).toBeInTheDocument()
  })

  it('ignores a stale request error after a newer report succeeds', async () => {
    const first = deferred<{ reportId: string; cached: boolean; provider: string; report?: string }>()
    let generation = 0
    invokeMock.mockImplementation((command: string) => {
      if (command === 'ai_generate_report') {
        generation += 1
        return generation === 1
          ? first.promise
          : Promise.resolve({
              reportId: 'r-new', cached: true, provider: 'openai', report: '## Health summary\nNew verdict.',
            })
      }
      if (command === 'ai_report_cancel') return Promise.resolve()
      return Promise.reject(new Error(`Unexpected command: ${command}`))
    })

    const view = render(<ScanReportPanel />)
    fireEvent.click(screen.getByRole('button', { name: /Generate report/ }))

    contextValue = makeContext({
      results: { os_info: { success: true, output: '{"new":true}', error: null, duration_ms: 2 } },
    })
    view.rerender(<ScanReportPanel />)
    fireEvent.click(await screen.findByRole('button', { name: /Generate report/ }))
    await waitFor(() => expect(screen.getByText('New verdict.')).toBeInTheDocument())

    await act(async () => {
      first.reject(new Error('old request failed'))
      await first.promise.catch(() => undefined)
    })

    expect(screen.getByText('New verdict.')).toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
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
    fireEvent.click(screen.getByRole('button', { name: /Generate report/ }))
    await waitFor(() => expect(screen.getByTitle('Regenerate')).toBeInTheDocument())

    fireEvent.click(screen.getByTitle('Regenerate'))

    await waitFor(() => {
      expect(invokeMock).toHaveBeenLastCalledWith('ai_generate_report', {
        previousScanId: null,
        forceRefresh: true,
      })
    })
  })

  it('shows backend errors with a retry action', async () => {
    invokeMock.mockRejectedValue('No scan data yet')
    render(<ScanReportPanel />)
    fireEvent.click(screen.getByRole('button', { name: /Generate report/ }))
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('No scan data yet'))
    expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument()
  })

  it('cancels an in-flight report', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'ai_generate_report') return Promise.resolve({ reportId: 'r1', cached: false, provider: 'openai' })
      if (command === 'ai_report_cancel') return Promise.resolve()
      return Promise.reject(new Error(`Unexpected command: ${command}`))
    })
    render(<ScanReportPanel />)
    fireEvent.click(screen.getByRole('button', { name: /Generate report/ }))
    const stop = await screen.findByRole('button', { name: /Stop/ })
    fireEvent.click(stop)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('ai_report_cancel', { reportId: 'r1' }))
  })

  it('waits for backend cancellation cleanup before allowing regeneration', async () => {
    const cancellation = deferred<void>()
    let generations = 0
    invokeMock.mockImplementation((command: string) => {
      if (command === 'ai_generate_report') {
        generations += 1
        return Promise.resolve({ reportId: `r${generations}`, cached: false, provider: 'openai' })
      }
      if (command === 'ai_report_cancel') return cancellation.promise
      return Promise.reject(new Error(`Unexpected command: ${command}`))
    })

    render(<ScanReportPanel />)
    fireEvent.click(screen.getByRole('button', { name: /Generate report/ }))
    fireEvent.click(await screen.findByRole('button', { name: /^Stop$/ }))

    expect(await screen.findByRole('button', { name: /Stopping/ })).toBeDisabled()
    expect(screen.queryByRole('button', { name: /Generate report/ })).not.toBeInTheDocument()

    await act(async () => {
      cancellation.resolve()
      await cancellation.promise
    })
    fireEvent.click(await screen.findByRole('button', { name: /Generate report/ }))
    await waitFor(() => expect(generations).toBe(2))
  })

  it('stops locally even when the backend cancel request fails', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'ai_generate_report') return Promise.resolve({ reportId: 'r1', cached: false, provider: 'openai' })
      if (command === 'ai_report_cancel') return Promise.reject(new Error('cancel transport failed'))
      return Promise.reject(new Error(`Unexpected command: ${command}`))
    })
    render(<ScanReportPanel />)
    fireEvent.click(screen.getByRole('button', { name: /Generate report/ }))
    fireEvent.click(await screen.findByRole('button', { name: /Stop/ }))

    expect(await screen.findByRole('button', { name: /Generate report/ })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Stop/ })).not.toBeInTheDocument()
  })

  it('honors stop while report setup is still waiting for its acknowledgement', async () => {
    const ack = deferred<{ reportId: string; cached: boolean; provider: string }>()
    invokeMock.mockImplementation((command: string) => {
      if (command === 'ai_generate_report') return ack.promise
      if (command === 'ai_report_cancel') return Promise.resolve()
      return Promise.reject(new Error(`Unexpected command: ${command}`))
    })
    render(<ScanReportPanel />)
    fireEvent.click(screen.getByRole('button', { name: /Generate report/ }))
    fireEvent.click(await screen.findByRole('button', { name: /Stop/ }))

    await act(async () => {
      ack.resolve({ reportId: 'r-late', cached: false, provider: 'openai' })
      await ack.promise
    })

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('ai_report_cancel', { reportId: 'r-late' }))
    expect(screen.getByRole('button', { name: /Generate report/ })).toBeInTheDocument()
  })

  it('does not generate when AI insights are disabled', async () => {
    contextValue = makeContext({
      settings: { openAiApiKey: 'sk-test', aiEnabled: false },
    })
    render(<ScanReportPanel />)

    expect(screen.getByRole('heading', { name: /AI insights are turned off/ })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Generate report/ })).not.toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith('ai_generate_report', expect.anything())
  })
})
