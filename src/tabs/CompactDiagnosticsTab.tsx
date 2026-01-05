import React, { useMemo } from 'react'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import {
  makeStyles,
  tokens,
  shorthands,
  Text
} from '@fluentui/react-components'
import {
  ChevronRight12Regular,
  ChevronDown12Regular,
  Folder16Regular,
  FolderOpen16Regular
} from '@fluentui/react-icons'
import { CompactToolbar } from '../components/CompactToolbar'
import { StatusBar } from '../components/StatusBar'
import { DiagnosticRow } from '../components/DiagnosticRow'
import { useAppContext } from '../contexts/AppContext'
import { useDiagnostics } from '../hooks/useDiagnostics'
import { useScanner } from '../hooks/useScanner'

const useStyles = makeStyles({
  container: {
    display: 'flex',
    flexDirection: 'column',
    height: '100%',
    backgroundColor: tokens.colorNeutralBackground1,
  },
  content: {
    flex: 1,
    overflowY: 'auto',
    overflowX: 'hidden',
  },
  section: {
    borderBottom: `1px solid ${tokens.colorNeutralStroke1}`,
  },
  sectionHeader: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('6px'),
    ...shorthands.padding('6px', '8px'),
    backgroundColor: tokens.colorNeutralBackground3,
    cursor: 'pointer',
    ':hover': {
      backgroundColor: tokens.colorNeutralBackground3Hover,
    },
    fontSize: tokens.fontSizeBase200,
    fontWeight: tokens.fontWeightSemibold,
  },
  sectionIcon: {
    width: '16px',
    height: '16px',
    color: tokens.colorNeutralForeground2,
  },
  expandIcon: {
    width: '12px',
    height: '12px',
    color: tokens.colorNeutralForeground3,
  },
  sectionTitle: {
    flex: 1,
    color: tokens.colorNeutralForeground1,
  },
  sectionStats: {
    fontSize: tokens.fontSizeBase100,
    color: tokens.colorNeutralForeground3,
  },
  sectionContent: {
    display: 'flex',
    flexDirection: 'column',
  },
  listHeader: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.padding('4px', '8px'),
    ...shorthands.gap('8px'),
    backgroundColor: tokens.colorNeutralBackground2,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
    fontSize: tokens.fontSizeBase100,
    color: tokens.colorNeutralForeground3,
    fontWeight: tokens.fontWeightSemibold,
  },
  colExpand: {
    width: '12px',
  },
  colStatus: {
    width: '12px',
  },
  colName: {
    flex: 1,
  },
  colResult: {
    width: '80px',
    textAlign: 'right',
  },
  colTime: {
    width: '50px',
    textAlign: 'right',
  },
  emptyState: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    ...shorthands.padding('48px'),
    color: tokens.colorNeutralForeground3,
  },
  emptyTitle: {
    fontSize: tokens.fontSizeBase400,
    fontWeight: tokens.fontWeightSemibold,
    marginBottom: '8px',
  },
  emptyText: {
    fontSize: tokens.fontSizeBase200,
  },
})

interface SectionState {
  [key: string]: boolean
}

export const CompactDiagnosticsTab: React.FC = () => {
  const styles = useStyles()
  const {
    systemInfo,
    availableTasks,
    results,
    isRunning,
    currentProgress,
    currentTaskName,
    setShowComparison,
    scanStartTime,
    scanEndTime,
  } = useAppContext()

  const { copyToClipboard, exportResults } = useDiagnostics()
  const { runQuickScan, runFullScan, stopScan, clearResults } = useScanner()

  const [expandedSections, setExpandedSections] = React.useState<SectionState>({
    Hardware: true,
    System: true,
    Storage: true,
    Network: true,
  })

  const toggleSection = (section: string) => {
    setExpandedSections(prev => ({
      ...prev,
      [section]: !prev[section]
    }))
  }

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
      Hardware: [],
      System: [],
      Storage: [],
      Network: []
    }

    let passed = 0
    let failed = 0
    let warnings = 0
    const endTime = scanEndTime > 0 ? scanEndTime : Date.now()
    const duration = endTime - scanStartTime

    Object.entries(results).forEach(([taskId, result]) => {
      const task = availableTasks.find(t => t.id === taskId)
      if (!task) return

      if (result.success) passed++
      else failed++

      let group = 'System'
      if (categories.Hardware.includes(task.category)) group = 'Hardware'
      else if (categories.Storage.includes(task.category)) group = 'Storage'
      else if (categories.Network.includes(task.category)) group = 'Network'

      groupedResults[group].push({ task, result })
    })

    return { groupedResults, passed, failed, warnings, duration }
  }, [results, availableTasks, scanStartTime, scanEndTime])

  const hasResults = !!analysis
  const resultCount = Object.keys(results).length

  return (
    <div className={styles.container}>
      <CompactToolbar
        onQuickScan={runQuickScan}
        onFullScan={runFullScan}
        onStop={isRunning ? stopScan : undefined}
        onExport={hasResults ? exportResults : undefined}
        onCopy={hasResults ? copyToClipboard : undefined}
        onCompare={hasResults ? () => setShowComparison(true) : undefined}
        onClear={hasResults ? clearResults : undefined}
        isScanning={isRunning}
        progress={currentProgress}
        resultCount={resultCount}
      />

      <div className={styles.content}>
        {!hasResults && !isRunning ? (
          <div className={styles.emptyState}>
            <Text className={styles.emptyTitle}>No Results</Text>
            <Text className={styles.emptyText}>
              Click Quick or Full to start a diagnostic scan.
            </Text>
          </div>
        ) : (
          <>
            {/* Column headers */}
            <div className={styles.listHeader}>
              <span className={styles.colExpand}></span>
              <span className={styles.colStatus}></span>
              <span className={styles.colName}>Name</span>
              <span className={styles.colResult}>Status</span>
              <span className={styles.colTime}>Time</span>
            </div>

            {/* Sections */}
            {analysis && Object.entries(analysis.groupedResults).map(([section, items]) => {
              if (items.length === 0) return null
              const isExpanded = expandedSections[section]
              const passedCount = items.filter(i => i.result.success).length

              return (
                <div key={section} className={styles.section}>
                  <div
                    className={styles.sectionHeader}
                    onClick={() => toggleSection(section)}
                  >
                    {isExpanded
                      ? <ChevronDown12Regular className={styles.expandIcon} />
                      : <ChevronRight12Regular className={styles.expandIcon} />
                    }
                    {isExpanded
                      ? <FolderOpen16Regular className={styles.sectionIcon} />
                      : <Folder16Regular className={styles.sectionIcon} />
                    }
                    <Text className={styles.sectionTitle}>{section}</Text>
                    <Text className={styles.sectionStats}>
                      {passedCount}/{items.length} OK
                    </Text>
                  </div>
                  {isExpanded && (
                    <div className={styles.sectionContent}>
                      {items.map(({ task, result }) => (
                        <DiagnosticRow
                          key={task.id}
                          name={task.name}
                          status={result.success ? 'passed' : 'failed'}
                          duration={result.duration_ms}
                          output={typeof result.output === 'string' ? result.output : JSON.stringify(result.output)}
                          error={result.error}
                          onCopy={(text) => writeText(text)}
                        />
                      ))}
                    </div>
                  )}
                </div>
              )
            })}
          </>
        )}
      </div>

      <StatusBar
        passed={analysis?.passed ?? 0}
        failed={analysis?.failed ?? 0}
        warnings={analysis?.warnings ?? 0}
        duration={analysis?.duration}
        isAdmin={systemInfo?.is_admin}
        currentTask={isRunning ? currentTaskName : undefined}
      />
    </div>
  )
}

export default CompactDiagnosticsTab
