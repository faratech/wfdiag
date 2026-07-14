import { useState, useEffect, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import * as logger from '../utils/logger'

export interface ScanSummary {
  id: string
  timestamp: string
  computer_name: string
  task_count: number
  success_count: number
  failure_count: number
  duration_ms: number
  label?: string | null
  tags: string[]
}

interface UseScanHistoryReturn {
  scans: ScanSummary[]
  loading: boolean
  error: string | null
  refreshScans: () => Promise<void>
}

export function useScanHistory(): UseScanHistoryReturn {
  const [scans, setScans] = useState<ScanSummary[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  // Pure fetch + validation. Throws on failure and never calls setState, so the
  // mount effect can apply the result inside the promise callback (post-await),
  // off the synchronous-setState-in-effect path.
  const loadScans = useCallback(async (): Promise<ScanSummary[]> => {
    logger.debug('useScanHistory', 'Fetching scan history...')

    const scanList = await invoke<ScanSummary[]>('list_scan_history')

    // Validate the response
    if (!Array.isArray(scanList)) {
      throw new Error('Invalid response format: expected array of scans')
    }

    // Validate each scan
    const validScans = scanList.filter(scan =>
      scan &&
      typeof scan === 'object' &&
      scan.id &&
      scan.timestamp &&
      typeof scan.task_count === 'number'
    )

    if (validScans.length !== scanList.length) {
      logger.warn('useScanHistory', `Filtered out ${scanList.length - validScans.length} invalid scans`)
    }

    logger.info('useScanHistory', `Successfully loaded ${validScans.length} scans`)
    return validScans
  }, [])

  const fetchScans = useCallback(async () => {
    try {
      setScans(await loadScans())
      setError(null)
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err)
      logger.error('useScanHistory', 'Failed to fetch scan history', errorMessage)
      setError(`Failed to load scan history: ${errorMessage}`)
      setScans([])
    } finally {
      setLoading(false)
    }
  }, [loadScans])

  // User-initiated refresh: show the loading state immediately, then fetch.
  const refreshScans = useCallback(async () => {
    setLoading(true)
    setError(null)
    await fetchScans()
  }, [fetchScans])

  // Load scans on mount. `loading` already starts true; applying the result
  // inside the promise callback (post-await) keeps this off the
  // synchronous-setState-in-effect path.
  useEffect(() => {
    let cancelled = false
    loadScans()
      .then(validScans => { if (!cancelled) { setScans(validScans); setError(null) } })
      .catch(err => {
        if (cancelled) return
        const errorMessage = err instanceof Error ? err.message : String(err)
        logger.error('useScanHistory', 'Failed to fetch scan history', errorMessage)
        setError(`Failed to load scan history: ${errorMessage}`)
        setScans([])
      })
      .finally(() => { if (!cancelled) setLoading(false) })
    return () => { cancelled = true }
  }, [loadScans])

  return {
    scans,
    loading,
    error,
    refreshScans,
  }
}
