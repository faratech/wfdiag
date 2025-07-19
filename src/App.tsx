import React, { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/tauri'
import { save } from '@tauri-apps/api/dialog'
import { writeText } from '@tauri-apps/api/clipboard'
import { writeTextFile } from '@tauri-apps/api/fs'
import { SystemMonitoring } from './SystemMonitoring'
import { 
  Switch,
  Button,
  Card,
  Checkbox,
  ProgressBar,
  Spinner,
  MessageBar,
  MessageBarBody,
  MessageBarTitle,
  Dialog,
  DialogTrigger,
  DialogSurface,
  DialogTitle,
  DialogBody,
  DialogActions,
  DialogContent,
  RadioGroup,
  Radio,
  Input,
  Dropdown,
  Option,
  FluentProvider,
  webLightTheme,
  webDarkTheme,
  Badge,
} from '@fluentui/react-components'
import {
  bundleIcon,
  WindowFilled,
  WindowRegular,
  CheckmarkCircleFilled,
  CheckmarkCircleRegular,
  ShieldCheckmarkFilled,
  ShieldCheckmarkRegular,
  ArrowUploadFilled,
  ArrowUploadRegular,
  CopyFilled,
  CopyRegular,
} from '@fluentui/react-icons'

const WindowIcon = bundleIcon(WindowFilled, WindowRegular)
const CheckmarkIcon = bundleIcon(CheckmarkCircleFilled, CheckmarkCircleRegular)
const ShieldIcon = bundleIcon(ShieldCheckmarkFilled, ShieldCheckmarkRegular)
const UploadIcon = bundleIcon(ArrowUploadFilled, ArrowUploadRegular)
const CopyIcon = bundleIcon(CopyFilled, CopyRegular)

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

type ViewMode = 'home' | 'systemCheck' | 'progress' | 'results' | 'monitoring'
type CheckType = 'basic' | 'standard' | 'complete'

function App() {
  const [isAdvancedMode, setIsAdvancedMode] = useState(false)
  const [currentView, setCurrentView] = useState<ViewMode>('home')
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null)
  const [availableTasks, setAvailableTasks] = useState<DiagnosticTask[]>([])
  const [selectedTasks, setSelectedTasks] = useState<Set<string>>(new Set())
  const [lastAdvancedSelection, setLastAdvancedSelection] = useState<Set<string>>(new Set())
  const [checkType, setCheckType] = useState<CheckType>('basic')
  const [includeDXDiag, setIncludeDXDiag] = useState(false)
  const [includeAdminTasks, setIncludeAdminTasks] = useState(false)
  const [currentProgress, setCurrentProgress] = useState(0)
  const [currentTaskName, setCurrentTaskName] = useState('')
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [results, setResults] = useState<Record<string, TaskResult>>({})
  const [isRunning, setIsRunning] = useState(false)
  const [showExportDialog, setShowExportDialog] = useState(false)
  const [exportFormat, setExportFormat] = useState<'text' | 'json'>('text')
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null)
  const [expandedCategory, setExpandedCategory] = useState<string | null>(null)
  const [highlightedTask, setHighlightedTask] = useState<string | null>(null)
  const [outputMode, setOutputMode] = useState<'rich' | 'raw' | 'json'>('rich')
  const [isDarkMode, setIsDarkMode] = useState(() => {
    return window.matchMedia?.('(prefers-color-scheme: dark)').matches || false
  })
  const [windowsVersion, setWindowsVersion] = useState<string>('')
  const [systemUptime, setSystemUptime] = useState<string>('')
  const [isMonitoringActive, setIsMonitoringActive] = useState(false)

  useEffect(() => {
    loadSystemInfo()
    loadAvailableTasks()
    loadWindowsVersion()
    
    // Update uptime every second
    const uptimeInterval = setInterval(updateUptime, 1000)
    updateUptime() // Initial call
    
    return () => clearInterval(uptimeInterval)
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

  const loadWindowsVersion = async () => {
    try {
      // Get enhanced system info for Windows version
      const systemResult = await invoke('run_diagnostic_task', { taskId: 'systeminfo' })
      if (systemResult && typeof systemResult === 'object' && 'output' in systemResult) {
        const output = (systemResult as any).output
        try {
          const parsed = JSON.parse(output)
          if (parsed.os_version && parsed.os_version.windows_version) {
            setWindowsVersion(parsed.os_version.windows_version)
          }
        } catch {
          // Fallback to basic detection
          setWindowsVersion('Windows NT')
        }
      }
    } catch (error) {
      console.error('Failed to load Windows version:', error)
      setWindowsVersion('Windows NT')
    }
  }

  const updateUptime = async () => {
    try {
      const uptimeData = await invoke<any>('get_uptime')
      if (uptimeData && uptimeData.formatted) {
        setSystemUptime(uptimeData.formatted)
      }
    } catch (error) {
      console.error('Failed to get uptime:', error)
    }
  }

  const getSelectedTaskIds = (): string[] => {
    if (isAdvancedMode) {
      return Array.from(selectedTasks)
    }

    // Filter tasks based on check type
    let tasks = availableTasks.filter(task => {
      if (!includeAdminTasks && task.admin_required) return false
      if (!includeDXDiag && task.id === 'dxdiag') return false
      return true
    })

    switch (checkType) {
      case 'basic':
        // Basic check: essential system and hardware info
        return tasks
          .filter(t => ['System', 'Hardware', 'Storage'].includes(t.category))
          .filter(t => ['comp_system', 'os_info', 'processor', 'physical_memory', 'disk_drive', 'logical_disk', 'network_adapter'].includes(t.id))
          .map(t => t.id)
      case 'standard':
        // Standard check: all non-admin tasks except debug/developer
        return tasks
          .filter(t => !['Debug', 'Logs'].includes(t.category))
          .map(t => t.id)
      case 'complete':
        // Complete check: all selected tasks
        return tasks.map(t => t.id)
      default:
        return []
    }
  }

  const startDiagnostics = async () => {
    const taskIds = getSelectedTaskIds()
    if (taskIds.length === 0) {
      alert('Please select at least one task')
      return
    }

    setIsRunning(true)
    setCurrentProgress(0)
    setResults({})
    setCurrentView('progress')

    try {
      const sessionId = await invoke<string>('start_diagnostics', { taskIds })
      setSessionId(sessionId)

      // Run tasks in parallel batches for better performance
      const BATCH_SIZE = 5 // Run 5 tasks at a time
      let completedTasks = 0
      
      for (let i = 0; i < taskIds.length; i += BATCH_SIZE) {
        const batch = taskIds.slice(i, i + BATCH_SIZE)
        const batchTasks = batch.map(taskId => availableTasks.find(t => t.id === taskId)).filter(Boolean)
        
        if (batchTasks.length > 0) {
          setCurrentTaskName(`Running ${batchTasks.map(t => t!.name).join(', ')}`)
        }
        
        const batchPromises = batch.map(async (taskId) => {
          try {
            const result = await invoke<TaskResult>('run_diagnostic_task', { taskId })
            setResults(prev => ({ ...prev, [taskId]: result }))
            completedTasks++
            setCurrentProgress((completedTasks / taskIds.length) * 100)
            return { taskId, success: true }
          } catch (error) {
            console.error(`Failed to run task ${taskId}:`, error)
            setResults(prev => ({ 
              ...prev, 
              [taskId]: { 
                success: false, 
                output: '', 
                error: String(error),
                duration_ms: 0 
              } 
            }))
            completedTasks++
            setCurrentProgress((completedTasks / taskIds.length) * 100)
            return { taskId, success: false }
          }
        })
        
        await Promise.all(batchPromises)
      }

      setCurrentProgress(100)
      setTimeout(() => {
        setCurrentView('results')
        setIsRunning(false)
      }, 500)
    } catch (error) {
      console.error('Failed to start diagnostics:', error)
      setIsRunning(false)
      setCurrentView('home')
    }
  }

  const copyToClipboard = async () => {
    if (!sessionId) return
    
    try {
      // Create forum-formatted text
      const content = await invoke<string>('export_results', { 
        format: 'text',
        includeRaw: false 
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
      alert('Results copied to clipboard! You can now paste them in the forum.')
    } catch (error) {
      console.error('Failed to copy to clipboard:', error)
      alert('Failed to copy to clipboard')
    }
  }

  const handleExport = async () => {
    if (!sessionId) return

    try {
      const content = await invoke<string>('export_results', { 
        format: exportFormat,
        includeRaw: true 
      })

      // Save to file
      const filePath = await save({
        defaultPath: `wf-diagnostics-${new Date().toISOString().split('T')[0]}.${exportFormat === 'json' ? 'json' : 'txt'}`,
        filters: [{
          name: exportFormat === 'json' ? 'JSON' : 'Text',
          extensions: [exportFormat === 'json' ? 'json' : 'txt']
        }]
      })

      if (filePath) {
        await writeTextFile(filePath, content)
        alert('Results exported successfully!')
      }
    } catch (error) {
      console.error('Failed to export results:', error)
      alert('Failed to export results')
    }

    setShowExportDialog(false)
  }

  const restartAsAdmin = async () => {
    try {
      await invoke('restart_as_admin')
    } catch (error) {
      console.error('Failed to restart as admin:', error)
    }
  }

  const renderHome = () => {
    const hasResults = Object.keys(results).length > 0
    const healthAnalysis = hasResults ? analyzeResults() : null
    
    return (
      <div className="home-container">
        <Card className="welcome-card">
          <h2>Welcome to WindowsForum Diagnostic Tool</h2>
          <p>This tool helps you understand your system and diagnose potential issues. 
             You can share the results on WindowsForum.com to get help from our community.</p>
        </Card>

        {/* System Health Summary - always visible if we have results */}
        {hasResults && healthAnalysis && (
          <Card style={{ marginBottom: 24, border: '2px solid var(--colorBrandBackground)' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <div>
                <h3>Last System Health Check</h3>
                <div style={{ fontSize: 36, fontWeight: 'bold', color: 
                  healthAnalysis.healthScore >= 90 ? 'green' :
                  healthAnalysis.healthScore >= 70 ? 'orange' : 'red'
                }}>
                  {healthAnalysis.healthScore}%
                </div>
                <p style={{ opacity: 0.8 }}>
                  {healthAnalysis.successfulTasks} of {healthAnalysis.totalTasks} diagnostics completed
                </p>
              </div>
              <div style={{ textAlign: 'right' }}>
                {healthAnalysis.issues.length > 0 && (
                  <MessageBar intent="error" style={{ marginBottom: 8, maxWidth: 300 }}>
                    <MessageBarBody>
                      <MessageBarTitle>{healthAnalysis.issues.length} Issues Found</MessageBarTitle>
                      View detailed results for more information.
                    </MessageBarBody>
                  </MessageBar>
                )}
                {healthAnalysis.warnings.length > 0 && (
                  <MessageBar intent="warning" style={{ maxWidth: 300 }}>
                    <MessageBarBody>
                      <MessageBarTitle>{healthAnalysis.warnings.length} Warnings</MessageBarTitle>
                      Check detailed results for recommendations.
                    </MessageBarBody>
                  </MessageBar>
                )}
                <Button 
                  appearance="primary" 
                  onClick={() => setCurrentView('results')}
                  style={{ marginTop: 8 }}
                >
                  View Details
                </Button>
              </div>
            </div>
          </Card>
        )}

        <h3>What would you like to do?</h3>
      
      <div className="action-grid">
        <Card 
          className="action-card"
          onClick={() => setCurrentView('systemCheck')}
        >
          <CheckmarkIcon className="action-card-icon" />
          <h4>System Check</h4>
          <p>Analyze your system for potential issues</p>
        </Card>
        
        <Card 
          className="action-card"
          onClick={() => {
            setCurrentView('monitoring')
            setIsMonitoringActive(true)
          }}
        >
          <WindowIcon className="action-card-icon" />
          <h4>Real-time Monitor</h4>
          <p>View live system performance metrics</p>
        </Card>

        <Card 
          className="action-card"
          onClick={() => {
            // Show system info in results view
            setCurrentView('results')
          }}
        >
          <WindowIcon className="action-card-icon" />
          <h4>System Information</h4>
          <p>View detailed information about your PC</p>
        </Card>

        <Card 
          className="action-card"
          onClick={() => setShowExportDialog(true)}
          style={{ opacity: Object.keys(results).length === 0 ? 0.5 : 1 }}
        >
          <UploadIcon className="action-card-icon" />
          <h4>Export for Forum</h4>
          <p>Create a report to share on WindowsForum.com</p>
        </Card>

        <Card 
          className="action-card"
          style={{ opacity: 0.5 }}
        >
          <ShieldIcon className="action-card-icon" />
          <h4>BSOD Analysis</h4>
          <p>Coming Soon</p>
        </Card>
      </div>

      {systemInfo && !systemInfo.is_admin && (
        <MessageBar intent="warning" style={{ marginTop: 24 }}>
          <MessageBarBody>
            <MessageBarTitle>Limited functionality</MessageBarTitle>
            Some diagnostic tasks require administrator privileges.
            <Button 
              appearance="primary" 
              size="small" 
              onClick={restartAsAdmin}
              style={{ marginLeft: 12 }}
            >
              Restart as Admin
            </Button>
          </MessageBarBody>
        </MessageBar>
      )}
    </div>
    )
  }

  const renderSystemCheck = () => (
    <div className="home-container">
      <Button onClick={() => setCurrentView('home')} appearance="subtle">
        ← Back to Home
      </Button>

      <h2 style={{ marginTop: 24, marginBottom: 24 }}>System Check Options</h2>

      <Card style={{ padding: 24 }}>
        <h3 style={{ marginBottom: 16 }}>Choose check type:</h3>
        
        <RadioGroup 
          value={checkType} 
          onChange={(_, data) => setCheckType(data.value as CheckType)}
        >
          <Radio value="basic" label={
            <div>
              <strong>Basic Check</strong>
              <div style={{ fontSize: 14, opacity: 0.7 }}>Essential system information (30 seconds)</div>
            </div>
          } />
          <Radio value="standard" label={
            <div style={{ marginTop: 12 }}>
              <strong>Standard Check</strong>
              <div style={{ fontSize: 14, opacity: 0.7 }}>Comprehensive analysis (2-3 minutes)</div>
            </div>
          } />
          <Radio value="complete" label={
            <div style={{ marginTop: 12 }}>
              <strong>Complete Check</strong>
              <div style={{ fontSize: 14, opacity: 0.7 }}>Full system diagnostic (5-10 minutes)</div>
            </div>
          } />
        </RadioGroup>

        <Card style={{ marginTop: 16, padding: 16, background: 'var(--colorNeutralBackground2)' }}>
          <h4 style={{ marginBottom: 8 }}>Optional Tasks</h4>
          <Checkbox 
            label="Include DirectX Diagnostics (adds 2-4 seconds)"
            checked={includeDXDiag}
            onChange={(_, data) => setIncludeDXDiag(data.checked as boolean)}
          />
          <Checkbox 
            label="Include Administrator Tasks (DISM, Chkdsk, etc.)"
            checked={includeAdminTasks}
            onChange={(_, data) => setIncludeAdminTasks(data.checked as boolean)}
            style={{ marginTop: 8 }}
          />
        </Card>

        <Button 
          appearance="primary" 
          size="large"
          onClick={startDiagnostics}
          style={{ marginTop: 24 }}
        >
          Start Check
        </Button>
      </Card>
    </div>
  )

  const renderProgress = () => (
    <div className="progress-container">
      <h2>Checking Your System</h2>
      
      <ProgressBar 
        value={currentProgress} 
        max={100}
        className="progress-bar"
      />
      
      <p style={{ fontSize: 20, margin: '16px 0' }}>{Math.round(currentProgress)}%</p>
      <p style={{ opacity: 0.7 }}>{currentTaskName}</p>
      
      <Spinner size="large" style={{ marginTop: 32 }} />
      <p style={{ marginTop: 16, opacity: 0.7 }}>This may take a few minutes...</p>
    </div>
  )

  const parseOutput = (output: string) => {
    try {
      return JSON.parse(output)
    } catch {
      // If not JSON, return raw output
      return output
    }
  }

  const formatRichOutput = (data: any) => {
    if (typeof data === 'string') {
      // Handle plain text with better formatting
      return (
        <div style={{ lineHeight: 1.6 }}>
          {data.split('\n').map((line, index) => (
            <div key={index} style={{ marginBottom: line.trim() ? 4 : 8 }}>
              {line.trim() || <br />}
            </div>
          ))}
        </div>
      )
    }

    // Special handling for battery report
    if (typeof data === 'object' && data !== null && data.battery_summary) {
      return renderBatteryReport(data)
    }

    if (Array.isArray(data)) {
      if (data.length === 0) {
        return <span style={{ opacity: 0.6, fontStyle: 'italic' }}>No items found</span>
      }
      
      return (
        <div>
          <div style={{ marginBottom: 12, fontSize: 14, fontWeight: 600, color: 'var(--colorBrandBackground)' }}>
            {data.length} item{data.length !== 1 ? 's' : ''} found
          </div>
          {data.map((item, index) => (
            <div key={index} style={{ 
              marginBottom: 16, 
              padding: 12, 
              background: 'var(--colorNeutralBackground3)', 
              borderRadius: 6,
              border: '1px solid var(--colorNeutralStroke2)'
            }}>
              <div style={{ fontSize: 12, opacity: 0.7, marginBottom: 8 }}>
                Item {index + 1}
              </div>
              {typeof item === 'object' ? formatObjectAsTable(item) : String(item)}
            </div>
          ))}
        </div>
      )
    }

    if (typeof data === 'object' && data !== null) {
      return formatObjectAsTable(data)
    }

    return String(data)
  }

  const renderBatteryReport = (data: any) => {
    const summary = data.battery_summary || {}
    
    return (
      <div style={{ padding: 16 }}>
        {/* Battery Health Status */}
        {summary.battery_health_percentage && (
          <Card style={{ marginBottom: 16, padding: 16 }}>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
              <div>
                <h4 style={{ margin: 0, marginBottom: 8 }}>Battery Health</h4>
                <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
                  <div style={{ fontSize: 48, fontWeight: 'bold', color: 
                    summary.battery_health_percentage >= 80 ? 'green' :
                    summary.battery_health_percentage >= 60 ? 'orange' : 'red'
                  }}>
                    {summary.battery_health_percentage}%
                  </div>
                  <div>
                    <Badge 
                      appearance="filled" 
                      color={
                        summary.battery_health_status === 'Good' ? 'success' :
                        summary.battery_health_status === 'Fair' ? 'warning' : 'danger'
                      }
                    >
                      {summary.battery_health_status}
                    </Badge>
                  </div>
                </div>
              </div>
            </div>
          </Card>
        )}

        {/* Battery Information */}
        {summary.batteries && summary.batteries.length > 0 && (
          <Card style={{ marginBottom: 16, padding: 16 }}>
            <h4 style={{ margin: 0, marginBottom: 12 }}>Battery Information</h4>
            <table style={{ width: '100%', borderCollapse: 'collapse' }}>
              <tbody>
                {summary.batteries.map((item: any, index: number) => (
                  <tr key={index} style={{ borderBottom: '1px solid var(--colorNeutralStroke2)' }}>
                    <td style={{ padding: 8, fontWeight: 600, width: '40%' }}>{item.property}</td>
                    <td style={{ padding: 8 }}>{item.value}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Card>
        )}

        {/* Recent Usage */}
        {summary.recent_usage && summary.recent_usage.length > 0 && (
          <Card style={{ marginBottom: 16, padding: 16 }}>
            <h4 style={{ margin: 0, marginBottom: 12 }}>Recent Usage</h4>
            <div style={{ overflowX: 'auto' }}>
              <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 14 }}>
                <thead>
                  <tr style={{ borderBottom: '2px solid var(--colorNeutralStroke2)' }}>
                    <th style={{ padding: 8, textAlign: 'left' }}>Start Time</th>
                    <th style={{ padding: 8, textAlign: 'left' }}>State</th>
                    <th style={{ padding: 8, textAlign: 'left' }}>Capacity</th>
                    <th style={{ padding: 8, textAlign: 'left' }}>Duration</th>
                  </tr>
                </thead>
                <tbody>
                  {summary.recent_usage.slice(0, 10).map((usage: any, index: number) => (
                    <tr key={index} style={{ borderBottom: '1px solid var(--colorNeutralStroke2)' }}>
                      <td style={{ padding: 8 }}>{usage.start_time}</td>
                      <td style={{ padding: 8 }}>
                        <Badge color={usage.state === 'Active' ? 'success' : 'informative'}>
                          {usage.state}
                        </Badge>
                      </td>
                      <td style={{ padding: 8 }}>{usage.capacity_remaining}</td>
                      <td style={{ padding: 8 }}>{usage.duration}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </Card>
        )}

        {/* Battery Capacity History */}
        {summary.battery_capacity_history && summary.battery_capacity_history.length > 0 && (
          <Card style={{ marginBottom: 16, padding: 16 }}>
            <h4 style={{ margin: 0, marginBottom: 12 }}>Capacity History</h4>
            <div style={{ overflowX: 'auto' }}>
              <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 14 }}>
                <thead>
                  <tr style={{ borderBottom: '2px solid var(--colorNeutralStroke2)' }}>
                    <th style={{ padding: 8, textAlign: 'left' }}>Period</th>
                    <th style={{ padding: 8, textAlign: 'left' }}>Full Charge Capacity</th>
                    <th style={{ padding: 8, textAlign: 'left' }}>Design Capacity</th>
                  </tr>
                </thead>
                <tbody>
                  {summary.battery_capacity_history.slice(0, 5).map((history: any, index: number) => (
                    <tr key={index} style={{ borderBottom: '1px solid var(--colorNeutralStroke2)' }}>
                      <td style={{ padding: 8 }}>{history.period}</td>
                      <td style={{ padding: 8 }}>{history.full_charge_capacity}</td>
                      <td style={{ padding: 8 }}>{history.design_capacity}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </Card>
        )}

        {/* Fallback to HTML content if no parsed data */}
        {data.html_content && !summary.batteries && (
          <Card style={{ padding: 16 }}>
            <h4 style={{ margin: 0, marginBottom: 12 }}>Battery Report Available</h4>
            <p style={{ marginBottom: 12 }}>
              The battery report has been generated. You can view the raw HTML report in JSON mode or save it for detailed analysis.
            </p>
            <Button appearance="secondary" onClick={() => setOutputMode('json')}>
              View Raw Data
            </Button>
          </Card>
        )}
      </div>
    )
  }

  const formatObjectAsTable = (obj: any) => {
    const entries = Object.entries(obj).filter(([, value]) => 
      value !== null && value !== undefined && value !== '' && value !== 'null'
    )
    
    if (entries.length === 0) return <span style={{ opacity: 0.6, fontStyle: 'italic' }}>No data available</span>

    return (
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 2fr', gap: '6px 16px', fontSize: 13 }}>
        {entries.map(([key, value]) => {
          const formattedKey = key
            .replace(/([A-Z])/g, ' $1')
            .replace(/^./, str => str.toUpperCase())
            .replace(/Id$/, 'ID')
            .replace(/Cpu/, 'CPU')
            .replace(/Ram/, 'RAM')
            .replace(/Usb/, 'USB')
            .replace(/Pci/, 'PCI')
            .replace(/Bios/, 'BIOS')
            .replace(/Os/, 'OS')
          
          let formattedValue: React.ReactNode = String(value)
          
          // Format specific data types
          if (typeof value === 'number') {
            // Format large numbers with commas
            if (value > 1000) {
              formattedValue = value.toLocaleString()
            } else {
              formattedValue = String(value)
            }
          } else if (typeof value === 'boolean') {
            formattedValue = (
              <span style={{ color: value ? 'green' : 'red', fontWeight: 600 }}>
                {value ? '✓ Yes' : '✗ No'}
              </span>
            )
          } else if (typeof value === 'string') {
            // Format file sizes
            if (key.toLowerCase().includes('size') && /^\d+$/.test(value)) {
              const bytes = parseInt(value)
              if (bytes > 1024 * 1024 * 1024) {
                formattedValue = `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`
              } else if (bytes > 1024 * 1024) {
                formattedValue = `${(bytes / (1024 * 1024)).toFixed(2)} MB`
              } else if (bytes > 1024) {
                formattedValue = `${(bytes / 1024).toFixed(2)} KB`
              } else {
                formattedValue = `${bytes} bytes`
              }
            }
            // Format URLs as links
            else if (value.startsWith('http://') || value.startsWith('https://')) {
              formattedValue = (
                <a href={value} target="_blank" rel="noopener noreferrer" style={{ color: 'var(--colorBrandBackground)' }}>
                  {value}
                </a>
              )
            }
            // Format file paths
            else if (value.includes('\\') || value.includes('/')) {
              formattedValue = (
                <span style={{ fontFamily: 'Consolas, monospace', fontSize: 12 }}>
                  {value}
                </span>
              )
            }
            // Format dates
            else if (value.match(/^\d{4}-\d{2}-\d{2}/) || value.includes('GMT') || value.includes('UTC')) {
              try {
                const date = new Date(value)
                if (!isNaN(date.getTime())) {
                  formattedValue = date.toLocaleString()
                }
              } catch {
                // Keep original value if parsing fails
              }
            }
          } else if (typeof value === 'object') {
            formattedValue = (
              <details style={{ marginTop: 4 }}>
                <summary style={{ cursor: 'pointer', color: 'var(--colorBrandBackground)' }}>
                  View details
                </summary>
                <div style={{ marginTop: 8, padding: 8, background: 'var(--colorNeutralBackground2)', borderRadius: 4 }}>
                  <pre style={{ margin: 0, fontSize: 11, whiteSpace: 'pre-wrap' }}>
                    {JSON.stringify(value, null, 2)}
                  </pre>
                </div>
              </details>
            )
          }

          return (
            <React.Fragment key={key}>
              <strong style={{ 
                color: 'var(--colorBrandBackground)', 
                textAlign: 'right',
                paddingRight: 8,
                fontSize: 12
              }}>
                {formattedKey}:
              </strong>
              <span style={{ wordBreak: 'break-word' }}>
                {formattedValue}
              </span>
            </React.Fragment>
          )
        })}
      </div>
    )
  }

  const renderTaskOutput = (result: TaskResult) => {
    if (!result.output) return null

    const parsedData = parseOutput(result.output)

    switch (outputMode) {
      case 'rich':
        return (
          <div className="result-data" style={{ background: 'var(--colorNeutralBackground1)', border: '1px solid var(--colorNeutralStroke2)' }}>
            {formatRichOutput(parsedData)}
          </div>
        )
      
      case 'json':
        return (
          <div className="result-data">
            <pre style={{ 
              fontSize: 12, 
              fontFamily: 'Consolas, monospace',
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
              maxHeight: 400,
              overflow: 'auto',
              margin: 0,
              padding: 0,
              background: 'transparent'
            }}>
              {typeof parsedData === 'object' ? JSON.stringify(parsedData, null, 2) : result.output}
            </pre>
          </div>
        )
      
      case 'raw':
      default:
        return (
          <div className="result-data">
            <pre style={{ 
              fontSize: 12, 
              fontFamily: 'Consolas, monospace',
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
              maxHeight: 400,
              overflow: 'auto',
              margin: 0,
              padding: 0,
              background: 'transparent'
            }}>
              {result.output}
            </pre>
          </div>
        )
    }
  }

  const analyzeResults = () => {
    const issues: string[] = []
    const warnings: string[] = []
    let totalTasks = 0
    let successfulTasks = 0
    let failedTasks = 0
    
    Object.entries(results).forEach(([taskId, result]) => {
      totalTasks++
      if (result.success) {
        successfulTasks++
        
        // Analyze output for potential issues
        const output = result.output.toLowerCase()
        
        try {
          // Try to parse JSON output for more accurate analysis
          const parsedOutput = JSON.parse(result.output)
          
          // Check for BSOD minidumps
          if (taskId === 'minidump' && Array.isArray(parsedOutput) && parsedOutput.length > 0) {
            issues.push(`${parsedOutput.length} crash dump(s) found - system has experienced crashes`)
          }
          
          // Check memory information
          if (taskId === 'physical_memory' && Array.isArray(parsedOutput)) {
            const totalMemoryGB = parsedOutput.reduce((sum, mem) => {
              const capacity = parseInt(mem.Capacity || '0')
              return sum + (capacity / (1024 * 1024 * 1024))
            }, 0)
            if (totalMemoryGB < 4) {
              warnings.push('Low system memory detected (less than 4GB)')
            }
          }
          
          // Check disk space
          if (taskId === 'disk_drive' && Array.isArray(parsedOutput)) {
            parsedOutput.forEach(disk => {
              // WMI disk_drive doesn't have FreeSpace, that's in disk_partition or logical_disk
              // Only analyze if we have meaningful size data
              const size = parseInt(disk.Size || '0')
              if (size > 0) {
                // For physical drives, we can't easily determine free space from WMI Win32_DiskDrive
                // We'll skip the analysis here and rely on logical disk info instead
                // This prevents false 100% full reports
              }
            })
          }
          
          // Check logical disk space (accurate free space analysis)
          if (taskId === 'logical_disk' && Array.isArray(parsedOutput)) {
            parsedOutput.forEach(disk => {
              const size = parseInt(disk.Size || '0')
              const freeSpace = parseInt(disk.FreeSpace || '0')
              const deviceId = disk.DeviceID || disk.Caption || 'Unknown'
              
              if (size > 0 && !isNaN(freeSpace)) {
                const usedSpace = size - freeSpace
                const usedPercent = (usedSpace / size) * 100
                
                // Only report on actual drives (not network drives, etc.)
                if (disk.DriveType === 3 || disk.DriveType === '3') { // Fixed disk
                  if (usedPercent > 90) {
                    issues.push(`Drive ${deviceId} is ${Math.round(usedPercent)}% full`)
                  } else if (usedPercent > 80) {
                    warnings.push(`Drive ${deviceId} is ${Math.round(usedPercent)}% full`)
                  }
                }
              }
            })
          }
          
          // Check network adapters
          if (taskId === 'network_adapter' && Array.isArray(parsedOutput)) {
            // Filter out virtual, loopback, and other non-physical adapters
            const physicalAdapters = parsedOutput.filter(adapter => {
              const name = (adapter.Name || adapter.Description || '').toLowerCase()
              const isPhysical = !name.includes('virtual') && 
                               !name.includes('loopback') && 
                               !name.includes('miniport') &&
                               !name.includes('teredo') &&
                               !name.includes('isatap') &&
                               adapter.PhysicalAdapter !== false
              return isPhysical
            })
            
            const disabledPhysicalAdapters = physicalAdapters.filter(adapter => 
              adapter.NetEnabled === false || 
              adapter.NetConnectionStatus === 'Disconnected' ||
              adapter.NetConnectionStatus === '0' || // Disconnected
              adapter.NetConnectionStatus === '7'    // Media disconnected
            )
            
            if (disabledPhysicalAdapters.length > 0) {
              warnings.push(`${disabledPhysicalAdapters.length} physical network adapter(s) disabled or disconnected`)
            }
          }
          
          // Check system drivers
          if (taskId === 'system_driver' && Array.isArray(parsedOutput)) {
            const problemDrivers = parsedOutput.filter(driver => {
              const state = driver.State || ''
              const startMode = driver.StartMode || ''
              const name = (driver.Name || '').toLowerCase()
              
              // Only consider drivers that should be running but aren't
              const shouldBeRunning = startMode === 'Auto' || startMode === 'System' || startMode === 'Boot'
              const notRunning = state !== 'Running' && state !== 'Stopped'
              
              // Filter out known system drivers that are legitimately stopped
              const isSystemDriver = !name.includes('test') && 
                                   !name.includes('sample') && 
                                   !name.includes('debug')
              
              return shouldBeRunning && notRunning && isSystemDriver
            })
            
            if (problemDrivers.length > 0) {
              warnings.push(`${problemDrivers.length} system driver(s) not running properly`)
            }
          }
          
          // Check services
          if (taskId === 'services' && Array.isArray(parsedOutput)) {
            const criticalStopped = parsedOutput.filter(service => 
              service.StartMode === 'Auto' && service.State !== 'Running' &&
              ['Windows Update', 'Security Center', 'Windows Defender'].some(critical => 
                (service.DisplayName || service.Name || '').includes(critical)
              )
            )
            if (criticalStopped.length > 0) {
              issues.push(`${criticalStopped.length} critical service(s) stopped`)
            }
          }
          
        } catch {
          // Fallback to string analysis if JSON parsing fails
          
          // Check for common error patterns in string output
          if (output.includes('error') || output.includes('failed') || output.includes('critical')) {
            if (taskId === 'event_logs') {
              warnings.push('Critical events found in system logs')
            } else if (taskId === 'chkdsk') {
              issues.push('Disk errors detected by chkdsk')
            } else if (taskId === 'dism_health') {
              issues.push('Windows image corruption detected')
            }
          }
          
          // Check for low disk space patterns
          if (taskId === 'disk_drive' && (output.includes('low') || output.includes('full'))) {
            warnings.push('Low disk space detected')
          }
          
          // Check for BSOD dumps in string format
          if (taskId === 'minidump' && output.includes('.dmp')) {
            issues.push('System crash dumps found')
          }
        }
      } else {
        failedTasks++
        issues.push(`Failed to run ${taskId}`)
      }
    })
    
    // Calculate health score based on execution success AND detected issues
    let baseScore = Math.round((successfulTasks / totalTasks) * 100)
    
    // Reduce score based on issues found
    let healthScore = baseScore
    healthScore -= issues.length * 15  // Major issues: -15 points each
    healthScore -= warnings.length * 8  // Warnings: -8 points each
    
    // Ensure score doesn't go below 0
    healthScore = Math.max(0, healthScore)
    
    // If no tasks completed successfully, cap at 20%
    if (successfulTasks === 0) {
      healthScore = Math.min(healthScore, 20)
    }
    
    return {
      totalTasks,
      successfulTasks,
      failedTasks,
      issues,
      warnings,
      healthScore
    }
  }

  const renderResults = () => {
    // Filter tasks based on search and category
    const filteredTasks = availableTasks.filter(task => {
      const hasResult = results[task.id] !== undefined
      const matchesSearch = searchQuery === '' || 
        task.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
        task.description.toLowerCase().includes(searchQuery.toLowerCase()) ||
        (results[task.id]?.output || '').toLowerCase().includes(searchQuery.toLowerCase())
      const matchesCategory = !selectedCategory || task.category === selectedCategory
      
      return hasResult && matchesSearch && matchesCategory
    })
    
    const tasksByCategory = filteredTasks.reduce((acc, task) => {
      if (!acc[task.category]) acc[task.category] = []
      acc[task.category].push(task)
      return acc
    }, {} as Record<string, DiagnosticTask[]>)

    const healthAnalysis = analyzeResults()
    
    return (
      <div className="results-container">
        <div className="results-main">
          {/* Health Summary Card */}
          <Card style={{ marginBottom: 24, border: '2px solid var(--colorBrandBackground)' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <div>
                <h3>System Health Summary</h3>
                <div style={{ fontSize: 48, fontWeight: 'bold', color: 
                  healthAnalysis.healthScore >= 90 ? 'green' :
                  healthAnalysis.healthScore >= 70 ? 'orange' : 'red'
                }}>
                  {healthAnalysis.healthScore}%
                </div>
                <p style={{ opacity: 0.8 }}>
                  {healthAnalysis.successfulTasks} of {healthAnalysis.totalTasks} diagnostics completed
                </p>
              </div>
              <div style={{ textAlign: 'right' }}>
                {healthAnalysis.issues.length > 0 && (
                  <MessageBar intent="error" style={{ marginBottom: 8 }}>
                    <MessageBarBody>
                      <MessageBarTitle>{healthAnalysis.issues.length} Issues Found</MessageBarTitle>
                      <ul style={{ margin: '8px 0', paddingLeft: 20 }}>
                        {healthAnalysis.issues.map((issue, i) => (
                          <li key={i}>{issue}</li>
                        ))}
                      </ul>
                    </MessageBarBody>
                  </MessageBar>
                )}
                {healthAnalysis.warnings.length > 0 && (
                  <MessageBar intent="warning">
                    <MessageBarBody>
                      <MessageBarTitle>{healthAnalysis.warnings.length} Warnings</MessageBarTitle>
                      <ul style={{ margin: '8px 0', paddingLeft: 20 }}>
                        {healthAnalysis.warnings.map((warning, i) => (
                          <li key={i}>{warning}</li>
                        ))}
                      </ul>
                    </MessageBarBody>
                  </MessageBar>
                )}
              </div>
            </div>
          </Card>

          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 24 }}>
            <div>
              <h2>Detailed Results</h2>
              <p style={{ opacity: 0.8 }}>
                {Object.keys(results).length} diagnostics completed
              </p>
            </div>
            <div>
              <Button 
                appearance="primary"
                icon={<CopyIcon />}
                onClick={copyToClipboard}
              >
                Copy for Forum
              </Button>
              <Button 
                icon={<UploadIcon />}
                onClick={() => setShowExportDialog(true)}
                style={{ marginLeft: 8 }}
              >
                Export File
              </Button>
              <Button 
                onClick={() => {
                  setResults({})
                  setCurrentView('home')
                }}
                style={{ marginLeft: 8 }}
              >
                New Check
              </Button>
            </div>
          </div>

          <div style={{ 
            display: 'flex', 
            gap: 12, 
            marginBottom: 24,
            alignItems: 'center',
            flexWrap: 'wrap'
          }}>
            <Input
              placeholder="Search results..."
              value={searchQuery}
              onChange={(_, data) => setSearchQuery(data.value)}
              style={{ flex: 1, minWidth: 200 }}
            />
            <Dropdown
              placeholder="All Categories"
              value={selectedCategory || ''}
              onOptionSelect={(_, data) => setSelectedCategory(data.optionValue === '' ? null : (data.optionValue || null))}
              style={{ minWidth: 200 }}
            >
              <Option value="">All Categories</Option>
              {Object.keys(availableTasks.reduce((acc, task) => {
                if (results[task.id]) acc[task.category] = true
                return acc
              }, {} as Record<string, boolean>)).map(cat => (
                <Option key={cat} value={cat}>{cat}</Option>
              ))}
            </Dropdown>
            
            {/* Output Mode Toggle */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <span style={{ fontSize: 14, opacity: 0.8 }}>View:</span>
              <RadioGroup
                value={outputMode}
                onChange={(_, data) => setOutputMode(data.value as 'rich' | 'raw' | 'json')}
                layout="horizontal"
              >
                <Radio value="rich" label="Rich" />
                <Radio value="raw" label="Raw" />
                <Radio value="json" label="JSON" />
              </RadioGroup>
            </div>
            
            <Button 
              onClick={() => {
                setSearchQuery('')
                setSelectedCategory(null)
              }}
            >
              Clear Filters
            </Button>
          </div>

          {Object.entries(tasksByCategory).map(([category, tasks]) => (
            <div key={category} data-category={category}>
              <h3 style={{ 
                marginTop: 32, 
                marginBottom: 16,
                padding: '8px 0',
                borderBottom: selectedCategory === category ? '2px solid var(--colorBrandBackground)' : '1px solid var(--colorNeutralStroke1)'
              }}>
                {category}
                <span style={{ 
                  fontSize: 14, 
                  opacity: 0.7, 
                  marginLeft: 12,
                  fontWeight: 'normal'
                }}>
                  ({tasks.filter(task => results[task.id]?.success).length}/{tasks.length} completed)
                </span>
              </h3>
              {tasks.map(task => {
                const result = results[task.id]
                if (!result) return null

                return (
                  <Card 
                    key={task.id} 
                    className="result-card" 
                    data-task-id={task.id}
                    style={{
                      border: highlightedTask === task.id ? '2px solid var(--colorBrandBackground)' : undefined,
                      boxShadow: highlightedTask === task.id ? '0 4px 12px rgba(0, 123, 255, 0.2)' : undefined,
                      transition: 'all 0.3s ease'
                    }}
                  >
                    <div className="result-card-header">
                      <div>
                        <h4>{task.name}</h4>
                        <p style={{ fontSize: 12, opacity: 0.7 }}>{task.description}</p>
                        <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>
                          <span style={{ 
                            fontSize: 10, 
                            padding: '2px 6px', 
                            borderRadius: 2,
                            background: 'var(--colorBrandBackground)',
                            color: 'white'
                          }}>
                            {task.category}
                          </span>
                          {task.admin_required && (
                            <span style={{ 
                              fontSize: 10, 
                              padding: '2px 6px', 
                              borderRadius: 2,
                              background: 'orange',
                              color: 'white'
                            }}>
                              Admin
                            </span>
                          )}
                          <span style={{ fontSize: 10, opacity: 0.6 }}>
                            {result.duration_ms}ms
                          </span>
                        </div>
                      </div>
                      <div style={{ 
                        color: result.success ? 'green' : 'red',
                        display: 'flex',
                        flexDirection: 'column',
                        alignItems: 'flex-end'
                      }}>
                        <span style={{ fontSize: 14, fontWeight: 600 }}>
                          {result.success ? '✓ Success' : '✗ Failed'}
                        </span>
                        {highlightedTask === task.id && (
                          <span style={{ 
                            fontSize: 10, 
                            color: 'var(--colorBrandBackground)',
                            marginTop: 4
                          }}>
                            📍 Selected
                          </span>
                        )}
                      </div>
                    </div>
                    {renderTaskOutput(result)}
                    {result.error && (
                      <MessageBar intent="error" style={{ marginTop: 12 }}>
                        <MessageBarBody>{result.error}</MessageBarBody>
                      </MessageBar>
                    )}
                  </Card>
                )
              })}
            </div>
          ))}
        </div>

        <div className="category-sidebar">
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 }}>
            <h3 style={{ margin: 0 }}>Categories</h3>
            {selectedCategory && (
              <Button
                appearance="transparent"
                size="small"
                onClick={() => {
                  setSelectedCategory(null)
                  setExpandedCategory(null)
                  setHighlightedTask(null)
                  setSearchQuery('')
                  // Scroll to top of results
                  document.querySelector('.results-main')?.scrollTo({ top: 0, behavior: 'smooth' })
                }}
              >
                View All
              </Button>
            )}
          </div>
          {Object.keys(tasksByCategory).map(category => {
            const categoryTasks = tasksByCategory[category]
            const isExpanded = expandedCategory === category
            const completedTasks = categoryTasks.filter(task => results[task.id]?.success).length
            const totalTasks = categoryTasks.length
            
            return (
              <div key={category} style={{ marginBottom: 12 }}>
                <Button 
                  appearance={selectedCategory === category ? "primary" : "subtle"}
                  style={{ 
                    width: '100%', 
                    justifyContent: 'space-between', 
                    marginBottom: 4,
                    padding: '8px 12px'
                  }}
                  onClick={() => {
                    // Toggle expansion and scroll to category
                    if (isExpanded) {
                      setExpandedCategory(null)
                    } else {
                      setExpandedCategory(category)
                    }
                    setSelectedCategory(category)
                    
                    // Scroll to category section
                    setTimeout(() => {
                      const element = document.querySelector(`[data-category="${category}"]`)
                      element?.scrollIntoView({ behavior: 'smooth', block: 'start' })
                    }, 100)
                  }}
                >
                  <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-start' }}>
                    <span style={{ fontWeight: selectedCategory === category ? 600 : 400 }}>
                      {category}
                    </span>
                    <span style={{ fontSize: 11, opacity: 0.7 }}>
                      {completedTasks}/{totalTasks} completed
                    </span>
                  </div>
                  <span style={{ fontSize: 12 }}>
                    {isExpanded ? '▼' : '▶'}
                  </span>
                </Button>
                
                {/* Expanded task list */}
                {isExpanded && (
                  <div style={{ 
                    paddingLeft: 12, 
                    borderLeft: '2px solid var(--colorNeutralStroke2)',
                    marginLeft: 8 
                  }}>
                    {categoryTasks.map(task => {
                      const taskResult = results[task.id]
                      return (
                        <Button
                          key={task.id}
                          appearance="transparent"
                          size="small"
                          style={{ 
                            width: '100%', 
                            justifyContent: 'flex-start', 
                            marginBottom: 2,
                            padding: '4px 8px',
                            minHeight: 'auto'
                          }}
                          onClick={() => {
                            // Highlight and scroll to specific task
                            setHighlightedTask(task.id)
                            
                            // Clear highlight after a few seconds
                            setTimeout(() => setHighlightedTask(null), 3000)
                            
                            // Scroll to specific task
                            setTimeout(() => {
                              const element = document.querySelector(`[data-task-id="${task.id}"]`)
                              element?.scrollIntoView({ behavior: 'smooth', block: 'center' })
                            }, 100)
                          }}
                        >
                          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                            <span style={{ 
                              fontSize: 10,
                              color: taskResult?.success ? 'green' : taskResult ? 'red' : 'orange'
                            }}>
                              {taskResult?.success ? '✓' : taskResult ? '✗' : '○'}
                            </span>
                            <span style={{ fontSize: 12 }}>{task.name}</span>
                          </div>
                        </Button>
                      )
                    })}
                  </div>
                )}
              </div>
            )
          })}
        </div>
      </div>
    )
  }

  const renderAdvancedView = () => (
    <div className="advanced-container">
      <div className="task-sidebar">
        <div style={{ padding: '16px 12px', borderBottom: '1px solid var(--colorNeutralStroke1)' }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <h3>Diagnostic Tasks</h3>
            <div>
              <Button size="small" onClick={() => {
                const allIds = new Set(availableTasks.map(t => t.id))
                setSelectedTasks(allIds)
              }}>All</Button>
              <Button size="small" onClick={() => setSelectedTasks(new Set())} style={{ marginLeft: 4 }}>
                None
              </Button>
            </div>
          </div>
        </div>

        <div className="task-list">
          {availableTasks.map(task => (
            <div key={task.id} className="task-item">
              <Checkbox
                label={
                  <div>
                    <div style={{ fontWeight: 600 }}>{task.name}</div>
                    <div style={{ fontSize: 12, opacity: 0.7 }}>{task.description}</div>
                    <div style={{ marginTop: 4 }}>
                      <span style={{ 
                        fontSize: 10, 
                        padding: '2px 6px', 
                        borderRadius: 2,
                        background: 'var(--colorBrandBackground)',
                        color: 'white',
                        marginRight: 4
                      }}>
                        {task.category}
                      </span>
                      {task.admin_required && (
                        <span style={{ 
                          fontSize: 10, 
                          padding: '2px 6px', 
                          borderRadius: 2,
                          background: 'orange',
                          color: 'white'
                        }}>
                          Admin
                        </span>
                      )}
                    </div>
                  </div>
                }
                checked={selectedTasks.has(task.id)}
                onChange={(_, data) => {
                  const newSelected = new Set(selectedTasks)
                  if (data.checked) {
                    newSelected.add(task.id)
                  } else {
                    newSelected.delete(task.id)
                  }
                  setSelectedTasks(newSelected)
                }}
              />
            </div>
          ))}
        </div>

        <div className="task-controls">
          <Button 
            appearance="primary"
            disabled={selectedTasks.size === 0 || isRunning}
            onClick={startDiagnostics}
            style={{ width: '100%' }}
          >
            Run Diagnostics ({selectedTasks.size} selected)
          </Button>
        </div>
      </div>

      <div style={{ flex: 1, padding: 24, overflow: 'auto' }}>
        {currentView === 'progress' ? renderProgress() : 
         currentView === 'results' ? renderResults() :
         (
          <div>
            {/* Health Summary for Advanced Mode */}
            {Object.keys(results).length > 0 && (() => {
              const healthAnalysis = analyzeResults()
              return (
                <Card style={{ marginBottom: 24, border: '2px solid var(--colorBrandBackground)' }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <div>
                      <h3>System Health Summary</h3>
                      <div style={{ fontSize: 36, fontWeight: 'bold', color: 
                        healthAnalysis.healthScore >= 90 ? 'green' :
                        healthAnalysis.healthScore >= 70 ? 'orange' : 'red'
                      }}>
                        {healthAnalysis.healthScore}%
                      </div>
                      <p style={{ opacity: 0.8 }}>
                        {healthAnalysis.successfulTasks} of {healthAnalysis.totalTasks} diagnostics completed
                      </p>
                    </div>
                    <Button 
                      appearance="primary" 
                      onClick={() => setCurrentView('results')}
                    >
                      View Detailed Results
                    </Button>
                  </div>
                </Card>
              )
            })()}
            
            <div style={{ textAlign: 'center', paddingTop: Object.keys(results).length > 0 ? 20 : 100 }}>
              <h2>Advanced Diagnostics</h2>
              <p style={{ marginTop: 16, opacity: 0.7 }}>
                Select diagnostic tasks from the left panel to begin.<br/>
                This mode provides full control over all diagnostic options.
              </p>
            </div>
          </div>
         )}
      </div>
    </div>
  )

  return (
    <FluentProvider theme={isDarkMode ? webDarkTheme : webLightTheme}>
      <div className="app-container">
        <header className="header">
          <div className="header-title">
            <WindowIcon style={{ fontSize: 32 }} />
            <div>
              <h1>WindowsForum Diagnostic Tool</h1>
              <div style={{ fontSize: 14, opacity: 0.7, margin: 0, display: 'flex', flexDirection: 'column', gap: 2 }}>
                {systemInfo && (
                  <p style={{ margin: 0 }}>
                    {systemInfo.computer_name} • {windowsVersion || systemInfo.os_version}
                  </p>
                )}
                {systemUptime && (
                  <p style={{ margin: 0, fontSize: 12, opacity: 0.6 }}>
                    Uptime: {systemUptime}
                  </p>
                )}
              </div>
            </div>
          </div>

          <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
            <Switch 
              checked={isDarkMode}
              onChange={(_, data) => setIsDarkMode(data.checked)}
              label={isDarkMode ? '🌙' : '☀️'}
            />
            <span>View:</span>
            <div style={{ position: 'relative' }}>
              <Switch 
                checked={isAdvancedMode}
                disabled={isRunning}
                onChange={(_, data) => {
                const newMode = data.checked
                const oldMode = isAdvancedMode
                
                // Preserve selections when switching modes
                if (oldMode && !newMode) {
                  // Switching from advanced to standard - save current selection
                  setLastAdvancedSelection(new Set(selectedTasks))
                  setSelectedTasks(new Set()) // Clear for standard mode
                } else if (!oldMode && newMode) {
                  // Switching from standard to advanced - restore previous selection
                  setSelectedTasks(new Set(lastAdvancedSelection))
                }
                
                setIsAdvancedMode(newMode)
                
                // Smart view switching logic
                if (currentView === 'progress') {
                  // Don't change view if diagnostics are running (this shouldn't happen due to disabled state)
                  return
                } else if (currentView === 'results') {
                  // Stay in results view if we have results - user can see results in both modes
                  return
                } else if (currentView === 'systemCheck' && newMode) {
                  // If switching to advanced from system check, systemCheck is not available in advanced mode
                  setCurrentView('home')
                } else if (currentView === 'systemCheck' && !newMode) {
                  // Switching from advanced to standard while somehow in systemCheck - shouldn't happen but handle it
                  return
                } else {
                  // For home view or any other view, no change needed
                  return
                }
              }}
              label={isAdvancedMode ? "Advanced" : "Standard"}
            />
            {isRunning && (
              <span style={{ 
                fontSize: 11, 
                opacity: 0.6, 
                position: 'absolute', 
                bottom: -16, 
                left: 0,
                whiteSpace: 'nowrap'
              }}>
                Mode switching disabled during diagnostics
              </span>
            )}
            </div>
          </div>
        </header>

        <main className="main-content">
          {isAdvancedMode ? renderAdvancedView() : (
            <div className="view-container">
              {currentView === 'home' && renderHome()}
              {currentView === 'systemCheck' && renderSystemCheck()}
              {currentView === 'progress' && renderProgress()}
              {currentView === 'results' && renderResults()}
              {currentView === 'monitoring' && (
                <SystemMonitoring 
                  isActive={isMonitoringActive} 
                  onToggle={setIsMonitoringActive} 
                />
              )}
            </div>
          )}
        </main>

        <footer className="status-bar">
          <span>
            {isRunning ? 'Running diagnostics...' : 
             Object.keys(results).length > 0 ? `${Object.keys(results).length} tasks completed` :
             'Ready'}
          </span>
          <div>
            <a href="https://www.windowsforum.com" target="_blank" style={{ marginRight: 16 }}>
              WindowsForum.com
            </a>
            <span style={{ opacity: 0.5 }}>v1.0.0</span>
          </div>
        </footer>

        {/* Export Dialog */}
        <Dialog open={showExportDialog} onOpenChange={(_, data) => setShowExportDialog(data.open)}>
          <DialogSurface>
            <DialogBody>
              <DialogTitle>Export for WindowsForum.com</DialogTitle>
              <DialogContent>
                <p>Your diagnostic report is ready to share!</p>
                
                <Card style={{ marginTop: 16, padding: 16, background: 'var(--colorNeutralBackground2)' }}>
                  <strong>Report includes:</strong>
                  <p style={{ marginTop: 8, fontSize: 14 }}>
                    • {Object.keys(results).length} diagnostic tasks<br/>
                    • System information<br/>
                    • All test results and outputs
                  </p>
                </Card>

                <h4 style={{ marginTop: 16, marginBottom: 8 }}>Choose export format:</h4>
                <RadioGroup 
                  value={exportFormat} 
                  onChange={(_, data) => setExportFormat(data.value as 'text' | 'json')}
                >
                  <Radio value="text" label={
                    <div>
                      <strong>Forum Text Format</strong>
                      <div style={{ fontSize: 12, opacity: 0.7 }}>Formatted for easy reading in forum posts</div>
                    </div>
                  } />
                  <Radio value="json" label={
                    <div style={{ marginTop: 8 }}>
                      <strong>JSON Format</strong>
                      <div style={{ fontSize: 12, opacity: 0.7 }}>Complete data for advanced analysis</div>
                    </div>
                  } />
                </RadioGroup>

                <MessageBar intent="info" style={{ marginTop: 16 }}>
                  <MessageBarBody>
                    Tip: Create a new thread on WindowsForum.com and paste or attach this report
                  </MessageBarBody>
                </MessageBar>
              </DialogContent>
              <DialogActions>
                <DialogTrigger disableButtonEnhancement>
                  <Button appearance="secondary">Cancel</Button>
                </DialogTrigger>
                <Button appearance="primary" onClick={handleExport}>Export</Button>
              </DialogActions>
            </DialogBody>
          </DialogSurface>
        </Dialog>
      </div>
    </FluentProvider>
  )
}

export default App