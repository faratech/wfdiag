import React from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useAppContext } from '../contexts/AppContext'
import { useDiagnostics } from '../hooks/useDiagnostics'
import { useToast } from '../contexts/ToastContext'

const sevClass = (s: string) =>
  ({ critical: 'critical', warning: 'warning', info: 'info', ok: 'ok' } as Record<string, string>)[s.toLowerCase()] || 'info'

export const IssuesScreen: React.FC = () => {
  const { issues, fixingIssue, setFixingIssue } = useAppContext()
  const { detectIssues, restartAsAdmin } = useDiagnostics()
  const { showSuccess, showError } = useToast()

  const detected = issues.filter(i => i.detected)
  const critical = issues.filter(i => i.severity.toLowerCase() === 'critical').length
  const warnings = issues.filter(i => i.severity.toLowerCase() === 'warning').length

  const handleFix = async (id: string) => {
    setFixingIssue(id)
    try {
      await invoke('fix_issue', { issueId: id })
      showSuccess('Fix applied', 'Re-running issue detection…')
      await detectIssues()
    } catch (e) {
      showError('Fix failed', String(e))
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
          <div className="wf-block" style={{ padding: 24, textAlign: 'center', color: 'var(--wf-text-muted)' }}>
            <i className="fa-solid fa-shield-halved" style={{ fontSize: 28, color: 'var(--ok-fg)' }} />
            <p>No issues detected. Run a scan to refresh.</p>
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
                {issue.id ? (
                  <button className="btn primary" disabled={fixingIssue === issue.id} onClick={() => handleFix(issue.id!)}>
                    {fixingIssue === issue.id
                      ? <><i className="fa-solid fa-spinner fa-spin" /> Fixing…</>
                      : <><i className="fa-solid fa-wand-magic-sparkles" /> Fix</>}
                  </button>
                ) : (
                  <button className="btn" onClick={() => restartAsAdmin()}><i className="fa-solid fa-shield-halved" /> Restart as Admin</button>
                )}
              </div>
            </div>
          )
        })}
      </div>
    </>
  )
}
