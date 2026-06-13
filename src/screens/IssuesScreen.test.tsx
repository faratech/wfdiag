import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { IssuesScreen } from './IssuesScreen'
import type { Issue, RemediationSummary } from '../contexts/AppContext'

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

const setPendingChatPrompt = vi.fn()
const setSelectedTab = vi.fn()
const setFixingIssue = vi.fn()
const detectIssues = vi.fn().mockResolvedValue(undefined)
const restartAsAdmin = vi.fn().mockResolvedValue(undefined)
const showToast = { showInfo: vi.fn(), showWarning: vi.fn(), showError: vi.fn() }
const prioritizeIssues = vi.fn().mockResolvedValue('1. Fix the disk first')

const repairRemediation: RemediationSummary = {
  id: 'dism_restorehealth', label: 'Repair Windows image (DISM)',
  description: "Runs 'DISM /Online /Cleanup-Image /RestoreHealth'.",
  tier: 'repair', admin_required: true, requires_restart: false,
  long_running: true, maintenance: true,
}
const safeRemediation: RemediationSummary = {
  id: 'flush_dns', label: 'Flush DNS cache',
  description: "Runs 'ipconfig /flushdns'.",
  tier: 'auto_safe', admin_required: false, requires_restart: false,
  long_running: false, maintenance: true,
}

let issues: Issue[]
let isAdmin = true

vi.mock('../contexts/AppContext', async () => ({
  useAppContext: () => ({
    issues,
    results: { dism_health: { success: true, output: '{"status":"Repairable"}', duration_ms: 1 } },
    fixingIssue: null,
    setFixingIssue,
    isRunning: false,
    systemInfo: { computer_name: 'PC', os_version: 'Win11', is_admin: isAdmin },
    setPendingChatPrompt,
    setSelectedTab,
    settings: { openAiApiKey: 'sk-test' },
  }),
}))
vi.mock('../contexts/AIContext', () => ({
  useAIContext: () => ({ prioritizeIssues, isAnalyzing: {} }),
}))
vi.mock('../contexts/ToastContext', () => ({
  useToast: () => showToast,
}))
vi.mock('../hooks/useDiagnostics', () => ({
  useDiagnostics: () => ({ detectIssues, restartAsAdmin }),
}))
vi.mock('../hooks/useScanner', () => ({
  useScanner: () => ({ runQuickScan: vi.fn() }),
}))

function makeIssues(): Issue[] {
  return [
    {
      id: 'dism_corruption', title: 'Windows Component Store Corruption',
      description: 'The Windows component store has repairable corruption.',
      severity: 'Warning', category: 'System', detected: true,
      recommendation: 'Run the DISM repair.', source_tasks: ['dism_health'],
      remediation: repairRemediation,
    },
    {
      id: 'dns_misconfigured', title: 'DNS Not Configured',
      description: 'Adapter has a gateway but no DNS.', severity: 'Warning',
      category: 'Network', detected: true, remediation: safeRemediation,
    },
    {
      id: 'firewall_disabled', title: 'Firewall Status',
      description: 'Firewall is enabled.', severity: 'Ok',
      category: 'Security', detected: false,
    },
  ]
}

beforeEach(() => {
  vi.clearAllMocks()
  issues = makeIssues()
  isAdmin = true
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'get_remediations':
        return Promise.resolve([repairRemediation, safeRemediation])
      case 'run_remediation':
        return Promise.resolve({ status: 'completed', result: { success: true, message: 'done' } })
      case 'ai_propose_fix_plan':
        return Promise.resolve({
          entries: [{ issue_id: 'dism_corruption', remediation_id: 'dism_restorehealth', rationale: 'Repairs the store.', tier: 'repair' }],
          notes: 'One repair recommended.',
        })
      default:
        return Promise.reject(new Error(`unexpected ${cmd}`))
    }
  })
})

describe('IssuesScreen remediation flow', () => {
  it('repair tier opens the confirm modal; confirming runs with confirmed: true', async () => {
    render(<IssuesScreen />)
    fireEvent.click(screen.getByRole('button', { name: /Repair Windows image/ }))
    // Modal shows exactly what runs; nothing invoked yet
    expect(screen.getByText(/DISM \/Online \/Cleanup-Image \/RestoreHealth/)).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith('run_remediation', expect.anything())

    fireEvent.click(screen.getByRole('button', { name: 'Run repair' }))
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('run_remediation', {
        remediationId: 'dism_restorehealth',
        confirmed: true,
      })
    )
    await waitFor(() => expect(detectIssues).toHaveBeenCalled())
  })

  it('cancel in the confirm modal runs nothing', async () => {
    render(<IssuesScreen />)
    fireEvent.click(screen.getByRole('button', { name: /Repair Windows image/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(invokeMock).not.toHaveBeenCalledWith('run_remediation', expect.anything())
  })

  it('auto-safe fixes run immediately without the modal', async () => {
    render(<IssuesScreen />)
    fireEvent.click(screen.getByRole('button', { name: /Flush DNS cache/ }))
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('run_remediation', {
        remediationId: 'flush_dns',
        confirmed: false,
      })
    )
  })

  it('Ask AI seeds the chat prompt with issue + diagnostic data and switches tabs', () => {
    render(<IssuesScreen />)
    fireEvent.click(screen.getAllByRole('button', { name: /Ask AI/ })[0])
    expect(setSelectedTab).toHaveBeenCalledWith('ai')
    const prompt = setPendingChatPrompt.mock.calls[0][0] as string
    expect(prompt).toContain('dism_corruption')
    expect(prompt).toContain('Diagnostic data (dism_health)')
  })

  it('renders the AI fix plan with Run buttons wired to the normal flow', async () => {
    render(<IssuesScreen />)
    fireEvent.click(screen.getByRole('button', { name: /Propose fix plan/ }))
    await waitFor(() => expect(screen.getByText('Repairs the store.')).toBeInTheDocument())
    expect(screen.getByText('One repair recommended.')).toBeInTheDocument()
    // The plan's Run button goes through the same confirm gate (repair tier)
    const planButtons = screen.getAllByRole('button', { name: /Repair Windows image/ })
    fireEvent.click(planButtons[0])
    expect(screen.getByRole('button', { name: 'Run repair' })).toBeInTheDocument()
  })

  it('passed checks are collapsed and triage renders markdown', async () => {
    render(<IssuesScreen />)
    expect(screen.getByText('1 checks passed')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /Prioritize/ }))
    await waitFor(() => expect(prioritizeIssues).toHaveBeenCalled())
    await waitFor(() => expect(screen.getByText(/Fix the disk first/)).toBeInTheDocument())
  })
})

describe('IssuesScreen admin-gated checks notice', () => {
  it('shows the elevation notice when not admin and a scan has run', () => {
    isAdmin = false
    render(<IssuesScreen />)
    expect(screen.getByText(/need administrator access/i)).toBeInTheDocument()
    const restart = screen.getByRole('button', { name: /Restart as administrator/i })
    fireEvent.click(restart)
    expect(restartAsAdmin).toHaveBeenCalled()
  })

  it('hides the elevation notice when the app is elevated', () => {
    isAdmin = true
    render(<IssuesScreen />)
    expect(screen.queryByText(/need administrator access/i)).not.toBeInTheDocument()
  })
})
