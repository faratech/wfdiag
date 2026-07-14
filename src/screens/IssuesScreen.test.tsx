import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react'
import { IssuesScreen } from './IssuesScreen'
import type {
  ActionFixResult,
  ActionProposal,
  ActionRequest,
  ActionRunSummary,
  Issue,
  RemediationSummary,
} from '../contexts/AppContext'

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

const setPendingChatPrompt = vi.fn()
const setAIMode = vi.fn()
const setSelectedTab = vi.fn()
const setFixingIssue = vi.fn()
const restartAsAdmin = vi.fn().mockResolvedValue(undefined)
const showToast = { showInfo: vi.fn(), showWarning: vi.fn(), showError: vi.fn() }
const prioritizeIssues = vi.fn().mockResolvedValue('1. Fix the disk first')

const repairRemediation: RemediationSummary = {
  id: 'dism_restorehealth', label: 'Repair Windows image (DISM)',
  description: "Runs 'DISM /Online /Cleanup-Image /RestoreHealth'.",
  tier: 'repair', admin_required: true, requires_restart: false,
  long_running: true, maintenance: true, batch_eligible: false, cancellable: false,
}
const safeRemediation: RemediationSummary = {
  id: 'flush_dns', label: 'Flush DNS cache',
  description: "Runs 'ipconfig /flushdns'.",
  tier: 'auto_safe', admin_required: false, requires_restart: false,
  long_running: false, maintenance: true, batch_eligible: true, cancellable: true,
}
const secondSafeRemediation: RemediationSummary = {
  id: 'clear_temp_files', label: 'Clear temporary files',
  description: 'Removes stale files from the current user temporary directory.',
  tier: 'auto_safe', admin_required: false, requires_restart: false,
  long_running: false, maintenance: true, batch_eligible: true, cancellable: true,
}
const adminSafeRemediation: RemediationSummary = {
  id: 'clear_prefetch', label: 'Clear prefetch files',
  description: 'Deletes .pf files from C:\\Windows\\Prefetch.',
  tier: 'auto_safe', admin_required: true, requires_restart: false,
  long_running: false, maintenance: true, batch_eligible: false, cancellable: false,
}
const openToolRemediation: RemediationSummary = {
  id: 'open_windows_update', label: 'Open Windows Update',
  description: 'Opens the Windows Update settings page.',
  tier: 'open_tool', admin_required: false, requires_restart: false,
  long_running: false, maintenance: false, batch_eligible: false, cancellable: false,
}
const restartRemediation: RemediationSummary = {
  id: 'restart_system', label: 'Restart Windows (60s)',
  description: 'Schedules a Windows restart in 60 seconds.',
  tier: 'repair', admin_required: false, requires_restart: true,
  long_running: false, maintenance: false, batch_eligible: false, cancellable: false,
}

const allRemediations = [
  repairRemediation,
  safeRemediation,
  adminSafeRemediation,
  secondSafeRemediation,
  openToolRemediation,
  restartRemediation,
]

interface MockFixPlan {
  entries: Array<{ issue_id: string; remediation_id: string; rationale: string; tier: RemediationSummary['tier'] }>
  notes: string
  scan_fingerprint: string
  catalog_fingerprint: string
}

let issues: Issue[]
let isAdmin = true
let appSettings: Record<string, unknown>
let runResult: ActionFixResult
let fixPlan: MockFixPlan
let proposalCounter = 0
let proposals: Map<string, ActionProposal>

vi.mock('../contexts/AppContext', async () => ({
  useAppContext: () => ({
    issues,
    results: { dism_health: { success: true, output: '{"status":"Repairable"}', duration_ms: 1 } },
    fixingIssue: null,
    setFixingIssue,
    isRunning: false,
    systemInfo: { computer_name: 'PC', os_version: 'Win11', is_admin: isAdmin },
    setPendingChatPrompt,
    setAIMode,
    setSelectedTab,
    settings: appSettings,
  }),
}))
vi.mock('../contexts/AIContext', () => ({
  useAIContext: () => ({ prioritizeIssues, isAnalyzing: {} }),
}))
vi.mock('../contexts/ToastContext', () => ({
  useToast: () => showToast,
}))
vi.mock('../hooks/useDiagnostics', () => ({
  useDiagnostics: () => ({ restartAsAdmin }),
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
      category: 'Security', detected: false, status: 'ok',
    },
    {
      id: 'disk_health', title: 'Disk Health',
      description: 'Required diagnostic data was not available for this check.',
      severity: 'Info', category: 'Storage', detected: false, status: 'skipped',
    },
  ]
}

function makeProposal(requests: ActionRequest[]): ActionProposal {
  proposalCounter += 1
  const proposal: ActionProposal = {
    proposalId: `proposal-${proposalCounter}`,
    approvalScope: requests.length > 1 ? 'batch' : 'exact',
    actions: requests.map(request => {
      const remediation = allRemediations.find(item => item.id === request.remediationId)
      if (!remediation) throw new Error(`Unknown test remediation ${request.remediationId}`)
      return {
        remediation,
        issueId: request.issueId,
        steps: remediation.id === 'flush_dns'
          ? ['ipconfig /flushdns']
          : remediation.id === 'dism_restorehealth'
            ? ['dism /online /cleanup-image /restorehealth']
            : [remediation.description],
      }
    }),
    scanFingerprint: 'scan-current',
    catalogFingerprint: 'catalog-current',
    createdAtMs: 1_000,
    expiresAtMs: 601_000,
  }
  proposals.set(proposal.proposalId, proposal)
  return proposal
}

function makeRun(proposal: ActionProposal): ActionRunSummary {
  const status = runResult.completion_status === 'succeeded'
    ? 'succeeded'
    : runResult.completion_status === 'cancelled'
      ? 'cancelled'
      : runResult.completion_status
  return {
    runId: `run-${proposal.proposalId}`,
    proposalId: proposal.proposalId,
    authorizationId: `grant-${proposal.proposalId}`,
    status,
    actions: proposal.actions.map(action => ({
      remediationId: action.remediation.id,
      label: action.remediation.label,
      cancellable: action.remediation.cancellable,
      status: runResult.completion_status,
      result: runResult,
    })),
    approvedAtMs: 2_000,
    completedAtMs: 2_001,
    scanFingerprint: proposal.scanFingerprint,
    catalogFingerprint: proposal.catalogFingerprint,
  }
}

beforeEach(() => {
  vi.clearAllMocks()
  issues = makeIssues()
  isAdmin = true
  appSettings = { openAiApiKey: 'sk-test', aiEnabled: true }
  runResult = {
    success: true,
    message: 'done',
    actions_taken: ['Completed test action'],
    requires_restart: false,
    completion_status: 'succeeded',
    steps: [],
  }
  fixPlan = {
    entries: [{ issue_id: 'dism_corruption', remediation_id: 'dism_restorehealth', rationale: 'Repairs the store.', tier: 'repair' }],
    notes: 'One repair recommended.',
    scan_fingerprint: 'scan-plan',
    catalog_fingerprint: 'catalog-plan',
  }
  proposalCounter = 0
  proposals = new Map()
  invokeMock.mockImplementation((cmd: string, args?: unknown) => {
    switch (cmd) {
      case 'get_remediations':
        return Promise.resolve(allRemediations.filter(remediation => remediation.maintenance))
      case 'action_prepare': {
        const payload = args as { request: { actions: ActionRequest[] } }
        return Promise.resolve(makeProposal(payload.request.actions))
      }
      case 'action_approve': {
        const payload = args as { proposalId: string }
        const proposal = proposals.get(payload.proposalId)
        if (!proposal) return Promise.reject(new Error('proposal not found'))
        return Promise.resolve(makeRun(proposal))
      }
      case 'action_discard_proposal':
      case 'action_cancel':
        return Promise.resolve(undefined)
      case 'ai_propose_fix_plan':
        return Promise.resolve(fixPlan)
      default:
        return Promise.reject(new Error(`unexpected ${cmd}`))
    }
  })
})

describe('IssuesScreen remediation flow', () => {
  it('reviews an immutable repair proposal and approves only its opaque id', async () => {
    render(<IssuesScreen />)
    fireEvent.click(screen.getAllByRole('button', { name: /Repair Windows image/ })[0])

    expect(await screen.findByText('dism /online /cleanup-image /restorehealth')).toBeInTheDocument()
    expect(screen.getByText(/cannot be stopped safely once it starts/i)).toBeInTheDocument()
    expect(invokeMock).toHaveBeenCalledWith('action_prepare', {
      request: {
        actions: [{ remediationId: 'dism_restorehealth', issueId: 'dism_corruption' }],
        expectedScanFingerprint: undefined,
        expectedCatalogFingerprint: undefined,
      },
    })
    expect(JSON.stringify(invokeMock.mock.calls.find(call => call[0] === 'action_prepare'))).not.toMatch(/argv|program|confirmed/)
    expect(invokeMock).not.toHaveBeenCalledWith('action_approve', expect.anything())

    fireEvent.click(screen.getByRole('button', { name: 'Run repair once' }))
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('action_approve', { proposalId: 'proposal-1' }))
    expect(invokeMock).not.toHaveBeenCalledWith('action_approve', expect.objectContaining({ remediationId: expect.anything() }))
    await waitFor(() => expect(showToast.showInfo).toHaveBeenCalledWith(
      'Fix applied',
      'Re-run the relevant diagnostic to verify the current state.'
    ))
  })

  it('describes the restart action as scheduling a restart instead of a repair', async () => {
    issues = [{
      id: 'pending_reboot',
      title: 'Windows Restart Required',
      description: 'Windows Update reports that a restart is required.',
      severity: 'Warning',
      category: 'System',
      detected: true,
      recommendation: 'Save your work and restart Windows.',
      remediation: restartRemediation,
    }]
    runResult = {
      success: true,
      message: 'Restart scheduled.',
      actions_taken: ['Scheduled Windows restart'],
      requires_restart: true,
      completion_status: 'succeeded',
      steps: [],
    }

    render(<IssuesScreen />)
    fireEvent.click(screen.getByRole('button', { name: /Restart Windows \(60s\)/ }))

    expect(await screen.findByRole('button', { name: 'Schedule restart' })).toBeInTheDocument()
    expect(screen.getByText(/Save your work first/)).toBeInTheDocument()
    expect(screen.getByText(/shutdown \/a/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Run repair once' })).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Schedule restart' }))

    await waitFor(() => expect(showToast.showInfo).toHaveBeenCalledWith(
      'Restart scheduled',
      'Windows will restart in 60 seconds. Save your work now; run “shutdown /a” to cancel.'
    ))
    expect(showToast.showInfo).not.toHaveBeenCalledWith(
      'Done — restart required',
      expect.anything()
    )
  })

  it('discards a cancelled proposal and never approves it', async () => {
    render(<IssuesScreen />)
    fireEvent.click(screen.getAllByRole('button', { name: /Repair Windows image/ })[0])
    await screen.findByRole('button', { name: 'Run repair once' })
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('action_discard_proposal', { proposalId: 'proposal-1' }))
    expect(invokeMock).not.toHaveBeenCalledWith('action_approve', expect.anything())
  })

  it('requires review for low-impact actions too', async () => {
    render(<IssuesScreen />)
    fireEvent.click(screen.getAllByRole('button', { name: /Flush DNS cache/ })[0])

    expect(await screen.findByText('ipconfig /flushdns')).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith('action_approve', expect.anything())
    fireEvent.click(screen.getByRole('button', { name: 'Run once' }))
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('action_approve', { proposalId: 'proposal-1' }))
  })

  it('reports completed and failed steps plus a required restart for partial repairs', async () => {
    runResult = {
      success: false,
      completion_status: 'partial',
      message: 'Network reset partly completed.',
      actions_taken: ['Reset Winsock'],
      requires_restart: true,
      steps: [
        { action: 'Reset Winsock', status: 'succeeded' },
        { action: 'Flush DNS', status: 'already_satisfied' },
        { action: 'Reset TCP/IP', status: 'failed', detail: 'Access denied' },
      ],
    }
    render(<IssuesScreen />)
    fireEvent.click(screen.getAllByRole('button', { name: /Flush DNS cache/ })[0])
    await screen.findByRole('button', { name: 'Run once' })
    fireEvent.click(screen.getByRole('button', { name: 'Run once' }))

    await waitFor(() => expect(showToast.showWarning).toHaveBeenCalledWith(
      'Partly completed — restart required',
      expect.stringMatching(/Completed: Reset Winsock.*Already satisfied: Flush DNS.*Could not complete: Reset TCP\/IP \(Access denied\).*Restart Windows/)
    ))
  })

  it('describes open-tool actions as handoffs rather than completed fixes', async () => {
    issues = [{
      id: 'windows_update_pending',
      title: 'Updates available',
      description: 'Windows Update needs attention.',
      severity: 'Warning',
      category: 'System',
      detected: true,
      remediation: openToolRemediation,
    }]
    render(<IssuesScreen />)
    fireEvent.click(screen.getByRole('button', { name: /Open Windows Update/ }))
    await screen.findByRole('button', { name: 'Run once' })
    fireEvent.click(screen.getByRole('button', { name: 'Run once' }))

    await waitFor(() => expect(showToast.showInfo).toHaveBeenCalledWith(
      'Tool opened',
      'Complete the action in the Windows tool, then re-run the relevant diagnostic.'
    ))
  })

  it('asks standard users to restart as administrator before approval', async () => {
    isAdmin = false
    render(<IssuesScreen />)
    await waitFor(() => expect(screen.getAllByRole('button', { name: /^Run$/ })).toHaveLength(4))

    fireEvent.click(screen.getAllByRole('button', { name: /^Run$/ })[2])

    expect(await screen.findByRole('button', { name: /Restart as Administrator/ })).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith('action_approve', expect.anything())
  })

  it('prepares bounded low-impact plan actions as a batch with plan fingerprints', async () => {
    issues = [
      ...makeIssues(),
      {
        id: 'temp_files', title: 'Temporary files', description: 'Stale files can be removed.',
        severity: 'Info', category: 'Storage', detected: true, remediation: secondSafeRemediation,
      },
    ]
    fixPlan = {
      entries: [
        { issue_id: 'dns_misconfigured', remediation_id: 'flush_dns', rationale: 'Refresh DNS.', tier: 'auto_safe' },
        { issue_id: 'temp_files', remediation_id: 'clear_temp_files', rationale: 'Clear stale files.', tier: 'auto_safe' },
      ],
      notes: 'Two low-impact actions.',
      scan_fingerprint: 'scan-plan',
      catalog_fingerprint: 'catalog-plan',
    }
    render(<IssuesScreen />)
    fireEvent.click(screen.getByRole('button', { name: /Propose fix plan/ }))
    fireEvent.click(await screen.findByRole('button', { name: /Review 2 low-impact actions together/ }))

    expect(await screen.findByRole('button', { name: 'Run these 2 actions' })).toBeInTheDocument()
    expect(invokeMock).toHaveBeenCalledWith('action_prepare', {
      request: {
        actions: [
          { remediationId: 'flush_dns', issueId: 'dns_misconfigured' },
          { remediationId: 'clear_temp_files', issueId: 'temp_files' },
        ],
        expectedScanFingerprint: 'scan-plan',
        expectedCatalogFingerprint: 'catalog-plan',
      },
    })
  })

  it('suppresses stale AI fix plan entries after issues change', async () => {
    const { rerender } = render(<IssuesScreen />)
    fireEvent.click(screen.getByRole('button', { name: /Propose fix plan/ }))
    await waitFor(() => expect(screen.getByText('Repairs the store.')).toBeInTheDocument())

    issues = [makeIssues()[2]]
    rerender(<IssuesScreen />)

    await waitFor(() => expect(screen.queryByText('Repairs the store.')).not.toBeInTheDocument())
  })

  it('Ask AI sends a concise prompt with structured issue and diagnostic references', () => {
    render(<IssuesScreen />)
    fireEvent.click(screen.getAllByRole('button', { name: /Ask AI/ })[0])
    expect(setSelectedTab).toHaveBeenCalledWith('ai')
    expect(setAIMode).toHaveBeenCalledWith('assistant')
    const prompt = setPendingChatPrompt.mock.calls[0][0] as { displayText: string; query: string; contextRefs: unknown[] }
    expect(prompt.displayText).toBe('Help me understand and fix “Windows Component Store Corruption”.')
    expect(prompt.query).toContain('dism_corruption')
    expect(prompt.query).not.toContain('Diagnostic data (dism_health)')
    expect(prompt.contextRefs).toEqual([
      { kind: 'issue', id: 'dism_corruption' },
      { kind: 'diagnostic', id: 'dism_health' },
    ])
  })

  it('renders AI plan actions through the same proposal review flow', async () => {
    render(<IssuesScreen />)
    fireEvent.click(screen.getByRole('button', { name: /Propose fix plan/ }))
    const rationale = await screen.findByText('Repairs the store.')
    expect(screen.getByText('One repair recommended.')).toBeInTheDocument()

    const planCard = rationale.closest('.issue-card')
    expect(planCard).not.toBeNull()
    fireEvent.click(within(planCard as HTMLElement).getByRole('button', { name: /Repair Windows image/ }))
    expect(await screen.findByRole('button', { name: 'Run repair once' })).toBeInTheDocument()
  })

  it('collapses passed checks and renders triage markdown', async () => {
    render(<IssuesScreen />)
    expect(screen.getByText('1 checks passed')).toBeInTheDocument()
    expect(screen.getByText('Couldn’t verify (1)')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /Prioritize/ }))
    await waitFor(() => expect(prioritizeIssues).toHaveBeenCalled())
    await waitFor(() => expect(screen.getByText(/Fix the disk first/)).toBeInTheDocument())
  })

  it('does not claim all clear when some checks could not verify', () => {
    issues = makeIssues().slice(2)
    render(<IssuesScreen />)

    expect(screen.getByText(/No issues detected in completed checks/)).toBeInTheDocument()
    expect(screen.queryByText(/All clear/)).not.toBeInTheDocument()
  })

  it('disables AI assistance actions when AI insights are disabled', () => {
    appSettings = { openAiApiKey: 'sk-test', aiEnabled: false }
    render(<IssuesScreen />)

    expect(screen.getByRole('button', { name: /Prioritize/ })).toBeDisabled()
    expect(screen.getByRole('button', { name: /Propose fix plan/ })).toBeDisabled()
    expect(screen.getAllByRole('button', { name: /Ask AI/ })[0]).toBeDisabled()
    expect(invokeMock).not.toHaveBeenCalledWith('ai_propose_fix_plan', expect.anything())
    expect(setPendingChatPrompt).not.toHaveBeenCalled()
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
