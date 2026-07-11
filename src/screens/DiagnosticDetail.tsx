import React, { useState } from 'react'
import type { DiagnosticTask, TaskResult } from '../contexts/AppContext'
import { useAIContext, diagnosticCacheKey, type AIAnalysisMeta } from '../contexts/AIContext'
import { taskIcon } from '../ui/diagnostic-icons'
import { formatDuration, parseOutput, toKeyValues } from './util'
import * as logger from '../utils/logger'
import { renderMarkdownLite } from '../utils/markdownLite'

export interface DiagItem extends DiagnosticTask {
  result: TaskResult
}

// Grounding sources come from an external search/grounding API response, not
// our own code — same convention as the backend's open_url command (only
// http/https/mailto are allowed) before a URL is used as a clickable link.
function isSafeHttpUrl(url: string): boolean {
  try {
    const parsed = new URL(url)
    return parsed.protocol === 'http:' || parsed.protocol === 'https:'
  } catch {
    return false
  }
}

export const DiagnosticDetail: React.FC<{ item: DiagItem }> = ({ item }) => {
  const [tab, setTab] = useState<'output' | 'raw'>('output')
  const [aiOpen, setAiOpen] = useState(true)
  const [feedback, setFeedback] = useState<'up' | 'down' | null>(null)
  const {
    analyzeDiagnostic,
    isAnalyzing,
    interpretations,
    analysisMeta,
    errors,
    isAIAvailable,
    aiEnabled,
    activeProvider,
  } = useAIContext()

  const success = item.result.success
  const parsed = parseOutput(item.result.output)
  const kv = parsed ? toKeyValues(parsed) : []
  // Must match AIContext's internal key (content hash included) or the
  // spinner/result/error land under a key this component never reads
  const cacheKey = diagnosticCacheKey(item.id, item.result.output)
  const aiText = interpretations[cacheKey]
  const aiMeta = analysisMeta[cacheKey]
  const aiBusy = !!isAnalyzing[cacheKey]
  const aiErr = errors[cacheKey]

  // Tab resets to 'output' when a different diagnostic is selected because the
  // parent remounts this component with key={item.id} (see DiagnosticsScreen).

  const runAi = (forceRefresh = false) => {
    if ((!aiText || forceRefresh) && !aiBusy) {
      if (forceRefresh) setFeedback(null)
      // Failures surface via errors[cacheKey]; the catch only silences the
      // duplicate unhandled-rejection noise
      analyzeDiagnostic(item.id, item.name, item.result.output, forceRefresh).catch(err =>
        logger.error('DiagnosticDetail', 'AI interpretation failed', String(err)))
    }
  }

  const providerLabel = providerLabelFor(aiMeta?.provider_used ?? activeProvider)

  return (
    <div className="diag-detail">
      <div className="diag-detail-head">
        <div className="head-tile"><i className={`fa-solid ${taskIcon(item.id, item.category)}`} /></div>
        <div className="head-text">
          <h2>{item.name}</h2>
          <div className="diag-meta">{item.category}{item.result.duration_ms > 0 ? ` · completed in ${formatDuration(item.result.duration_ms)}` : ''}</div>
        </div>
        <div className="seg-control">
          <button className={tab === 'output' ? 'active' : ''} onClick={() => setTab('output')}>Output</button>
          <button className={tab === 'raw' ? 'active' : ''} onClick={() => setTab('raw')}>Raw</button>
        </div>
        <span className={`tag ${success ? 'success' : 'error'}`}>
          <i className={`fa-solid ${success ? 'fa-circle-check' : 'fa-circle-xmark'}`} />
          {success ? 'Passed' : 'Failed'}
        </span>
        {item.admin_required && <span className="tag warning"><i className="fa-solid fa-shield" /> Admin</span>}
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
                <div className="kv-row" key={k}>
                  <span className="k">{k}</span>
                  <span className="v">{v}</span>
                </div>
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
                <span>AI Analysis · {providerLabel}</span>
                <div className="ai-panel-actions" style={{ marginLeft: 'auto' }}>
                  {aiText && (
                    <>
                      <button
                        className={`btn-icon ${feedback === 'up' ? 'active' : ''}`}
                        onClick={() => setFeedback(value => value === 'up' ? null : 'up')}
                        title="Good analysis"
                        aria-label="Good analysis"
                        aria-pressed={feedback === 'up'}
                      >
                        <i className="fa-solid fa-thumbs-up" />
                      </button>
                      <button
                        className={`btn-icon ${feedback === 'down' ? 'active' : ''}`}
                        onClick={() => setFeedback(value => value === 'down' ? null : 'down')}
                        title="Bad analysis"
                        aria-label="Bad analysis"
                        aria-pressed={feedback === 'down'}
                      >
                        <i className="fa-solid fa-thumbs-down" />
                      </button>
                      <button
                        className="btn-icon"
                        onClick={() => runAi(true)}
                        disabled={aiBusy}
                        title="Retry analysis"
                        aria-label="Retry analysis"
                      >
                        <i className={`fa-solid ${aiBusy ? 'fa-circle-notch fa-spin' : 'fa-rotate-right'}`} />
                      </button>
                    </>
                  )}
                  <button className="btn-icon" onClick={() => setAiOpen(false)} title="Collapse" aria-label="Collapse AI analysis"><i className="fa-solid fa-minus" /></button>
                </div>
              </div>
              <div className="ai-body">
                {aiErr && <p style={{ color: 'var(--err-fg)' }}>{aiErr}</p>}
                {!aiErr && aiBusy && <AIActivity providerLabel={providerLabel} />}
                {!aiErr && !aiBusy && aiMeta && <AITrace meta={aiMeta} />}
                {!aiErr && !aiBusy && aiText && <div className="report-body">{renderMarkdownLite(aiText)}</div>}
                {!aiErr && !aiBusy && !aiText && (
                  <button className="btn" onClick={() => runAi()}><i className="fa-solid fa-wand-magic-sparkles" /> Interpret this diagnostic</button>
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

function providerLabelFor(provider: string): string {
  if (provider === 'phi_silica') return 'on-device (Phi Silica)'
  if (provider === 'foundry_local') return 'local (Foundry)'
  if (provider === 'ollama') return 'local (Ollama)'
  if (provider === 'custom_openai') return 'custom API'
  if (provider === 'codex_cli') return 'subscription (Codex CLI)'
  if (provider === 'claude_code') return 'subscription (Claude Code)'
  if (provider === 'none') return 'not configured'
  return 'cloud'
}

const AIActivity: React.FC<{ providerLabel: string }> = ({ providerLabel }) => (
  <div className="ai-activity" aria-live="polite">
    <span><i className="fa-solid fa-circle-notch fa-spin" /> Fetching live grounding through WindowsForum MCP</span>
    <span><i className="fa-solid fa-brain" /> Reasoning with {providerLabel}</span>
  </div>
)

const AITrace: React.FC<{ meta: AIAnalysisMeta }> = ({ meta }) => {
  const grounding = meta.grounding
  return (
    <div className="ai-trace">
      <div className="ai-trace-row">
        <span><i className="fa-solid fa-brain" /> Reasoning summary</span>
        <span>{meta.cached ? 'cached response' : 'fresh response'}</span>
      </div>
      <p>
        Compared the diagnostic values with available live grounding, then generated the final answer.
        Raw private model reasoning is not exposed by providers.
      </p>
      {grounding && (
        <details open>
          <summary>
            <i className="fa-solid fa-screwdriver-wrench" /> Tool call: WindowsForum MCP grounding
            <span>{grounding.source_count} source{grounding.source_count === 1 ? '' : 's'}</span>
          </summary>
          {grounding.query && <div className="ai-trace-query">Query: {grounding.query}</div>}
          {grounding.error && <div className="ai-trace-error">{grounding.error}</div>}
          {grounding.sources.length > 0 && (
            <ul>
              {grounding.sources.map((source, index) => (
                <li key={`${source.url ?? source.title}-${index}`}>
                  {source.url && isSafeHttpUrl(source.url) ? (
                    <a href={source.url} target="_blank" rel="noreferrer">{source.title}</a>
                  ) : (
                    <span>{source.title}</span>
                  )}
                  <small>{source.source}</small>
                </li>
              ))}
            </ul>
          )}
        </details>
      )}
    </div>
  )
}
