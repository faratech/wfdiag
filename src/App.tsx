import React, { useEffect, useRef } from 'react'
import { AppProvider, useAppContext } from './contexts/AppContext'
import { ThemeProvider, useTheme } from './contexts/ThemeContext'
import { AIProvider } from './contexts/AIContext'
import { ToastProvider } from './contexts/ToastContext'
import { useDiagnostics } from './hooks/useDiagnostics'
import { useScanner } from './hooks/useScanner'
import type { TabValue } from './components'
import { SettingsDialog, AboutDialog } from './components'
import { NAV_TAB_ICON } from './ui/diagnostic-icons'
import { DiagnosticsScreen } from './screens/DiagnosticsScreen'
import { MonitorScreen } from './screens/MonitorScreen'
import { ProcessesScreen } from './screens/ProcessesScreen'
import { IssuesScreen } from './screens/IssuesScreen'
import { AIScreen } from './screens/AIScreen'
import { HistoryScreen } from './screens/HistoryScreen'

const TABS: { id: TabValue; label: string }[] = [
  { id: 'diagnostics', label: 'Diagnostics' },
  { id: 'monitoring', label: 'Live Monitor' },
  { id: 'processes', label: 'Processes' },
  { id: 'ai', label: 'AI Analysis' },
  { id: 'issues', label: 'Issues' },
  { id: 'history', label: 'History' },
]

const PAGE_META: Record<TabValue, { title: string; sub: string }> = {
  diagnostics: { title: 'System Analysis', sub: 'Comprehensive diagnostic engine across hardware, drivers, storage, network and logs' },
  monitoring: { title: 'Live Monitor', sub: 'Real-time CPU, memory, disk, network and NPU telemetry' },
  processes: { title: 'Running Processes', sub: 'Live process explorer with per-process resource tracking' },
  ai: { title: 'AI Analysis', sub: 'Interpret diagnostics on-device or with a configured cloud model' },
  issues: { title: 'Detected Issues', sub: 'Problems found in the most recent scan with remediation guidance' },
  history: { title: 'Scan History', sub: 'Compare past scans to spot drift and regressions' },
}

const AppContent: React.FC = () => {
  const {
    selectedTab, setSelectedTab, systemInfo, results, isRunning, currentProgress, currentTaskName,
    scanStartTime, scanEndTime, issues, navRailCollapsed, setNavRailCollapsed,
    showSettings, setShowSettings, showAbout, setShowAbout, settings, saveSettings,
  } = useAppContext()
  const { themeMode, setThemeMode } = useTheme()
  const { detectIssues, copyToClipboard, exportResults, shareToWindowsForum, emailReport, generateSupportPackage } = useDiagnostics()
  const { runQuickScan, runFullScan, stopScan } = useScanner()

  // Detect issues whenever results change to a non-empty set
  const resultCount = Object.keys(results).length
  const lastDetect = useRef(0)
  useEffect(() => {
    if (resultCount > 0 && resultCount !== lastDetect.current) {
      lastDetect.current = resultCount
      detectIssues()
    }
  }, [resultCount, detectIssues])

  const resultValues = Object.values(results)
  const passed = resultValues.filter(r => r.success).length
  const failed = resultValues.filter(r => !r.success).length
  const hasResults = passed + failed > 0
  const durationMs = scanEndTime > 0 ? scanEndTime - scanStartTime : 0
  const issueCount = issues.filter(i => i.detected).length
  const meta = PAGE_META[selectedTab]
  const isDark = themeMode === 'dark'

  return (
    <div className="app-window">
      <div className={`app-body ${navRailCollapsed ? 'rail-collapsed' : ''}`}>
        {/* Nav rail */}
        <nav className="nav-rail">
          <div className="rail-brand">
            <div className="rail-brand-mark"><img src="/wf-ds/icon-only.png" alt="" /></div>
            <div className="rail-brand-text">
              <div className="b1">WindowsForum</div>
              <div className="b2">Diagnostics · 2.2.0</div>
            </div>
          </div>

          <div className="rail-section-title">Workspace</div>
          <div className="nav-list">
            {TABS.map(t => (
              <button key={t.id} className={`nav-item ${selectedTab === t.id ? 'active' : ''}`} onClick={() => setSelectedTab(t.id)} title={t.label}>
                <i className={`fa-solid ${NAV_TAB_ICON[t.id]} item-icon`} />
                <span className="item-label">{t.label}</span>
                {t.id === 'issues' && issueCount > 0 && <span className="item-badge">{issueCount}</span>}
              </button>
            ))}
          </div>

          <div className="rail-section-title">Tools</div>
          <div className="nav-list">
            <button className="nav-item" onClick={() => runQuickScan()} disabled={isRunning}>
              <i className="fa-solid fa-bolt item-icon" />
              <span className="item-label">Quick Scan</span>
            </button>
            <button className="nav-item" onClick={() => generateSupportPackage()} disabled={!hasResults}>
              <i className="fa-solid fa-box-archive item-icon" />
              <span className="item-label">Support Package</span>
            </button>
            <button className="nav-item" onClick={() => shareToWindowsForum()} disabled={!hasResults}>
              <i className="fa-solid fa-share-nodes item-icon" />
              <span className="item-label">Share to Forum</span>
            </button>
          </div>

          <div className="rail-footer">
            <div className="sysinfo">
              <div className="si-row"><i className="fa-solid fa-desktop" /><strong>{systemInfo?.computer_name || '—'}</strong></div>
              <div className="si-row"><i className="fa-brands fa-windows" />{systemInfo?.os_version || 'Windows'}</div>
              <div className="si-row"><i className="fa-solid fa-user-shield" />{systemInfo?.is_admin ? 'Administrator' : 'Standard user'}</div>
            </div>
            <button className="nav-item" onClick={() => setShowSettings(true)}><i className="fa-solid fa-gear item-icon" /><span className="item-label">Settings</span></button>
            <button className="nav-item" onClick={() => setShowAbout(true)}><i className="fa-solid fa-circle-info item-icon" /><span className="item-label">About</span></button>
            <button className="nav-item" onClick={() => setNavRailCollapsed(!navRailCollapsed)}>
              <i className={`fa-solid ${navRailCollapsed ? 'fa-angles-right' : 'fa-angles-left'} item-icon`} />
              <span className="item-label">Collapse</span>
            </button>
          </div>
        </nav>

        {/* Content */}
        <main className="content-area">
          <div className="command-bar">
            <div className="cb-group">
              <button className="cb-btn primary" onClick={() => runQuickScan()} disabled={isRunning}>
                <i className="fa-solid fa-bolt" /> Quick Scan
              </button>
              <button className="cb-btn" onClick={() => runFullScan()} disabled={isRunning}>
                <i className="fa-solid fa-list-check" /> Full Scan
              </button>
              {isRunning && (
                <button className="cb-btn danger" onClick={() => stopScan()}><i className="fa-solid fa-stop" /> Stop</button>
              )}
            </div>
            <div className="cb-divider" />
            <div className="cb-group">
              <button className="cb-btn" onClick={() => copyToClipboard()} disabled={!hasResults}><i className="fa-solid fa-copy" /> Copy</button>
              <button className="cb-btn" onClick={() => exportResults()} disabled={!hasResults}><i className="fa-solid fa-file-export" /> Export</button>
              <button className="cb-btn" onClick={() => shareToWindowsForum()} disabled={!hasResults}><i className="fa-solid fa-share-nodes" /> Share</button>
              <button className="cb-btn" onClick={() => emailReport()} disabled={!hasResults}><i className="fa-solid fa-envelope" /> Email</button>
            </div>
            <div className="cb-divider" />
            <div className="cb-group">
              <button className="cb-btn" onClick={() => setSelectedTab('history')}><i className="fa-solid fa-code-compare" /> Compare</button>
            </div>
            <div className="cb-spacer" />
            <div className="cb-group">
              <button className="cb-btn" onClick={() => setThemeMode(isDark ? 'light' : 'dark')}>
                <i className={`fa-solid ${isDark ? 'fa-sun' : 'fa-moon'}`} />
                {isDark ? 'Light' : 'Dark'}
              </button>
            </div>
          </div>

          <div className="page-header">
            <div>
              <h1>{meta.title}</h1>
              <p className="sub">{meta.sub}</p>
            </div>
            <div className="ph-actions">
              {isRunning && (
                <span className="tag info"><i className="fa-solid fa-circle-notch fa-spin" /> Scanning · {Math.round(currentProgress)}%</span>
              )}
              {!isRunning && hasResults && selectedTab === 'diagnostics' && (
                <span className="tag success"><i className="fa-solid fa-circle-check" /> Scan complete</span>
              )}
            </div>
          </div>

          {selectedTab === 'diagnostics' && <DiagnosticsScreen />}
          {selectedTab === 'monitoring' && <MonitorScreen />}
          {selectedTab === 'processes' && <ProcessesScreen />}
          {selectedTab === 'ai' && <AIScreen />}
          {selectedTab === 'issues' && <IssuesScreen />}
          {selectedTab === 'history' && <HistoryScreen />}
        </main>

        {/* Status bar */}
        <div className={`status-bar ${isRunning ? 'scanning' : ''}`}>
          <div className="sb-segments">
            {isRunning ? (
              <div className="sb-seg"><i className="fa-solid fa-circle-notch fa-spin" /> Running: {currentTaskName || '…'}</div>
            ) : (
              <>
                <div className="sb-seg ok"><i className="fa-solid fa-circle-check" /> <strong>{passed}</strong> passed</div>
                {failed > 0 && <div className="sb-seg fail"><i className="fa-solid fa-circle-xmark" /> <strong>{failed}</strong> failed</div>}
              </>
            )}
          </div>
          <div className="sb-segments sb-right">
            {durationMs > 0 && <div className="sb-seg"><i className="fa-solid fa-stopwatch" /> <strong>{(durationMs / 1000).toFixed(1)}s</strong></div>}
            <div className="sb-seg"><i className="fa-solid fa-user" /> {systemInfo?.is_admin ? 'Administrator' : 'Standard user'}</div>
          </div>
        </div>
      </div>

      <SettingsDialog
        open={showSettings}
        onOpenChange={setShowSettings}
        settings={settings}
        onSave={async (s) => { await saveSettings(s); setShowSettings(false) }}
      />
      <AboutDialog open={showAbout} onOpenChange={setShowAbout} />
    </div>
  )
}

const ThemedApp: React.FC = () => {
  const { settings, settingsLoaded } = useAppContext()
  if (!settingsLoaded) return null
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

const App: React.FC = () => (
  <AppProvider>
    <ThemedApp />
  </AppProvider>
)

export default App
