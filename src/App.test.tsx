import React from 'react'
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, waitFor, screen, fireEvent } from '@testing-library/react'
import App from './App'

const mocks = vi.hoisted(() => ({
  runQuickScan: vi.fn(),
  detectIssues: vi.fn(),
  appContextValue: {} as Record<string, unknown>,
}))

vi.mock('./contexts/AppContext', () => ({
  AppProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useAppContext: () => mocks.appContextValue,
}))

vi.mock('./contexts/ThemeContext', () => ({
  ThemeProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useTheme: () => ({ setThemeMode: vi.fn(), isDark: false }),
}))

vi.mock('./contexts/AIContext', () => ({
  AIProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}))

vi.mock('./contexts/ToastContext', () => ({
  ToastProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}))

vi.mock('./hooks/useDiagnostics', () => ({
  useDiagnostics: () => ({
    detectIssues: mocks.detectIssues,
    copyToClipboard: vi.fn(),
    exportResults: vi.fn(),
    shareToWindowsForum: vi.fn(),
    emailReport: vi.fn(),
    generateSupportPackage: vi.fn(),
  }),
}))

vi.mock('./hooks/useScanner', () => ({
  useScanner: () => ({
    runQuickScan: mocks.runQuickScan,
    runFullScan: vi.fn(),
    stopScan: vi.fn(),
  }),
}))

vi.mock('./hooks/useGlobalShortcuts', () => ({ useGlobalShortcuts: vi.fn() }))
vi.mock('./hooks/useMediaQuery', () => ({ useMediaQuery: () => false }))
vi.mock('./hooks/useUpdateCheck', () => ({ useUpdateCheck: () => null }))

vi.mock('./components', () => ({
  SettingsDialog: () => null,
  AboutDialog: () => null,
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  Kbd: ({ children }: { children: React.ReactNode }) => <kbd>{children}</kbd>,
}))
vi.mock('./components/CommandPalette', () => ({ CommandPalette: () => null }))
vi.mock('./components/ShortcutHelp', () => ({ ShortcutHelp: () => null }))
vi.mock('./components/Titlebar', () => ({ Titlebar: () => null }))

vi.mock('./screens/DiagnosticsScreen', () => ({ DiagnosticsScreen: () => <div /> }))
vi.mock('./screens/MonitorScreen', () => ({ MonitorScreen: () => <div /> }))
vi.mock('./screens/ProcessesScreen', () => ({ ProcessesScreen: () => <div /> }))
vi.mock('./screens/IssuesScreen', () => ({ IssuesScreen: () => <div /> }))
vi.mock('./screens/AIScreen', () => ({ AIScreen: () => <div /> }))
vi.mock('./screens/HistoryScreen', () => ({ HistoryScreen: () => <div /> }))

beforeEach(() => {
  vi.clearAllMocks()
  mocks.appContextValue = {
    selectedTab: 'diagnostics',
    setSelectedTab: vi.fn(),
    systemInfo: { is_admin: true },
    availableTasks: [{ id: 'os_info', name: 'OS Info', admin_required: false }],
    results: {},
    sessionId: null,
    isRunning: false,
    currentProgress: 0,
    currentTaskName: '',
    scanStartTime: 0,
    scanEndTime: 0,
    issues: [],
    navRailCollapsed: false,
    setNavRailCollapsed: vi.fn(),
    showSettings: false,
    setShowSettings: vi.fn(),
    showAbout: false,
    setShowAbout: vi.fn(),
    settings: { scanOnStartup: true, theme: 'dark' },
    saveSettings: vi.fn(),
    settingsLoaded: true,
  }
})

describe('App startup scan', () => {
  it('runs one quick scan when scanOnStartup is enabled and tasks are loaded', async () => {
    render(<App />)

    await waitFor(() => {
      expect(mocks.runQuickScan).toHaveBeenCalledTimes(1)
    })
  })
})

describe('App command bar', () => {
  it('switches to Diagnostics when Quick Scan starts from another tab', () => {
    const setSelectedTab = vi.fn()
    mocks.appContextValue = {
      ...mocks.appContextValue,
      selectedTab: 'issues',
      setSelectedTab,
      settings: { scanOnStartup: false, theme: 'dark' },
    }

    render(<App />)
    fireEvent.click(screen.getAllByRole('button', { name: /Quick Scan/i })[0])

    expect(setSelectedTab).toHaveBeenCalledWith('diagnostics')
    expect(mocks.runQuickScan).toHaveBeenCalledTimes(1)
  })

  it('keeps the command-bar theme toggle icon-only', () => {
    mocks.appContextValue = {
      ...mocks.appContextValue,
      settings: { scanOnStartup: false, theme: 'dark' },
    }

    render(<App />)

    const themeButton = screen.getByRole('button', { name: 'Switch to dark theme' })
    expect(themeButton).toHaveClass('icon-only')
    expect(themeButton).not.toHaveTextContent(/Light|Dark/)
  })
})
