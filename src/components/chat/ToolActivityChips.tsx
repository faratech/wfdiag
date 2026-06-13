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

  const running = tools.filter(t => t.status === 'running')
  const expanded = expandedId ? tools.find(t => t.callId === expandedId) : undefined

  return (
    <div className="tool-chips">
      {running.length > 0 && (
        <span className="tool-chips-label">Running: {running.map(chipLabel).join(', ')}…</span>
      )}
      <div className="tool-chip-row">
        {tools.map(tool => (
          <button
            key={tool.callId}
            type="button"
            className={`tool-chip ${tool.status}`}
            title={tool.argsSummary || tool.tool}
            onClick={() => setExpandedId(id => (id === tool.callId ? null : tool.callId))}
          >
            <i
              className={
                tool.status === 'running'
                  ? 'fa-solid fa-circle-notch fa-spin'
                  : tool.status === 'done'
                    ? 'fa-solid fa-check'
                    : 'fa-solid fa-xmark'
              }
              aria-hidden="true"
            />
            {chipLabel(tool)}
            {tool.durationMs !== undefined && tool.status !== 'running' && (
              <span className="tool-chip-ms">{(tool.durationMs / 1000).toFixed(1)}s</span>
            )}
          </button>
        ))}
      </div>
      {expanded?.resultPreview && (
        <pre className="tool-chip-detail">{expanded.resultPreview}</pre>
      )}
    </div>
  )
}
