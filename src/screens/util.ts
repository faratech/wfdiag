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
