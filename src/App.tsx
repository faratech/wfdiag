import React, { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { save } from '@tauri-apps/plugin-dialog'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { writeTextFile } from '@tauri-apps/plugin-fs'
import { SystemMonitoring } from './SystemMonitoring'
import { OpenAIIntegration } from './OpenAIIntegration'
import { ComparisonView } from './ComparisonView'
import './styles.css'
import { 
  Button,
  Tab,
  TabList,
  SelectTabData,
  FluentProvider,
  webDarkTheme,
  Text,
  Divider,
  tokens,
  Badge,
} from '@fluentui/react-components'

interface SystemInfo {
  computer_name: string
  os_version: string
  is_admin: boolean
}

interface DiagnosticTask {
  id: string
  name: string
  description: string
  category: string
  admin_required: boolean
}

interface TaskResult {
  success: boolean
  output: string
  error?: string
  duration_ms: number
}

type TabValue = 'diagnostics' | 'monitoring' | 'ai' | 'issues'

function App() {
  const [selectedTab, setSelectedTab] = useState<TabValue>('diagnostics')
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null)
  const [availableTasks, setAvailableTasks] = useState<DiagnosticTask[]>([])
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [results, setResults] = useState<Record<string, TaskResult>>({})
  const [isRunning, setIsRunning] = useState(false)
  const [currentProgress, setCurrentProgress] = useState(0)
  const [currentTaskName, setCurrentTaskName] = useState('')
  const [isMonitoringActive, setIsMonitoringActive] = useState(false)
  const [showComparison, setShowComparison] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const [filteredResults, setFilteredResults] = useState<Record<string, TaskResult>>({})
  const [scanStartTime, setScanStartTime] = useState<number>(0)
  const [issues, setIssues] = useState<any[]>([])

  useEffect(() => {
    loadSystemInfo()
    loadAvailableTasks()
  }, [])

  const loadSystemInfo = async () => {
    try {
      const info = await invoke<SystemInfo>('get_system_info')
      setSystemInfo(info)
    } catch (error) {
      console.error('Failed to load system info:', error)
    }
  }

  const loadAvailableTasks = async () => {
    try {
      const tasks = await invoke<DiagnosticTask[]>('get_available_tasks')
      setAvailableTasks(tasks)
    } catch (error) {
      console.error('Failed to load tasks:', error)
    }
  }

  const runQuickScan = async () => {
    // Essential tasks for quick system overview
    const quickTasks = availableTasks.filter(task => 
      ['comp_system', 'os_info', 'processor', 'physical_memory', 'disk_drive', 
       'logical_disk', 'network_adapter', 'systeminfo'].includes(task.id)
    ).map(t => t.id)
    
    await runDiagnostics(quickTasks)
  }

  const runFullScan = async () => {
    // All available tasks
    const allTasks = availableTasks
      .filter(task => !task.admin_required || systemInfo?.is_admin)
      .map(t => t.id)
    
    await runDiagnostics(allTasks)
  }

  const runDiagnostics = async (taskIds: string[]) => {
    if (taskIds.length === 0) return

    setIsRunning(true)
    setCurrentProgress(0)
    setResults({})
    setScanStartTime(Date.now())

    try {
      const sessionId = await invoke<string>('start_diagnostics', { taskIds })
      setSessionId(sessionId)

      // Set up progress tracking
      let completedTasks = 0
      const totalTasks = taskIds.length
      
      // Listen for task progress events
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
        // Run all diagnostics in parallel with max 5 concurrent
        const results = await invoke<Array<[string, TaskResult]>>('run_diagnostics_parallel', { 
          taskIds,
          maxConcurrent: 5 
        })
        
        // Convert array of tuples to object
        const resultsObj = results.reduce((acc, [taskId, result]) => {
          acc[taskId] = result
          return acc
        }, {} as Record<string, TaskResult>)
        
        setResults(resultsObj)
      } finally {
        // Clean up event listener
        unlisten()
      }

      setCurrentProgress(100)
      
      // Auto-save scan to history after results are set
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
        detectIssues()
      }, 500)
    } catch (error) {
      console.error('Failed to start diagnostics:', error)
      setIsRunning(false)
    }
  }

  const detectIssues = async () => {
    try {
      const issues = await invoke<any[]>('detect_issues')
      setIssues(issues)
    } catch (error) {
      console.error('Failed to detect issues:', error)
    }
  }

  const copyToClipboard = async () => {
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
  }

  const exportResults = async () => {
    if (!sessionId) return

    try {
      const content = await invoke<string>('export_results', { 
        format: 'text',
        includeRaw: true 
      })

      const fullReport = `=== WindowsForum Diagnostic Report ===
Generated: ${new Date().toLocaleString()}
Computer: ${systemInfo?.computer_name}
OS: ${systemInfo?.os_version}
Admin Mode: ${systemInfo?.is_admin ? 'Yes' : 'No'}
${content}`

      const filePath = await save({
        defaultPath: `wf-diagnostics-${new Date().toISOString().split('T')[0]}.txt`,
        filters: [{
          name: 'Text',
          extensions: ['txt']
        }]
      })

      if (filePath) {
        try {
          await writeTextFile(filePath, fullReport)
          console.log('File exported successfully to:', filePath)
        } catch (writeError) {
          console.error('Failed to write file:', writeError)
          // Try using the backend save function as a fallback
          try {
            await invoke('save_results_to_file', { 
              path: filePath,
              content: fullReport
            })
            console.log('File exported successfully using backend to:', filePath)
          } catch (backendError) {
            console.error('Backend save also failed:', backendError)
          }
        }
      }
    } catch (error) {
      console.error('Failed to export results:', error)
    }
  }

  const restartAsAdmin = async () => {
    try {
      await invoke('restart_as_admin')
    } catch (error) {
      console.error('Failed to restart as admin:', error)
    }
  }


  const getHealthScore = () => {
    const totalTasks = Object.keys(results).length
    if (totalTasks === 0) return null
    
    const successfulTasks = Object.values(results).filter(r => r.success).length
    return Math.round((successfulTasks / totalTasks) * 100)
  }

  // Search and filter functionality
  useEffect(() => {
    if (!searchQuery) {
      setFilteredResults(results)
      return
    }
    
    const query = searchQuery.toLowerCase()
    const filtered: Record<string, TaskResult> = {}
    
    for (const [taskId, result] of Object.entries(results)) {
      // Search in task ID
      if (taskId.toLowerCase().includes(query)) {
        filtered[taskId] = result
        continue
      }
      
      // Search in error messages
      if (result.error && result.error.toLowerCase().includes(query)) {
        filtered[taskId] = result
        continue
      }
      
      // Search in output
      if (result.output.toLowerCase().includes(query)) {
        filtered[taskId] = result
      }
    }
    
    setFilteredResults(filtered)
  }, [searchQuery, results])

  const parseOutput = (output: string) => {
    try {
      return JSON.parse(output)
    } catch {
      return output
    }
  }

  const renderResultItem = (key: string, value: any, depth: number = 0): React.ReactNode => {
    if (value === null || value === undefined || value === '') return null
    
    // Prevent infinite recursion
    if (depth > 10) {
      return (
        <div key={key} className="result-section" style={{ marginLeft: depth * 16 }}>
          <div className="result-label">
            <i className="fas fa-exclamation-triangle" style={{ fontSize: 10 }}></i>
            {key}
          </div>
          <div className="result-value">[Content too deeply nested to display]</div>
        </div>
      )
    }
    
    // Format key for better readability
    const formattedKey = key
      .replace(/_/g, ' ')
      .replace(/([A-Z])/g, ' $1')
      .replace(/^./, str => str.toUpperCase())
      .trim()
    
    // Add helpful icons based on key content
    const getIcon = (key: string) => {
      const lowerKey = key.toLowerCase()
      if (lowerKey.includes('error') || lowerKey.includes('fail')) return 'fa-times-circle'
      if (lowerKey.includes('success') || lowerKey.includes('pass')) return 'fa-check-circle'
      if (lowerKey.includes('warning')) return 'fa-exclamation-triangle'
      if (lowerKey.includes('info')) return 'fa-info-circle'
      if (lowerKey.includes('cpu')) return 'fa-microchip'
      if (lowerKey.includes('memory') || lowerKey.includes('ram')) return 'fa-memory'
      if (lowerKey.includes('disk') || lowerKey.includes('storage')) return 'fa-hdd'
      if (lowerKey.includes('network')) return 'fa-network-wired'
      if (lowerKey.includes('driver')) return 'fa-cogs'
      if (lowerKey.includes('service')) return 'fa-server'
      if (lowerKey.includes('process')) return 'fa-tasks'
      if (lowerKey.includes('version')) return 'fa-code-branch'
      if (lowerKey.includes('name')) return 'fa-tag'
      if (lowerKey.includes('path')) return 'fa-folder'
      if (lowerKey.includes('date') || lowerKey.includes('time')) return 'fa-clock'
      if (lowerKey.includes('size')) return 'fa-weight'
      if (lowerKey.includes('status')) return 'fa-signal'
      return 'fa-angle-right'
    }
    
    if (typeof value === 'boolean') {
      return (
        <div key={key} className="result-grid" style={{ marginLeft: depth * 16 }}>
          <div className="result-label">
            <i className={`fas ${getIcon(key)}`} style={{ fontSize: 10 }}></i>
            {formattedKey}
          </div>
          <div className="result-value" style={{ color: value ? '#10b981' : '#ef4444' }}>
            <i className={`fas ${value ? 'fa-check' : 'fa-times'}`} style={{ marginRight: 6 }}></i>
            {value ? 'Enabled' : 'Disabled'}
          </div>
        </div>
      )
    }
    
    if (typeof value === 'object' && value !== null) {
      // Check for circular reference by trying to stringify
      try {
        JSON.stringify(value)
      } catch (e) {
        return (
          <div key={key} className="result-section" style={{ marginLeft: depth * 16 }}>
            <div className="result-label">
              <i className="fas fa-exclamation-circle" style={{ fontSize: 10 }}></i>
              {formattedKey}
            </div>
            <div className="result-value" style={{ color: '#f59e0b' }}>[Circular reference detected]</div>
          </div>
        )
      }
      
      return (
        <div key={key} className="collapsible-section" style={{ marginTop: 8, marginLeft: depth * 16 }}>
          <div className="collapsible-header">
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <i className={`fas ${getIcon(key)}`} style={{ color: '#60a5fa' }}></i>
              <Text size={300} weight="semibold" style={{ color: '#f1f5f9' }}>{formattedKey}</Text>
              {Array.isArray(value) && (
                <span className="status-badge" style={{ fontSize: 11, padding: '2px 8px' }}>
                  {value.length} items
                </span>
              )}
            </div>
            <i className="fas fa-chevron-down" style={{ color: '#94a3b8' }}></i>
          </div>
          <div className="collapsible-content">
            {Array.isArray(value) ? (
              value.slice(0, 100).map((item, idx) => ( // Limit array items to prevent UI freeze
                <div key={`${key}-${idx}`} style={{ marginBottom: 8 }}>
                  {typeof item === 'object' && item !== null ? 
                    Object.entries(item).slice(0, 50).map(([k, v]) => renderResultItem(k, v, depth + 1)) : // Limit object properties
                    <Text size={200}>{String(item)}</Text>
                  }
                  {idx < value.length - 1 && idx < 99 && <Divider style={{ margin: '8px 0' }} />}
                </div>
              ))
            ) : (
              Object.entries(value).slice(0, 50).map(([k, v]) => renderResultItem(k, v, depth + 1)) // Limit object properties
            )}
            {Array.isArray(value) && value.length > 100 && (
              <Text size={200} style={{ fontStyle: 'italic', color: tokens.colorNeutralForeground3 }}>
                ... and {value.length - 100} more items
              </Text>
            )}
          </div>
        </div>
      )
    }
    
    return (
      <div key={key} className="result-grid" style={{ marginLeft: depth * 16 }}>
        <div className="result-label">
          <i className={`fas ${getIcon(key)}`} style={{ fontSize: 10 }}></i>
          {formattedKey}
        </div>
        <div className="result-value">
          {String(value)}
        </div>
      </div>
    )
  }

  const renderDiagnosticsTab = () => {
    const healthScore = getHealthScore()
    const hasResults = Object.keys(results).length > 0
    const resultsByCategory = availableTasks.reduce((acc, task) => {
      if (results[task.id]) {
        if (!acc[task.category]) acc[task.category] = []
        acc[task.category].push({ task, result: results[task.id] })
      }
      return acc
    }, {} as Record<string, Array<{ task: DiagnosticTask; result: TaskResult }>>)

    // Show comparison view if selected
    if (showComparison) {
      return (
        <ComparisonView
          onClose={() => setShowComparison(false)}
        />
      )
    }

    return (
      <div style={{ maxWidth: 1200, margin: '0 auto' }}>
        {/* Quick Actions - Welcome Screen */}
        {!isRunning && !hasResults && (
          <div className="glass-card" style={{ marginBottom: 24, padding: 48, textAlign: 'center' }}>
            <div className="gradient-bg" style={{ 
              width: 80, 
              height: 80, 
              borderRadius: '50%', 
              display: 'flex', 
              alignItems: 'center', 
              justifyContent: 'center',
              margin: '0 auto 24px',
              boxShadow: '0 8px 24px rgba(59, 130, 246, 0.4)'
            }}>
              <i className="fas fa-laptop-medical" style={{ color: 'white', fontSize: 40 }}></i>
            </div>
            
            <Text size={700} weight="bold" block style={{ marginBottom: 12, color: '#f1f5f9' }}>
              Welcome to System Diagnostics
            </Text>
            <Text size={400} block style={{ marginBottom: 32, color: '#94a3b8', maxWidth: 600, margin: '0 auto 32px' }}>
              Choose a scan type to analyze your Windows system. Quick Scan covers essential checks, 
              while Full Scan performs comprehensive analysis of all components.
            </Text>
            
            <div style={{ display: 'flex', gap: 20, justifyContent: 'center', marginBottom: 32 }}>
              <button 
                className="modern-button primary"
                onClick={runQuickScan}
                style={{ fontSize: 16, padding: '14px 28px' }}
              >
                <i className="fas fa-bolt"></i>
                Quick Scan
                <span style={{ opacity: 0.8, fontSize: 14, marginLeft: 8 }}>(~30 sec)</span>
              </button>
              <button 
                className="modern-button success"
                onClick={runFullScan}
                style={{ fontSize: 16, padding: '14px 28px' }}
              >
                <i className="fas fa-search-plus"></i>
                Full Scan
                <span style={{ opacity: 0.8, fontSize: 14, marginLeft: 8 }}>(3-5 min)</span>
              </button>
            </div>

            {/* Info Cards */}
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 16, marginTop: 32 }}>
              <div style={{ background: 'rgba(59, 130, 246, 0.1)', padding: 16, borderRadius: 8, border: '1px solid rgba(59, 130, 246, 0.3)' }}>
                <i className="fas fa-shield-alt" style={{ color: '#3b82f6', fontSize: 24, marginBottom: 8 }}></i>
                <Text size={300} weight="semibold" block style={{ color: '#f1f5f9', marginBottom: 4 }}>Security Check</Text>
                <Text size={200} style={{ color: '#94a3b8' }}>Analyzes system security settings</Text>
              </div>
              <div style={{ background: 'rgba(16, 185, 129, 0.1)', padding: 16, borderRadius: 8, border: '1px solid rgba(16, 185, 129, 0.3)' }}>
                <i className="fas fa-microchip" style={{ color: '#10b981', fontSize: 24, marginBottom: 8 }}></i>
                <Text size={300} weight="semibold" block style={{ color: '#f1f5f9', marginBottom: 4 }}>Hardware Analysis</Text>
                <Text size={200} style={{ color: '#94a3b8' }}>Checks CPU, RAM, and devices</Text>
              </div>
              <div style={{ background: 'rgba(139, 92, 246, 0.1)', padding: 16, borderRadius: 8, border: '1px solid rgba(139, 92, 246, 0.3)' }}>
                <i className="fas fa-cogs" style={{ color: '#8b5cf6', fontSize: 24, marginBottom: 8 }}></i>
                <Text size={300} weight="semibold" block style={{ color: '#f1f5f9', marginBottom: 4 }}>Performance</Text>
                <Text size={200} style={{ color: '#94a3b8' }}>Identifies bottlenecks</Text>
              </div>
            </div>

            {systemInfo && !systemInfo.is_admin && (
              <div className="status-badge warning" style={{ marginTop: 24, display: 'flex', flexDirection: 'column', alignItems: 'flex-start' }}>
                <div style={{ display: 'flex', alignItems: 'center', marginBottom: 8 }}>
                  <i className="fas fa-info-circle"></i>
                  <Text size={300} weight="semibold" style={{ color: '#f59e0b', marginLeft: 8 }}>
                    Running without administrator privileges
                  </Text>
                </div>
                <Text size={200} style={{ color: '#94a3b8', marginBottom: 8 }}>
                  {availableTasks.filter(task => !task.admin_required || systemInfo?.is_admin).length} of {availableTasks.length} diagnostic tasks available • 5 admin-only tasks hidden (disk check, DISM health, battery report, driver verifier, crash dumps)
                </Text>
                <Button 
                  appearance="transparent" 
                  size="small"
                  onClick={restartAsAdmin}
                  style={{ color: '#f59e0b', padding: '4px 12px', border: '1px solid rgba(245, 158, 11, 0.3)' }}
                >
                  <i className="fas fa-shield-alt" style={{ marginRight: 6 }}></i>
                  Restart as Administrator for Full Access
                </Button>
              </div>
            )}
          </div>
        )}

        {/* Progress - Scanning Animation */}
        {isRunning && (
          <div className="glass-card scan-animation" style={{ marginBottom: 24, padding: 48, textAlign: 'center' }}>
            <div style={{ position: 'relative', display: 'inline-block' }}>
              <i className="fas fa-spinner icon-spin" style={{ fontSize: 64, color: '#3b82f6', marginBottom: 24 }}></i>
            </div>
            <Text size={600} weight="bold" block style={{ marginBottom: 8, color: '#f1f5f9' }}>
              Scanning Your System...
            </Text>
            <Text size={300} block style={{ marginBottom: 24, color: '#94a3b8' }}>
              <i className="fas fa-tasks" style={{ marginRight: 8 }}></i>
              {currentTaskName}
            </Text>
            <div className="modern-progress" style={{ maxWidth: 500, margin: '0 auto 16px', height: 12 }}>
              <div className="modern-progress-bar" style={{ width: `${currentProgress}%` }}></div>
            </div>
            <Text size={400} weight="semibold" style={{ color: '#3b82f6' }}>
              {Math.round(currentProgress)}% Complete
            </Text>
          </div>
        )}

        {/* Search and Comparison Bar */}
        {hasResults && !isRunning && (
          <div className="glass-card" style={{ 
            marginBottom: 24, 
            padding: 16,
            background: 'linear-gradient(135deg, rgba(59, 130, 246, 0.05), rgba(139, 92, 246, 0.05))'
          }}>
            <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
              <div style={{ flex: 1, position: 'relative' }}>
                <input
                  type="text"
                  placeholder="Search in results (task names, errors, output)..."
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  style={{
                    width: '100%',
                    padding: '10px 40px 10px 16px',
                    background: 'rgba(30, 41, 59, 0.6)',
                    border: '1px solid rgba(59, 130, 246, 0.3)',
                    borderRadius: 8,
                    color: '#f1f5f9',
                    fontSize: 14
                  }}
                />
                <i className="fas fa-search" style={{
                  position: 'absolute',
                  right: 16,
                  top: '50%',
                  transform: 'translateY(-50%)',
                  color: '#94a3b8'
                }}></i>
              </div>
              <button 
                className="modern-button"
                onClick={() => {
                  console.log('Opening comparison view')
                  setShowComparison(true)
                }}
                style={{
                  background: 'linear-gradient(135deg, #8b5cf6, #3b82f6)',
                  padding: '10px 20px'
                }}
              >
                <i className="fas fa-history" style={{ marginRight: 8 }}></i>
                Compare with Previous
              </button>
              {searchQuery && (
                <div style={{ 
                  padding: '6px 12px',
                  background: 'rgba(59, 130, 246, 0.2)',
                  borderRadius: 6,
                  fontSize: 13,
                  color: '#60a5fa',
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8
                }}>
                  <i className="fas fa-filter" style={{ fontSize: 12 }}></i>
                  {Object.keys(filteredResults).length} of {Object.keys(results).length} results
                </div>
              )}
              {searchQuery && (
                <button
                  className="modern-button"
                  onClick={() => setSearchQuery('')}
                  style={{
                    background: 'rgba(239, 68, 68, 0.2)',
                    border: '1px solid rgba(239, 68, 68, 0.3)',
                    padding: '6px 12px',
                    fontSize: 13
                  }}
                >
                  <i className="fas fa-times" style={{ marginRight: 6 }}></i>
                  Clear
                </button>
              )}
            </div>
          </div>
        )}

        {/* Results with Left Sidebar Navigation */}
        {hasResults && !isRunning && !showComparison && (
          <div style={{ display: 'flex', gap: 24, height: 'calc(100vh - 250px)' }}>
            {/* Left Sidebar Navigation */}
            <div className="glass-card scrollable-container" style={{ 
              width: 280, 
              padding: 20,
              height: '100%',
              position: 'sticky',
              top: 20,
              overflowY: 'scroll',
              overflowX: 'hidden'
            }}>
              <div style={{ marginBottom: 24 }}>
                <Text size={400} weight="bold" style={{ color: '#f1f5f9', marginBottom: 12, display: 'block' }}>
                  <i className="fas fa-chart-pie" style={{ marginRight: 8 }}></i>
                  Health Score
                </Text>
                <div className="health-score-circle" style={{ 
                  '--score': healthScore,
                  width: 80,
                  height: 80,
                  margin: '0 auto',
                  position: 'relative'
                } as React.CSSProperties}>
                  <div className="health-score-value" style={{ fontSize: 24, fontWeight: 'bold' }}>
                    {healthScore}%
                  </div>
                </div>
                
                <div style={{ marginTop: 12, textAlign: 'center' }}>
                  {healthScore !== null && healthScore >= 90 && (
                    <div className="status-badge success">
                      <i className="fas fa-check-circle"></i>
                      Excellent
                    </div>
                  )}
                  {healthScore !== null && healthScore >= 70 && healthScore < 90 && (
                    <div className="status-badge warning">
                      <i className="fas fa-exclamation-circle"></i>
                      Good
                    </div>
                  )}
                  {healthScore !== null && healthScore < 70 && (
                    <div className="status-badge danger">
                      <i className="fas fa-times-circle"></i>
                      Needs Attention
                    </div>
                  )}
                </div>
              </div>

              <Divider style={{ margin: '20px 0' }} />
              
              {/* Navigation Menu */}
              <Text size={300} weight="semibold" style={{ color: '#94a3b8', marginBottom: 12, display: 'block' }}>
                CATEGORIES
              </Text>
              {Object.entries(resultsByCategory).map(([category, items]) => {
                const catInfo = (() => {
                  const categoryMap: Record<string, { icon: string, color: string }> = {
                    'System': { icon: 'fa-desktop', color: '#3b82f6' },
                    'Hardware': { icon: 'fa-microchip', color: '#10b981' },
                    'Storage': { icon: 'fa-hdd', color: '#8b5cf6' },
                    'Network': { icon: 'fa-network-wired', color: '#f59e0b' },
                    'Drivers': { icon: 'fa-cogs', color: '#ef4444' },
                    'Software': { icon: 'fa-th', color: '#06b6d4' },
                    'Logs': { icon: 'fa-file-alt', color: '#ec4899' },
                    'Debug': { icon: 'fa-bug', color: '#a855f7' },
                    'Performance': { icon: 'fa-tachometer-alt', color: '#14b8a6' }
                  }
                  return categoryMap[category] || { icon: 'fa-folder', color: '#64748b' }
                })()
                
                return (
                  <a
                    key={category}
                    href={`#category-${category}`}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'space-between',
                      padding: '10px 12px',
                      borderRadius: 8,
                      marginBottom: 4,
                      textDecoration: 'none',
                      background: 'rgba(30, 41, 59, 0.3)',
                      transition: 'all 0.2s'
                    }}
                    onMouseEnter={(e) => {
                      e.currentTarget.style.background = 'rgba(59, 130, 246, 0.1)'
                    }}
                    onMouseLeave={(e) => {
                      e.currentTarget.style.background = 'rgba(30, 41, 59, 0.3)'
                    }}
                  >
                    <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                      <i className={`fas ${catInfo.icon}`} style={{ color: catInfo.color, width: 16 }}></i>
                      <Text size={200} style={{ color: '#f1f5f9' }}>{category}</Text>
                    </div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                      <span style={{ 
                        fontSize: 11, 
                        color: items.every(i => i.result.success) ? '#10b981' : '#f59e0b'
                      }}>
                        {items.filter(i => i.result.success).length}/{items.length}
                      </span>
                    </div>
                  </a>
                )
              })}
              
              <Divider style={{ margin: '20px 0' }} />
              
              {/* Actions */}
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                <button className="modern-button primary" onClick={copyToClipboard} style={{ width: '100%' }}>
                  <i className="fas fa-copy"></i>
                  Copy Report
                </button>
                <button className="modern-button success" onClick={exportResults} style={{ width: '100%' }}>
                  <i className="fas fa-download"></i>
                  Export
                </button>
                <button className="modern-button" onClick={() => setResults({})} style={{ 
                  width: '100%',
                  background: 'rgba(30, 41, 59, 0.8)',
                  color: '#94a3b8'
                }}>
                  <i className="fas fa-redo"></i>
                  New Scan
                </button>
              </div>
            </div>

            {/* Main Content Area - All Results Visible */}
            <div className="scrollable-container" style={{ 
              flex: 1, 
              overflowY: 'scroll',
              overflowX: 'hidden',
              height: '100%',
              paddingRight: 10
            }}>
              {searchQuery && Object.keys(filteredResults).length === 0 && (
                <div className="glass-card" style={{ 
                  padding: 48, 
                  textAlign: 'center',
                  marginBottom: 24
                }}>
                  <i className="fas fa-search" style={{ fontSize: 48, color: '#64748b', marginBottom: 16 }}></i>
                  <Text size={400} style={{ color: '#94a3b8', display: 'block' }}>
                    No results found for "{searchQuery}"
                  </Text>
                  <Text size={200} style={{ color: '#64748b', display: 'block', marginTop: 8 }}>
                    Try searching for different keywords
                  </Text>
                </div>
              )}
              {Object.entries(searchQuery ? 
                // Filter categories based on search results
                availableTasks.reduce((acc, task) => {
                  if (filteredResults[task.id]) {
                    if (!acc[task.category]) acc[task.category] = []
                    acc[task.category].push({ task, result: filteredResults[task.id] })
                  }
                  return acc
                }, {} as Record<string, Array<{ task: DiagnosticTask; result: TaskResult }>>)
                : resultsByCategory
              ).map(([category, items]) => {
                const getCategoryInfo = (cat: string) => {
                  const categoryMap: Record<string, { icon: string, color: string, description: string }> = {
                    'System': { icon: 'fa-desktop', color: '#3b82f6', description: 'Operating system and configuration' },
                    'Hardware': { icon: 'fa-microchip', color: '#10b981', description: 'CPU, RAM, motherboard details' },
                    'Storage': { icon: 'fa-hdd', color: '#8b5cf6', description: 'Disk drives and storage information' },
                    'Network': { icon: 'fa-network-wired', color: '#f59e0b', description: 'Network adapters and connectivity' },
                    'Drivers': { icon: 'fa-cogs', color: '#ef4444', description: 'Device drivers and versions' },
                    'Software': { icon: 'fa-th', color: '#06b6d4', description: 'Installed programs and features' },
                    'Logs': { icon: 'fa-file-alt', color: '#ec4899', description: 'Event logs and system history' },
                    'Debug': { icon: 'fa-bug', color: '#a855f7', description: 'Debugging and crash information' },
                    'Performance': { icon: 'fa-tachometer-alt', color: '#14b8a6', description: 'System performance metrics' }
                  }
                  return categoryMap[cat] || { icon: 'fa-folder', color: '#64748b', description: 'System information' }
                }
                
                const catInfo = getCategoryInfo(category)
                
                try {
                  return (
                    <div key={category} id={`category-${category}`} className="glass-card" style={{ marginBottom: 24, padding: 0 }}>
                      {/* Category Header - Always Visible */}
                      <div style={{ 
                        background: `linear-gradient(135deg, ${catInfo.color}22, ${catInfo.color}11)`,
                        borderBottom: '1px solid rgba(255, 255, 255, 0.1)',
                        padding: 20
                      }}>
                        <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
                          <div className="category-icon" style={{ background: catInfo.color }}>
                            <i className={`fas ${catInfo.icon}`}></i>
                          </div>
                          <div style={{ flex: 1 }}>
                            <Text size={500} weight="bold" style={{ color: '#f1f5f9' }}>{category}</Text>
                            <Text size={300} style={{ color: '#94a3b8', display: 'block', marginTop: 2 }}>
                              {catInfo.description}
                            </Text>
                          </div>
                          {items.every(i => i.result.success) ? (
                            <div className="status-badge success">
                              <i className="fas fa-check-circle"></i>
                              All Tests Passed
                            </div>
                          ) : (
                            <div className="status-badge warning">
                              <i className="fas fa-exclamation-circle"></i>
                              {items.filter(i => i.result.success).length} of {items.length} Passed
                            </div>
                          )}
                        </div>
                      </div>
                      
                      {/* All Results - Always Visible */}
                      <div style={{ padding: 20 }}>
                        {items.map(({ task, result }) => {
                          try {
                            return (
                              <div key={task.id} className="result-section" style={{ 
                                marginBottom: 16,
                                padding: 16,
                                borderRadius: 8,
                                background: 'rgba(30, 41, 59, 0.3)'
                              }}>
                                {/* Task Header */}
                                <div style={{ 
                                  display: 'flex',
                                  alignItems: 'center',
                                  gap: 12,
                                  marginBottom: 12,
                                  paddingBottom: 12,
                                  borderBottom: '1px solid rgba(255, 255, 255, 0.05)'
                                }}>
                                  {result.success ? 
                                    <i className="fas fa-check-circle" style={{ color: '#10b981', fontSize: 18 }}></i> :
                                    <i className="fas fa-times-circle" style={{ color: '#ef4444', fontSize: 18 }}></i>
                                  }
                                  <Text size={300} weight="semibold" style={{ color: '#f1f5f9' }}>{task.name}</Text>
                                  <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 8 }}>
                                    <i className="fas fa-clock" style={{ fontSize: 10, color: '#64748b' }}></i>
                                    <Text size={200} style={{ color: '#64748b' }}>
                                      {result.duration_ms}ms
                                    </Text>
                                  </div>
                                </div>
                                
                                {/* Task Results - Always Visible */}
                                <div style={{ paddingLeft: 30 }}>
                            {result.error ? (
                              <Text size={200} style={{ color: tokens.colorPaletteRedForeground1 }}>
                                Error: {result.error}
                              </Text>
                            ) : (
                              <div>
                                {(() => {
                                  try {
                                    const parsed = parseOutput(result.output)
                                    if (typeof parsed === 'object' && parsed !== null) {
                                      if (Array.isArray(parsed)) {
                                        return parsed.map((item, idx) => (
                                          <div key={idx} style={{ marginBottom: 12 }}>
                                            {typeof item === 'object' && item !== null ? 
                                              Object.entries(item).filter(([, v]) => v !== undefined).map(([k, v]) => <React.Fragment key={k}>{renderResultItem(k, v)}</React.Fragment>) :
                                              <Text size={200}>{String(item)}</Text>
                                            }
                                            {idx < parsed.length - 1 && <Divider style={{ marginTop: 8 }} />}
                                          </div>
                                        ))
                                      } else {
                                        return Object.entries(parsed).filter(([, v]) => v !== undefined).map(([k, v]) => <React.Fragment key={k}>{renderResultItem(k, v)}</React.Fragment>)
                                      }
                                    }
                                    return <Text size={200} style={{ whiteSpace: 'pre-wrap' }}>{result.output}</Text>
                                  } catch (error) {
                                    console.error('Error rendering result:', error, 'Task:', task.name, 'Output:', result.output)
                                    return <Text size={200} style={{ whiteSpace: 'pre-wrap' }}>{String(result.output)}</Text>
                                  }
                                })()}
                              </div>
                            )}
                                </div>
                              </div>
                            )
                          } catch (error) {
                            console.error('Error rendering task:', task.name, error)
                            return (
                              <div key={task.id} className="result-section" style={{ marginBottom: 16, padding: 12 }}>
                                <Text size={200} style={{ color: '#ef4444' }}>
                                  <i className="fas fa-exclamation-triangle" style={{ marginRight: 8 }}></i>
                                  Error rendering {task.name}
                                </Text>
                              </div>
                            )
                          }
                        })}
                      </div>
                    </div>
                  )
                } catch (categoryError) {
                  console.error('Error rendering category:', category, categoryError)
                  return (
                    <div key={category} className="glass-card" style={{ marginBottom: 16, padding: 20 }}>
                      <Text size={200} style={{ color: '#ef4444' }}>
                        <i className="fas fa-exclamation-triangle" style={{ marginRight: 8 }}></i>
                        Error rendering category: {category}
                      </Text>
                    </div>
                  )
                }
              })}
            </div>
          </div>
        )}
        
      </div>
    )
  }

  const renderIssuesTab = () => {
    return (
      <div style={{ maxWidth: 1200, margin: '0 auto' }}>
        <div className="glass-card" style={{ padding: 24 }}>
          <Text size={500} weight="semibold" style={{ color: '#f1f5f9', display: 'block', marginBottom: 16 }}>
            <i className="fas fa-exclamation-triangle" style={{ marginRight: 8, color: '#ef4444' }}></i>
            Detected Issues
          </Text>
          {issues.length === 0 ? (
            <div style={{ textAlign: 'center', padding: 48, color: '#94a3b8' }}>
              <Text size={300}>No issues detected.</Text>
            </div>
          ) : (
            <div>
              {issues.map((issue, index) => (
                <div
                  key={index}
                  style={{
                    padding: 16,
                    marginBottom: 8,
                    background: 'rgba(30, 41, 59, 0.3)',
                    borderRadius: 8,
                    border: '1px solid rgba(71, 85, 105, 0.3)'
                  }}
                >
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                      <Text size={400} weight="semibold" style={{ color: '#f1f5f9' }}>
                        {issue.title}
                      </Text>
                    </div>
                    <div style={{ display: 'flex', gap: 8 }}>
                      <Badge appearance="tint" style={{ background: 'rgba(100, 116, 139, 0.2)', color: '#94a3b8' }}>
                        {issue.category}
                      </Badge>
                      <Badge 
                        appearance="filled"
                        style={{ 
                          background: issue.severity === 'Critical' ? '#ef4444' :
                                     issue.severity === 'Warning' ? '#f59e0b' :
                                     '#10b981'
                        }}
                      >
                        {issue.severity}
                      </Badge>
                    </div>
                  </div>
                  <Text size={200} style={{ color: '#cbd5e1', display: 'block', marginBottom: 8 }}>
                    {issue.description}
                  </Text>
                  <Text size={200} style={{ color: '#94a3b8' }}>
                    {issue.recommendation}
                  </Text>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    )
  }

  return (
    <FluentProvider theme={webDarkTheme}>
      <div style={{ 
        minHeight: '100vh', 
        background: '#0f172a', 
        color: '#f1f5f9',
        height: '100vh',
        position: 'relative',
        overflow: 'hidden'
      }}>
        {/* Modern Header with Glass Effect */}
        <header className="glass-card" style={{ 
          borderRadius: 0,
          borderTop: 'none',
          borderLeft: 'none',
          borderRight: 'none',
          padding: '20px 24px',
          marginBottom: 0
        }}>
          <div style={{ maxWidth: 1200, margin: '0 auto', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
              <div style={{ 
                width: 45, 
                height: 45, 
                borderRadius: 10, 
                display: 'flex', 
                alignItems: 'center', 
                justifyContent: 'center',
                boxShadow: '0 4px 12px rgba(59, 130, 246, 0.3)',
                overflow: 'hidden',
                background: 'white'
              }}>
                <img 
                  src="/icon.png" 
                  alt="WF Diagnostics" 
                  style={{ 
                    width: '100%', 
                    height: '100%', 
                    objectFit: 'contain' 
                  }}
                />
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                <Text size={600} weight="bold" style={{ color: '#f1f5f9' }}>WF Diagnostics</Text>
                <Text size={200} style={{ color: '#94a3b8' }}>Professional System Analysis Tool</Text>
              </div>
              <div className="status-badge success" style={{ marginLeft: 16 }}>
                <i className="fas fa-check-circle"></i>
                v2.0.8
              </div>
            </div>
            {systemInfo && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 20 }}>
                <div className="tooltip">
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <i className="fas fa-desktop" style={{ color: '#94a3b8' }}></i>
                    <Text size={300} style={{ color: '#94a3b8' }}>{systemInfo.computer_name}</Text>
                  </div>
                  <span className="tooltip-text">{systemInfo.os_version}</span>
                </div>
                {systemInfo.is_admin ? (
                  <div className="status-badge success">
                    <i className="fas fa-shield-alt"></i>
                    Admin
                  </div>
                ) : (
                  <div className="status-badge warning">
                    <i className="fas fa-user"></i>
                    <span>Standard</span>
                  </div>
                )}
              </div>
            )}
          </div>
        </header>

        {/* Modern Tab Navigation */}
        <div style={{ 
          background: 'rgba(30, 41, 59, 0.5)',
          borderBottom: '1px solid rgba(255, 255, 255, 0.1)',
          padding: '0 24px'
        }}>
          <div style={{ maxWidth: 1200, margin: '0 auto' }}>
            <TabList 
              selectedValue={selectedTab} 
              onTabSelect={(_, data: SelectTabData) => setSelectedTab(data.value as TabValue)}
              style={{ borderBottom: 'none' }}
            >
              <Tab value="diagnostics" style={{ 
                color: selectedTab === 'diagnostics' ? '#3b82f6' : '#94a3b8',
                fontWeight: selectedTab === 'diagnostics' ? 600 : 400
              }}>
                <i className="fas fa-stethoscope" style={{ marginRight: 8 }}></i>
                Diagnostics
              </Tab>
              <Tab value="monitoring" style={{ 
                color: selectedTab === 'monitoring' ? '#3b82f6' : '#94a3b8',
                fontWeight: selectedTab === 'monitoring' ? 600 : 400
              }}>
                <i className="fas fa-chart-line" style={{ marginRight: 8 }}></i>
                System Monitor
              </Tab>
              <Tab value="ai" style={{ 
                color: selectedTab === 'ai' ? '#3b82f6' : '#94a3b8',
                fontWeight: selectedTab === 'ai' ? 600 : 400
              }}>
                <i className="fas fa-brain" style={{ marginRight: 8 }}></i>
                AI Analysis
              </Tab>
              <Tab value="issues" style={{ 
                color: selectedTab === 'issues' ? '#ef4444' : '#94a3b8',
                fontWeight: selectedTab === 'issues' ? 600 : 400
              }}>
                <i className="fas fa-exclamation-triangle" style={{ marginRight: 8 }}></i>
                Issues ({issues.length})
              </Tab>
            </TabList>
          </div>
        </div>

        {/* Main Content Area */}
        <main style={{ 
          padding: '24px',
          height: 'calc(100vh - 180px)',
          overflowY: 'auto',
          overflowX: 'hidden'
        }}>
          <div style={{ maxWidth: selectedTab === 'ai' ? 1400 : 1200, margin: '0 auto' }}>
            {selectedTab === 'diagnostics' && renderDiagnosticsTab()}
            {selectedTab === 'monitoring' && (
              <SystemMonitoring 
                isActive={isMonitoringActive} 
                onToggle={setIsMonitoringActive}
              />
            )}
            {selectedTab === 'ai' && (
              <OpenAIIntegration 
                sessionId={sessionId || ''}
              />
            )}
            {selectedTab === 'issues' && renderIssuesTab()}
          </div>
        </main>
      </div>
    </FluentProvider>
  )
}

export default App