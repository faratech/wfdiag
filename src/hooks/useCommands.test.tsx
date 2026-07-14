import React from 'react'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { renderHook } from '@testing-library/react'
import { ThemeProvider } from '../contexts/ThemeContext'
import { useCommands } from './useCommands'

const saveSettings = vi.fn()

vi.mock('../contexts/AppContext', () => ({
  useAppContext: () => ({
    setSelectedTab: vi.fn(),
    availableTasks: [],
    results: {},
    isRunning: false,
    systemInfo: { is_admin: true },
    setShowSettings: vi.fn(),
    setShowAbout: vi.fn(),
    navRailCollapsed: false,
    setNavRailCollapsed: vi.fn(),
    setSelectedDiagnosticId: vi.fn(),
    settings: { theme: 'auto' },
    saveSettings,
  }),
}))

vi.mock('./useScanner', () => ({
  useScanner: () => ({
    rerunDiagnostic: vi.fn(),
    runQuickScan: vi.fn(),
    runFullScan: vi.fn(),
    stopScan: vi.fn(),
  }),
}))

vi.mock('./useDiagnostics', () => ({
  useDiagnostics: () => ({
    copyToClipboard: vi.fn(),
    exportResults: vi.fn(),
    shareToWindowsForum: vi.fn(),
    emailReport: vi.fn(),
    generateSupportPackage: vi.fn(),
  }),
}))

beforeEach(() => {
  vi.clearAllMocks()
  vi.stubGlobal('matchMedia', vi.fn().mockImplementation(() => ({
    matches: true,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  })))
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('useCommands theme command', () => {
  it('uses the effective system dark theme when mode is Auto', () => {
    const wrapper: React.FC<{ children: React.ReactNode }> = ({ children }) => (
      <ThemeProvider initialMode="auto">{children}</ThemeProvider>
    )

    const { result } = renderHook(() => useCommands(), { wrapper })
    const themeCommand = result.current.find(command => command.id === 'app:theme')

    expect(themeCommand?.title).toBe('Switch to Light Theme')
    expect(themeCommand?.icon).toBe('fa-sun')
  })
})
