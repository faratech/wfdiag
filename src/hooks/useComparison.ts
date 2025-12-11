import { useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { ScanSummary } from './useScanHistory'
import * as logger from '../utils/logger'

export interface TaskChange {
  task_id: string
  task_name: string
  category: string
  current_success: boolean
  previous_success: boolean
  current_output: string
  previous_output: string
  output_changed: boolean
}

export interface ComparisonResult {
  current_scan: ScanSummary
  previous_scan: ScanSummary
  total_changes: number
  new_failures: TaskChange[]
  new_successes: TaskChange[]
  status_unchanged: TaskChange[]
}

export type ComparisonFilter = 'all' | 'failures' | 'successes' | 'changes'

interface UseComparisonReturn {
  comparison: ComparisonResult | null
  loading: boolean
  error: string | null
  compareScans: (currentId: string, previousId: string) => Promise<void>
  clearComparison: () => void
  getFilteredChanges: (filter: ComparisonFilter) => TaskChange[]
}

export function useComparison(): UseComparisonReturn {
  const [comparison, setComparison] = useState<ComparisonResult | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const compareScans = async (currentId: string, previousId: string) => {
    try {
      setLoading(true)
      setError(null)
      setComparison(null)
      
      logger.debug('useComparison', 'Comparing scans', { currentId, previousId })

      // Validate input
      if (!currentId || !previousId) {
        throw new Error('Both scan IDs are required for comparison')
      }
      
      if (currentId === previousId) {
        throw new Error('Cannot compare a scan with itself')
      }
      
      // Call backend comparison
      const result = await invoke<ComparisonResult>('compare_scans', {
        currentId,
        previousId
      })

      logger.debug('useComparison', 'Comparison result received', { scanCount: result?.total_changes })

      // Validate response structure
      if (!result) {
        throw new Error('No comparison result received')
      }
      
      if (!result.current_scan || !result.previous_scan) {
        throw new Error('Invalid comparison result: missing scan information')
      }
      
      if (typeof result.total_changes !== 'number') {
        throw new Error('Invalid comparison result: missing total_changes')
      }
      
      // Ensure arrays exist
      result.new_failures = Array.isArray(result.new_failures) ? result.new_failures : []
      result.new_successes = Array.isArray(result.new_successes) ? result.new_successes : []
      result.status_unchanged = Array.isArray(result.status_unchanged) ? result.status_unchanged : []

      logger.info('useComparison', `Comparison complete: ${result.total_changes} total changes`)
      setComparison(result)
      
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err)
      logger.error('useComparison', 'Comparison failed', errorMessage)
      setError(`Failed to compare scans: ${errorMessage}`)
      setComparison(null)
    } finally {
      setLoading(false)
    }
  }

  const clearComparison = () => {
    setComparison(null)
    setError(null)
  }

  const getFilteredChanges = (filter: ComparisonFilter): TaskChange[] => {
    if (!comparison) return []

    switch (filter) {
      case 'failures':
        return comparison.new_failures

      case 'successes':
        return comparison.new_successes

      case 'changes':
        return comparison.status_unchanged.filter(change => change.output_changed)

      case 'all':
      default:
        return [
          ...comparison.new_failures,
          ...comparison.new_successes,
          ...comparison.status_unchanged.filter(change => change.output_changed)
        ]
    }
  }

  return {
    comparison,
    loading,
    error,
    compareScans,
    clearComparison,
    getFilteredChanges
  }
}