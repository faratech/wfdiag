import React, { useMemo, useState } from 'react'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { ComparisonView } from '../ComparisonView'
import {
  CommandBar,
  QuickActionPanel,
  StatusCard,
  PageHeader,
  HealthModel,
  SystemSummary,
  SectionHeader,
  InsightPanel,
  DiagnosticCard
} from '../components'
import { useAppContext } from '../contexts/AppContext'
import { useDiagnostics } from '../hooks/useDiagnostics'
import { useScanner } from '../hooks/useScanner'
import {
  makeStyles,
  tokens,
  shorthands
} from '@fluentui/react-components'
import {
  Stethoscope20Regular,
  Desktop24Regular,
  Database24Regular,
  NetworkAdapter16Regular,
  Shield24Regular,
  HeartPulse24Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  diagnosticsContainer: {
    maxWidth: '1200px',
    margin: '0 auto',
    display: 'flex',
    flexDirection: 'column',
    height: '100%',
    paddingBottom: tokens.spacingVerticalXL,
  },
  contentContainer: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalL),
  },
  section: {
    marginBottom: tokens.spacingVerticalXL,
  },
  grid: {
    display: 'grid',
    gridTemplateColumns: 'repeat(auto-fill, minmax(400px, 1fr))',
    gap: tokens.spacingHorizontalL,
  }
})

export const DiagnosticsTab: React.FC = () => {
  const styles = useStyles()
  const {
    systemInfo,
    availableTasks,
    results,
    isRunning,
    currentProgress,
    currentTaskName,
    showComparison,
    setShowComparison,
    globalSearchQuery,
    setGlobalSearchQuery,
    globalSearchResults,
    performGlobalSearch,
    setSelectedTab,
    scanStartTime,
    scanEndTime,
    highlightedTaskId,
    setHighlightedTaskId,
    searchHighlight,
    setSearchHighlight
  } = useAppContext()

  const {
    copyToClipboard,
    exportResults,
    shareToWindowsForum,
    emailReport,
    generateSupportPackage,
    restartAsAdmin,
  } = useDiagnostics()

  const {
    runQuickScan,
    runFullScan,
    stopScan,
    clearResults,
  } = useScanner()

  const [activeMetric, setActiveMetric] = useState<string | null>(null)

  // Calculate stats and organize results
  const analysis = useMemo(() => {
    const hasResults = Object.keys(results).length > 0
    if (!hasResults) return null

    const categories = {
      System: ['System', 'Drivers', 'Software', 'Logs'],
      Hardware: ['Hardware'],
      Storage: ['Storage'],
      Network: ['Network']
    }

    const groupedResults: Record<string, any[]> = {
      System: [],
      Hardware: [],
      Storage: [],
      Network: []
    }

    let faultCount = 0
    let warningCount = 0
    // Use scanEndTime if available, otherwise use current time (for in-progress scans)
    const endTime = scanEndTime > 0 ? scanEndTime : Date.now()
    let duration = endTime - scanStartTime

    // Group results
    Object.entries(results).forEach(([taskId, result]) => {
      const task = availableTasks.find(t => t.id === taskId)
      if (!task) return

      if (!result.success) faultCount++
      
      // Determine high-level group
      let group = 'System'
      if (categories.Hardware.includes(task.category)) group = 'Hardware'
      else if (categories.Storage.includes(task.category)) group = 'Storage'
      else if (categories.Network.includes(task.category)) group = 'Network'
      
      groupedResults[group].push({ task, result })
    })

    // Calculate scores
    const calculateScore = (group: string) => {
      const groupItems = groupedResults[group]
      if (groupItems.length === 0) return 100
      const passed = groupItems.filter(i => i.result.success).length
      return Math.round((passed / groupItems.length) * 100)
    }

    const scores = {
      System: calculateScore('System'),
      Hardware: calculateScore('Hardware'),
      Storage: calculateScore('Storage'),
      Network: calculateScore('Network'),
      Integrity: calculateScore('System') // Reuse system for now
    }

    return {
      groupedResults,
      scores,
      stats: {
        totalTasks: Object.keys(results).length,
        faultCount,
        warningCount,
        duration
      }
    }
  }, [results, availableTasks, scanStartTime, scanEndTime])

  const handleSearchChange = (value: string) => {
    setGlobalSearchQuery(value)
    performGlobalSearch(value)
  }

  const handleSearchResultSelect = (result: any) => {
    if (result.navigateTo) {
      setSelectedTab(result.navigateTo)
    }
    // Set highlight state for scroll and text highlighting
    if (result.data?.taskId) {
      setHighlightedTaskId(result.data.taskId)
      setSearchHighlight(globalSearchQuery)
      // Scroll to the element after a brief delay for render
      setTimeout(() => {
        const element = document.getElementById(`diagnostic-card-${result.data.taskId}`)
        if (element) {
          element.scrollIntoView({ behavior: 'smooth', block: 'center' })
          // Clear highlight after a few seconds
          setTimeout(() => {
            setHighlightedTaskId(null)
            setSearchHighlight('')
          }, 3000)
        }
      }, 100)
    }
  }

  const getRiskLevel = (score: number) => {
    if (score >= 90) return 'Low'
    if (score >= 70) return 'Monitor'
    return 'Elevated'
  }

  if (showComparison) {
    return (
      <ComparisonView
        onClose={() => setShowComparison(false)}
      />
    )
  }

  const hasResults = !!analysis
  const isComplete = hasResults && !isRunning

  return (
    <div className={styles.diagnosticsContainer}>
      <PageHeader
        title="System Analysis"
        description="Comprehensive diagnostic engine"
        icon={<Stethoscope20Regular />}
        showSearch
        searchValue={globalSearchQuery}
        onSearchChange={handleSearchChange}
        searchResults={globalSearchResults}
        onSearchResultSelect={handleSearchResultSelect}
        onNavigate={setSelectedTab}
        searchPlaceholder="Search system components..."
      />

      {/* Action Bar / Status */}
      <CommandBar
        onQuickScan={runQuickScan}
        onFullScan={runFullScan}
        onStopScan={isRunning ? stopScan : undefined}
        isScanning={isRunning}
        onExport={hasResults ? exportResults : undefined}
        onCopyToClipboard={hasResults ? copyToClipboard : undefined}
        onShareToForum={hasResults ? shareToWindowsForum : undefined}
        onEmailReport={hasResults ? emailReport : undefined}
        onGenerateSupportPackage={hasResults ? generateSupportPackage : undefined}
        onToggleFilter={() => {}}
        onClearResults={hasResults ? clearResults : undefined}
        onCompareScans={() => setShowComparison(true)}
        scanStatus={isRunning ? 'scanning' : hasResults ? 'complete' : 'idle'}
        resultCount={Object.keys(results).length}
      />

      {/* Progress or Empty State */}
      {(!hasResults || isRunning) && (
        <div style={{ marginTop: tokens.spacingVerticalL }}>
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
          />
        </div>
      )}

      {/* Analysis Results */}
      {isComplete && analysis && (
        <div className={styles.contentContainer}>
          <SystemSummary
            taskCount={analysis.stats.totalTasks}
            durationMs={analysis.stats.duration}
            faultCount={analysis.stats.faultCount}
            warningCount={analysis.stats.warningCount}
            completed={true}
          />

          <HealthModel
            activeMetricId={activeMetric}
            onMetricClick={(id) => {
              setActiveMetric(activeMetric === id ? null : id)
              const el = document.getElementById(`section-${id}`)
              if (el) el.scrollIntoView({ behavior: 'smooth' })
            }}
            metrics={[
              { 
                id: 'cpu', 
                label: 'Hardware', 
                score: analysis.scores.Hardware, 
                risk: getRiskLevel(analysis.scores.Hardware),
                icon: <Desktop24Regular />
              },
              { 
                id: 'memory', 
                label: 'System', 
                score: analysis.scores.System, 
                risk: getRiskLevel(analysis.scores.System),
                icon: <HeartPulse24Regular />
              },
              { 
                id: 'storage', 
                label: 'Storage', 
                score: analysis.scores.Storage, 
                risk: getRiskLevel(analysis.scores.Storage),
                icon: <Database24Regular />
              },
              { 
                id: 'network', 
                label: 'Network', 
                score: analysis.scores.Network, 
                risk: getRiskLevel(analysis.scores.Network),
                icon: <NetworkAdapter16Regular />
              },
              { 
                id: 'integrity', 
                label: 'Integrity', 
                score: analysis.scores.Integrity, 
                risk: getRiskLevel(analysis.scores.Integrity),
                icon: <Shield24Regular />
              },
            ]}
          />

          {/* Sections */}
          {Object.entries(analysis.groupedResults).map(([group, items]) => {
            if (items.length === 0) return null
            
            // Map group to metric ID for scrolling
            let metricId = 'system'
            if (group === 'Hardware') metricId = 'cpu'
            if (group === 'Storage') metricId = 'storage'
            if (group === 'Network') metricId = 'network'

            const successCount = items.filter(i => i.result.success).length
            const isPerfect = successCount === items.length

            return (
              <div key={group} id={`section-${metricId}`} className={styles.section}>
                <SectionHeader
                  title={group}
                  count={items.length}
                  successCount={successCount}
                />

                <InsightPanel
                  title={`${group} Analysis`}
                  content={isPerfect 
                    ? `All ${group.toLowerCase()} tests passed successfully. Configuration matches recommended baselines for optimal performance.`
                    : `${items.length - successCount} issues detected in ${group.toLowerCase()} configuration. Review specific findings below for resolution steps.`
                  }
                />

                <div className={styles.grid}>
                  {items.map(({ task, result }) => (
                    <div key={task.id} id={`diagnostic-card-${task.id}`}>
                      <DiagnosticCard
                        title={task.name}
                        description={task.description}
                        status={result.success ? 'verified' : 'action_required'}
                        importance={task.category === group ? 'primary' : 'secondary'}
                        executionTime={result.duration_ms}
                        output={result.output}
                        error={result.error}
                        category={task.category}
                        onCopyOutput={(text) => writeText(text)}
                        isHighlighted={highlightedTaskId === task.id}
                        highlightText={highlightedTaskId === task.id ? searchHighlight : ''}
                      />
                    </div>
                  ))}
                </div>
              </div>
            )
          })}
        </div>
      )}

      {/* Admin Warning Footer if needed */}
      {isComplete && !systemInfo?.is_admin && (
        <div style={{ marginTop: tokens.spacingVerticalXL }}>
          <StatusCard
            status="warning"
            title="Administrator Access Required for Deep Analysis"
            description="Some advanced diagnostics were skipped due to permission restrictions."
            details={['Storage health checks', 'Crash dump analysis', 'System file verification']}
            actions={[{ label: 'Restart as Administrator', onClick: restartAsAdmin, primary: true }]}
          />
        </div>
      )}
    </div>
  )
}