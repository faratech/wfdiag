import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import type { ActionProposal, ActionRunSummary } from '../../contexts/AppContext'
import { StagedActionCards } from './StagedActionCards'

const invokeMock = vi.hoisted(() => vi.fn())
const showInfoMock = vi.hoisted(() => vi.fn())
const showWarningMock = vi.hoisted(() => vi.fn())
const showErrorMock = vi.hoisted(() => vi.fn())

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

vi.mock('../../contexts/AppContext', () => ({
  useAppContext: () => ({ systemInfo: { is_admin: true } }),
}))

vi.mock('../../contexts/ToastContext', () => ({
  useToast: () => ({
    showInfo: showInfoMock,
    showWarning: showWarningMock,
    showError: showErrorMock,
  }),
}))

vi.mock('../issues/ConfirmFixModal', () => ({
  ConfirmFixModal: ({
    proposal,
    onCancel,
  }: {
    proposal: { proposalId: string } | null
    onCancel: () => void
  }) => proposal ? <button type="button" onClick={onCancel}>Discard staged action</button> : null,
}))

let nextId = 0
let pending: ActionProposal[]
let history: ActionRunSummary[]

function proposal(): ActionProposal {
  nextId += 1
  return {
    proposalId: `proposal_${nextId}`,
    approvalScope: 'exact',
    actions: [{
      remediation: {
        id: 'flush_dns',
        label: 'Flush DNS',
        description: 'Clear the DNS cache',
        tier: 'auto_safe',
        admin_required: false,
        requires_restart: false,
        long_running: false,
        maintenance: true,
        batch_eligible: true,
        cancellable: true,
      },
      steps: ['Run the vetted command'],
    }],
    scanFingerprint: 'scan',
    catalogFingerprint: 'catalog',
    createdAtMs: Date.now(),
    expiresAtMs: Date.now() + 60_000,
  }
}

function completedRun(item: ActionProposal): ActionRunSummary {
  return {
    runId: `run_${item.proposalId}`,
    proposalId: item.proposalId,
    authorizationId: 'authorization_1',
    status: 'succeeded',
    actions: [{
      remediationId: 'flush_dns',
      label: 'Flush DNS',
      cancellable: true,
      status: 'succeeded',
    }],
    approvedAtMs: Date.now(),
    completedAtMs: Date.now(),
    scanFingerprint: 'scan',
    catalogFingerprint: 'catalog',
  }
}

beforeEach(() => {
  pending = []
  history = []
  invokeMock.mockReset()
  showInfoMock.mockReset()
  showWarningMock.mockReset()
  showErrorMock.mockReset()
  invokeMock.mockImplementation((command: string, args?: { proposalId?: string }) => {
    switch (command) {
      case 'action_list_pending_proposals':
        return Promise.resolve(pending)
      case 'action_list_history':
        return Promise.resolve(history)
      case 'action_discard_proposal':
        pending = pending.filter(item => item.proposalId !== args?.proposalId)
        return Promise.resolve()
      default:
        return Promise.reject(new Error(`unexpected command ${command}`))
    }
  })
})

describe('StagedActionCards', () => {
  it('keeps a discarded proposal gone after remounting with stale message props', async () => {
    const item = proposal()
    pending = [item]
    const first = render(<StagedActionCards proposals={[item]} />)
    const review = await screen.findByRole('button', { name: 'Review & approve' })

    fireEvent.click(review)
    fireEvent.click(screen.getByRole('button', { name: 'Discard staged action' }))
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('action_discard_proposal', {
      proposalId: item.proposalId,
    }))
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Review & approve' })).toBeNull())
    first.unmount()

    render(<StagedActionCards proposals={[item]} />)
    await waitFor(() => {
      const checks = invokeMock.mock.calls.filter(call => call[0] === 'action_list_pending_proposals')
      expect(checks.length).toBeGreaterThanOrEqual(2)
    })
    expect(screen.queryByRole('button', { name: 'Review & approve' })).toBeNull()
  })

  it('shows broker history for an already consumed proposal instead of re-enabling approval', async () => {
    const item = proposal()
    history = [completedRun(item)]

    render(<StagedActionCards proposals={[item]} />)

    expect(await screen.findByText('Status: succeeded')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Review & approve' })).toBeNull()
  })

  it('restores the proposal and reports an error when discard fails', async () => {
    const item = proposal()
    pending = [item]
    invokeMock.mockImplementation((command: string) => {
      if (command === 'action_list_pending_proposals') return Promise.resolve(pending)
      if (command === 'action_list_history') return Promise.resolve([])
      if (command === 'action_discard_proposal') return Promise.reject(new Error('broker unavailable'))
      return Promise.reject(new Error(`unexpected command ${command}`))
    })
    render(<StagedActionCards proposals={[item]} />)

    fireEvent.click(await screen.findByRole('button', { name: 'Review & approve' }))
    fireEvent.click(screen.getByRole('button', { name: 'Discard staged action' }))

    expect(await screen.findByRole('button', { name: 'Review & approve' })).toBeTruthy()
    expect(showErrorMock).toHaveBeenCalledWith('Could not discard action', 'broker unavailable')
  })

  it('never enables approval when initial broker reconciliation fails', async () => {
    const item = proposal()
    invokeMock.mockImplementation((command: string) => {
      if (command === 'action_list_pending_proposals') return Promise.reject(new Error('offline'))
      if (command === 'action_list_history') return Promise.resolve([])
      return Promise.reject(new Error(`unexpected command ${command}`))
    })

    render(<StagedActionCards proposals={[item]} />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('action_list_pending_proposals'))

    expect(screen.queryByRole('button', { name: 'Review & approve' })).toBeNull()
  })
})
