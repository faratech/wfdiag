import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { renderHook } from '@testing-library/react'
import { useGlobalShortcuts } from './useGlobalShortcuts'

const setSelectedTab = vi.fn()
const runQuickScan = vi.fn()
const runFullScan = vi.fn()

vi.mock('../contexts/AppContext', () => ({
  useAppContext: () => ({
    setSelectedTab,
    isRunning: false,
  }),
}))

vi.mock('./useScanner', () => ({
  useScanner: () => ({
    runQuickScan,
    runFullScan,
  }),
}))

describe('useGlobalShortcuts', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    document.body.innerHTML = ''
  })

  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('runs app shortcuts when no overlay is open', () => {
    renderHook(() => useGlobalShortcuts({ onTogglePalette: vi.fn(), onShowHelp: vi.fn() }))

    window.dispatchEvent(new KeyboardEvent('keydown', { key: '2', ctrlKey: true }))

    expect(setSelectedTab).toHaveBeenCalledWith('monitoring')
  })

  it('does not run global shortcuts behind modal dialogs', () => {
    document.body.innerHTML = '<div role="dialog" aria-modal="true"></div>'
    renderHook(() => useGlobalShortcuts({ onTogglePalette: vi.fn(), onShowHelp: vi.fn() }))

    window.dispatchEvent(new KeyboardEvent('keydown', { key: '2', ctrlKey: true }))
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Q', ctrlKey: true, shiftKey: true }))

    expect(setSelectedTab).not.toHaveBeenCalled()
    expect(runQuickScan).not.toHaveBeenCalled()
  })

  it('respects the disabled option from the app shell', () => {
    renderHook(() => useGlobalShortcuts({ onTogglePalette: vi.fn(), onShowHelp: vi.fn(), disabled: true }))

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'F', ctrlKey: true, shiftKey: true }))

    expect(runFullScan).not.toHaveBeenCalled()
  })
})
