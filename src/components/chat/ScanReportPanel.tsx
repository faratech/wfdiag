import React from 'react'
import { useScanReport, type ScanReportState } from '../../hooks/useScanReport'
import { renderMarkdownLite } from '../../utils/markdownLite'

/** Presentational report workspace used by the app-lifetime AI provider. */
export const ScanReportPanelView: React.FC<{
  state: ScanReportState
  available?: boolean
  loading?: boolean
  onConfigure?: () => void
  onPrepare?: () => void
  preparation?: { status: 'idle' | 'waiting' | 'running' | 'failed'; error?: string }
  onRetryPrepare?: () => void
  onCancelPrepare?: () => void
}> = ({
  state,
  available = true,
  loading = false,
  onConfigure,
  onPrepare,
  preparation = { status: 'idle' },
  onRetryPrepare,
  onCancelPrepare,
}) => {
  const { report, generating, cancelling, error, hasResults, aiEnabled, generate, cancel, copy } = state
  const canGenerate = aiEnabled && available && !loading

  return (
    <div className="wf-block scan-report-panel">
      <header className="wf-block-header scan-report-header">
        <div>
          <span>Latest scan report</span>
          <small>AI summary of health, priorities, and practical next steps</small>
        </div>
        {report && !generating && (
          <div className="count report-actions">
            <button className="btn ghost" aria-label="Copy report" title="Copy report" onClick={() => { void copy() }}>
              <i className="fa-solid fa-copy" aria-hidden="true" /> Copy
            </button>
            <button className="btn ghost" aria-label="Regenerate report" title="Regenerate" disabled={!canGenerate} onClick={() => { void generate(true) }}>
              <i className="fa-solid fa-rotate-right" aria-hidden="true" /> Regenerate
            </button>
          </div>
        )}
        {(generating || cancelling) && (
          <button className="btn ghost count" type="button" disabled={cancelling} onClick={() => { void cancel() }}>
            <i className={`fa-solid ${cancelling ? 'fa-circle-notch fa-spin' : 'fa-stop'}`} aria-hidden="true" /> {cancelling ? 'Stopping…' : 'Stop'}
          </button>
        )}
      </header>

      <div className="scan-report-body">
        {!hasResults && !report ? (
          <div className="ai-empty-state">
            <i className={`fa-solid ${preparation.status === 'failed' ? 'fa-triangle-exclamation' : preparation.status === 'idle' ? 'fa-stethoscope' : 'fa-circle-notch fa-spin'}`} aria-hidden="true" />
            <h2>
              {preparation.status === 'failed'
                ? 'Quick Scan did not finish'
                : preparation.status === 'idle'
                  ? onPrepare ? 'Create the first scan report' : 'Run a scan first'
                  : 'Running Quick Scan…'}
            </h2>
            <p>
              {preparation.status === 'failed'
                ? preparation.error
                : preparation.status === 'idle'
                  ? 'A Quick Scan will run first, then the AI report will generate automatically.'
                  : 'Collecting the diagnostic evidence needed for the report.'}
            </p>
            {preparation.status === 'idle' && onPrepare && (
              <button className="btn primary" type="button" onClick={onPrepare}>Run Quick Scan &amp; Generate</button>
            )}
            {preparation.status === 'failed' && onRetryPrepare && (
              <button className="btn primary" type="button" onClick={onRetryPrepare}>Retry Quick Scan</button>
            )}
            {preparation.status !== 'idle' && onCancelPrepare && (
              <button className="btn" type="button" onClick={onCancelPrepare}>Cancel</button>
            )}
          </div>
        ) : !aiEnabled && !report ? (
          <div className="ai-empty-state">
            <i className="fa-solid fa-power-off" aria-hidden="true" />
            <h2>AI insights are turned off</h2>
            <p>Enable AI insights in Settings to create a report.</p>
          </div>
        ) : loading && !report ? (
          <div className="ai-empty-state" role="status">
            <i className="fa-solid fa-circle-notch fa-spin" aria-hidden="true" />
            <h2>Checking AI provider…</h2>
            <p>Report actions will be available when the provider check finishes.</p>
          </div>
        ) : !available && !report ? (
          <div className="ai-empty-state">
            <i className="fa-solid fa-plug-circle-xmark" aria-hidden="true" />
            <h2>Connect an AI provider</h2>
            <p>Configure a local, subscription, or API provider before generating a report.</p>
            {onConfigure && <button className="btn primary" type="button" onClick={onConfigure}>Configure AI</button>}
          </div>
        ) : error ? (
          <div className="ai-empty-state chat-error" role="alert">
            <i className="fa-solid fa-triangle-exclamation" aria-hidden="true" />
            <h2>Report could not be generated</h2>
            <p>{error}</p>
            <button className="btn" disabled={!canGenerate} onClick={() => { void generate(true) }}>Try again</button>
          </div>
        ) : report || generating || cancelling ? (
          <article
            className={`report-body scan-report-content${generating ? ' streaming' : ''}`}
            aria-busy={generating || cancelling}
            aria-live="polite"
            aria-label="AI scan report"
          >
            {report
              ? renderMarkdownLite(report)
              : (
                <div className="report-loading" role="status">
                  <i className="fa-solid fa-circle-notch fa-spin" aria-hidden="true" />
                  <span>{cancelling ? 'Stopping report…' : 'Reading the latest scan…'}</span>
                </div>
              )}
          </article>
        ) : (
          <div className="ai-empty-state">
            <i className="fa-solid fa-file-circle-check" aria-hidden="true" />
            <h2>Turn scan data into a clear plan</h2>
            <p>Get a concise health summary, prioritized concerns, and suggested next steps.</p>
            <button className="btn primary" disabled={!canGenerate} onClick={() => { void generate() }}>
              <i className="fa-solid fa-wand-magic-sparkles" aria-hidden="true" /> Generate report
            </button>
          </div>
        )}
      </div>
    </div>
  )
}

/** Standalone compatibility wrapper used by focused component tests. */
export const ScanReportPanel: React.FC = () => <ScanReportPanelView state={useScanReport()} />
