import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { HistoryScreen } from './HistoryScreen'

const invokeMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

vi.mock('../contexts/ToastContext', () => ({
  useToast: () => ({ showSuccess: vi.fn(), showError: vi.fn(), showInfo: vi.fn() }),
}))

const scans = [
  {
    id: 'scan_2', timestamp: '2026-06-12T10:00:00Z', computer_name: 'PC-A',
    task_count: 2, success_count: 1, failure_count: 1, duration_ms: 1500, tags: ['after-update'],
  },
  {
    id: 'scan_1', timestamp: '2026-06-11T10:00:00Z', computer_name: 'PC-A',
    task_count: 2, success_count: 2, failure_count: 0, duration_ms: 1200, tags: [],
  },
]

const comparison = {
  current_scan: { ...scans[0] },
  previous_scan: { ...scans[1] },
  total_changes: 1,
  new_failures: [{
    task_id: 'os_info', task_name: 'OS Information', category: 'System',
    current_success: false, previous_success: true,
    current_output: '{"build": 26200}', previous_output: '{"build": 26100}',
    output_changed: true,
  }],
  new_successes: [],
  status_unchanged: [],
}

beforeEach(() => {
  vi.clearAllMocks()
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === 'list_scan_history') return scans
    if (cmd === 'get_task_trends') return [
      { task_id: 'os_info', failed: 3, seen_in: 10, scans_considered: 10 },
    ]
    if (cmd === 'compare_scans') return comparison
    return undefined
  })
})

describe('HistoryScreen compare flow', () => {
  it('lists scans and marks the latest', async () => {
    render(<HistoryScreen />)
    await waitFor(() => expect(screen.getByText('after-update')).toBeInTheDocument())
    expect(screen.getByText('Latest')).toBeInTheDocument()
  })

  it('filters the scan list by label', async () => {
    render(<HistoryScreen />)
    await waitFor(() => expect(screen.getByText('after-update')).toBeInTheDocument())
    fireEvent.change(screen.getByLabelText('Filter scan history'), { target: { value: 'after-up' } })
    expect(screen.getByText('after-update')).toBeInTheDocument()
    expect(screen.queryByText('Scan')).not.toBeInTheDocument()
  })

  it('selecting an older scan compares it and expands a side-by-side diff', async () => {
    render(<HistoryScreen />)
    await waitFor(() => expect(screen.getByText('after-update')).toBeInTheDocument())

    // Click the older (non-latest) scan row
    fireEvent.click(screen.getByText(new Date(scans[1].timestamp).toLocaleString()))

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('compare_scans', { currentId: 'scan_2', previousId: 'scan_1' })
    )
    await waitFor(() => expect(screen.getByText('regressed')).toBeInTheDocument())
    // Trend badge from get_task_trends
    expect(screen.getByText('3/10 fails')).toBeInTheDocument()

    // Expand the regressed task — side-by-side panes appear
    fireEvent.click(screen.getByText('OS Information'))
    expect(screen.getByText('Previous')).toBeInTheDocument()
    expect(screen.getByText('Current')).toBeInTheDocument()
    // Field-level JSON diff line
    expect(screen.getByText(/Changed: build from 26100 to 26200/)).toBeInTheDocument()
  })

  it('saves an edited label through update_scan_tags', async () => {
    render(<HistoryScreen />)
    await waitFor(() => expect(screen.getByText('after-update')).toBeInTheDocument())

    fireEvent.click(screen.getByText(new Date(scans[1].timestamp).toLocaleString()))
    await waitFor(() => expect(screen.getByLabelText('Edit scan label')).toBeInTheDocument())

    fireEvent.click(screen.getByLabelText('Edit scan label'))
    fireEvent.change(screen.getByLabelText('Scan label'), { target: { value: 'baseline' } })
    fireEvent.keyDown(screen.getByLabelText('Scan label'), { key: 'Enter' })

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('update_scan_tags', { scanId: 'scan_1', tags: ['baseline'] })
    )
  })
})
