import React, { useEffect, useState, useRef } from 'react'
import { SystemMonitoring } from './SystemMonitoring'
import { OpenAIIntegration } from './OpenAIIntegration'
import { ComparisonView } from './ComparisonView'
import { DiagnosticsTab } from './tabs/DiagnosticsTab'
import { IssuesTab } from './tabs/IssuesTab'
import { ProcessesTab } from './tabs/ProcessesTab'
import {
  NavRail,
  SettingsDialog,
  AboutDialog,
  StatusBar,
  type SettingsData
} from './components'
import { AppProvider, useAppContext } from './contexts/AppContext'
import { AIProvider } from './contexts/AIContext'
import { ToastProvider, useToast } from './contexts/ToastContext'
import { ThemeProvider } from './contexts/ThemeContext'
import { useDiagnostics } from './hooks/useDiagnostics'
import { useScanner } from './hooks/useScanner'
import {
  NAV_RAIL_WIDTH_COLLAPSED,
  NAV_RAIL_WIDTH_EXPANDED,
  NAV_RAIL_BREAKPOINT,
} from './theme'
import './styles.css'
import {
  makeStyles,
  tokens,
  shorthands,
} from '@fluentui/react-components'

const useStyles = makeStyles({
  appContainer: {
    display: 'flex',
    minHeight: '100vh',
    background: tokens.colorNeutralBackground1,
    color: tokens.colorNeutralForeground1,
  },
  mainArea: {
    flex: 1,
    display: 'flex',
    flexDirection: 'column',
    height: '100vh',
    overflow: 'hidden',
    transitionProperty: 'margin-left',
    transitionDuration: tokens.durationNormal,
    transitionTimingFunction: tokens.curveEasyEase,
  },
  mainAreaCollapsed: {
    marginLeft: `${NAV_RAIL_WIDTH_COLLAPSED}px`,
  },
  mainAreaExpanded: {
    marginLeft: `${NAV_RAIL_WIDTH_EXPANDED}px`,
    [`@media (max-width: ${NAV_RAIL_BREAKPOINT}px)`]: {
      marginLeft: `${NAV_RAIL_WIDTH_COLLAPSED}px`,
    },
  },
  contentArea: {
    flex: 1,
    ...shorthands.padding(tokens.spacingVerticalL, tokens.spacingHorizontalXL),
    overflowY: 'auto',
    overflowX: 'hidden',
  },
  contentWrapper: {
    maxWidth: '1400px',
    margin: '0 auto',
    width: '100%',
    animation: 'fadeSlideIn 0.2s ease-out',
  },
})

const AppContent: React.FC = () => {
  const styles = useStyles()
  const {
    selectedTab,
    setSelectedTab,
    systemInfo,
    isMonitoringActive,
    setIsMonitoringActive,
    showSettings,
    setShowSettings,
    showAbout,
    setShowAbout,
    settings,
    saveSettings,
    issues,
    sessionId,
    navRailCollapsed,
    setNavRailCollapsed,
    results,
    currentTaskName,
    scanStartTime,
    scanEndTime,
    isRunning,
  } = useAppContext()

  const { detectIssues } = useDiagnostics()
  const { runQuickScan } = useScanner()
  const { showSuccess, showError } = useToast()

  // Calculate status bar metrics from results
  const resultValues = Object.values(results)
  const passedCount = resultValues.filter(r => r.success).length
  const failedCount = resultValues.filter(r => !r.success).length
  const scanDuration = scanEndTime > 0 ? scanEndTime - scanStartTime :
                       (isRunning ? Date.now() - scanStartTime : 0)

  // Track window width for responsive behavior
  const [isSmallScreen, setIsSmallScreen] = useState(false)
  // Track if we've already run the startup scan
  const hasRunStartupScan = useRef(false)

  useEffect(() => {
    const handleResize = () => {
      setIsSmallScreen(window.innerWidth < NAV_RAIL_BREAKPOINT)
    }
    handleResize()
    window.addEventListener('resize', handleResize)
    return () => window.removeEventListener('resize', handleResize)
  }, [])

  // Detect issues when session changes
  useEffect(() => {
    if (sessionId) {
      detectIssues()
    }
  }, [sessionId, detectIssues])

  // Get available tasks from context
  const { availableTasks } = useAppContext()

  // Scan on startup if enabled - wait for tasks to load first
  useEffect(() => {
    if (
      settings.scanOnStartup &&
      !isRunning &&
      !sessionId &&
      availableTasks.length > 0 &&
      !hasRunStartupScan.current
    ) {
      hasRunStartupScan.current = true
      console.log('Starting automatic scan on startup...', availableTasks.length, 'tasks available')
      runQuickScan()
    }
  }, [settings.scanOnStartup, availableTasks.length, isRunning, sessionId, runQuickScan])

  const handleSettingsSave = async (newSettings: SettingsData) => {
    try {
      await saveSettings(newSettings)
      setShowSettings(false)
      showSuccess('Settings Saved', 'Your preferences have been updated.')
    } catch (error) {
      console.error('Failed to save settings:', error)
      showError('Save Failed', 'Could not save settings. Please try again.')
    }
  }

  // Determine if nav rail should appear collapsed
  const effectiveCollapsed = isSmallScreen || navRailCollapsed

  return (
    <div className={styles.appContainer}>
      {/* NavRail - Vertical Navigation */}
      <NavRail
        selectedItem={selectedTab}
        onItemSelect={setSelectedTab}
        collapsed={navRailCollapsed}
        onToggleCollapse={() => setNavRailCollapsed(!navRailCollapsed)}
        issueCount={issues.filter(i => i.detected).length}
        isMonitoringActive={isMonitoringActive}
        hasAIKey={!!settings.openAiApiKey}
        onOpenSettings={() => setShowSettings(true)}
        onOpenAbout={() => setShowAbout(true)}
        systemInfo={systemInfo}
      />

      {/* Main Content Area */}
      <main className={`${styles.mainArea} ${effectiveCollapsed ? styles.mainAreaCollapsed : styles.mainAreaExpanded}`}>
        <div className={styles.contentArea}>
          <div key={selectedTab} className={styles.contentWrapper}>
            {selectedTab === 'diagnostics' && <DiagnosticsTab />}
            {selectedTab === 'monitoring' && (
              <SystemMonitoring
                isActive={isMonitoringActive}
                onToggle={setIsMonitoringActive}
              />
            )}
            {selectedTab === 'processes' && <ProcessesTab />}
            {selectedTab === 'ai' && (
              <OpenAIIntegration
                sessionId={sessionId || ''}
              />
            )}
            {selectedTab === 'issues' && <IssuesTab />}
            {selectedTab === 'history' && (
              <ComparisonView
                onClose={() => setSelectedTab('diagnostics')}
              />
            )}
          </div>
        </div>

        {/* Status Bar */}
        <StatusBar
          passed={passedCount}
          failed={failedCount}
          duration={scanDuration}
          isAdmin={systemInfo?.is_admin}
          currentTask={isRunning ? currentTaskName : undefined}
        />
      </main>

      {/* Settings Dialog */}
      <SettingsDialog
        open={showSettings}
        onOpenChange={setShowSettings}
        settings={settings}
        onSave={handleSettingsSave}
      />

      {/* About Dialog */}
      <AboutDialog
        open={showAbout}
        onOpenChange={setShowAbout}
      />
    </div>
  )
}

// Wrapper that reads settings and applies theme
const ThemedApp: React.FC = () => {
  const { settings, settingsLoaded } = useAppContext()

  // Wait for settings to load before rendering themed content
  if (!settingsLoaded) {
    return null // Or a loading spinner
  }

  return (
    <ThemeProvider initialMode={(settings.theme as 'dark' | 'light' | 'auto') || 'dark'}>
      <AIProvider>
        <ToastProvider>
          <AppContent />
        </ToastProvider>
      </AIProvider>
    </ThemeProvider>
  )
}

const App: React.FC = () => {
  return (
    <AppProvider>
      <ThemedApp />
    </AppProvider>
  )
}

export default App