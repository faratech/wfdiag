import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'

export interface ScanSummary {
  id: string
  timestamp: string
  computer_name: string
  task_count: number
  success_count: number
  failure_count: number
  duration_ms: number
  tags: string[]
}

export interface ScanRecord {
  id: string
  timestamp: string
  computer_name: string
  os_version: string
  is_admin: boolean
  results: Record<string, any>
  task_count: number
  success_count: number
  failure_count: number
  duration_ms: number
  tags: string[]
}

interface UseScanHistoryReturn {
  scans: ScanSummary[]
  loading: boolean
  error: string | null
  refreshScans: () => Promise<void>
  loadScan: (scanId: string) => Promise<ScanRecord | null>
}

export function useScanHistory(): UseScanHistoryReturn {
  const [scans, setScans] = useState<ScanSummary[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const refreshScans = async () => {
    try {
      setLoading(true)
      setError(null)
      
      console.log('Fetching scan history...')
      
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
        console.warn(`Filtered out ${scanList.length - validScans.length} invalid scans`)
      }
      
      console.log(`Successfully loaded ${validScans.length} scans`)
      setScans(validScans)
      
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err)
      console.error('Failed to fetch scan history:', errorMessage)
      setError(`Failed to load scan history: ${errorMessage}`)
      setScans([])
    } finally {
      setLoading(false)
    }
  }

  const loadScan = async (scanId: string): Promise<ScanRecord | null> => {
    try {
      console.log('Loading scan:', scanId)
      
      if (!scanId) {
        throw new Error('Scan ID is required')
      }
      
      const scan = await invoke<ScanRecord>('load_scan', { scanId })
      
      if (!scan) {
        throw new Error('Scan not found')
      }
      
      // Validate scan structure
      if (!scan.id || !scan.timestamp || !scan.results) {
        throw new Error('Invalid scan data structure')
      }
      
      console.log(`Successfully loaded scan ${scan.id} with ${scan.task_count} results`)
      return scan
      
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err)
      console.error(`Failed to load scan ${scanId}:`, errorMessage)
      return null
    }
  }

  // Load scans on mount
  useEffect(() => {
    refreshScans()
  }, [])

  return {
    scans,
    loading,
    error,
    refreshScans,
    loadScan
  }
}