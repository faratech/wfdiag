import { useCallback, useEffect, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useAppContext, type TaskResult } from '../contexts/AppContext'
import * as logger from '../utils/logger'

export const useScanner = () => {
  const {
    availableTasks,
    systemInfo,
    sessionId,
    setSessionId,
    results,
    setResults,
    isRunning,
    setIsRunning,
    setCurrentProgress,
    setCurrentTaskName,
    scanStartTime,
    setScanStartTime,
    setScanEndTime,
    settings,
    searchQuery,
    setFilteredResults,
  } = useAppContext()

  // Track the current running session to prevent race conditions
  const currentSessionRef = useRef<string | null>(null)
  const isRunningRef = useRef<boolean>(false)
  // Track auto-save timeout for cleanup
  const autoSaveTimeoutRef = useRef<number | null>(null)

  // Helper to clear any pending auto-save timeout
  const clearAutoSaveTimeout = useCallback(() => {
    if (autoSaveTimeoutRef.current !== null) {
      clearTimeout(autoSaveTimeoutRef.current)
      autoSaveTimeoutRef.current = null
    }
  }, [])

  const runDiagnostics = useCallback(async (taskIds: string[]) => {
    if (taskIds.length === 0) return

    // Prevent concurrent scans - race condition guard
    if (isRunningRef.current) {
      logger.warn('useScanner', 'Scan already in progress, ignoring new scan request')
      return
    }

    // Clear any pending auto-save from previous scan
    clearAutoSaveTimeout()

    isRunningRef.current = true
    setIsRunning(true)
    setCurrentProgress(0)
    setResults({})
    setScanStartTime(Date.now())
    setScanEndTime(0) // Reset end time for new scan

    let unlisten: (() => void) | null = null

    try {
      const newSessionId = await invoke<string>('start_diagnostics', { taskIds })

      // Store the session ID in ref to prevent race conditions
      currentSessionRef.current = newSessionId
      setSessionId(newSessionId)

      let completedTasks = 0
      const totalTasks = taskIds.length

      unlisten = await listen<{
        task_id: string
        status: 'running' | 'completed'
        task_name?: string
        success?: boolean
      }>('task-progress', (event: any) => {
        // Only process events for the current session
        if (currentSessionRef.current !== newSessionId) {
          logger.debug('useScanner', 'Ignoring event from old session')
          return
        }

        if (event.payload.status === 'running' && event.payload.task_name) {
          setCurrentTaskName(event.payload.task_name)
        } else if (event.payload.status === 'completed') {
          completedTasks++
          setCurrentProgress((completedTasks / totalTasks) * 100)
        }
      })

      try {
        const scanResults = await invoke<Array<[string, TaskResult]>>('run_diagnostics_parallel', {
          taskIds,
          maxConcurrent: settings.maxConcurrentTasks || 5
        })

        // Verify this is still the current session before updating results
        if (currentSessionRef.current !== newSessionId) {
          logger.warn('useScanner', 'Scan session changed, discarding results from old session')
          return
        }

        const resultsObj = scanResults.reduce((acc, [taskId, result]) => {
          acc[taskId] = result
          return acc
        }, {} as Record<string, TaskResult>)

        setResults(resultsObj)
      } finally {
        // Always cleanup listener
        if (unlisten) {
          unlisten()
          unlisten = null
        }
      }

      setCurrentProgress(100)

      if (settings.autoSave) {
        // Store timeout ID so it can be cancelled on unmount or scan stop
        autoSaveTimeoutRef.current = window.setTimeout(async () => {
          // Check if we're still the current session (guard against race conditions)
          if (currentSessionRef.current !== newSessionId) {
            logger.warn('useScanner', 'Session changed, skipping auto-save for stale session')
            return
          }

          const scanDuration = Date.now() - scanStartTime
          try {
            const savedScanId = await invoke<string>('save_current_scan', {
              durationMs: scanDuration,
              tags: taskIds.length === availableTasks.length ? ['Full Scan'] : ['Quick Scan']
            })
            logger.info('useScanner', 'Scan auto-saved successfully', savedScanId)
          } catch (error) {
            logger.error('useScanner', 'Failed to auto-save scan', error)
          } finally {
            // Clear the ref since timeout completed
            autoSaveTimeoutRef.current = null

            // Only update state if this is still the current session
            if (currentSessionRef.current === newSessionId) {
              setScanEndTime(Date.now())
              isRunningRef.current = false
              setIsRunning(false)
            }
          }
        }, 500)
      } else {
        setScanEndTime(Date.now())
        isRunningRef.current = false
        setIsRunning(false)
      }
    } catch (error) {
      logger.error('useScanner', 'Failed to start diagnostics', error)

      // Cleanup listener on error
      if (unlisten) {
        unlisten()
      }

      // Clear any pending auto-save timeout on error
      clearAutoSaveTimeout()

      setScanEndTime(Date.now())
      isRunningRef.current = false
      setIsRunning(false)
      currentSessionRef.current = null
    }
  }, [
    availableTasks.length,
    scanStartTime,
    setCurrentProgress,
    setCurrentTaskName,
    setIsRunning,
    setResults,
    setScanStartTime,
    setScanEndTime,
    setSessionId,
    settings.autoSave,
    settings.maxConcurrentTasks,
    clearAutoSaveTimeout
  ])

  // Default Quick Scan task IDs (used when no custom list is configured)
  const DEFAULT_QUICK_SCAN_TASKS = [
    'comp_system', 'os_info', 'processor', 'physical_memory', 'disk_drive',
    'logical_disk', 'network_adapter', 'systeminfo'
  ]

  const runQuickScan = useCallback(async () => {
    // Use custom quick scan tasks from settings, or fall back to defaults
    const customTasks = settings.quickScanTasks
    const taskIdsToRun = (customTasks && customTasks.length > 0) ? customTasks : DEFAULT_QUICK_SCAN_TASKS

    // Filter to only tasks that exist in availableTasks
    const quickTasks = availableTasks
      .filter(task => taskIdsToRun.includes(task.id))
      .map(t => t.id)

    await runDiagnostics(quickTasks)
  }, [availableTasks, settings.quickScanTasks, runDiagnostics])

  const runFullScan = useCallback(async () => {
    const allTasks = availableTasks
      .filter(task => !task.admin_required || systemInfo?.is_admin)
      .map(t => t.id)

    await runDiagnostics(allTasks)
  }, [availableTasks, systemInfo, runDiagnostics])

  const stopScan = useCallback(() => {
    // Cancel any pending auto-save
    clearAutoSaveTimeout()
    isRunningRef.current = false
    currentSessionRef.current = null
    setIsRunning(false)
    setCurrentProgress(0)
    setCurrentTaskName('')
  }, [setIsRunning, setCurrentProgress, setCurrentTaskName, clearAutoSaveTimeout])

  const clearResults = useCallback(() => {
    // Cancel any pending auto-save
    clearAutoSaveTimeout()
    currentSessionRef.current = null
    setResults({})
    setSessionId(null)
    setCurrentProgress(0)
    setCurrentTaskName('')
  }, [setResults, setSessionId, setCurrentProgress, setCurrentTaskName, clearAutoSaveTimeout])

  // Search and filter functionality
  useEffect(() => {
    if (!searchQuery) {
      setFilteredResults(results)
      return
    }

    const query = searchQuery.toLowerCase()
    const filtered: Record<string, TaskResult> = {}

    for (const [taskId, result] of Object.entries(results)) {
      if (taskId.toLowerCase().includes(query)) {
        filtered[taskId] = result
        continue
      }

      if (result.error && result.error.toLowerCase().includes(query)) {
        filtered[taskId] = result
        continue
      }

      if (result.output.toLowerCase().includes(query)) {
        filtered[taskId] = result
      }
    }

    setFilteredResults(filtered)
  }, [searchQuery, results, setFilteredResults])

  // Cleanup on unmount - ensure refs are reset and timeouts are cleared
  useEffect(() => {
    return () => {
      isRunningRef.current = false
      currentSessionRef.current = null
      // Clear any pending auto-save timeout on unmount
      if (autoSaveTimeoutRef.current !== null) {
        clearTimeout(autoSaveTimeoutRef.current)
        autoSaveTimeoutRef.current = null
      }
    }
  }, [])

  return {
    runQuickScan,
    runFullScan,
    stopScan,
    clearResults,
    isRunning,
    sessionId,
    results,
  }
}