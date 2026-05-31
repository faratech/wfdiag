/**
 * Shared formatting utility functions
 * Used across SystemMonitoring.tsx, ProcessesTab.tsx, and other components
 */

/**
 * Format bytes to human readable string (e.g., "1.5 MB", "2.3 GB")
 */
export function formatBytes(bytes: number, decimals = 1): string {
  if (bytes === 0) return '0 B'
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(decimals)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(decimals)} MB`
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(decimals + 1)} GB`
}

/**
 * Format CPU time in seconds to human readable string (e.g., "1:23:45", "5:30")
 */
export function formatCpuTime(seconds: number): string {
  const secs = Math.floor(seconds)
  const hours = Math.floor(secs / 3600)
  const mins = Math.floor((secs % 3600) / 60)
  const remainingSecs = secs % 60

  if (hours > 0) {
    return `${hours}:${mins.toString().padStart(2, '0')}:${remainingSecs.toString().padStart(2, '0')}`
  }
  return `${mins}:${remainingSecs.toString().padStart(2, '0')}`
}

/**
 * Format uptime in seconds to human readable string (e.g., "5 days, 3 hours")
 */
export function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400)
  const hours = Math.floor((seconds % 86400) / 3600)
  const mins = Math.floor((seconds % 3600) / 60)

  const parts: string[] = []
  if (days > 0) parts.push(`${days} day${days !== 1 ? 's' : ''}`)
  if (hours > 0) parts.push(`${hours} hour${hours !== 1 ? 's' : ''}`)
  if (mins > 0 && days === 0) parts.push(`${mins} min${mins !== 1 ? 's' : ''}`)

  return parts.length > 0 ? parts.join(', ') : 'Just started'
}

/**
 * Format duration in milliseconds to human readable string (e.g., "5.2s", "1m 23s")
 */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`

  // Round to deciseconds BEFORE choosing the branch, so a value that rounds up to 60.0s
  // rolls into the minutes branch ("1m 0s") instead of displaying "60.0s".
  const deciseconds = Math.round(ms / 100)
  if (deciseconds < 600) {
    return `${(deciseconds / 10).toFixed(1)}s`
  }

  const totalSecs = Math.round(deciseconds / 10)
  const mins = Math.floor(totalSecs / 60)
  const remainingSecs = totalSecs % 60
  return `${mins}m ${remainingSecs}s`
}

/**
 * Format memory in MB to human readable string (e.g., "1.5 GB", "256 MB")
 */
export function formatMemoryMB(mb: number, decimals = 1): string {
  if (mb < 1024) return `${mb.toFixed(decimals)} MB`
  return `${(mb / 1024).toFixed(decimals)} GB`
}

/**
 * Format percentage with consistent decimal places
 */
export function formatPercent(value: number, decimals = 1): string {
  return `${value.toFixed(decimals)}%`
}
