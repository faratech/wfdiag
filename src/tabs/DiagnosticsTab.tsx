import React, { useMemo, useState, useCallback, useEffect } from 'react'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { ComparisonView } from '../ComparisonView'
import {
  CommandBar,
  QuickActionPanel,
  StatusCard,
  PageHeader,
  DiagnosticListPanel,
  DiagnosticDetailPanel,
  DiagnosticItem
} from '../components'
import { useAppContext } from '../contexts/AppContext'
import { useDiagnostics } from '../hooks/useDiagnostics'
import { useScanner } from '../hooks/useScanner'
import { useAI } from '../hooks/useAI'
import {
  makeStyles,
  tokens,
  shorthands,
  useThemeClassName
} from '@fluentui/react-components'
import { Stethoscope20Regular } from '@fluentui/react-icons'

const useStyles = makeStyles({
  container: {
    display: 'flex',
    flexDirection: 'column',
    height: '100%',
    overflow: 'hidden',
  },
  headerSection: {
    flexShrink: 0,
    ...shorthands.padding('0', tokens.spacingHorizontalL),
  },
  splitContainer: {
    display: 'flex',
    flex: 1,
    overflow: 'hidden',
    marginTop: tokens.spacingVerticalM,
  },
  emptyContainer: {
    flex: 1,
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.padding(tokens.spacingVerticalL, tokens.spacingHorizontalL),
  },
  adminWarning: {
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalL),
    flexShrink: 0,
  },
})

export const DiagnosticsTab: React.FC = () => {
  const styles = useStyles()
  const themeClassName = useThemeClassName()
  const isDarkMode = themeClassName.includes('dark')

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

  // State for two-panel layout
  const [selectedDiagnosticId, setSelectedDiagnosticId] = useState<string | null>(null)

  // AI integration
  const {
    aiEnabled,
    isAIAvailable,
    requestDiagnosticInterpretation,
    getCachedDiagnosticInterpretation,
    isDiagnosticLoading,
  } = useAI()

  // Convert results to DiagnosticItem array
  const diagnosticItems = useMemo((): DiagnosticItem[] => {
    const items: DiagnosticItem[] = []
    for (const [taskId, result] of Object.entries(results)) {
      const task = availableTasks.find(t => t.id === taskId)
      if (!task) continue

      items.push({
        id: taskId,
        name: task.name,
        category: task.category,
        success: result.success,
        error: result.error,
        output: typeof result.output === 'string'
          ? result.output
          : JSON.stringify(result.output, null, 2),
        durationMs: result.duration_ms || 0,
      })
    }
    return items
  }, [results, availableTasks])

  // Get selected item
  const selectedItem = useMemo(() => {
    if (!selectedDiagnosticId) return null
    return diagnosticItems.find(item => item.id === selectedDiagnosticId) || null
  }, [diagnosticItems, selectedDiagnosticId])

  // Auto-select first item when results come in
  useEffect(() => {
    if (diagnosticItems.length > 0 && !selectedDiagnosticId) {
      setSelectedDiagnosticId(diagnosticItems[0].id)
    }
  }, [diagnosticItems, selectedDiagnosticId])

  // Clear selection when results are cleared
  useEffect(() => {
    if (diagnosticItems.length === 0) {
      setSelectedDiagnosticId(null)
    }
  }, [diagnosticItems.length])

  // Keyboard navigation
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!diagnosticItems.length) return
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return

      const currentIndex = diagnosticItems.findIndex(item => item.id === selectedDiagnosticId)

      switch (e.key) {
        case 'ArrowDown':
        case 'j':
          e.preventDefault()
          const nextIndex = currentIndex < diagnosticItems.length - 1 ? currentIndex + 1 : 0
          setSelectedDiagnosticId(diagnosticItems[nextIndex].id)
          break
        case 'ArrowUp':
        case 'k':
          e.preventDefault()
          const prevIndex = currentIndex > 0 ? currentIndex - 1 : diagnosticItems.length - 1
          setSelectedDiagnosticId(diagnosticItems[prevIndex].id)
          break
        case 'Home':
          e.preventDefault()
          setSelectedDiagnosticId(diagnosticItems[0].id)
          break
        case 'End':
          e.preventDefault()
          setSelectedDiagnosticId(diagnosticItems[diagnosticItems.length - 1].id)
          break
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [diagnosticItems, selectedDiagnosticId])

  // Handle search
  const handleSearchChange = (value: string) => {
    setGlobalSearchQuery(value)
    performGlobalSearch(value)
  }

  const handleSearchResultSelect = (result: any) => {
    if (result.navigateTo) {
      setSelectedTab(result.navigateTo)
    }
    if (result.data?.taskId) {
      setSelectedDiagnosticId(result.data.taskId)
    }
  }

  // Handle copy for selected item
  const handleCopySelected = useCallback(() => {
    if (selectedItem) {
      const text = JSON.stringify({
        name: selectedItem.name,
        category: selectedItem.category,
        success: selectedItem.success,
        output: selectedItem.output,
        error: selectedItem.error,
      }, null, 2)
      writeText(text)
    }
  }, [selectedItem])

  // Handle AI request for selected item
  const handleRequestAI = useCallback(() => {
    if (selectedItem && selectedDiagnosticId) {
      requestDiagnosticInterpretation(selectedDiagnosticId, selectedItem.name, selectedItem.output)
    }
  }, [selectedItem, selectedDiagnosticId, requestDiagnosticInterpretation])

  // Show comparison view if active
  if (showComparison) {
    return <ComparisonView onClose={() => setShowComparison(false)} />
  }

  const hasResults = diagnosticItems.length > 0
  const isComplete = hasResults && !isRunning

  return (
    <div className={styles.container}>
      {/* Header */}
      <div className={styles.headerSection}>
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
          searchPlaceholder="Search diagnostics..."
        />

        {/* Command Bar */}
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
          resultCount={diagnosticItems.length}
        />
      </div>

      {/* Main Content Area */}
      {!hasResults || isRunning ? (
        // Empty/Scanning state
        <div className={styles.emptyContainer}>
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
      ) : (
        // Two-panel layout with results
        <div className={styles.splitContainer}>
          <DiagnosticListPanel
            diagnostics={diagnosticItems}
            selectedId={selectedDiagnosticId}
            onSelect={setSelectedDiagnosticId}
            isDarkMode={isDarkMode}
          />
          <DiagnosticDetailPanel
            item={selectedItem}
            onCopy={handleCopySelected}
            aiEnabled={aiEnabled && isAIAvailable}
            aiContent={selectedDiagnosticId ? getCachedDiagnosticInterpretation(selectedDiagnosticId) ?? undefined : undefined}
            onRequestAI={handleRequestAI}
            isLoadingAI={selectedDiagnosticId ? isDiagnosticLoading(selectedDiagnosticId) : false}
          />
        </div>
      )}

      {/* Admin Warning */}
      {isComplete && !systemInfo?.is_admin && (
        <div className={styles.adminWarning}>
          <StatusCard
            status="warning"
            title="Administrator Access Required"
            description="Some diagnostics were skipped due to permission restrictions."
            details={['Storage health', 'Crash dumps', 'System files']}
            actions={[{ label: 'Restart as Admin', onClick: restartAsAdmin, primary: true }]}
          />
        </div>
      )}
    </div>
  )
}
