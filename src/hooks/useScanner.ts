import { useCallback, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useAppContext, type TaskResult } from '../contexts/AppContext'

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
    settings,
    searchQuery,
    setFilteredResults,
  } = useAppContext()

  const runDiagnostics = useCallback(async (taskIds: string[]) => {
    if (taskIds.length === 0) return

    setIsRunning(true)
    setCurrentProgress(0)
    setResults({})
    setScanStartTime(Date.now())

    try {
      const newSessionId = await invoke<string>('start_diagnostics', { taskIds })
      setSessionId(newSessionId)

      let completedTasks = 0
      const totalTasks = taskIds.length

      const unlisten = await listen<{
        task_id: string
        status: 'running' | 'completed'
        task_name?: string
        success?: boolean
      }>('task-progress', (event: any) => {
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

        const resultsObj = scanResults.reduce((acc, [taskId, result]) => {
          acc[taskId] = result
          return acc
        }, {} as Record<string, TaskResult>)

        setResults(resultsObj)
      } finally {
        unlisten()
      }

      setCurrentProgress(100)

      if (settings.autoSave) {
        setTimeout(async () => {
          const scanDuration = Date.now() - scanStartTime
          try {
            const savedScanId = await invoke<string>('save_current_scan', {
              durationMs: scanDuration,
              tags: taskIds.length === availableTasks.length ? ['Full Scan'] : ['Quick Scan']
            })
            console.log('Scan auto-saved successfully with ID:', savedScanId)
          } catch (error) {
            console.error('Failed to auto-save scan:', error)
          }
          setIsRunning(false)
        }, 500)
      } else {
        setIsRunning(false)
      }
    } catch (error) {
      console.error('Failed to start diagnostics:', error)
      setIsRunning(false)
    }
  }, [
    availableTasks.length,
    scanStartTime,
    setCurrentProgress,
    setCurrentTaskName,
    setIsRunning,
    setResults,
    setScanStartTime,
    setSessionId,
    settings.autoSave,
    settings.maxConcurrentTasks
  ])

  const runQuickScan = useCallback(async () => {
    const quickTasks = availableTasks.filter(task =>
      ['comp_system', 'os_info', 'processor', 'physical_memory', 'disk_drive',
       'logical_disk', 'network_adapter', 'systeminfo'].includes(task.id)
    ).map(t => t.id)

    await runDiagnostics(quickTasks)
  }, [availableTasks, runDiagnostics])

  const runFullScan = useCallback(async () => {
    const allTasks = availableTasks
      .filter(task => !task.admin_required || systemInfo?.is_admin)
      .map(t => t.id)

    await runDiagnostics(allTasks)
  }, [availableTasks, systemInfo, runDiagnostics])

  const stopScan = useCallback(() => {
    setIsRunning(false)
    setCurrentProgress(0)
    setCurrentTaskName('')
  }, [setIsRunning, setCurrentProgress, setCurrentTaskName])

  const clearResults = useCallback(() => {
    setResults({})
    setSessionId(null)
    setCurrentProgress(0)
    setCurrentTaskName('')
  }, [setResults, setSessionId, setCurrentProgress, setCurrentTaskName])

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