import React, { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { save } from '@tauri-apps/plugin-dialog'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { writeTextFile } from '@tauri-apps/plugin-fs'
import { SystemMonitoring } from './SystemMonitoring'
import { OpenAIIntegration } from './OpenAIIntegration'
import { ComparisonView } from './ComparisonView'
import { wfDarkTheme } from './theme'
import {
  NavigationHeader,
  TabNavigation,
  QuickActionPanel,
  CommandBar,
  ScanResultCard,
  SettingsDialog,
  StatusCard,
  type TabValue,
  type SettingsData
} from './components'
import './styles.css'
import {
  FluentProvider,
  Text,
  Divider,
  tokens,
  Badge,
  Spinner,
  Card,
  Caption1,
  Title3,
  Body1,
  SearchBox,
  makeStyles,
  shorthands
} from '@fluentui/react-components'
import {
  CheckmarkCircle20Regular,
  Warning20Regular,
  Info20Regular,
  Shield20Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  mainContainer: {
    minHeight: '100vh',
    background: tokens.colorNeutralBackground1,
    color: tokens.colorNeutralForeground1,
    height: '100vh',
    position: 'relative',
    overflow: 'hidden',
  },
  contentArea: {
    padding: tokens.spacingVerticalXL,
    height: 'calc(100vh - 180px)',
    overflowY: 'auto',
    overflowX: 'hidden',
  },
  diagnosticsContainer: {
    maxWidth: '1400px',
    margin: '0 auto',
  },
  resultsContainer: {
    display: 'flex',
    ...shorthands.gap(tokens.spacingHorizontalL),
    height: 'calc(100vh - 250px)',
  },
  sidebar: {
    width: '280px',
    position: 'sticky',
    top: '20px',
    height: 'fit-content',
    maxHeight: 'calc(100vh - 300px)',
    overflowY: 'auto',
  },
  mainContent: {
    flex: 1,
    overflowY: 'auto',
    paddingRight: tokens.spacingHorizontalM,
  },
  searchBar: {
    marginBottom: tokens.spacingVerticalL,
  },
  categoryNav: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    marginBottom: tokens.spacingVerticalXS,
    textDecoration: 'none',
    background: 'rgba(30, 41, 59, 0.3)',
    transition: 'all 0.2s',
    ':hover': {
      background: 'rgba(59, 130, 246, 0.1)',
    }
  },
  healthScore: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    marginBottom: tokens.spacingVerticalL,
  },
  healthCircle: {
    width: '80px',
    height: '80px',
    borderRadius: '50%',
    position: 'relative',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    background: `conic-gradient(
      from 0deg,
      ${tokens.colorPaletteGreenBackground1} 0deg,
      ${tokens.colorPaletteGreenBackground1} var(--score-angle),
      ${tokens.colorNeutralBackground3} var(--score-angle)
    )`,
    '&::before': {
      content: '""',
      position: 'absolute',
      width: '60px',
      height: '60px',
      borderRadius: '50%',
      backgroundColor: tokens.colorNeutralBackground2,
    }
  }
})

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

function App() {
  const styles = useStyles()
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
  // const [copyingMinidumps, setCopyingMinidumps] = useState(false)
  const [fixingIssue, setFixingIssue] = useState<string | null>(null)
  // const [fixResults, setFixResults] = useState<Record<string, any>>({})
  const [showSettings, setShowSettings] = useState(false)
  const [settings, setSettings] = useState<SettingsData>({
    autoSave: true,
    scanOnStartup: false,
    maxConcurrentTasks: 5,
    exportFormat: 'text',
    theme: 'dark',
  })
  const [showDebug, setShowDebug] = useState(false)

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

  /*
  const handleCopyMinidumps = async () => {
    setCopyingMinidumps(true)
    try {
      const result = await invoke<any>('copy_minidumps_to_desktop')
      if (result.success) {
        alert(`Success! ${result.message}\n\nFiles copied to: ${result.destination_path}\n\nYou can now easily share these minidump files on WindowsForum.com for analysis.`)
      } else {
        alert(`Failed to copy minidumps: ${result.message}`)
      }
    } catch (error) {
      console.error('Failed to copy minidumps:', error)
      alert(`Error copying minidumps: ${error}`)
    } finally {
      setCopyingMinidumps(false)
    }
  }
  */

  const loadAvailableTasks = async () => {
    try {
      const tasks = await invoke<DiagnosticTask[]>('get_available_tasks')
      setAvailableTasks(tasks)
    } catch (error) {
      console.error('Failed to load tasks:', error)
    }
  }

  const runQuickScan = async () => {
    const quickTasks = availableTasks.filter(task =>
      ['comp_system', 'os_info', 'processor', 'physical_memory', 'disk_drive',
       'logical_disk', 'network_adapter', 'systeminfo'].includes(task.id)
    ).map(t => t.id)

    await runDiagnostics(quickTasks)
  }

  const runFullScan = async () => {
    const allTasks = availableTasks
      .filter(task => !task.admin_required || systemInfo?.is_admin)
      .map(t => t.id)

    await runDiagnostics(allTasks)
  }

  const stopScan = () => {
    setIsRunning(false)
    setCurrentProgress(0)
    setCurrentTaskName('')
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
        const results = await invoke<Array<[string, TaskResult]>>('run_diagnostics_parallel', {
          taskIds,
          maxConcurrent: settings.maxConcurrentTasks || 5
        })

        const resultsObj = results.reduce((acc, [taskId, result]) => {
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
          detectIssues()
        }, 500)
      } else {
        setIsRunning(false)
        detectIssues()
      }
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
          console.log('File exported successfully to:', filePath)
        } catch (writeError) {
          console.error('Failed to write file:', writeError)
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

  const handleSettingsSave = (newSettings: SettingsData) => {
    setSettings(newSettings)
    setShowSettings(false)
  }

  const clearResults = () => {
    setResults({})
    setSessionId(null)
    setCurrentProgress(0)
    setCurrentTaskName('')
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
  }, [searchQuery, results])

  const handleFixIssue = async (issueId: string) => {
    setFixingIssue(issueId)
    try {
      const result = await invoke<any>('fix_issue', { issueId })
      // setFixResults(prev => ({ ...prev, [issueId]: result }))
      if (result.success) {
        setTimeout(() => detectIssues(), 2000)
      }
    } catch (error) {
      console.error('Failed to fix issue:', error)
      // setFixResults(prev => ({
      //   ...prev,
      //   [issueId]: { success: false, message: String(error) }
      // }))
    } finally {
      setFixingIssue(null)
    }
  }

  const renderDiagnosticsTab = () => {
    const healthScore = getHealthScore()
    const hasResults = Object.keys(results).length > 0
    const resultsByCategory = availableTasks.reduce((acc, task) => {
      if ((searchQuery ? filteredResults : results)[task.id]) {
        if (!acc[task.category]) acc[task.category] = []
        acc[task.category].push({
          task,
          result: (searchQuery ? filteredResults : results)[task.id]
        })
      }
      return acc
    }, {} as Record<string, Array<{ task: DiagnosticTask; result: TaskResult }>>)

    const stats = hasResults ? {
      totalTasks: Object.keys(results).length,
      successfulTasks: Object.values(results).filter(r => r.success).length,
      failedTasks: Object.values(results).filter(r => !r.success).length,
      duration: Date.now() - scanStartTime
    } : undefined

    if (showComparison) {
      return (
        <ComparisonView
          onClose={() => setShowComparison(false)}
        />
      )
    }

    return (
      <div className={styles.diagnosticsContainer}>
        {/* Command Bar */}
        {(hasResults || isRunning) && (
          <CommandBar
            onQuickScan={runQuickScan}
            onFullScan={runFullScan}
            onStopScan={isRunning ? stopScan : undefined}
            isScanning={isRunning}
            onExport={hasResults ? exportResults : undefined}
            onCopyToClipboard={hasResults ? copyToClipboard : undefined}
            onToggleFilter={() => {}}
            onClearResults={hasResults ? clearResults : undefined}
            onCompareScans={() => setShowComparison(true)}
            scanStatus={isRunning ? 'scanning' : hasResults ? 'complete' : 'idle'}
            resultCount={Object.keys(results).length}
            debugMode={showDebug}
            onToggleDebug={() => setShowDebug(!showDebug)}
          />
        )}

        {/* Quick Action Panel - Welcome Screen or Progress */}
        {(!hasResults || isRunning) && (
          <QuickActionPanel
            onQuickScan={runQuickScan}
            onFullScan={runFullScan}
            onCompare={() => setShowComparison(true)}
            onExport={exportResults}
            onCopyToClipboard={copyToClipboard}
            isScanning={isRunning}
            scanProgress={currentProgress}
            currentTask={currentTaskName}
            hasResults={hasResults}
            stats={stats}
          />
        )}

        {/* Admin Warning */}
        {!isRunning && !hasResults && systemInfo && !systemInfo.is_admin && (
          <StatusCard
            status="warning"
            title="Limited Access"
            description="Running without administrator privileges"
            details={[
              `${availableTasks.filter(task => !task.admin_required || systemInfo?.is_admin).length} of ${availableTasks.length} diagnostic tasks available`,
              '5 admin-only tasks hidden (disk check, DISM health, battery report, driver verifier, crash dumps)'
            ]}
            actions={[
              {
                label: 'Restart as Administrator',
                onClick: restartAsAdmin,
                primary: true
              }
            ]}
          />
        )}

        {/* Search Bar */}
        {hasResults && !isRunning && (
          <Card style={{ marginBottom: tokens.spacingVerticalL }}>
            <SearchBox
              placeholder="Search in results (task names, errors, output)..."
              value={searchQuery}
              onChange={(_, data) => setSearchQuery(data?.value || '')}
              style={{ width: '100%' }}
            />
            {searchQuery && (
              <Caption1 style={{
                marginTop: tokens.spacingVerticalS,
                color: tokens.colorNeutralForeground3
              }}>
                {Object.keys(filteredResults).length} of {Object.keys(results).length} results
              </Caption1>
            )}
          </Card>
        )}

        {/* Results Display */}
        {hasResults && !isRunning && !showComparison && (
          <div className={styles.resultsContainer}>
            {/* Left Sidebar */}
            <Card className={styles.sidebar}>
              <div className={styles.healthScore}>
                <Title3>Health Score</Title3>
                <div
                  className={styles.healthCircle}
                  style={{ '--score-angle': `${(healthScore || 0) * 3.6}deg` } as React.CSSProperties}
                >
                  <Text
                    size={600}
                    weight="bold"
                    style={{ zIndex: 1 }}
                  >
                    {healthScore}%
                  </Text>
                </div>
                {healthScore !== null && (
                  <Badge
                    appearance="filled"
                    color={healthScore >= 90 ? 'success' : healthScore >= 70 ? 'warning' : 'danger'}
                    size="medium"
                  >
                    {healthScore >= 90 ? 'Excellent' : healthScore >= 70 ? 'Good' : 'Needs Attention'}
                  </Badge>
                )}
              </div>

              <Divider />

              {/* Category Navigation */}
              <div style={{ marginTop: tokens.spacingVerticalL }}>
                <Caption1 style={{ color: tokens.colorNeutralForeground3, fontWeight: 600 }}>
                  CATEGORIES
                </Caption1>
                {Object.entries(resultsByCategory).map(([category, items]) => {
                  const getCategoryIcon = (cat: string) => {
                    switch(cat) {
                      case 'System': return <Info20Regular />
                      case 'Hardware': return <CheckmarkCircle20Regular />
                      case 'Network': return <Warning20Regular />
                      default: return <Info20Regular />
                    }
                  }

                  return (
                    <a
                      key={category}
                      href={`#category-${category}`}
                      className={styles.categoryNav}
                    >
                      <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalS }}>
                        {getCategoryIcon(category)}
                        <Body1>{category}</Body1>
                      </div>
                      <Caption1>
                        {items.filter(i => i.result.success).length}/{items.length}
                      </Caption1>
                    </a>
                  )
                })}
              </div>
            </Card>

            {/* Main Results Area */}
            <div className={styles.mainContent}>
              {Object.entries(resultsByCategory).map(([category, items]) => (
                <Card
                  key={category}
                  id={`category-${category}`}
                  style={{ marginBottom: tokens.spacingVerticalL }}
                >
                  <Title3>{category}</Title3>
                  <Caption1 style={{ color: tokens.colorNeutralForeground3 }}>
                    {items.filter(i => i.result.success).length} of {items.length} tests passed
                  </Caption1>

                  <div style={{ marginTop: tokens.spacingVerticalL }}>
                    {items.map(({ task, result }) => (
                      <ScanResultCard
                        key={task.id}
                        taskName={task.name}
                        taskDescription={task.description}
                        success={result.success}
                        duration={result.duration_ms}
                        output={result.output}
                        error={result.error}
                        category={task.category}
                        onCopyOutput={(output) => writeText(output)}
                      />
                    ))}
                  </div>
                </Card>
              ))}
            </div>
          </div>
        )}
      </div>
    )
  }

  const renderIssuesTab = () => {
    const sortedIssues = [...issues].sort((a, b) => {
      if (a.detected !== b.detected) {
        return a.detected ? -1 : 1;
      }
      const severityOrder: Record<string, number> = { Critical: 0, Warning: 1, Info: 2, Ok: 3 };
      return (severityOrder[a.severity] || 999) - (severityOrder[b.severity] || 999);
    });

    return (
      <div style={{ maxWidth: '1200px', margin: '0 auto' }}>
        <Card>
          <Title3>
            <Shield20Regular style={{ marginRight: tokens.spacingHorizontalS }} />
            System Health Check
          </Title3>

          {sortedIssues.length === 0 ? (
            <div style={{ textAlign: 'center', padding: tokens.spacingVerticalXXL }}>
              <Spinner size="large" label="Checking for issues..." />
            </div>
          ) : (
            <div style={{ marginTop: tokens.spacingVerticalL }}>
              {sortedIssues.map((issue, index) => {
                /*const getStatusIcon = () => {
                  switch(issue.severity) {
                    case 'Critical': return <ErrorCircle20Regular primaryFill="#EF4444" />
                    case 'Warning': return <Warning20Regular primaryFill="#F59E0B" />
                    case 'Info': return <Info20Regular primaryFill="#3B82F6" />
                    case 'Ok': return <CheckmarkCircle20Regular primaryFill="#10B981" />
                    default: return <Info20Regular />
                  }
                }*/

                return (
                  <StatusCard
                    key={index}
                    status={issue.severity === 'Ok' ? 'success' :
                            issue.severity === 'Critical' ? 'error' :
                            issue.severity === 'Warning' ? 'warning' : 'info'}
                    title={issue.title}
                    description={issue.description}
                    badge={issue.category}
                    details={issue.recommendation ? [issue.recommendation] : undefined}
                    actions={issue.detected && issue.id ? [{
                      label: fixingIssue === issue.id ? 'Fixing...' : 'Fix Issue',
                      onClick: () => handleFixIssue(issue.id),
                      disabled: fixingIssue !== null,
                      primary: true
                    }] : undefined}
                  />
                )
              })}
            </div>
          )}
        </Card>
      </div>
    )
  }

  return (
    <FluentProvider theme={wfDarkTheme}>
      <div className={styles.mainContainer}>
        {/* Navigation Header */}
        <NavigationHeader
          computerName={systemInfo?.computer_name}
          osVersion={systemInfo?.os_version}
          isAdmin={systemInfo?.is_admin}
          onRestartAsAdmin={restartAsAdmin}
          onOpenSettings={() => setShowSettings(true)}
          onExportDiagnostics={exportResults}
          version="2.1.0"
        />

        {/* Tab Navigation */}
        <TabNavigation
          selectedTab={selectedTab}
          onTabSelect={setSelectedTab}
          issueCount={issues.filter(i => i.detected).length}
          isMonitoringActive={isMonitoringActive}
          hasAIKey={!!settings.openAiApiKey}
        />

        {/* Main Content Area */}
        <main className={styles.contentArea}>
          <div style={{ maxWidth: selectedTab === 'ai' ? '1400px' : '1200px', margin: '0 auto' }}>
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
            {selectedTab === 'history' && (
              <ComparisonView
                onClose={() => setSelectedTab('diagnostics')}
              />
            )}
          </div>
        </main>

        {/* Settings Dialog */}
        <SettingsDialog
          open={showSettings}
          onOpenChange={setShowSettings}
          settings={settings}
          onSave={handleSettingsSave}
        />
      </div>
    </FluentProvider>
  )
}

export default App