export function formatDuration(ms: number): string {
  if (!ms || ms < 0) return '—'
  if (ms < 1000) return `${Math.round(ms)} ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  const s = Math.floor(ms / 1000)
  return `${Math.floor(s / 60)}m ${s % 60}s`
}

export function formatBytesMb(mb: number): string {
  if (mb >= 1024) return `${(mb / 1024).toFixed(2)} GB`
  return `${mb.toFixed(1)} MB`
}

/** Try to parse a backend TaskResult.output string into structured data. */
export function parseOutput(output: string): unknown {
  if (typeof output !== 'string') return output
  const trimmed = output.trim()
  if (!trimmed) return null
  if (trimmed[0] === '{' || trimmed[0] === '[') {
    try { return JSON.parse(trimmed) } catch { /* fall through */ }
  }
  return null
}

/** Flatten an object into [key, value] string pairs for the kv-grid. */
export function toKeyValues(obj: unknown, prefix = ''): [string, string][] {
  const out: [string, string][] = []
  if (obj == null || typeof obj !== 'object') return out
  for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
    const key = prefix ? `${prefix} · ${k}` : k
    if (v != null && typeof v === 'object' && !Array.isArray(v)) {
      out.push(...toKeyValues(v, key))
    } else if (Array.isArray(v)) {
      out.push([key, v.map(x => (typeof x === 'object' ? JSON.stringify(x) : String(x))).join(', ')])
    } else {
      out.push([key, String(v)])
    }
  }
  return out
}

/** Human-facing rows for diagnostics whose raw schema needs explanation. */
export function toDiagnosticKeyValues(taskId: string, obj: unknown): [string, string][] {
  if (taskId !== 'pending_reboot' || obj == null || typeof obj !== 'object' || Array.isArray(obj)) {
    return toKeyValues(obj)
  }
  const value = obj as Record<string, unknown>
  const reasons = Array.isArray(value.reasons)
    ? value.reasons.filter((reason): reason is string => typeof reason === 'string')
    : []
  const highConfidenceReason = reasons.some(
    reason => reason === 'windows_update' || reason === 'component_based_servicing',
  )
  const legacyDeferredReason = reasons.some(
    reason => reason === 'pending_file_rename' || reason === 'pending_file_operations',
  )
  const explicitRestart = typeof value.restart_required === 'boolean'
    ? value.restart_required
    : undefined
  const legacyPending = typeof value.pending === 'boolean' ? value.pending : undefined
  const contradictory = explicitRestart !== undefined
    ? explicitRestart !== highConfidenceReason
      || (legacyPending !== undefined && legacyPending !== explicitRestart)
    : (legacyPending === true && !highConfidenceReason && !legacyDeferredReason)
      || (legacyPending === false && highConfidenceReason)
  const restartRequired = explicitRestart !== undefined
    ? explicitRestart
    : value.pending === true && highConfidenceReason
  const requiredBy = reasons.flatMap(reason => {
    if (reason === 'windows_update') return ['Windows Update']
    if (reason === 'component_based_servicing') return ['Windows component servicing']
    return []
  })
  const deferred = value.deferred_file_operations && typeof value.deferred_file_operations === 'object'
    ? value.deferred_file_operations as Record<string, unknown>
    : {}
  const deferredPending = deferred.pending === true || reasons.some(
    reason => reason === 'pending_file_rename' || reason === 'pending_file_operations',
  )
  const count = typeof deferred.operation_count === 'number' ? deferred.operation_count : null
  const rows: [string, string][] = [
    ['Restart required', contradictory ? 'Could not determine — retry this check' : restartRequired ? 'Yes' : 'No'],
    ['Required by', contradictory
      ? 'Conflicting restart-marker data'
      : requiredBy.length ? requiredBy.join(' and ') : 'No Windows Update or component-servicing marker'],
    ['Deferred file operations', deferredPending
      ? `${count ?? 'Some'} operation${count === 1 ? '' : 's'} queued for the next restart; this marker alone does not establish that you must restart now`
      : 'None queued'],
  ]
  if (typeof value.summary === 'string' && value.summary.trim()) rows.push(['Summary', value.summary])
  return rows
}
