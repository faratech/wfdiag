import React, { useState, useEffect, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  useAppContext,
  type ActionProposal,
  type ActionRequest,
  type ActionRunSummary,
  type Issue,
  type RemediationSummary,
} from '../contexts/AppContext'
import { useAIContext } from '../contexts/AIContext'
import { useDiagnostics } from '../hooks/useDiagnostics'
import { useToast } from '../contexts/ToastContext'
import { useScanner } from '../hooks/useScanner'
import { EmptyState, Button } from '../components/ui'
import { ConfirmFixModal } from '../components/issues/ConfirmFixModal'
import { renderMarkdownLite } from '../utils/markdownLite'
import * as logger from '../utils/logger'

interface FixPlanEntry { issue_id: string; remediation_id: string; rationale: string; tier: RemediationSummary['tier'] }
interface FixPlan {
  entries: FixPlanEntry[]
  notes: string
  scan_fingerprint?: string
  catalog_fingerprint?: string
}

const terminalRun = (status: ActionRunSummary['status']) =>
  status === 'succeeded' || status === 'partial' || status === 'failed' || status === 'cancelled'

const sevClass = (s: string) =>
  ({ critical: 'critical', warning: 'warning', info: 'info', ok: 'ok' } as Record<string, string>)[s.toLowerCase()] || 'info'

const sevIcon = (s: string) =>
  ({ critical: 'fa-circle-xmark', warning: 'fa-triangle-exclamation', info: 'fa-circle-info', ok: 'fa-circle-check' } as Record<string, string>)[s.toLowerCase()] || 'fa-circle-info'

const TIER_ICON: Record<RemediationSummary['tier'], string> = {
  open_tool: 'fa-arrow-up-right-from-square',
  auto_safe: 'fa-wand-magic-sparkles',
  repair: 'fa-screwdriver-wrench',
}

export const IssuesScreen: React.FC = () => {
  const {
    issues, fixingIssue, setFixingIssue, isRunning, systemInfo,
    setPendingChatPrompt, setAIMode, setSelectedTab, settings,
  } = useAppContext()
  const { prioritizeIssues } = useAIContext()
  const { restartAsAdmin } = useDiagnostics()
  const { runQuickScan } = useScanner()
  const { showInfo, showWarning, showError } = useToast()

  const [maintenance, setMaintenance] = useState<RemediationSummary[]>([])
  const [confirming, setConfirming] = useState<ActionProposal | null>(null)
  const [activeRun, setActiveRun] = useState<ActionRunSummary | null>(null)
  const [triageState, setTriageState] = useState({ signature: '', text: '' })
  const [triageBusy, setTriageBusy] = useState(false)
  const [planState, setPlanState] = useState<{ signature: string; plan: FixPlan | null }>({ signature: '', plan: null })
  const [planBusy, setPlanBusy] = useState(false)
  const actionRequestInFlightRef = useRef(false)
  const mountedRef = useRef(true)

  useEffect(() => {
    mountedRef.current = true
    return () => { mountedRef.current = false }
  }, [])

  useEffect(() => {
    invoke<RemediationSummary[]>('get_remediations')
      .then(all => setMaintenance(all.filter(r => r.maintenance)))
      .catch(error => logger.error('IssuesScreen', 'Failed to load remediations', error))
  }, [])

  const detected = issues.filter(i => i.detected || i.status === 'detected')
  // `skipped` is retained for compatibility with older backends, but both
  // statuses mean the app could not verify the condition — never count them
  // as green/passed.
  const unknown = issues.filter(i => !i.detected && (i.status === 'unknown' || i.status === 'skipped'))
  const passed = issues.filter(i => !i.detected && i.status !== 'unknown' && i.status !== 'skipped')
  const critical = detected.filter(i => i.severity.toLowerCase() === 'critical').length
  const warnings = detected.filter(i => i.severity.toLowerCase() === 'warning').length
  const aiEnabled = settings.aiEnabled ?? true
  const remediationBusy = fixingIssue !== null || confirming !== null || (!!activeRun && !terminalRun(activeRun.status))
  const detectedIssueSignature = detected.map(i => i.id || i.title).sort().join('\n')
  const detectedIssueIds = new Set(detected.map(i => i.id).filter((id): id is string => !!id))
  const triage = triageState.signature === detectedIssueSignature ? triageState.text : ''
  const plan = planState.signature === detectedIssueSignature ? planState.plan : null

  const reportRun = useCallback((run: ActionRunSummary, proposal: ActionProposal) => {
    const results = run.actions.map(action => action.result).filter((result): result is NonNullable<typeof result> => !!result)
    const failedSteps = results.flatMap(result => result.steps.filter(step => step.status === 'failed'))
    const completedSteps = results.flatMap(result => result.steps.filter(step => step.status === 'succeeded'))
    const alreadySatisfied = results.flatMap(result => result.steps.filter(step => step.status === 'already_satisfied'))
    const restart = results.some(result => result.requires_restart)
    const scheduledRestart = proposal.actions.some(action => action.remediation.id === 'restart_system')
    if (run.status === 'cancelled') {
      showWarning('Action cancelled', 'Review the completed steps below, then re-run the relevant diagnostic.')
    } else if (run.status === 'partial') {
      const details = [
        completedSteps.length ? `Completed: ${completedSteps.map(step => step.action).join('; ')}.` : '',
        alreadySatisfied.length ? `Already satisfied: ${alreadySatisfied.map(step => step.action).join('; ')}.` : '',
        failedSteps.length ? `Could not complete: ${failedSteps.map(step => `${step.action}${step.detail ? ` (${step.detail})` : ''}`).join('; ')}.` : '',
        restart ? 'Restart Windows, then re-run the relevant diagnostic.' : 'Re-run the relevant diagnostic.',
      ].filter(Boolean).join(' ')
      showWarning(restart ? 'Partly completed — restart required' : 'Action partly completed', details)
    } else if (run.status === 'failed') {
      const message = run.actions.find(action => action.error)?.error || results.find(result => result.message)?.message || 'The action could not be applied.'
      showWarning('Action did not complete', message)
    } else if (scheduledRestart) {
      showInfo('Restart scheduled', 'Windows will restart in 60 seconds. Save your work now; run “shutdown /a” to cancel.')
    } else if (proposal.actions.length === 1 && proposal.actions[0].remediation.tier === 'open_tool') {
      showInfo('Tool opened', 'Complete the action in the Windows tool, then re-run the relevant diagnostic.')
    } else {
      showInfo(
        restart ? 'Done — restart required' : proposal.actions.length > 1 ? 'Actions completed' : 'Fix applied',
        restart ? 'Restart Windows, then re-run the relevant diagnostic.' : 'Re-run the relevant diagnostic to verify the current state.'
      )
    }
  }, [showInfo, showWarning])

  const monitorRun = useCallback(async (initial: ActionRunSummary, proposal: ActionProposal) => {
    let current = initial
    if (mountedRef.current) setActiveRun(current)
    while (!terminalRun(current.status)) {
      await new Promise(resolve => setTimeout(resolve, 250))
      if (!mountedRef.current) return
      current = await invoke<ActionRunSummary>('action_get_status', { runId: current.runId })
      if (mountedRef.current) setActiveRun(current)
    }
    if (mountedRef.current) reportRun(current, proposal)
  }, [reportRun])

  const prepareActions = useCallback(async (
    actions: ActionRequest[],
    expected?: Pick<FixPlan, 'scan_fingerprint' | 'catalog_fingerprint'>,
  ) => {
    if (actionRequestInFlightRef.current || remediationBusy) return
    actionRequestInFlightRef.current = true
    try {
      const proposal = await invoke<ActionProposal>('action_prepare', {
        request: {
          actions,
          expectedScanFingerprint: expected?.scan_fingerprint,
          expectedCatalogFingerprint: expected?.catalog_fingerprint,
        },
      })
      setConfirming(proposal)
    } catch (error) {
      showError('Could not prepare action', error instanceof Error ? error.message : String(error))
    } finally {
      actionRequestInFlightRef.current = false
    }
  }, [remediationBusy, showError])

  const handleFixClick = (remediation: RemediationSummary, issueId?: string, sourcePlan?: FixPlan) => {
    void prepareActions(
      [{ remediationId: remediation.id, ...(issueId ? { issueId } : {}) }],
      sourcePlan,
    )
  }

  const approveProposal = useCallback(async (proposalId: string) => {
    const proposal = confirming
    if (!proposal || proposal.proposalId !== proposalId || actionRequestInFlightRef.current) return
    actionRequestInFlightRef.current = true
    setConfirming(null)
    setFixingIssue(proposal.actions[0].remediation.id)
    if (proposal.actions.some(action => action.remediation.long_running)) {
      const canStop = proposal.actions.every(action => action.remediation.cancellable)
      showInfo(
        'Repair started',
        canStop
          ? 'This can take 10+ minutes. Keep the app open; you can stop it from the status panel.'
          : 'This can take 10+ minutes. Keep the app open; this repair cannot be stopped safely once it starts.'
      )
    }
    try {
      const run = await invoke<ActionRunSummary>('action_approve', { proposalId })
      await monitorRun(run, proposal)
    } catch (error) {
      logger.error('IssuesScreen', 'Failed to run authorized action', error)
      setActiveRun(null)
      showError('Action failed', error instanceof Error ? error.message : String(error))
    } finally {
      actionRequestInFlightRef.current = false
      setFixingIssue(null)
    }
  }, [confirming, monitorRun, setFixingIssue, showError, showInfo])

  const dismissProposal = useCallback(() => {
    const proposalId = confirming?.proposalId
    setConfirming(null)
    if (proposalId) {
      void invoke('action_discard_proposal', { proposalId }).catch(error =>
        logger.error('IssuesScreen', 'Failed to discard action preview', error))
    }
  }, [confirming])

  const cancelActiveRun = useCallback(async () => {
    if (!activeRun || terminalRun(activeRun.status)) return
    try {
      const run = await invoke<ActionRunSummary>('action_cancel', { runId: activeRun.runId })
      setActiveRun(run)
    } catch (error) {
      showError('Could not stop action', error instanceof Error ? error.message : String(error))
    }
  }, [activeRun, showError])

  // ---- AI: per-issue chat handoff ----
  const askAi = (issue: Issue) => {
    if (!aiEnabled) {
      showError('AI disabled', 'Enable AI insights in Settings to use AI assistance.')
      return
    }
    const query = `Help me with this issue found by a diagnostic scan:\n${JSON.stringify({
      id: issue.id,
      severity: issue.severity,
      title: issue.title,
      description: issue.description,
      recommendation: issue.recommendation,
    })}\n\nExplain what this means on my machine and walk me through fixing it.`
    setPendingChatPrompt({
      displayText: `Help me understand and fix “${issue.title}”.`,
      query,
      contextRefs: [
        ...(issue.id ? [{ kind: 'issue' as const, id: issue.id }] : []),
        ...(issue.source_tasks ?? []).map(id => ({ kind: 'diagnostic' as const, id })),
      ],
    })
    setAIMode('assistant')
    setSelectedTab('ai')
  }

  // ---- AI: triage + fix plan ----
  const runTriage = async (force = false) => {
    if (!aiEnabled) {
      showError('AI disabled', 'Enable AI insights in Settings to use AI assistance.')
      return
    }
    setTriageBusy(true)
    try {
      const signature = detectedIssueSignature
      const payload = JSON.stringify(detected.map(i => ({
        id: i.id, severity: i.severity, title: i.title, description: i.description,
      })))
      const text = await prioritizeIssues(payload, force)
      setTriageState({ signature, text })
    } catch (e) {
      showError('AI triage failed', e instanceof Error ? e.message : String(e))
    } finally {
      setTriageBusy(false)
    }
  }

  const runPlan = async () => {
    if (!aiEnabled) {
      showError('AI disabled', 'Enable AI insights in Settings to use AI assistance.')
      return
    }
    setPlanBusy(true)
    const signature = detectedIssueSignature
    setPlanState({ signature, plan: null })
    try {
      const result = await invoke<FixPlan>('ai_propose_fix_plan')
      setPlanState({ signature, plan: result })
    } catch (e) {
      showError('AI fix plan failed', e instanceof Error ? e.message : String(e))
    } finally {
      setPlanBusy(false)
    }
  }

  const remediationById = (id: string): RemediationSummary | undefined =>
    maintenance.find(r => r.id === id) ||
    issues.map(i => i.remediation).find(r => r?.id === id) || undefined
  const visiblePlanEntries = plan?.entries.filter(entry => detectedIssueIds.has(entry.issue_id)) ?? []
  const batchPlanEntries = visiblePlanEntries
    .map(entry => ({ entry, remediation: remediationById(entry.remediation_id) }))
    .filter((item): item is { entry: FixPlanEntry; remediation: RemediationSummary } => !!item.remediation?.batch_eligible)
    .slice(0, 5)

  return (
    <>
      <div className="scrollable screen-pad">
        {issues.length === 0 && (
          <EmptyState
            icon="fa-shield-halved"
            title="No scan data yet"
            sub="Run a Quick Scan and any detected problems will appear here with one-click fixes."
            actions={<Button variant="primary" icon="fa-bolt" onClick={() => runQuickScan()} disabled={isRunning}>Quick Scan</Button>}
          />
        )}
        {issues.length > 0 && (
          <div className="issues-summary">
            {detected.length === 0
              ? unknown.length
                ? `No issues detected in completed checks · ${passed.length} checks passed · ${unknown.length} couldn’t verify`
                : `All clear — no issues detected · ${passed.length} checks passed`
              : `${detected.length} issue${detected.length === 1 ? '' : 's'} need${detected.length === 1 ? 's' : ''} attention · detected by the latest scan${critical > 0 ? ` · ${critical} critical` : ''}${warnings > 0 ? ` · ${warnings} warning${warnings === 1 ? '' : 's'}` : ''}`}
          </div>
        )}

        {activeRun && (
          <div className="wf-block" style={{ marginBottom: 12, padding: 12 }} role="status" aria-live="polite">
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <i className={`fa-solid ${terminalRun(activeRun.status) ? activeRun.status === 'succeeded' ? 'fa-circle-check' : 'fa-circle-info' : 'fa-circle-notch fa-spin'}`} aria-hidden="true" />
              <div style={{ flex: 1 }}>
                <strong>{activeRun.status.replace('_', ' ')}</strong>
                <div style={{ color: 'var(--wf-text-muted)', fontSize: 12 }}>
                  {activeRun.actions.map(action => `${action.label}: ${action.status.replace('_', ' ')}`).join(' · ')}
                </div>
              </div>
              {!terminalRun(activeRun.status) && activeRun.status !== 'cancel_requested' &&
                (activeRun.currentIndex == null || activeRun.actions[activeRun.currentIndex]?.cancellable) && (
                <button className="btn" type="button" onClick={() => { void cancelActiveRun() }}>Stop safely</button>
              )}
            </div>
          </div>
        )}

        {/* ---- AI assistance ---- */}
        {detected.length > 0 && (
          <div className="wf-block" style={{ marginBottom: 12 }}>
            <header className="wf-block-header">
              <span className="accent-bar" /><span>AI Assistance</span>
              <span className="count" style={{ display: 'flex', gap: 6 }}>
                <button className="btn ghost" disabled={triageBusy || !aiEnabled} onClick={() => { void runTriage(!!triage) }}>
                  {triageBusy ? <i className="fa-solid fa-circle-notch fa-spin" /> : <i className="fa-solid fa-ranking-star" />} Prioritize
                </button>
                <button className="btn ghost" disabled={planBusy || !aiEnabled} onClick={() => { void runPlan() }}>
                  {planBusy ? <i className="fa-solid fa-circle-notch fa-spin" /> : <i className="fa-solid fa-list-check" />} Propose fix plan
                </button>
              </span>
            </header>
            {(triage || triageBusy || plan) && (
              <div style={{ padding: 12, fontSize: 13 }}>
                {triageBusy && !triage && (
                  <p style={{ color: 'var(--wf-text-muted)' }}>
                    <i className="fa-solid fa-circle-notch fa-spin" /> Prioritizing issues…
                  </p>
                )}
                {triage && <div className="report-body">{renderMarkdownLite(triage)}</div>}
                {plan && (
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 8, marginTop: triage ? 10 : 0 }}>
                    {batchPlanEntries.length > 1 && (
                      <div>
                        <button
                          className="btn primary"
                          disabled={remediationBusy}
                          onClick={() => { void prepareActions(
                            batchPlanEntries.map(({ entry, remediation }) => ({ remediationId: remediation.id, issueId: entry.issue_id })),
                            plan,
                          ) }}
                        >
                          <i className="fa-solid fa-list-check" /> Review {batchPlanEntries.length} low-impact actions together
                        </button>
                      </div>
                    )}
                    {visiblePlanEntries.length === 0 && <span style={{ color: 'var(--wf-text-muted)' }}>{plan.entries.length === 0 ? (plan.notes || 'No plan entries.') : 'This plan no longer matches the current issue set.'}</span>}
                    {visiblePlanEntries.map(entry => {
                      const remediation = remediationById(entry.remediation_id)
                      const issue = issues.find(i => i.id === entry.issue_id)
                      return (
                        <div key={`${entry.issue_id}:${entry.remediation_id}`} className={`issue-card ${sevClass(issue?.severity || 'info')}`} style={{ marginBottom: 0 }}>
                          <div className="isev"><i className={`fa-solid ${sevIcon(issue?.severity || 'info')}`} /></div>
                          <div className="body">
                            <div className="title-row">
                              <h3>{issue?.title || entry.issue_id}</h3>
                              <span className="badge">{entry.tier.replace('_', ' ')}</span>
                            </div>
                            <p className="desc">{entry.rationale}</p>
                          </div>
                          <div className="actions">
                            {remediation && (
                              <button className="btn primary" disabled={remediationBusy} onClick={() => handleFixClick(remediation, entry.issue_id, plan)}>
                                {fixingIssue === remediation.id
                                  ? <><i className="fa-solid fa-spinner fa-spin" /> Running…</>
                                  : <><i className={`fa-solid ${TIER_ICON[remediation.tier]}`} /> {remediation.label}</>}
                              </button>
                            )}
                          </div>
                        </div>
                      )
                    })}
                    {plan.notes && visiblePlanEntries.length > 0 && (
                      <p style={{ margin: 0, fontSize: 12, color: 'var(--wf-text-muted)' }}>{plan.notes}</p>
                    )}
                  </div>
                )}
              </div>
            )}
          </div>
        )}

        {/* ---- Admin-gated checks notice ---- */}
        {issues.length > 0 && !systemInfo?.is_admin && (
          <div className="issue-card info">
            <div className="isev"><i className="fa-solid fa-user-shield" /></div>
            <div className="body">
              <div className="title-row"><h3>Some checks need administrator access</h3></div>
              <p className="desc">
                Crash dumps (BSOD), SMART &amp; disk health, system-file (DISM) and battery
                checks only run when the app is elevated, so they were skipped. Restart as
                administrator to include them.
              </p>
            </div>
            <div className="actions">
              <button className="btn primary" onClick={() => { void restartAsAdmin() }}>
                <i className="fa-solid fa-user-shield" /> Restart as administrator
              </button>
            </div>
          </div>
        )}

        {/* ---- Detected issues ---- */}
        {detected.map((issue, idx) => {
          const sev = issue.severity
          const id = issue.id || `${issue.title}-${idx}`
          const remediation = issue.remediation
          return (
            <div key={id} className={`issue-card ${sevClass(sev)}`}>
              <div className="isev"><i className={`fa-solid ${sevIcon(sev)}`} /></div>
              <div className="body">
                <div className="title-row"><h3>{issue.title}</h3></div>
                <p className="desc">{issue.description}</p>
                {issue.recommendation && (
                  <div className="reco"><strong>Recommended:</strong> {issue.recommendation}</div>
                )}
                <div className="chips">
                  <span className="chip"><i className="fa-solid fa-stethoscope" />{issue.category}</span>
                  <span className="chip sev">{sev}</span>
                </div>
              </div>
              <div className="actions">
                {remediation && (
                  <button className="btn primary" disabled={remediationBusy} onClick={() => handleFixClick(remediation, issue.id)}>
                    {fixingIssue === remediation.id
                      ? <><i className="fa-solid fa-spinner fa-spin" /> Running…</>
                      : <><i className={`fa-solid ${TIER_ICON[remediation.tier]}`} /> {remediation.label}</>}
                  </button>
                )}
                <button className="btn ghost" disabled={!aiEnabled} onClick={() => askAi(issue)}>
                  <i className="fa-solid fa-comment-dots" /> Ask AI
                </button>
              </div>
            </div>
          )
        })}

        {/* ---- Passed checks (collapsed; 28 specs would flood the screen) ---- */}
        {passed.length > 0 && (
          <details className="wf-block" style={{ marginTop: 12, padding: '10px 14px' }}>
            <summary style={{ cursor: 'pointer', fontSize: 13, fontWeight: 600 }}>
              {passed.length} checks passed
            </summary>
            <div style={{ paddingTop: 8 }}>
              {passed.map((issue, idx) => (
                <div key={issue.id || idx} style={{ display: 'flex', gap: 8, alignItems: 'baseline', padding: '4px 0', fontSize: 12.5, borderBottom: '1px solid var(--hairline)' }}>
                  <i className="fa-solid fa-circle-check" style={{ color: 'var(--wf-success-feature)', fontSize: 11 }} />
                  <strong>{issue.title}</strong>
                  <span style={{ color: 'var(--wf-text-muted)' }}>{issue.description}</span>
                </div>
              ))}
            </div>
          </details>
        )}

        {unknown.length > 0 && (
          <details className="wf-block" style={{ marginTop: 12, padding: '10px 14px' }}>
            <summary style={{ cursor: 'pointer', fontSize: 13, fontWeight: 600 }}>
              Couldn’t verify ({unknown.length})
            </summary>
            <div style={{ paddingTop: 8 }}>
              {unknown.map((issue, idx) => (
                <div key={issue.id || idx} style={{ display: 'flex', gap: 8, alignItems: 'baseline', padding: '4px 0', fontSize: 12.5, borderBottom: '1px solid var(--hairline)' }}>
                  <i className="fa-solid fa-circle-minus" style={{ color: 'var(--wf-text-muted)', fontSize: 11 }} />
                  <strong>{issue.title}</strong>
                  <span style={{ color: 'var(--wf-text-muted)' }}>{issue.description}</span>
                </div>
              ))}
            </div>
          </details>
        )}

        {/* ---- Maintenance (always-available cleanups & repairs) ---- */}
        {maintenance.length > 0 && (
          <div className="wf-block" style={{ marginTop: 12 }}>
            <header className="wf-block-header"><span className="accent-bar" /><span>Maintenance</span></header>
            <div style={{ padding: 12, display: 'flex', flexDirection: 'column', gap: 6 }}>
              {maintenance.map(remediation => (
                <div key={remediation.id} style={{ display: 'flex', gap: 10, alignItems: 'center', padding: '6px 0', borderBottom: '1px solid var(--hairline)' }}>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontWeight: 600, fontSize: 13 }}>
                      {remediation.label}
                      {remediation.tier === 'repair' && <span className="badge" style={{ marginLeft: 6 }}>repair</span>}
                      {remediation.admin_required && <span className="badge" style={{ marginLeft: 6 }}>admin</span>}
                    </div>
                    <div style={{ fontSize: 11.5, color: 'var(--wf-text-muted)' }}>{remediation.description}</div>
                  </div>
                  <button className="btn" disabled={remediationBusy} onClick={() => handleFixClick(remediation)}>
                    {fixingIssue === remediation.id
                      ? <><i className="fa-solid fa-spinner fa-spin" /> Running…</>
                      : 'Run'}
                  </button>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>

      <ConfirmFixModal
        proposal={confirming}
        isAdmin={!!systemInfo?.is_admin}
        onCancel={dismissProposal}
        onRestartAsAdmin={() => { dismissProposal(); void restartAsAdmin() }}
        onConfirm={proposalId => { void approveProposal(proposalId) }}
      />
    </>
  )
}
