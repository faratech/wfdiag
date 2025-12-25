import React, { useEffect } from 'react'
import { SystemMonitoring } from './SystemMonitoring'
import { OpenAIIntegration } from './OpenAIIntegration'
import { ComparisonView } from './ComparisonView'
import { DiagnosticsTab } from './tabs/DiagnosticsTab'
import { IssuesTab } from './tabs/IssuesTab'
import {
  NavigationHeader,
  TabNavigation,
  SettingsDialog,
  AboutDialog,
  type SettingsData
} from './components'
import { AppProvider, useAppContext } from './contexts/AppContext'
import { ToastProvider } from './contexts/ToastContext'
import { ThemeProvider } from './contexts/ThemeContext'
import { useDiagnostics } from './hooks/useDiagnostics'
import { useScanner } from './hooks/useScanner'
import './styles.css'
import {
  makeStyles,
  tokens,
} from '@fluentui/react-components'

const useStyles = makeStyles({
  mainContainer: {
    minHeight: '100vh',
    background: tokens.colorNeutralBackground1,
    color: tokens.colorNeutralForeground1,
    height: '100vh',
    position: 'relative',
    overflow: 'hidden',
    display: 'flex',
    flexDirection: 'column',
  },
  contentArea: {
    padding: tokens.spacingVerticalXXL,
    height: 'calc(100vh - 180px)', // Account for header + tabs (180px)
    overflowY: 'auto',
    overflowX: 'hidden',
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
  } = useAppContext()

  const { exportResults, restartAsAdmin, detectIssues } = useDiagnostics()
  const { runQuickScan, isRunning } = useScanner()

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
    if (settings.scanOnStartup && !isRunning && !sessionId && availableTasks.length > 0) {
      console.log('Starting automatic scan on startup...', availableTasks.length, 'tasks available')
      runQuickScan()
    }
  }, [settings.scanOnStartup, availableTasks.length]) // Run when settings load AND tasks are available

  const handleSettingsSave = async (newSettings: SettingsData) => {
    try {
      await saveSettings(newSettings)
      setShowSettings(false)
    } catch (error) {
      console.error('Failed to save settings:', error)
      // Settings dialog will remain open if save fails
    }
  }

  return (
    <div className={styles.mainContainer}>
      {/* Navigation Header */}
      <NavigationHeader
        computerName={systemInfo?.computer_name}
        osVersion={systemInfo?.os_version}
        isAdmin={systemInfo?.is_admin}
        onRestartAsAdmin={restartAsAdmin}
        onOpenSettings={() => setShowSettings(true)}
        onOpenAbout={() => setShowAbout(true)}
        onExportDiagnostics={exportResults}
        version="2.1.6"
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
          {selectedTab === 'diagnostics' && <DiagnosticsTab />}
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
          {selectedTab === 'issues' && <IssuesTab />}
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
      <ToastProvider>
        <AppContent />
      </ToastProvider>
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