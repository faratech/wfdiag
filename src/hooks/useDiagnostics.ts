import { useEffect, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { save } from '@tauri-apps/plugin-dialog'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { useAppContext, type SystemInfo, type DiagnosticTask, type Issue } from '../contexts/AppContext'
import { useToast } from '../contexts/ToastContext'
import * as logger from '../utils/logger'

export const useDiagnostics = () => {
  const {
    systemInfo,
    setSystemInfo,
    availableTasks,
    setAvailableTasks,
    sessionId,
    results,
    settings,
    setIssues,
  } = useAppContext()

  const { showSuccess, showError } = useToast()

  const loadSystemInfo = useCallback(async () => {
    try {
      const info = await invoke<SystemInfo>('get_system_info')
      setSystemInfo(info)
    } catch (error) {
      logger.error('useDiagnostics', 'Failed to load system info', error)
    }
  }, [setSystemInfo])

  const loadAvailableTasks = useCallback(async () => {
    try {
      const tasks = await invoke<DiagnosticTask[]>('get_available_tasks')
      setAvailableTasks(tasks)
    } catch (error) {
      logger.error('useDiagnostics', 'Failed to load tasks', error)
    }
  }, [setAvailableTasks])

  const detectIssues = useCallback(async () => {
    try {
      const issues = await invoke<Issue[]>('detect_issues')
      setIssues(issues)
    } catch (error) {
      logger.error('useDiagnostics', 'Failed to detect issues', error)
    }
  }, [setIssues])

  const getHealthScore = useCallback(() => {
    const totalTasks = Object.keys(results).length
    if (totalTasks === 0) return null

    const successfulTasks = Object.values(results).filter(r => r.success).length
    return Math.round((successfulTasks / totalTasks) * 100)
  }, [results])

  const copyToClipboard = useCallback(async () => {
    if (!sessionId) return

    try {
      const content = await invoke<string>('export_results', {
        format: 'text',
        includeRaw: true
      })

      const forumPost = `[CODE]
=== WindowsForum Diagnostic Report ===
Generated: ${new Date().toLocaleString()}
Computer: ${systemInfo?.computer_name}
OS: ${systemInfo?.os_version}
Admin Mode: ${systemInfo?.is_admin ? 'Yes' : 'No'}
${content}
[/CODE]`

      await writeText(forumPost)
    } catch (error) {
      logger.error('useDiagnostics', 'Failed to copy to clipboard', error)
    }
  }, [sessionId, systemInfo])

  const exportResults = useCallback(async () => {
    if (!sessionId) {
      showError('No Results', 'Please run a scan first before exporting.')
      return
    }

    try {
      const content = await invoke<string>('export_results', {
        format: settings.exportFormat || 'text',
        includeRaw: true
      })

      const fullReport = `=== WindowsForum Diagnostic Report ===
Generated: ${new Date().toLocaleString()}
Computer: ${systemInfo?.computer_name}
OS: ${systemInfo?.os_version}
Admin Mode: ${systemInfo?.is_admin ? 'Yes' : 'No'}
${content}`

      const extension = settings.exportFormat === 'json' ? 'json' :
                       settings.exportFormat === 'html' ? 'html' : 'txt'

      const filePath = await save({
        defaultPath: `wf-diagnostics-${new Date().toISOString().split('T')[0]}.${extension}`,
        filters: [{
          name: extension.toUpperCase(),
          extensions: [extension]
        }]
      })

      if (filePath) {
        // Use backend to write files (no fs:scope restrictions)
        await invoke('save_results_to_file', {
          path: filePath,
          content: fullReport
        })
        showSuccess('Export Complete', `Results saved to ${filePath}`)
      }
    } catch (error) {
      logger.error('useDiagnostics', 'Failed to export results', error)
      showError('Export Failed', 'Failed to save the file. Please try a different location.')
    }
  }, [sessionId, systemInfo, settings.exportFormat, showSuccess, showError])

  const shareToWindowsForum = useCallback(async () => {
    if (!sessionId) return

    try {
      const content = await invoke<string>('export_results', {
        format: 'text',
        includeRaw: false
      })

      const forumPost = `[B]WindowsForum Diagnostic Report[/B]
[CODE]
Generated: ${new Date().toLocaleString()}
Computer: ${systemInfo?.computer_name}
OS: ${systemInfo?.os_version}
Admin Mode: ${systemInfo?.is_admin ? 'Yes' : 'No'}

${content}
[/CODE]

[I]Generated using WindowsForum Diagnostics Tool[/I]`

      await writeText(forumPost)

      await invoke('open_url', { url: 'https://windowsforum.com/forums/windows-help-and-support.302/post-thread' })

      showSuccess(
        'Report Ready to Share!',
        'Diagnostic report copied to clipboard. The WindowsForum new thread page will open in your browser. Simply paste (Ctrl+V) the report into your post.'
      )
    } catch (error) {
      logger.error('useDiagnostics', 'Failed to share to WindowsForum', error)
      showError(
        'Failed to Share',
        'Failed to prepare share. Please try copying to clipboard instead.'
      )
    }
  }, [sessionId, systemInfo, showSuccess, showError])

  const emailReport = useCallback(async () => {
    if (!sessionId) {
      showError('No Results', 'Please run a scan first before emailing a report.')
      return
    }

    try {
      const content = await invoke<string>('export_results', {
        format: 'text',
        includeRaw: false
      })

      // Copy full report to clipboard first (mailto has length limits)
      const fullReport = `WindowsForum Diagnostic Report

Generated: ${new Date().toLocaleString()}
Computer: ${systemInfo?.computer_name}
OS: ${systemInfo?.os_version}

${content}`

      await writeText(fullReport)

      // Create a short mailto link - body will be pasted by user
      const subject = `Diagnostic Report - ${systemInfo?.computer_name} - ${new Date().toLocaleDateString()}`
      const mailtoLink = `mailto:?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent('[Report copied to clipboard - paste here with Ctrl+V]')}`

      await invoke('open_url', { url: mailtoLink })

      showSuccess(
        'Email Ready!',
        'The diagnostic report has been copied to your clipboard. Paste it into the email body with Ctrl+V.'
      )
    } catch (error) {
      logger.error('useDiagnostics', 'Failed to email report', error)
      showError(
        'Failed to Email Report',
        'Failed to prepare email. Please try exporting the report instead.'
      )
    }
  }, [sessionId, systemInfo, showError, showSuccess])

  const generateSupportPackage = useCallback(async () => {
    if (!sessionId) {
      showError('No Results', 'Please run a scan first before generating a support package.')
      return
    }

    try {
      logger.info('useDiagnostics', 'Starting support package generation...')

      const jsonContent = await invoke<string>('export_results', {
        format: 'json',
        includeRaw: true
      })

      const textContent = await invoke<string>('export_results', {
        format: 'text',
        includeRaw: true
      })

      const htmlContent = await invoke<string>('export_results', {
        format: 'html',
        includeRaw: true
      })

      const timestamp = new Date().toISOString().replace(/[:.]/g, '-')
      const packageName = `support-package-${timestamp}`

      const jsonPath = await save({
        defaultPath: `${packageName}.json`,
        filters: [{ name: 'JSON', extensions: ['json'] }]
      })

      if (jsonPath) {
        // Get base path by removing extension (handle paths with or without .json)
        const basePath = jsonPath.endsWith('.json')
          ? jsonPath.slice(0, -5)
          : jsonPath

        const actualJsonPath = `${basePath}.json`
        const textPath = `${basePath}.txt`
        const htmlPath = `${basePath}.html`

        // Use backend to write files (no fs:scope restrictions)
        await invoke('save_results_to_file', { path: actualJsonPath, content: jsonContent })
        await invoke('save_results_to_file', { path: textPath, content: textContent })
        await invoke('save_results_to_file', { path: htmlPath, content: htmlContent })

        showSuccess(
          'Support Package Generated!',
          `Files saved:\n• ${actualJsonPath}\n• ${textPath}\n• ${htmlPath}`
        )
      }
    } catch (error: unknown) {
      logger.error('useDiagnostics', 'Failed to generate support package', error)
      let errorMsg = 'Unknown error'
      if (error instanceof Error) {
        errorMsg = error.message
      } else if (typeof error === 'string') {
        errorMsg = error
      } else if (error && typeof error === 'object' && 'message' in error) {
        errorMsg = String((error as { message: unknown }).message)
      } else if (error) {
        errorMsg = String(error)
      }
      showError(
        'Failed to Generate Support Package',
        `Error: ${errorMsg}. Please try exporting individual files.`
      )
    }
  }, [sessionId, showSuccess, showError])

  const restartAsAdmin = useCallback(async () => {
    try {
      await invoke('restart_as_admin')
    } catch (error) {
      logger.error('useDiagnostics', 'Failed to restart as admin', error)
    }
  }, [])

  useEffect(() => {
    loadSystemInfo()
    loadAvailableTasks()
  }, [loadSystemInfo, loadAvailableTasks])

  return {
    systemInfo,
    availableTasks,
    loadSystemInfo,
    loadAvailableTasks,
    detectIssues,
    getHealthScore,
    copyToClipboard,
    exportResults,
    shareToWindowsForum,
    emailReport,
    generateSupportPackage,
    restartAsAdmin,
  }
}