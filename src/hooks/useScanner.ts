import { useCallback, useEffect, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow, ProgressBarStatus } from '@tauri-apps/api/window'
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification'
import { useAppContext, type TaskResult } from '../contexts/AppContext'
import * as logger from '../utils/logger'

// Inventory tasks shown as the quick-scan baseline.
const QUICK_INVENTORY_TASKS = [
  'comp_system', 'os_info', 'processor', 'physical_memory',
  'disk_drive', 'logical_disk', 'network_adapter', 'systeminfo',
]

// Cheap, non-admin sources that feed the issue detectors. Unioned into EVERY
// quick scan (see runQuickScan) so the Issues screen isn't empty after a Quick
// Scan just because most detectors had no source data. The heavier sources
// (event_logs, windows_update, drivers_list) and the admin-only ones (minidump,
// chkdsk, dism_health, battery_report, disk_fragmentation) stay full-scan-only.
const DETECTION_SOURCE_TASKS = [
  'logical_disk', 'network_adapter', 'pending_reboot', 'device_errors',
  'defender_status', 'event_codes_critical', 'services', 'performance',
  'startup_command', 'hosts_file', 'firewall_status',
]

// Default Quick Scan task IDs (used when no custom list is configured). Module
// scope keeps the reference stable so callbacks needn't depend on it.
const DEFAULT_QUICK_SCAN_TASKS = Array.from(
  new Set([...QUICK_INVENTORY_TASKS, ...DETECTION_SOURCE_TASKS])
)

// Mirror scan progress onto the Windows taskbar button. All calls are
// best-effort: failures (missing permission, non-Tauri dev context) must
// never affect the scan itself.
function setTaskbarProgress(progress: number | null) {
  if (!('__TAURI_INTERNALS__' in window)) return
  const state = progress === null
    ? { status: ProgressBarStatus.None }
    : { status: ProgressBarStatus.Normal, progress: Math.round(progress) }
  getCurrentWindow().setProgressBar(state).catch(() => {})
}

// Fire a native Windows toast when a scan finishes while the app is in the
// background. Best-effort; gated on the user's showNotifications setting.
async function notifyScanComplete(passed: number, failed: number) {
  if (!('__TAURI_INTERNALS__' in window) || document.hasFocus()) return
  try {
    let granted = await isPermissionGranted()
    if (!granted) {
      granted = (await requestPermission()) === 'granted'
    }
    if (granted) {
      sendNotification({
        title: 'Diagnostics complete',
        body: failed > 0 ? `${passed} passed, ${failed} failed` : `All ${passed} checks passed`,
      })
    }
  } catch (error) {
    logger.warn('useScanner', 'Failed to send scan notification', error)
  }
}

// Module-level lock shared across ALL useScanner instances. App.tsx (startup scan) and
// DiagnosticsTab.tsx (manual scan) each call useScanner() independently, so a per-instance
// ref cannot block a scan started by the other instance. The lock stores an owner token
// (not a boolean) so only the scan that acquired it can release it, and release happens in
// a finally around the whole runDiagnostics body — every exit path (success, stop,
// session-changed early return, error) frees the lock.
let scanLockOwner: string | null = null
let activeScanSessionId: string | null = null
let cancelledScanSessionId: string | null = null
let activeAutoSaveCancel: (() => void) | null = null

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
    setScanStartTime,
    setScanEndTime,
    settings,
    searchQuery,
    setFilteredResults,
    setTaskStatuses,
    setIssues,
  } = useAppContext()

  // Track the current running session to prevent race conditions
  const currentSessionRef = useRef<string | null>(null)
  const cancelledSessionRef = useRef<string | null>(null)
  const isRunningRef = useRef<boolean>(false)
  // Track auto-save delay for cleanup. The delay is AWAITED inside runDiagnostics
  // (not fire-and-forget), so cancelling must RESOLVE the pending promise — merely
  // clearTimeout-ing it would leave the await hanging and the scan lock held forever.
  const autoSaveTimeoutRef = useRef<number | null>(null)
  const autoSaveCancelRef = useRef<(() => void) | null>(null)

  // Cancel any pending auto-save delay, resolving its promise as "cancelled"
  const clearAutoSaveTimeout = useCallback(() => {
    if (autoSaveTimeoutRef.current !== null) {
      clearTimeout(autoSaveTimeoutRef.current)
      autoSaveTimeoutRef.current = null
    }
    if (autoSaveCancelRef.current) {
      const cancel = autoSaveCancelRef.current
      autoSaveCancelRef.current = null
      if (activeAutoSaveCancel === cancel) {
        activeAutoSaveCancel = null
      }
      cancel()
    }
  }, [])

  // Cancellable delay; resolves true when cancelled early, false after the full delay
  const waitAutoSaveDelay = useCallback((ms: number) => {
    return new Promise<boolean>(resolve => {
      const cancel = () => resolve(true)
      autoSaveCancelRef.current = cancel
      activeAutoSaveCancel = cancel
      autoSaveTimeoutRef.current = window.setTimeout(() => {
        autoSaveTimeoutRef.current = null
        autoSaveCancelRef.current = null
        if (activeAutoSaveCancel === cancel) {
          activeAutoSaveCancel = null
        }
        resolve(false)
      }, ms)
    })
  }, [])

  const runDiagnostics = useCallback(async (taskIds: string[], scanTag = 'Manual Diagnostic') => {
    if (taskIds.length === 0) return

    // Prevent concurrent scans - race condition guard (per-instance AND cross-instance)
    if (isRunningRef.current || scanLockOwner !== null) {
      logger.warn('useScanner', 'Scan already in progress, ignoring new scan request')
      return
    }

    // Clear any pending auto-save from previous scan
    clearAutoSaveTimeout()

    const lockId = `lock_${Date.now()}_${Math.random().toString(36).slice(2)}`
    scanLockOwner = lockId
    isRunningRef.current = true
    cancelledSessionRef.current = null
    cancelledScanSessionId = null
    setIsRunning(true)
    setCurrentProgress(0)
    setResults({})
    setIssues([])
    setTaskStatuses({})
    setTaskbarProgress(0)
    // Capture the start time in a local so the auto-save timeout below uses THIS scan's
    // start, not the stale `scanStartTime` from a prior render's closure (which made the
    // first scan's duration ~the full epoch and later scans include all idle time).
    const startedAt = Date.now()
    setScanStartTime(startedAt)
    setScanEndTime(0) // Reset end time for new scan

    let unlisten: (() => void) | null = null

    try {
      const newSessionId = await invoke<string>('start_diagnostics', { taskIds })

      // Store the session ID in ref to prevent race conditions
      currentSessionRef.current = newSessionId
      activeScanSessionId = newSessionId
      setSessionId(newSessionId)

      let completedTasks = 0
      const totalTasks = taskIds.length

      unlisten = await listen<{
        session_id: string
        task_id: string
        status: 'running' | 'completed'
        task_name?: string
        success?: boolean
      }>('task-progress', event => {
        // Only process events for the current session
        if (event.payload.session_id !== newSessionId || activeScanSessionId !== newSessionId) {
          logger.debug('useScanner', 'Ignoring event from old session')
          return
        }

        if (cancelledScanSessionId === newSessionId) {
          return
        }

        if (event.payload.status === 'running' && event.payload.task_name) {
          setCurrentTaskName(event.payload.task_name)
          setTaskStatuses(prev => ({ ...prev, [event.payload.task_id]: 'running' }))
        } else if (event.payload.status === 'completed') {
          completedTasks++
          setCurrentProgress((completedTasks / totalTasks) * 100)
          setTaskStatuses(prev => ({ ...prev, [event.payload.task_id]: 'done' }))
          setTaskbarProgress((completedTasks / totalTasks) * 100)
        }
      })

      try {
        const scanResults = await invoke<Array<[string, TaskResult]>>('run_diagnostics_parallel', {
          taskIds,
          maxConcurrent: settings.maxConcurrentTasks || 5,
          sessionId: newSessionId
        })

        // Verify this is still the current session before updating results
        if (activeScanSessionId !== newSessionId) {
          logger.warn('useScanner', 'Scan session changed, discarding results from old session')
          return
        }

        if (cancelledScanSessionId !== newSessionId) {
          const resultsObj = scanResults.reduce((acc, [taskId, result]) => {
            acc[taskId] = result
            return acc
          }, {} as Record<string, TaskResult>)

          setResults(resultsObj)

          const resultList = Object.values(resultsObj)
          const passedCount = resultList.filter(r => r.success).length
          if (settings.showNotifications) {
            notifyScanComplete(passedCount, resultList.length - passedCount)
          }
        }
      } finally {
        // Always cleanup listener
        if (unlisten) {
          unlisten()
          unlisten = null
        }
      }

      const wasCancelled = cancelledScanSessionId === newSessionId
      setCurrentProgress(wasCancelled ? 0 : 100)
      setTaskbarProgress(null)

      if (settings.autoSave && !wasCancelled) {
        // Awaited (not fire-and-forget) so the scan lock is held through the
        // auto-save, yet always released by the finally below. stopScan and
        // unmount cancel the delay early via clearAutoSaveTimeout.
        const cancelled = await waitAutoSaveDelay(500)

        // Check if we're still the current session (guard against race conditions)
        if (!cancelled && activeScanSessionId === newSessionId) {
          const scanDuration = Date.now() - startedAt
          try {
            const savedScanId = await invoke<string>('save_current_scan', {
              durationMs: scanDuration,
              tags: [scanTag]
            })
            logger.info('useScanner', 'Scan auto-saved successfully', savedScanId)
          } catch (error) {
            logger.error('useScanner', 'Failed to auto-save scan', error)
          }
        } else if (cancelled) {
          logger.warn('useScanner', 'Auto-save cancelled, skipping save')
        }
      }

      // Only update state if this is still the current session (stopScan
      // already reset the UI state when it nulled the session ref)
      if (activeScanSessionId === newSessionId) {
        setScanEndTime(Date.now())
        setIsRunning(false)
        setCurrentTaskName('')
        currentSessionRef.current = null
        activeScanSessionId = null
      }
      if (cancelledScanSessionId === newSessionId) {
        cancelledSessionRef.current = null
        cancelledScanSessionId = null
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
      setIsRunning(false)
      setTaskbarProgress(null)
      currentSessionRef.current = null
      activeScanSessionId = null
      cancelledSessionRef.current = null
      cancelledScanSessionId = null
    } finally {
      isRunningRef.current = false
      // Release the cross-instance lock only if this scan still owns it
      if (scanLockOwner === lockId) {
        scanLockOwner = null
      }
    }
  }, [
    setCurrentProgress,
    setCurrentTaskName,
    setIsRunning,
    setIssues,
    setResults,
    setScanStartTime,
    setScanEndTime,
    setSessionId,
    settings.autoSave,
    settings.maxConcurrentTasks,
    settings.showNotifications,
    clearAutoSaveTimeout,
    waitAutoSaveDelay,
    setTaskStatuses
  ])

  const runQuickScan = useCallback(async () => {
    // Use custom quick scan tasks from settings, or fall back to defaults, then
    // always union in the detection sources so the Issues screen has data to
    // work with even when the quick list was customised.
    const customTasks = settings.quickScanTasks
    const base = (customTasks && customTasks.length > 0) ? customTasks : DEFAULT_QUICK_SCAN_TASKS
    const taskIdsToRun = Array.from(new Set([...base, ...DETECTION_SOURCE_TASKS]))

    // Filter to only tasks that exist in availableTasks
    const quickTasks = availableTasks
      .filter(task => taskIdsToRun.includes(task.id))
      .map(t => t.id)

    await runDiagnostics(quickTasks, 'Quick Scan')
  }, [availableTasks, settings.quickScanTasks, runDiagnostics])

  const runFullScan = useCallback(async () => {
    const allTasks = availableTasks
      .filter(task => !task.admin_required || systemInfo?.is_admin)
      .map(t => t.id)

    await runDiagnostics(allTasks, 'Full Scan')
  }, [availableTasks, systemInfo, runDiagnostics])

  const stopScan = useCallback(() => {
    const sessionId = currentSessionRef.current ?? activeScanSessionId
    // Cancel any pending auto-save (resolves the awaited delay so the in-flight
    // runDiagnostics call can unwind and release the scan lock)
    clearAutoSaveTimeout()
    if (activeAutoSaveCancel) {
      const cancel = activeAutoSaveCancel
      activeAutoSaveCancel = null
      cancel()
    }
    // Tell the backend to skip this session's queued tasks; in-flight tasks
    // finish and the pending run_diagnostics_parallel invoke resolves quickly.
    if (sessionId) {
      cancelledSessionRef.current = sessionId
      cancelledScanSessionId = sessionId
      invoke('cancel_diagnostics', { sessionId }).catch(error =>
        logger.error('useScanner', 'Failed to cancel diagnostics session', error)
      )
    }
    setTaskbarProgress(null)
  }, [clearAutoSaveTimeout])

  const clearResults = useCallback(() => {
    // Cancel any pending auto-save
    clearAutoSaveTimeout()
    currentSessionRef.current = null
    activeScanSessionId = null
    cancelledSessionRef.current = null
    cancelledScanSessionId = null
    setResults({})
    setIssues([])
    setSessionId(null)
    setCurrentProgress(0)
    setCurrentTaskName('')
  }, [setResults, setIssues, setSessionId, setCurrentProgress, setCurrentTaskName, clearAutoSaveTimeout])

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

  // Cleanup on unmount - ensure refs are reset and pending delays are resolved
  // (an unresolved awaited delay would keep the module-level scan lock held).
  // Deliberately does NOT touch the module-global activeScanSessionId: a scan
  // started from this instance can legitimately keep running in the backend
  // after the owning component unmounts (e.g. the user switches tabs), and
  // its results/isRunning live in AppContext, not this component. Clearing
  // that global here used to make the in-flight runDiagnostics call discard
  // its results and skip setIsRunning(false), leaving isRunning stuck true
  // app-wide until restart.
  useEffect(() => {
    return () => {
      isRunningRef.current = false
      currentSessionRef.current = null
      cancelledSessionRef.current = null
      if (autoSaveTimeoutRef.current !== null) {
        clearTimeout(autoSaveTimeoutRef.current)
        autoSaveTimeoutRef.current = null
      }
      if (autoSaveCancelRef.current) {
        const cancel = autoSaveCancelRef.current
        autoSaveCancelRef.current = null
        if (activeAutoSaveCancel === cancel) {
          activeAutoSaveCancel = null
        }
        cancel()
      }
    }
  }, [])

  return {
    runDiagnostics,
    runQuickScan,
    runFullScan,
    stopScan,
    clearResults,
    isRunning,
    sessionId,
    results,
  }
}
