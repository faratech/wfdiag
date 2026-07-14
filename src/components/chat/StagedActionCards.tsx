import React, { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  useAppContext,
  type ActionProposal,
  type ActionRunSummary,
} from '../../contexts/AppContext'
import { useToast } from '../../contexts/ToastContext'
import { ConfirmFixModal } from '../issues/ConfirmFixModal'

const terminal = (status: ActionRunSummary['status']) =>
  status === 'succeeded' || status === 'partial' || status === 'failed' || status === 'cancelled'

// Covers the short window between a successful discard click and the next
// broker reconciliation if Chat unmounts/remounts. The broker remains the
// durable authority across full app reloads.
const discardedProposalIds = new Set<string>()

export const StagedActionCards: React.FC<{ proposals: ActionProposal[] }> = ({ proposals }) => {
  const { systemInfo } = useAppContext()
  const { showInfo, showWarning, showError } = useToast()
  const proposalKey = proposals.map(proposal => proposal.proposalId).join('\u0000')
  const [confirming, setConfirming] = useState<ActionProposal | null>(null)
  const [brokerVisibility, setBrokerVisibility] = useState<{ key: string; ids: Set<string> } | null>(null)
  const [runs, setRuns] = useState<Record<string, ActionRunSummary>>({})
  const mountedRef = useRef(true)

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  // The AI workspace hook survives tab changes, but message components do
  // not. Rehydrate a run from the broker audit log and keep polling only while
  // that recovered run is active, so returning to Chat never shows a consumed
  // proposal as if it were still approvable.
  useEffect(() => {
    let disposed = false
    let timer: number | undefined
    const refresh = async () => {
      try {
        const [pending, history] = await Promise.all([
          invoke<ActionProposal[]>('action_list_pending_proposals'),
          invoke<ActionRunSummary[]>('action_list_history'),
        ])
        if (disposed) return
        const matching = history.filter(run => proposals.some(proposal => proposal.proposalId === run.proposalId))
        const matchingIds = new Set(matching.map(run => run.proposalId))
        const pendingIds = new Set(pending.map(proposal => proposal.proposalId))
        // A run is stronger evidence than the local discard tombstone: another
        // mounted view may have approved the proposal first.
        matchingIds.forEach(id => discardedProposalIds.delete(id))
        if (matching.length > 0) {
          setRuns(current => ({
            ...current,
            ...Object.fromEntries(matching.map(run => [run.proposalId, run])),
          }))
        }
        const visibleIds = new Set(proposals
          .map(proposal => proposal.proposalId)
          .filter(id => matchingIds.has(id) || (pendingIds.has(id) && !discardedProposalIds.has(id))))
        setBrokerVisibility({ key: proposalKey, ids: visibleIds })
        setConfirming(current => current && visibleIds.has(current.proposalId) ? current : null)
        if (matching.some(run => !terminal(run.status))) {
          timer = window.setTimeout(() => { void refresh() }, 500)
        }
      } catch {
        // Do not expose an approval control unless the broker has verified that
        // the immutable proposal is still pending or has a recorded run.
      }
    }
    void refresh()
    return () => {
      disposed = true
      if (timer !== undefined) window.clearTimeout(timer)
    }
  }, [proposalKey, proposals])

  const monitor = async (proposal: ActionProposal, initial: ActionRunSummary) => {
    let run = initial
    if (mountedRef.current) setRuns(current => ({ ...current, [proposal.proposalId]: run }))
    while (!terminal(run.status)) {
      await new Promise(resolve => setTimeout(resolve, 300))
      if (!mountedRef.current) return
      run = await invoke<ActionRunSummary>('action_get_status', { runId: run.runId })
      if (mountedRef.current) setRuns(current => ({ ...current, [proposal.proposalId]: run }))
    }
    if (run.status === 'succeeded') {
      showInfo('Action completed', 'Re-run the relevant diagnostic to verify the current state.')
    } else if (run.status === 'cancelled') {
      showWarning('Action cancelled', 'Review any completed steps before trying again.')
    } else {
      const detail = run.actions.find(action => action.error)?.error
        || run.actions.find(action => action.result?.message)?.result?.message
        || 'Review the action details and try the diagnostic again.'
      showWarning(run.status === 'partial' ? 'Action partly completed' : 'Action failed', detail)
    }
  }

  const approve = async (proposalId: string) => {
    const proposal = proposals.find(candidate => candidate.proposalId === proposalId)
    if (!proposal) return
    setConfirming(null)
    try {
      const run = await invoke<ActionRunSummary>('action_approve', { proposalId })
      await monitor(proposal, run)
    } catch (error) {
      showError('Could not run action', error instanceof Error ? error.message : String(error))
    }
  }

  const discard = async (proposal: ActionProposal) => {
    setConfirming(null)
    discardedProposalIds.add(proposal.proposalId)
    setBrokerVisibility(current => {
      if (current === null || current.key !== proposalKey) return current
      const next = new Set(current.ids)
      next.delete(proposal.proposalId)
      return { key: current.key, ids: next }
    })
    try {
      await invoke('action_discard_proposal', { proposalId: proposal.proposalId })
    } catch (error) {
      discardedProposalIds.delete(proposal.proposalId)
      if (mountedRef.current) {
        setBrokerVisibility(current => {
          const next = new Set(current?.key === proposalKey ? current.ids : [])
          next.add(proposal.proposalId)
          return { key: proposalKey, ids: next }
        })
        showError('Could not discard action', error instanceof Error ? error.message : String(error))
      }
    }
  }

  const cancelRun = async (run: ActionRunSummary) => {
    try {
      const updated = await invoke<ActionRunSummary>('action_cancel', { runId: run.runId })
      setRuns(current => ({ ...current, [updated.proposalId]: updated }))
    } catch (error) {
      showError('Could not stop action', error instanceof Error ? error.message : String(error))
    }
  }

  const restartAsAdmin = () => {
    void invoke('restart_as_admin').catch(error =>
      showError('Could not restart as administrator', error instanceof Error ? error.message : String(error)))
  }

  const brokerVisibleIds = brokerVisibility?.key === proposalKey ? brokerVisibility.ids : null
  const visible = brokerVisibleIds === null
    ? []
    : proposals.filter(proposal => brokerVisibleIds.has(proposal.proposalId))
  if (visible.length === 0) return null

  return (
    <div className="staged-action-list" aria-label="Staged actions awaiting review">
      {visible.map(proposal => {
        const run = runs[proposal.proposalId]
        const action = proposal.actions[0]
        const canCancel = run && !terminal(run.status) && run.status !== 'cancel_requested'
          && (run.currentIndex == null || run.actions[run.currentIndex]?.cancellable)
        return (
          <section className="staged-action-card" key={proposal.proposalId}>
            <div className="staged-action-icon"><i className="fa-solid fa-shield-halved" aria-hidden="true" /></div>
            <div className="staged-action-body">
              <strong>{action?.remediation.label ?? 'Vetted action'}</strong>
              <span>{run ? `Status: ${run.status.replace('_', ' ')}` : 'Staged only · nothing has run'}</span>
            </div>
            {run && canCancel ? (
              <button className="btn" type="button" onClick={() => { void cancelRun(run) }}>Stop safely</button>
            ) : run ? (
              <i className={`fa-solid ${terminal(run.status) ? 'fa-circle-check' : 'fa-circle-notch fa-spin'}`} aria-hidden="true" />
            ) : (
              <button className="btn primary" type="button" onClick={() => setConfirming(proposal)}>
                Review &amp; approve
              </button>
            )}
          </section>
        )
      })}
      <ConfirmFixModal
        proposal={confirming}
        isAdmin={!!systemInfo?.is_admin}
        onConfirm={proposalId => { void approve(proposalId) }}
        onCancel={() => { if (confirming) void discard(confirming) }}
        onRestartAsAdmin={restartAsAdmin}
      />
    </div>
  )
}
