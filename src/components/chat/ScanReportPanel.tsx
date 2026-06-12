import React, { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { useAppContext } from '../../contexts/AppContext'
import { useToast } from '../../contexts/ToastContext'
import { renderMarkdownLite } from '../../utils/markdownLite'
import * as logger from '../../utils/logger'

interface ReportAck { reportId: string; cached: boolean; provider: string; report?: string }
interface ReportDelta { reportId: string; text: string }
interface ReportDone { reportId: string; finishReason: string; provider: string }
interface ReportError { reportId: string; message: string }

/**
 * One-click AI health report for the current scan: streams via
 * `ai-report://*` events (instant when served from the backend cache).
 * Auto-generates when DiagnosticsScreen sets the pendingScanReport flag.
 */
export const ScanReportPanel: React.FC = () => {
  const { results, settings, pendingScanReport, setPendingScanReport } = useAppContext()
  const { showSuccess, showError } = useToast()
  const [report, setReport] = useState('')
  const [generating, setGenerating] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const reportIdRef = useRef<string | null>(null)
  const apiKeyRef = useRef<string | undefined>(settings.openAiApiKey)
  useEffect(() => { apiKeyRef.current = settings.openAiApiKey }, [settings.openAiApiKey])

  const hasResults = Object.keys(results).length > 0

  useEffect(() => {
    let disposed = false
    const unlistens: UnlistenFn[] = []
    const register = async () => {
      const resolved = await Promise.all([
        listen<ReportDelta>('ai-report://delta', event => {
          if (event.payload.reportId !== reportIdRef.current) return
          setReport(prev => prev + event.payload.text)
        }),
        listen<ReportDone>('ai-report://done', event => {
          if (event.payload.reportId !== reportIdRef.current) return
          setGenerating(false)
        }),
        listen<ReportError>('ai-report://error', event => {
          if (event.payload.reportId !== reportIdRef.current) return
          setError(event.payload.message)
          setGenerating(false)
        }),
      ])
      if (disposed) {
        resolved.forEach(u => u())
        return
      }
      unlistens.push(...resolved)
    }
    void register()
    return () => {
      disposed = true
      unlistens.forEach(u => u())
    }
  }, [])

  const generate = useCallback(async () => {
    setError(null)
    setReport('')
    setGenerating(true)
    try {
      const ack = await invoke<ReportAck>('ai_generate_report', {
        previousScanId: null,
        apiKey: apiKeyRef.current || null,
      })
      reportIdRef.current = ack.reportId
      if (ack.report) {
        // Cache hit — full text, no events coming
        setReport(ack.report)
        setGenerating(false)
      }
    } catch (err) {
      setError(String(err))
      setGenerating(false)
    }
  }, [])

  // "Explain this scan" deep-link: consume the flag and generate
  useEffect(() => {
    if (pendingScanReport) {
      setPendingScanReport(false)
      if (hasResults && !generating) {
        void generate()
      }
    }
  }, [pendingScanReport, setPendingScanReport, hasResults, generating, generate])

  const copy = async () => {
    try {
      await writeText(report)
      showSuccess('Copied', 'Report copied to clipboard')
    } catch (err) {
      logger.error('ScanReportPanel', 'Failed to copy report', String(err))
      showError('Copy failed', 'Could not copy the report to the clipboard')
    }
  }

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
            <button className="btn ghost" title="Regenerate" onClick={() => { void generate() }}>
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
        ) : error ? (
          <div className="chat-error" role="alert">
            <i className="fa-solid fa-triangle-exclamation" aria-hidden="true" /> {error}
            <div style={{ marginTop: 8 }}>
              <button className="btn" onClick={() => { void generate() }}>Try again</button>
            </div>
          </div>
        ) : report || generating ? (
          <div className={`report-body${generating ? ' streaming' : ''}`} aria-busy={generating}>
            {report
              ? renderMarkdownLite(report)
              : <span style={{ color: 'var(--wf-text-muted)' }}><i className="fa-solid fa-circle-notch fa-spin" aria-hidden="true" /> Reading scan data…</span>}
          </div>
        ) : (
          <button className="btn primary" onClick={() => { void generate() }}>
            <i className="fa-solid fa-wand-magic-sparkles" aria-hidden="true" /> Explain this scan
          </button>
        )}
      </div>
    </div>
  )
}
