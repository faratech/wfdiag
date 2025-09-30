import { useEffect, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { save } from '@tauri-apps/plugin-dialog'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { writeTextFile } from '@tauri-apps/plugin-fs'
import { useAppContext, type SystemInfo, type DiagnosticTask, type Issue } from '../contexts/AppContext'

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

  const loadSystemInfo = useCallback(async () => {
    try {
      const info = await invoke<SystemInfo>('get_system_info')
      setSystemInfo(info)
    } catch (error) {
      console.error('Failed to load system info:', error)
    }
  }, [setSystemInfo])

  const loadAvailableTasks = useCallback(async () => {
    try {
      const tasks = await invoke<DiagnosticTask[]>('get_available_tasks')
      setAvailableTasks(tasks)
    } catch (error) {
      console.error('Failed to load tasks:', error)
    }
  }, [setAvailableTasks])

  const detectIssues = useCallback(async () => {
    try {
      const issues = await invoke<Issue[]>('detect_issues')
      setIssues(issues)
    } catch (error) {
      console.error('Failed to detect issues:', error)
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
      console.error('Failed to copy to clipboard:', error)
    }
  }, [sessionId, systemInfo])

  const exportResults = useCallback(async () => {
    if (!sessionId) return

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
        try {
          await writeTextFile(filePath, fullReport)
        } catch (writeError) {
          console.error('Failed to write file:', writeError)
          try {
            await invoke('save_results_to_file', {
              path: filePath,
              content: fullReport
            })
          } catch (backendError) {
            console.error('Backend save also failed:', backendError)
          }
        }
      }
    } catch (error) {
      console.error('Failed to export results:', error)
    }
  }, [sessionId, systemInfo, settings.exportFormat])

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

      alert('Diagnostic report copied to clipboard!\n\nThe WindowsForum new thread page will open in your browser.\nSimply paste (Ctrl+V) the report into your post.')
    } catch (error) {
      console.error('Failed to share to WindowsForum:', error)
      alert('Failed to prepare share. Please try copying to clipboard instead.')
    }
  }, [sessionId, systemInfo])

  const emailReport = useCallback(async () => {
    if (!sessionId) return

    try {
      const content = await invoke<string>('export_results', {
        format: 'text',
        includeRaw: false
      })

      const subject = `Diagnostic Report - ${systemInfo?.computer_name} - ${new Date().toLocaleDateString()}`
      const body = encodeURIComponent(`WindowsForum Diagnostic Report

Generated: ${new Date().toLocaleString()}
Computer: ${systemInfo?.computer_name}
OS: ${systemInfo?.os_version}

${content}`)

      const mailtoLink = `mailto:?subject=${encodeURIComponent(subject)}&body=${body}`
      await invoke('open_url', { url: mailtoLink })
    } catch (error) {
      console.error('Failed to email report:', error)
      alert('Failed to prepare email. Please try exporting the report instead.')
    }
  }, [sessionId, systemInfo])

  const generateSupportPackage = useCallback(async () => {
    if (!sessionId) return

    try {
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
        await writeTextFile(jsonPath, jsonContent)

        const textPath = jsonPath.replace('.json', '.txt')
        await writeTextFile(textPath, textContent)

        const htmlPath = jsonPath.replace('.json', '.html')
        await writeTextFile(htmlPath, htmlContent)

        alert(`Support package generated successfully!\n\nFiles saved:\n- ${jsonPath}\n- ${textPath}\n- ${htmlPath}`)
      }
    } catch (error) {
      console.error('Failed to generate support package:', error)
      alert('Failed to generate support package. Please try exporting individual files.')
    }
  }, [sessionId])

  const restartAsAdmin = useCallback(async () => {
    try {
      await invoke('restart_as_admin')
    } catch (error) {
      console.error('Failed to restart as admin:', error)
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