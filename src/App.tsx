import React, { useCallback, useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { AppProvider, useAppContext } from './contexts/AppContext'
import { ThemeProvider, useTheme } from './contexts/ThemeContext'
import { AIProvider, useAIContext } from './contexts/AIContext'
import { ToastProvider } from './contexts/ToastContext'
import { AIWorkspaceProvider } from './contexts/AIWorkspaceContext'
import { useDiagnostics } from './hooks/useDiagnostics'
import { useScanner } from './hooks/useScanner'
import { useGlobalShortcuts } from './hooks/useGlobalShortcuts'
import { useMediaQuery } from './hooks/useMediaQuery'
import { useUpdateCheck } from './hooks/useUpdateCheck'
import type { TabValue } from './components'
import { SettingsDialog, AboutDialog, Tooltip } from './components'
import { CommandPalette } from './components/CommandPalette'
import { ShortcutHelp } from './components/ShortcutHelp'
import { Titlebar } from './components/Titlebar'
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
  diagnostics: { title: 'System Analysis', sub: 'Read-only diagnostics across hardware, storage, network and logs' },
  monitoring: { title: 'Live Monitor', sub: 'Real-time CPU, memory, disk, network and NPU telemetry' },
  processes: { title: 'Processes', sub: 'Running processes with live resource usage' },
  ai: { title: 'AI Analysis', sub: 'Ask about this PC or turn the latest scan into a focused health report' },
  issues: { title: 'Issues', sub: 'Problems detected in the latest scan, with one-click fixes' },
  history: { title: 'History', sub: 'Past scans — spot drift and regressions over time' },
}

const APP_VERSION = '2.5.8'

const AppContent: React.FC = () => {
  const {
    selectedTab, setSelectedTab, systemInfo, availableTasks, results, sessionId, isRunning, currentProgress, currentTaskName,
    scanStartTime, scanEndTime, issues, navRailCollapsed, setNavRailCollapsed,
    showSettings, setShowSettings, showAbout, setShowAbout, settings, saveSettings,
  } = useAppContext()
  const { aiStatus, isLoading: aiStatusLoading } = useAIContext()
  const { setThemeMode, isDark } = useTheme()
  const { detectIssues, exportResults, shareToWindowsForum, loadSystemInfo, loadAvailableTasks } = useDiagnostics()
  const { runQuickScan } = useScanner()
  const runQuickScanAndShow = useCallback(() => {
    setSelectedTab('diagnostics')
    void runQuickScan()
  }, [runQuickScan, setSelectedTab])

  const [paletteOpen, setPaletteOpen] = useState(false)
  const [helpOpen, setHelpOpen] = useState(false)
  const updateInfo = useUpdateCheck()
  // Auto-collapse the rail on narrow windows. The user's stored preference
  // (navRailCollapsed in localStorage) is never written by this path, so it
  // restores by itself when the window widens again.
  const forceCollapsed = useMediaQuery('(max-width: 1100px)')
  const railCollapsed = navRailCollapsed || forceCollapsed
  useGlobalShortcuts({
    onTogglePalette: () => setPaletteOpen(o => !o),
    onShowHelp: () => setHelpOpen(true),
    disabled: showSettings || showAbout || paletteOpen || helpOpen,
  })

  // Bootstrap once in the persistent shell. Other screens also use
  // useDiagnostics for actions, so initialization must not live inside the
  // hook itself or every tab mount repeats both native calls.
  useEffect(() => {
    void Promise.all([loadSystemInfo(), loadAvailableTasks()])
  }, [loadAvailableTasks, loadSystemInfo])

  // Re-detect issues once per scan, when its results first arrive. Keying on
  // the session id (not the result COUNT) is essential: the quick-scan task set
  // is fixed, so every scan yields the same count — a count-based guard would
  // refresh issues only on the first scan and then go stale on every re-scan.
  const resultCount = Object.keys(results).length
  const lastDetect = useRef<string | null>(null)
  useEffect(() => {
    if (resultCount > 0 && sessionId && sessionId !== lastDetect.current) {
      lastDetect.current = sessionId
      detectIssues()
    }
  }, [resultCount, sessionId, detectIssues])

  // "Quick Scan" from the tray menu (backend shows the window, then emits)
  const runQuickScanRef = useRef(runQuickScanAndShow)
  const startupScanStartedRef = useRef(false)
  useEffect(() => {
    runQuickScanRef.current = runQuickScanAndShow
  }, [runQuickScanAndShow])
  useEffect(() => {
    if (
      startupScanStartedRef.current ||
      !settings.scanOnStartup ||
      availableTasks.length === 0 ||
      isRunning
    ) {
      return
    }
    startupScanStartedRef.current = true
    runQuickScanRef.current()
  }, [settings.scanOnStartup, availableTasks.length, isRunning])
  useEffect(() => {
    let unlisten: (() => void) | undefined
    let cancelled = false
    listen('tray://quick-scan', () => runQuickScanRef.current())
      .then(fn => {
        // Effect may have cleaned up while listen() was still in flight (e.g.
        // React StrictMode's dev double-invoke) — tear the listener down
        // immediately instead of leaking it via an unlisten ref that's
        // already too late to be returned from cleanup.
        if (cancelled) {
          fn()
        } else {
          unlisten = fn
        }
      })
      .catch(() => {}) // not running under Tauri (plain vite dev)
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  const resultValues = Object.values(results)
  const collected = resultValues.filter(r => r.success).length
  const errors = resultValues.filter(r => !r.success).length
  const hasResults = collected + errors > 0
  const durationMs = scanEndTime > 0 ? scanEndTime - scanStartTime : 0
  const issueCount = issues.filter(i => i.detected).length
  const meta = PAGE_META[selectedTab]

  const setAndSaveTheme = useCallback((mode: 'dark' | 'light' | 'auto') => {
    setThemeMode(mode)
    void saveSettings({ ...settings, theme: mode })
  }, [setThemeMode, saveSettings, settings])

  const toggleTheme = useCallback(() => {
    setAndSaveTheme(isDark ? 'light' : 'dark')
  }, [isDark, setAndSaveTheme])

  return (
    <div className="app-window">
      {/* Wallpaper backdrop — two blurred layers cross-faded on theme change */}
      <div className="wp-layer wp-layer-light" aria-hidden="true" />
      <div className="wp-layer wp-layer-dark" aria-hidden="true" />
      <div className="wp-dim" aria-hidden="true" />

      <Titlebar isDark={isDark} onToggleTheme={toggleTheme} />

      <div className={`app-body ${railCollapsed ? 'rail-collapsed' : ''}`}>
        {/* Nav rail — transparent, on the wallpaper */}
        <nav className="nav-rail" aria-label="Primary">
          <div className="rail-brand">
            <div className="rail-brand-mark"><img src="/wf-ds/app-badge.png" alt="WindowsForum" /></div>
            <div className="rail-brand-text">
              <div className="b1">WindowsForum</div>
              <div className="b2">Diagnostics · {APP_VERSION}</div>
            </div>
          </div>

          <div className="nav-list">
            {TABS.map((t, i) => {
              const btn = (
                <button
                  key={t.id}
                  className={`nav-item ${selectedTab === t.id ? 'active' : ''}`}
                  onClick={() => setSelectedTab(t.id)}
                  aria-label={t.label}
                  aria-current={selectedTab === t.id ? 'page' : undefined}
                >
                  <i className={`fa-solid ${NAV_TAB_ICON[t.id]} item-icon`} aria-hidden="true" />
                  <span className="item-label">{t.label}</span>
                  {t.id === 'issues' && issueCount > 0 && <span className="item-badge">{issueCount}</span>}
                </button>
              )
              return railCollapsed
                ? <Tooltip key={t.id} content={t.label} shortcut={`Ctrl+${i + 1}`} side="right">{btn}</Tooltip>
                : btn
            })}
          </div>

          <div className="rail-section-title">Tools</div>
          <div className="nav-list">
            <button className="nav-item tool" onClick={() => exportResults()} disabled={!hasResults} aria-label="Export Report" title={railCollapsed ? 'Export Report' : undefined}>
              <i className="fa-solid fa-file-export item-icon" aria-hidden="true" />
              <span className="item-label">Export Report</span>
            </button>
            <button className="nav-item tool" onClick={() => shareToWindowsForum()} disabled={!hasResults} aria-label="Share to Forum" title={railCollapsed ? 'Share to Forum' : undefined}>
              <i className="fa-solid fa-share-nodes item-icon" aria-hidden="true" />
              <span className="item-label">Share to Forum</span>
            </button>
          </div>

          <div className="rail-footer">
            <button className="nav-item tool" onClick={() => setShowSettings(true)} aria-label="Settings" title={railCollapsed ? 'Settings' : undefined}><i className="fa-solid fa-gear item-icon" aria-hidden="true" /><span className="item-label">Settings</span></button>
            <button className="nav-item tool" onClick={() => setShowAbout(true)} aria-label="About" title={railCollapsed ? 'About' : undefined}><i className="fa-solid fa-circle-info item-icon" aria-hidden="true" /><span className="item-label">About</span></button>
            {/* Hidden while the narrow window forces a collapse — toggling a
                preference with no visible effect would just confuse */}
            {!forceCollapsed && (
              <button className="nav-item tool" onClick={() => setNavRailCollapsed(!navRailCollapsed)} aria-label={navRailCollapsed ? 'Expand navigation' : 'Collapse navigation'}>
                <i className={`fa-solid ${navRailCollapsed ? 'fa-angles-right' : 'fa-angles-left'} item-icon`} aria-hidden="true" />
                <span className="item-label">Collapse</span>
              </button>
            )}
            <div className="sysinfo">
              <div className="si-row"><i className="fa-solid fa-desktop" /><strong>{systemInfo?.computer_name || '—'}</strong></div>
              <div className="si-row"><i className="fa-brands fa-windows" /><span>{systemInfo?.os_version || 'Windows'}</span></div>
              <div className="si-row"><i className="fa-solid fa-user-shield" /><span>{systemInfo?.is_admin ? 'Administrator' : 'Standard user'}</span></div>
            </div>
          </div>
        </nav>

        {/* Content panel — the single acrylic surface */}
        <main className="content-area">
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

          <div aria-busy={isRunning} style={{ display: 'contents' }}>
          {selectedTab === 'diagnostics' && <DiagnosticsScreen />}
          {selectedTab === 'monitoring' && <MonitorScreen />}
          {selectedTab === 'processes' && <ProcessesScreen />}
          {selectedTab === 'ai' && <AIScreen />}
          {selectedTab === 'issues' && <IssuesScreen />}
          {selectedTab === 'history' && <HistoryScreen />}
          </div>

          {/* Status bar — inside the panel */}
          <div className="status-bar" role="status">
            <div className="sb-left">
              {isRunning ? (
                <><i className="fa-solid fa-circle-notch fa-spin ico-accent" aria-hidden="true" /><span>Running: {currentTaskName || '…'}</span></>
              ) : hasResults ? (
                <><i className={`fa-solid ${errors > 0 ? 'fa-triangle-exclamation ico-warn' : 'fa-circle-check ico-ok'}`} aria-hidden="true" /><span>{collected} collected · {errors} errors</span></>
              ) : (
                <><i className="fa-solid fa-circle-info" aria-hidden="true" /><span>Ready — no scan data</span></>
              )}
            </div>
            <div className="sb-spacer" />
            <div className="sb-right">
              <span>{durationMs > 0 ? `${(durationMs / 1000).toFixed(1)}s · ` : ''}{systemInfo?.is_admin ? 'Administrator' : 'Standard user'}</span>
              <span className="sb-brand">wfdiag {APP_VERSION} · WindowsForum.com</span>
            </div>
          </div>
        </main>
      </div>

      <SettingsDialog
        open={showSettings}
        onOpenChange={setShowSettings}
        settings={settings}
        aiStatus={aiStatus}
        aiStatusLoading={aiStatusLoading}
        onSave={async (s) => { await saveSettings(s); setThemeMode((s.theme as 'dark' | 'light' | 'auto') || 'dark'); setShowSettings(false) }}
      />
      <AboutDialog open={showAbout} onOpenChange={setShowAbout} updateInfo={updateInfo} />
      <CommandPalette open={paletteOpen} onClose={() => setPaletteOpen(false)} />
      <ShortcutHelp open={helpOpen} onClose={() => setHelpOpen(false)} />
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
          <AIWorkspaceProvider>
            <AppContent />
          </AIWorkspaceProvider>
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
