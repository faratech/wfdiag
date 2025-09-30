import React, { useEffect } from 'react'
import { SystemMonitoring } from './SystemMonitoring'
import { OpenAIIntegration } from './OpenAIIntegration'
import { ComparisonView } from './ComparisonView'
import { DiagnosticsTab } from './tabs/DiagnosticsTab'
import { IssuesTab } from './tabs/IssuesTab'
import { wfDarkTheme } from './theme'
import {
  NavigationHeader,
  TabNavigation,
  SettingsDialog,
  AboutDialog,
  type SettingsData
} from './components'
import { AppProvider, useAppContext } from './contexts/AppContext'
import { useDiagnostics } from './hooks/useDiagnostics'
import './styles.css'
import {
  FluentProvider,
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
  },
  contentArea: {
    padding: tokens.spacingVerticalXXL,
    height: 'calc(100vh - 180px)',
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
    setSettings,
    issues,
    sessionId,
  } = useAppContext()

  const { exportResults, restartAsAdmin, detectIssues } = useDiagnostics()

  useEffect(() => {
    if (sessionId) {
      detectIssues()
    }
  }, [sessionId, detectIssues])

  const handleSettingsSave = (newSettings: SettingsData) => {
    setSettings(newSettings)
    setShowSettings(false)
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
        version="2.1.1"
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

const App: React.FC = () => {
  return (
    <FluentProvider theme={wfDarkTheme}>
      <AppProvider>
        <AppContent />
      </AppProvider>
    </FluentProvider>
  )
}

export default App