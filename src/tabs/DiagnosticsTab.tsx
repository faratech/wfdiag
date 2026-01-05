import React, { useMemo, useState, useCallback, useEffect } from 'react'
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
import { useAI } from '../hooks/useAI'
import {
  makeStyles,
  tokens,
  shorthands,
  Button,
  Tooltip,
  Text
} from '@fluentui/react-components'
import {
  Stethoscope20Regular,
  Desktop24Regular,
  Database24Regular,
  NetworkAdapter16Regular,
  Shield24Regular,
  HeartPulse24Regular,
  ArrowUp20Regular,
  Filter16Regular,
  Checkmark16Regular,
  Warning16Regular
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
    gridTemplateColumns: 'repeat(auto-fill, minmax(340px, 1fr))',
    gap: tokens.spacingHorizontalL,
  },
  cardWrapper: {
    animation: 'fadeSlideIn 0.3s ease-out backwards',
    ...shorthands.outline('none'),
    ':focus': {
      ...shorthands.outline('2px', 'solid', tokens.colorBrandStroke1),
      outlineOffset: '2px',
      ...shorthands.borderRadius(tokens.borderRadiusMedium),
    },
    ':focus-visible': {
      ...shorthands.outline('2px', 'solid', tokens.colorBrandStroke1),
      outlineOffset: '2px',
    },
  },
  // Back to top button
  backToTop: {
    position: 'fixed',
    bottom: tokens.spacingVerticalXXL,
    right: tokens.spacingHorizontalXXL,
    zIndex: 1000,
    opacity: 0,
    transform: 'translateY(20px)',
    pointerEvents: 'none',
    transitionProperty: 'opacity, transform',
    transitionDuration: tokens.durationNormal,
    transitionTimingFunction: tokens.curveEasyEase,
    boxShadow: tokens.shadow16,
    ...shorthands.borderRadius(tokens.borderRadiusCircular),
  },
  backToTopVisible: {
    opacity: 1,
    transform: 'translateY(0)',
    pointerEvents: 'auto',
  },
  // Section quick-jump menu
  sectionNav: {
    position: 'sticky',
    top: 0,
    zIndex: 100,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
    backgroundColor: tokens.colorNeutralBackground1,
    backdropFilter: 'blur(8px)',
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke2),
    marginBottom: tokens.spacingVerticalM,
    boxShadow: tokens.shadow4,
  },
  sectionNavButton: {
    minWidth: 'auto',
    ...shorthands.padding(tokens.spacingVerticalXS, tokens.spacingHorizontalM),
    fontSize: tokens.fontSizeBase200,
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground1,
  },
  sectionNavActive: {
    backgroundColor: tokens.colorBrandBackground2,
    color: tokens.colorBrandForeground1,
  },
  // Result filter controls
  filterBar: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    flexWrap: 'wrap',
    ...shorthands.gap(tokens.spacingHorizontalM),
    ...shorthands.padding(tokens.spacingVerticalS, 0),
  },
  filterGroup: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
  },
  filterLabel: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
    marginRight: tokens.spacingHorizontalXS,
  },
  filterButton: {
    minWidth: 'auto',
    fontSize: tokens.fontSizeBase200,
  },
  filterButtonActive: {
    backgroundColor: tokens.colorBrandBackground2,
    color: tokens.colorBrandForeground1,
    fontWeight: tokens.fontWeightSemibold,
  },
  filterCount: {
    fontSize: tokens.fontSizeBase100,
    color: tokens.colorNeutralForeground3,
    marginLeft: tokens.spacingHorizontalXXS,
  },
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
  const [showBackToTop, setShowBackToTop] = useState(false)
  const [activeSection, setActiveSection] = useState<string | null>(null)
  const [resultFilter, setResultFilter] = useState<'all' | 'passed' | 'failed'>('all')

  // Keyboard navigation for diagnostic cards
  useEffect(() => {
    let currentIndex = -1

    const handleKeyDown = (e: KeyboardEvent) => {
      // Only handle when results are available and not scanning
      if (!Object.keys(results).length || isRunning) return

      // Don't handle if user is typing in an input
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return

      // Get all visible card elements
      const allCards = document.querySelectorAll('[id^="diagnostic-card-"]')
      const cardCount = allCards.length

      if (cardCount === 0) return

      switch (e.key) {
        case 'ArrowDown':
        case 'j': // vim-style navigation
          e.preventDefault()
          currentIndex = currentIndex < cardCount - 1 ? currentIndex + 1 : 0
          const downCard = allCards[currentIndex] as HTMLElement
          downCard?.scrollIntoView({ behavior: 'smooth', block: 'center' })
          downCard?.focus()
          break
        case 'ArrowUp':
        case 'k': // vim-style navigation
          e.preventDefault()
          currentIndex = currentIndex > 0 ? currentIndex - 1 : cardCount - 1
          const upCard = allCards[currentIndex] as HTMLElement
          upCard?.scrollIntoView({ behavior: 'smooth', block: 'center' })
          upCard?.focus()
          break
        case 'Home':
          e.preventDefault()
          currentIndex = 0
          const firstCard = allCards[0] as HTMLElement
          firstCard?.scrollIntoView({ behavior: 'smooth', block: 'center' })
          firstCard?.focus()
          break
        case 'End':
          e.preventDefault()
          currentIndex = cardCount - 1
          const lastCard = allCards[cardCount - 1] as HTMLElement
          lastCard?.scrollIntoView({ behavior: 'smooth', block: 'center' })
          lastCard?.focus()
          break
        case 'Escape':
          // Clear focus and reset index
          currentIndex = -1
          ;(document.activeElement as HTMLElement)?.blur()
          break
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [results, isRunning])

  // Scroll handling for back-to-top button and section tracking
  useEffect(() => {
    const handleScroll = () => {
      // Show back-to-top after scrolling 400px
      setShowBackToTop(window.scrollY > 400)

      // Track active section based on scroll position
      const sections = ['cpu', 'memory', 'storage', 'network']
      for (const sectionId of sections) {
        const element = document.getElementById(`section-${sectionId}`)
        if (element) {
          const rect = element.getBoundingClientRect()
          if (rect.top <= 150 && rect.bottom > 150) {
            setActiveSection(sectionId)
            break
          }
        }
      }
    }

    window.addEventListener('scroll', handleScroll, { passive: true })
    return () => window.removeEventListener('scroll', handleScroll)
  }, [])

  const scrollToTop = useCallback(() => {
    window.scrollTo({ top: 0, behavior: 'smooth' })
  }, [])

  const scrollToSection = useCallback((sectionId: string) => {
    const element = document.getElementById(`section-${sectionId}`)
    if (element) {
      element.scrollIntoView({ behavior: 'smooth', block: 'start' })
    }
  }, [])

  // AI integration for section summaries
  const {
    aiEnabled,
    isAIAvailable,
    activeProvider,
    requestSectionInterpretation,
    getCachedSectionInterpretation,
    isSectionLoading,
    getProviderDisplayName
  } = useAI()

  // Handle section AI analysis request
  const handleSectionAnalysis = useCallback(async (sectionName: string, items: any[]) => {
    // Format the section data for AI analysis
    const sectionData = items.map(({ task, result }) => ({
      task: task.name,
      category: task.category,
      success: result.success,
      output: typeof result.output === 'string' ? result.output : JSON.stringify(result.output),
      error: result.error
    }))

    await requestSectionInterpretation(sectionName, JSON.stringify(sectionData, null, 2))
  }, [requestSectionInterpretation])

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

          {/* Section Quick-Jump Navigation and Filters */}
          <div className={styles.filterBar}>
            <nav className={styles.sectionNav} aria-label="Jump to section">
              {[
                { id: 'cpu', label: 'Hardware', icon: <Desktop24Regular /> },
                { id: 'memory', label: 'System', icon: <HeartPulse24Regular /> },
                { id: 'storage', label: 'Storage', icon: <Database24Regular /> },
                { id: 'network', label: 'Network', icon: <NetworkAdapter16Regular /> },
              ].map((section) => {
                const groupName = section.label === 'Hardware' ? 'Hardware' :
                                 section.label === 'System' ? 'System' :
                                 section.label === 'Storage' ? 'Storage' : 'Network'
                const hasItems = analysis.groupedResults[groupName]?.length > 0
                if (!hasItems) return null

                return (
                  <Tooltip
                    key={section.id}
                    content={`Jump to ${section.label} section`}
                    relationship="label"
                  >
                    <Button
                      appearance="subtle"
                      size="small"
                      icon={section.icon}
                      onClick={() => scrollToSection(section.id)}
                      className={`${styles.sectionNavButton} ${activeSection === section.id ? styles.sectionNavActive : ''}`}
                    >
                      {section.label}
                    </Button>
                  </Tooltip>
                )
              })}
            </nav>

            {/* Result Filters */}
            <div className={styles.filterGroup}>
              <Filter16Regular style={{ color: tokens.colorNeutralForeground3 }} />
              <Text className={styles.filterLabel}>Show:</Text>
              <Button
                appearance="subtle"
                size="small"
                className={`${styles.filterButton} ${resultFilter === 'all' ? styles.filterButtonActive : ''}`}
                onClick={() => setResultFilter('all')}
              >
                All
                <span className={styles.filterCount}>({analysis.stats.totalTasks})</span>
              </Button>
              <Button
                appearance="subtle"
                size="small"
                icon={<Checkmark16Regular />}
                className={`${styles.filterButton} ${resultFilter === 'passed' ? styles.filterButtonActive : ''}`}
                onClick={() => setResultFilter('passed')}
              >
                Passed
                <span className={styles.filterCount}>({analysis.stats.totalTasks - analysis.stats.faultCount})</span>
              </Button>
              <Button
                appearance="subtle"
                size="small"
                icon={<Warning16Regular />}
                className={`${styles.filterButton} ${resultFilter === 'failed' ? styles.filterButtonActive : ''}`}
                onClick={() => setResultFilter('failed')}
              >
                Needs Attention
                <span className={styles.filterCount}>({analysis.stats.faultCount})</span>
              </Button>
            </div>
          </div>

          {/* Sections */}
          {Object.entries(analysis.groupedResults).map(([group, items]) => {
            if (items.length === 0) return null

            // Apply filter to items
            const filteredItems = resultFilter === 'all'
              ? items
              : resultFilter === 'passed'
                ? items.filter(i => i.result.success)
                : items.filter(i => !i.result.success)

            // Skip section entirely if no items match filter
            if (filteredItems.length === 0) return null

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
                  aiEnabled={aiEnabled && isAIAvailable}
                  aiContent={getCachedSectionInterpretation(group)}
                  onRequestAI={() => handleSectionAnalysis(group, items)}
                  isLoadingAI={isSectionLoading(group)}
                  aiProviderName={getProviderDisplayName(activeProvider)}
                  hasAIResult={!!getCachedSectionInterpretation(group)}
                />

                <div className={styles.grid}>
                  {filteredItems.map(({ task, result }, index) => (
                    <div
                      key={task.id}
                      id={`diagnostic-card-${task.id}`}
                      className={styles.cardWrapper}
                      style={{ animationDelay: `${index * 40}ms` }}
                      tabIndex={0}
                      role="article"
                      aria-label={`${task.name}: ${result.success ? 'Passed' : 'Needs attention'}`}
                    >
                      <DiagnosticCard
                        taskId={task.id}
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

      {/* Back to Top Button */}
      <Tooltip content="Back to top" relationship="label">
        <Button
          appearance="primary"
          shape="circular"
          size="large"
          icon={<ArrowUp20Regular />}
          onClick={scrollToTop}
          className={`${styles.backToTop} ${showBackToTop ? styles.backToTopVisible : ''}`}
          aria-label="Scroll back to top"
        />
      </Tooltip>
    </div>
  )
}