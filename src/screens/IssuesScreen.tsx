import React, { useState, useEffect, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useAppContext, type Issue, type RemediationSummary } from '../contexts/AppContext'
import { useAIContext } from '../contexts/AIContext'
import { useDiagnostics } from '../hooks/useDiagnostics'
import { useToast } from '../contexts/ToastContext'
import { useScanner } from '../hooks/useScanner'
import { EmptyState, Button } from '../components/ui'
import { ConfirmFixModal } from '../components/issues/ConfirmFixModal'
import { renderMarkdownLite } from '../utils/markdownLite'
import * as logger from '../utils/logger'

interface FixResultWire { success: boolean; message?: string; requires_restart?: boolean }
type FixOutcome =
  | { status: 'needs_confirmation'; remediation: RemediationSummary }
  | { status: 'completed'; result: FixResultWire }

interface FixPlanEntry { issue_id: string; remediation_id: string; rationale: string; tier: RemediationSummary['tier'] }
interface FixPlan { entries: FixPlanEntry[]; notes: string }

const sevClass = (s: string) =>
  ({ critical: 'critical', warning: 'warning', info: 'info', ok: 'ok' } as Record<string, string>)[s.toLowerCase()] || 'info'

const TIER_ICON: Record<RemediationSummary['tier'], string> = {
  open_tool: 'fa-arrow-up-right-from-square',
  auto_safe: 'fa-wand-magic-sparkles',
  repair: 'fa-screwdriver-wrench',
}

export const IssuesScreen: React.FC = () => {
  const {
    issues, results, fixingIssue, setFixingIssue, isRunning, systemInfo,
    setPendingChatPrompt, setSelectedTab, settings,
  } = useAppContext()
  const { prioritizeIssues } = useAIContext()
  const { detectIssues, restartAsAdmin } = useDiagnostics()
  const { runQuickScan } = useScanner()
  const { showInfo, showWarning, showError } = useToast()

  const [maintenance, setMaintenance] = useState<RemediationSummary[]>([])
  const [confirming, setConfirming] = useState<RemediationSummary | null>(null)
  const [triageState, setTriageState] = useState({ signature: '', text: '' })
  const [triageBusy, setTriageBusy] = useState(false)
  const [planState, setPlanState] = useState<{ signature: string; plan: FixPlan | null }>({ signature: '', plan: null })
  const [planBusy, setPlanBusy] = useState(false)
  const remediationInFlightRef = useRef(false)

  useEffect(() => {
    invoke<RemediationSummary[]>('get_remediations')
      .then(all => setMaintenance(all.filter(r => r.maintenance)))
      .catch(error => logger.error('IssuesScreen', 'Failed to load remediations', error))
  }, [])

  const detected = issues.filter(i => i.detected || i.status === 'detected')
  const skipped = issues.filter(i => !i.detected && i.status === 'skipped')
  const passed = issues.filter(i => !i.detected && i.status !== 'skipped')
  const critical = detected.filter(i => i.severity.toLowerCase() === 'critical').length
  const warnings = detected.filter(i => i.severity.toLowerCase() === 'warning').length
  const aiEnabled = settings.aiEnabled ?? true
  const remediationBusy = fixingIssue !== null
  const detectedIssueSignature = detected.map(i => i.id || i.title).sort().join('\n')
  const detectedIssueIds = new Set(detected.map(i => i.id).filter((id): id is string => !!id))
  const triage = triageState.signature === detectedIssueSignature ? triageState.text : ''
  const plan = planState.signature === detectedIssueSignature ? planState.plan : null

  // ---- remediation execution (the backend enforces the Repair gate) ----
  const runRemediation = useCallback(async (remediation: RemediationSummary, confirmed: boolean) => {
    if (remediationInFlightRef.current) return
    remediationInFlightRef.current = true
    setFixingIssue(remediation.id)
    if (remediation.long_running) {
      showInfo('Repair started', `${remediation.label} can take 10+ minutes. Keep the app open.`)
    }
    try {
      const outcome = await invoke<FixOutcome>('run_remediation', {
        remediationId: remediation.id,
        confirmed,
      })
      if (outcome.status === 'needs_confirmation') {
        // Defensive backstop — the UI normally confirms repair tier up front
        setConfirming(outcome.remediation)
        return
      }
      const result = outcome.result
      if (!result.success) {
        showWarning('Fix did not complete', result.message || 'The fix could not be applied.')
        return
      }
      showInfo(
        result.requires_restart ? 'Done — restart required' : 'Fix applied',
        result.message || 'Re-run a scan to verify the issue is resolved.'
      )
      try { await detectIssues() } catch { /* re-detect is best-effort */ }
    } catch (e) {
      logger.error('IssuesScreen', 'Failed to run remediation', e)
      showError('Fix failed', e instanceof Error ? e.message : String(e))
    } finally {
      remediationInFlightRef.current = false
      setFixingIssue(null)
    }
  }, [setFixingIssue, showInfo, showWarning, showError, detectIssues])

  const handleFixClick = (remediation: RemediationSummary) => {
    if (remediationInFlightRef.current || fixingIssue) return
    if (remediation.admin_required && !systemInfo?.is_admin) {
      setConfirming(remediation)
      return
    }
    if (remediation.tier === 'repair') {
      setConfirming(remediation)
    } else {
      void runRemediation(remediation, false)
    }
  }

  // ---- AI: per-issue chat handoff ----
  const askAi = (issue: Issue) => {
    if (!aiEnabled) {
      showError('AI disabled', 'Enable AI insights in Settings to use AI assistance.')
      return
    }
    let prompt = `Help me with this issue found by a diagnostic scan:\n${JSON.stringify({
      id: issue.id, severity: issue.severity, title: issue.title, description: issue.description,
    })}`
    for (const taskId of issue.source_tasks ?? []) {
      const output = results[taskId]?.output
      if (output) {
        prompt += `\n\nDiagnostic data (${taskId}):\n${output.slice(0, 1500)}`
      }
      if (prompt.length > 3000) break
    }
    prompt += '\n\nExplain what this means on my machine and walk me through fixing it.'
    setPendingChatPrompt(prompt)
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
      const result = await invoke<FixPlan>('ai_propose_fix_plan', {
        apiKey: settings.openAiApiKey || null,
      })
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

  return (
    <>
      <div className="stat-strip screen-toolbar">
        <div className="stat-card brand">
          <div className="label">Active Issues</div>
          <div className="value">{detected.length}</div>
          <i className="fa-solid fa-triangle-exclamation icon" />
        </div>
        <div className="stat-card failed">
          <div className="label">Critical</div>
          <div className="value">{critical}</div>
          <i className="fa-solid fa-circle-xmark icon" />
        </div>
        <div className="stat-card warning">
          <div className="label">Warnings</div>
          <div className="value">{warnings}</div>
          <i className="fa-solid fa-triangle-exclamation icon" />
        </div>
        <div className="stat-card passed">
          <div className="label">Checks Run</div>
          <div className="value">{issues.length - skipped.length}</div>
          <i className="fa-solid fa-list-check icon" />
        </div>
      </div>

      <div className="scrollable screen-pad">
        {issues.length === 0 && (
          <div className="wf-block">
            <EmptyState
              icon="fa-shield-halved"
              title="No issues detected"
              sub="Issues are derived from the most recent scan. Run a scan to refresh the analysis."
              actions={<Button variant="primary" icon="fa-bolt" onClick={() => runQuickScan()} disabled={isRunning}>Quick Scan</Button>}
            />
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
                    {visiblePlanEntries.length === 0 && <span style={{ color: 'var(--wf-text-muted)' }}>{plan.entries.length === 0 ? (plan.notes || 'No plan entries.') : 'This plan no longer matches the current issue set.'}</span>}
                    {visiblePlanEntries.map(entry => {
                      const remediation = remediationById(entry.remediation_id)
                      const issue = issues.find(i => i.id === entry.issue_id)
                      return (
                        <div key={`${entry.issue_id}:${entry.remediation_id}`} className={`issue-card ${sevClass(issue?.severity || 'info')}`} style={{ margin: 0 }}>
                          <div className="swatch" />
                          <div className="body">
                            <div className="title-row">
                              <h3 style={{ fontSize: 13 }}>{issue?.title || entry.issue_id}</h3>
                              <span className="badge">{entry.tier.replace('_', ' ')}</span>
                            </div>
                            <p className="desc">{entry.rationale}</p>
                          </div>
                          <div className="actions">
                            {remediation && (
                              <button className="btn primary" disabled={remediationBusy} onClick={() => handleFixClick(remediation)}>
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
          <div className="issue-card info" style={{ alignItems: 'center' }}>
            <div className="swatch" />
            <div className="body">
              <div className="title-row">
                <i className="fa-solid fa-user-shield" style={{ color: 'var(--wf-paletteColor1)' }} />
                <h3>Some checks need administrator access</h3>
              </div>
              <p className="desc">
                Crash dumps (BSOD), SMART &amp; disk health, system-file (DISM) and battery
                checks only run when the app is elevated, so they were skipped. Restart as
                administrator to include them.
              </p>
            </div>
            <div className="actions" style={{ display: 'flex', flexDirection: 'column', gap: 6, alignItems: 'stretch' }}>
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
              <div className="swatch" />
              <div className="body">
                <div className="title-row">
                  <i
                    className={`fa-solid ${sev.toLowerCase() === 'critical' ? 'fa-circle-xmark' : sev.toLowerCase() === 'warning' ? 'fa-triangle-exclamation' : 'fa-circle-info'}`}
                    style={{ color: sev.toLowerCase() === 'critical' ? 'var(--err-fg)' : sev.toLowerCase() === 'warning' ? 'var(--warn-fg)' : 'var(--wf-paletteColor1)' }}
                  />
                  <h3>{issue.title}</h3>
                  <span className="badge">{sev}</span>
                  <span className="badge">{issue.category}</span>
                </div>
                <p className="desc">{issue.description}</p>
                {issue.recommendation && (
                  <div className="reco"><strong>Recommended:</strong> {issue.recommendation}</div>
                )}
              </div>
              <div className="actions" style={{ display: 'flex', flexDirection: 'column', gap: 6, alignItems: 'stretch' }}>
                {remediation && (
                  <button className="btn primary" disabled={remediationBusy} onClick={() => handleFixClick(remediation)}>
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

        {skipped.length > 0 && (
          <details className="wf-block" style={{ marginTop: 12, padding: '10px 14px' }}>
            <summary style={{ cursor: 'pointer', fontSize: 13, fontWeight: 600 }}>
              {skipped.length} checks skipped
            </summary>
            <div style={{ paddingTop: 8 }}>
              {skipped.map((issue, idx) => (
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
        remediation={confirming}
        isAdmin={!!systemInfo?.is_admin}
        onCancel={() => setConfirming(null)}
        onRestartAsAdmin={() => { setConfirming(null); void restartAsAdmin() }}
        onConfirm={remediation => {
          setConfirming(null)
          void runRemediation(remediation, true)
        }}
      />
    </>
  )
}
