import React, { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useAppContext } from '../contexts/AppContext'
import { useDiagnostics } from '../hooks/useDiagnostics'
import { useToast } from '../contexts/ToastContext'
import { useScanner } from '../hooks/useScanner'
import { EmptyState, Button } from '../components/ui'
import * as logger from '../utils/logger'

interface FixResult { success: boolean; message?: string }

const sevClass = (s: string) =>
  ({ critical: 'critical', warning: 'warning', info: 'info', ok: 'ok' } as Record<string, string>)[s.toLowerCase()] || 'info'

export const IssuesScreen: React.FC = () => {
  const { issues, fixingIssue, setFixingIssue, isRunning } = useAppContext()
  const { detectIssues, restartAsAdmin } = useDiagnostics()
  const { runQuickScan } = useScanner()
  const { showInfo, showWarning, showError } = useToast()

  // IDs the backend can actually fix — used to gate the "Fix" button so it never appears
  // for issues with no fixer (which would silently no-op while showing a false success).
  const [fixableIds, setFixableIds] = useState<string[]>([])
  useEffect(() => {
    invoke<string[]>('get_fixable_issue_ids')
      .then(setFixableIds)
      .catch(error => logger.error('IssuesScreen', 'Failed to load fixable issue ids', error))
  }, [])

  const detected = issues.filter(i => i.detected)
  const critical = issues.filter(i => i.severity.toLowerCase() === 'critical').length
  const warnings = issues.filter(i => i.severity.toLowerCase() === 'warning').length

  const handleFix = async (id: string) => {
    setFixingIssue(id)

    let result: FixResult
    try {
      result = await invoke<FixResult>('fix_issue', { issueId: id })
    } catch (e) {
      logger.error('IssuesScreen', 'Failed to fix issue', e)
      showError('Fix failed', e instanceof Error ? e.message : String(e))
      setFixingIssue(null)
      return
    }

    if (!result.success) {
      // Surface the failure instead of falsely reporting success (the old behavior).
      showWarning('No automated fix', result.message || 'This issue cannot be fixed automatically.')
      setFixingIssue(null)
      return
    }

    // Fix launched. Most detectors re-read the stored scan snapshot rather than live state,
    // so advise a re-scan; keep the spinner until re-detection completes.
    showInfo('Fix applied', result.message || 'Re-run a diagnostic scan to verify the issue is resolved.')
    try {
      await detectIssues()
    } catch (e) {
      logger.error('IssuesScreen', 'Failed to detect issues', e)
    } finally {
      setFixingIssue(null)
    }
  }

  return (
    <>
      <div className="stat-strip" style={{ padding: '0 24px 12px' }}>
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
          <div className="label">Total Detected</div>
          <div className="value">{issues.length}</div>
          <i className="fa-solid fa-list-check icon" />
        </div>
      </div>

      <div className="scrollable" style={{ padding: '0 24px 24px' }}>
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
        {issues.map((issue, idx) => {
          const sev = issue.severity
          const id = issue.id || `${issue.title}-${idx}`
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
              <div className="actions">
                {issue.id && fixableIds.includes(issue.id) ? (
                  <button className="btn primary" disabled={fixingIssue === issue.id} onClick={() => handleFix(issue.id!)}>
                    {fixingIssue === issue.id
                      ? <><i className="fa-solid fa-spinner fa-spin" /> Fixing…</>
                      : <><i className="fa-solid fa-wand-magic-sparkles" /> Fix</>}
                  </button>
                ) : !issue.id ? (
                  <button className="btn" onClick={() => restartAsAdmin()}><i className="fa-solid fa-shield-halved" /> Restart as Admin</button>
                ) : null}
              </div>
            </div>
          )
        })}
      </div>
    </>
  )
}
