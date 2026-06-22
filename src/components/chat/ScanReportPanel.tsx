import React, { useEffect } from 'react'
import { useAppContext } from '../../contexts/AppContext'
import { useScanReport } from '../../hooks/useScanReport'
import { renderMarkdownLite } from '../../utils/markdownLite'

/**
 * One-click AI health report for the current scan. State, IPC and the
 * `ai-report://*` streaming live in useScanReport; this component is the view.
 * Auto-generates when DiagnosticsScreen sets the pendingScanReport flag.
 */
export const ScanReportPanel: React.FC = () => {
  const { pendingScanReport, setPendingScanReport } = useAppContext()
  const { report, generating, error, hasResults, aiEnabled, generate, copy } = useScanReport()

  // "Explain this scan" deep-link: consume once (clear BEFORE generating so a
  // re-render can never double-fire). The effect only calls the context setter
  // and the hook's generate() — no local setState, so no cascade.
  useEffect(() => {
    if (!pendingScanReport) return
    setPendingScanReport(false)
    if (hasResults && aiEnabled && !generating) {
      void generate()
    }
  }, [pendingScanReport, setPendingScanReport, hasResults, aiEnabled, generating, generate])

  return (
    <div className="wf-block">
      <header className="wf-block-header">
        <span className="accent-bar" />
        <span>Scan Report</span>
        {report && !generating && (
          <span className="count" style={{ display: 'flex', gap: 6 }}>
            <button className="btn ghost" title="Copy report" onClick={() => { void copy() }}>
              <i className="fa-solid fa-copy" aria-hidden="true" />
            </button>
            <button className="btn ghost" title="Regenerate" disabled={!aiEnabled} onClick={() => { void generate() }}>
              <i className="fa-solid fa-rotate-right" aria-hidden="true" />
            </button>
          </span>
        )}
      </header>
      <div style={{ padding: 12, fontSize: 12.5 }}>
        {!hasResults && !report ? (
          <div style={{ color: 'var(--wf-text-muted)' }}>
            Run a scan, then generate an AI health report: top issues, what changed since the
            last scan, and what to fix first.
          </div>
        ) : !aiEnabled && !report ? (
          <div style={{ color: 'var(--wf-text-muted)' }}>AI insights are disabled in Settings.</div>
        ) : error ? (
          <div className="chat-error" role="alert">
            <i className="fa-solid fa-triangle-exclamation" aria-hidden="true" /> {error}
            <div style={{ marginTop: 8 }}>
              <button className="btn" disabled={!aiEnabled} onClick={() => { void generate() }}>Try again</button>
            </div>
          </div>
        ) : report || generating ? (
          <div className={`report-body${generating ? ' streaming' : ''}`} aria-busy={generating}>
            {report
              ? renderMarkdownLite(report)
              : <span style={{ color: 'var(--wf-text-muted)' }}><i className="fa-solid fa-circle-notch fa-spin" aria-hidden="true" /> Reading scan data…</span>}
          </div>
        ) : (
          <button className="btn primary" disabled={!aiEnabled} onClick={() => { void generate() }}>
            <i className="fa-solid fa-wand-magic-sparkles" aria-hidden="true" /> Explain this scan
          </button>
        )}
      </div>
    </div>
  )
}
