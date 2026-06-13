import React, { useState } from 'react'
import type { DiagnosticTask, TaskResult } from '../contexts/AppContext'
import { useAIContext, diagnosticCacheKey } from '../contexts/AIContext'
import { taskIcon } from '../ui/diagnostic-icons'
import { formatDuration, parseOutput, toKeyValues } from './util'
import * as logger from '../utils/logger'

export interface DiagItem extends DiagnosticTask {
  result: TaskResult
}

export const DiagnosticDetail: React.FC<{ item: DiagItem }> = ({ item }) => {
  const [tab, setTab] = useState<'output' | 'raw'>('output')
  const [aiOpen, setAiOpen] = useState(true)
  const { analyzeDiagnostic, isAnalyzing, interpretations, errors, isAIAvailable, aiEnabled, activeProvider } = useAIContext()

  const success = item.result.success
  const parsed = parseOutput(item.result.output)
  const kv = parsed ? toKeyValues(parsed) : []
  // Must match AIContext's internal key (content hash included) or the
  // spinner/result/error land under a key this component never reads
  const cacheKey = diagnosticCacheKey(item.id, item.result.output)
  const aiText = interpretations[cacheKey]
  const aiBusy = !!isAnalyzing[cacheKey]
  const aiErr = errors[cacheKey]

  // Tab resets to 'output' when a different diagnostic is selected because the
  // parent remounts this component with key={item.id} (see DiagnosticsScreen).

  const runAi = () => {
    if (!aiText && !aiBusy) {
      // Failures surface via errors[cacheKey]; the catch only silences the
      // duplicate unhandled-rejection noise
      analyzeDiagnostic(item.id, item.name, item.result.output).catch(err =>
        logger.error('DiagnosticDetail', 'AI interpretation failed', String(err)))
    }
  }

  return (
    <div className="diag-detail">
      <div className="diag-detail-head">
        <div className="title-row">
          <i className={`fa-solid ${taskIcon(item.id, item.category)}`} style={{ fontSize: 18, color: 'var(--wf-paletteColor1)' }} />
          <h2>{item.name}</h2>
          <span style={{ marginLeft: 'auto' }}>
            <span className={`tag ${success ? 'success' : 'error'}`}>
              <i className={`fa-solid ${success ? 'fa-circle-check' : 'fa-circle-xmark'}`} />
              {success ? 'Passed' : 'Failed'}
            </span>
            {item.admin_required && <span className="tag warning" style={{ marginLeft: 6 }}><i className="fa-solid fa-shield" /> Admin</span>}
          </span>
        </div>
        <div className="diag-meta">
          <span><i className="fa-solid fa-folder" /> {item.category}</span>
          {item.result.duration_ms > 0 && <span><i className="fa-solid fa-stopwatch" /> {formatDuration(item.result.duration_ms)}</span>}
        </div>
      </div>

      <div className="diag-detail-tabs">
        {(['output', 'raw'] as const).map(t => (
          <button key={t} className={`diag-tab ${tab === t ? 'active' : ''}`} onClick={() => setTab(t)}>
            <i className={`fa-solid ${t === 'output' ? 'fa-table-list' : 'fa-code'}`} />
            <span style={{ marginLeft: 4, textTransform: 'capitalize' }}>{t}</span>
          </button>
        ))}
      </div>

      <div className="diag-detail-body">
        {!success && (
          <div style={{ padding: 14, background: 'var(--err-bg)', border: '1px solid var(--err-border)', borderRadius: 6, marginBottom: 12, display: 'flex', gap: 12 }}>
            <i className="fa-solid fa-circle-exclamation" style={{ color: 'var(--err-fg)', fontSize: 18, marginTop: 2 }} />
            <div>
              <strong style={{ color: 'var(--err-fg)' }}>{item.result.error || 'Diagnostic failed'}</strong>
              <p style={{ margin: '4px 0 0', fontSize: 13, color: 'var(--wf-text-muted)' }}>
                This check could not complete. Administrator-only checks require relaunching the app elevated.
              </p>
            </div>
          </div>
        )}

        {tab === 'output' && (
          kv.length > 0 ? (
            <div className="kv-grid">
              {kv.map(([k, v]) => (
                <React.Fragment key={k}>
                  <div className="k">{k}</div>
                  <div className="v">{v}</div>
                </React.Fragment>
              ))}
            </div>
          ) : (
            <pre className="code-block">{item.result.output || '(no output)'}</pre>
          )
        )}

        {tab === 'raw' && (
          <pre className="code-block">{JSON.stringify({
            task_id: item.id,
            name: item.name,
            category: item.category,
            success,
            duration_ms: item.result.duration_ms,
            admin_required: item.admin_required,
            error: item.result.error ?? null,
            output: parsed ?? item.result.output,
          }, null, 2)}</pre>
        )}

        {tab === 'output' && aiEnabled && isAIAvailable && (
          aiOpen ? (
            <div className="ai-panel">
              <div className="ai-panel-head">
                <img src="/wf-ds/chatgpt-bot-avatar.webp" alt="" />
                <span>AI Analysis · {activeProvider === 'phi_silica' ? 'on-device (Phi Silica)' : 'cloud'}</span>
                <button className="btn-icon" style={{ marginLeft: 'auto' }} onClick={() => setAiOpen(false)} title="Collapse"><i className="fa-solid fa-minus" /></button>
              </div>
              <div className="ai-body">
                {aiErr && <p style={{ color: 'var(--err-fg)' }}>{aiErr}</p>}
                {!aiErr && aiBusy && <p><i className="fa-solid fa-circle-notch fa-spin" /> Analyzing…</p>}
                {!aiErr && !aiBusy && aiText && <div style={{ whiteSpace: 'pre-wrap' }}>{aiText}</div>}
                {!aiErr && !aiBusy && !aiText && (
                  <button className="btn" onClick={runAi}><i className="fa-solid fa-wand-magic-sparkles" /> Interpret this diagnostic</button>
                )}
              </div>
            </div>
          ) : (
            <button className="btn" style={{ marginTop: 12 }} onClick={() => setAiOpen(true)}>
              <img src="/wf-ds/chatgpt-bot-avatar.webp" alt="" style={{ width: 16, height: 16, borderRadius: '50%' }} />
              Show AI analysis
            </button>
          )
        )}
      </div>
    </div>
  )
}
