import React from 'react'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { ComparisonView } from '../ComparisonView'
import {
  CommandBar,
  QuickActionPanel,
  StatusCard,
  ScanResultCard,
} from '../components'
import { useAppContext } from '../contexts/AppContext'
import { useDiagnostics } from '../hooks/useDiagnostics'
import { useScanner } from '../hooks/useScanner'
import {
  Card,
  Title3,
  Caption1,
  Text,
  Divider,
  tokens,
  Badge,
  Body1,
  SearchBox,
  makeStyles,
  shorthands
} from '@fluentui/react-components'
import {
  CheckmarkCircle20Regular,
  Warning20Regular,
  Info20Regular,
} from '@fluentui/react-icons'

const useStyles = makeStyles({
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
  categoryNav: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalL),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    marginBottom: tokens.spacingVerticalS,
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
    marginBottom: tokens.spacingVerticalXL,
    ...shorthands.gap(tokens.spacingVerticalM),
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
    searchQuery,
    setSearchQuery,
    filteredResults,
    scanStartTime,
    showDebug,
    setShowDebug,
  } = useAppContext()

  const {
    getHealthScore,
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
  }, {} as Record<string, Array<{ task: any; result: any }>>)

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
          onShareToForum={hasResults ? shareToWindowsForum : undefined}
          onEmailReport={hasResults ? emailReport : undefined}
          onGenerateSupportPackage={hasResults ? generateSupportPackage : undefined}
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
                      <Body1 style={{ color: tokens.colorNeutralForeground1 }}>{category}</Body1>
                    </div>
                    <Caption1 style={{ color: tokens.colorNeutralForeground2 }}>
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