import React, { useState } from 'react'
import type { ChatToolActivity } from '../../hooks/useAIChat'

/** Short human label for a tool chip, e.g. "run: logical_disk". */
export function chipLabel(tool: ChatToolActivity): string {
  if (tool.tool === 'run_diagnostic' && tool.argsSummary) {
    return tool.argsSummary.replace(/^task_id:\s*/, 'run: ')
  }
  return tool.tool.replace(/_/g, ' ')
}

/**
 * Tool activity above an assistant bubble: spinner chips while running,
 * check/cross when finished; clicking a finished chip reveals the result
 * preview the model saw.
 */
export const ToolActivityChips: React.FC<{ tools: ChatToolActivity[] }> = ({ tools }) => {
  const [expandedId, setExpandedId] = useState<string | null>(null)
  if (tools.length === 0) return null

  const active = tools.filter(t => t.status === 'queued' || t.status === 'running' || t.status === 'cancel_requested')
  const expanded = expandedId ? tools.find(t => t.callId === expandedId) : undefined

  return (
    <div className="tool-chips" aria-label="Diagnostic tools used">
      {active.length > 0 && (
        <span className="tool-chips-label">
          {active.some(t => t.status === 'cancel_requested') ? 'Stopping' : active.some(t => t.status === 'running') ? 'Running' : 'Queued'}: {active.map(chipLabel).join(', ')}…
        </span>
      )}
      <div className="tool-chip-row">
        {tools.map(tool => (
          <button
            key={tool.callId}
            type="button"
            className={`tool-chip ${tool.status}`}
            title={tool.argsSummary || tool.tool}
            aria-expanded={expandedId === tool.callId}
            aria-controls={tool.resultPreview ? `tool-result-${tool.callId}` : undefined}
            onClick={() => setExpandedId(id => (id === tool.callId ? null : tool.callId))}
          >
            <i
              className={
                tool.status === 'running' || tool.status === 'cancel_requested'
                  ? 'fa-solid fa-circle-notch fa-spin'
                  : tool.status === 'queued'
                    ? 'fa-regular fa-clock'
                  : tool.status === 'done'
                    ? 'fa-solid fa-check'
                    : tool.status === 'cancelled'
                      ? 'fa-solid fa-ban'
                      : tool.status === 'timed_out'
                        ? 'fa-solid fa-clock-rotate-left'
                    : 'fa-solid fa-xmark'
              }
              aria-hidden="true"
            />
            {chipLabel(tool)}
            {tool.durationMs !== undefined && !['queued', 'running', 'cancel_requested'].includes(tool.status) && (
              <span className="tool-chip-ms">{(tool.durationMs / 1000).toFixed(1)}s</span>
            )}
          </button>
        ))}
      </div>
      {expanded?.resultPreview && (
        <pre id={`tool-result-${expanded.callId}`} className="tool-chip-detail">{expanded.resultPreview}</pre>
      )}
    </div>
  )
}
