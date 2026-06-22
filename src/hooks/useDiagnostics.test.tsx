import { describe, it, expect, vi, beforeEach } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import { useDiagnostics } from './useDiagnostics'

const invokeMock = vi.fn()
const saveMock = vi.fn()
const showSuccessMock = vi.fn()
const showErrorMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

vi.mock('@tauri-apps/plugin-dialog', () => ({
  save: (...args: unknown[]) => saveMock(...args),
}))

vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({
  writeText: vi.fn(),
}))

let contextValue: Record<string, unknown>

vi.mock('../contexts/AppContext', () => ({
  useAppContext: () => contextValue,
}))

vi.mock('../contexts/ToastContext', () => ({
  useToast: () => ({ showSuccess: showSuccessMock, showError: showErrorMock }),
}))

function makeContext(exportFormat: 'text' | 'json' | 'html' = 'text') {
  return {
    systemInfo: {
      computer_name: 'PC-A',
      os_version: 'Windows 11',
      is_admin: true,
    },
    setSystemInfo: vi.fn(),
    availableTasks: [],
    setAvailableTasks: vi.fn(),
    sessionId: 'session-1',
    results: {},
    settings: { exportFormat },
    setIssues: vi.fn(),
  }
}

function savedContent(): string {
  const call = invokeMock.mock.calls.find(([cmd]) => cmd === 'save_results_to_file')
  return call?.[1] && typeof call[1] === 'object'
    ? String((call[1] as { content?: unknown }).content)
    : ''
}

beforeEach(() => {
  vi.clearAllMocks()
  contextValue = makeContext()
  saveMock.mockResolvedValue('/tmp/wf-diagnostics.txt')
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'get_system_info') return Promise.resolve(contextValue.systemInfo)
    if (cmd === 'get_available_tasks') return Promise.resolve([])
    if (cmd === 'export_results') return Promise.resolve('backend export payload')
    if (cmd === 'save_results_to_file') return Promise.resolve()
    return Promise.reject(new Error(`unexpected command ${cmd}`))
  })
})

describe('useDiagnostics exportResults', () => {
  it('keeps the report header for text exports', async () => {
    const { result } = renderHook(() => useDiagnostics())

    await act(async () => {
      await result.current.exportResults()
    })

    expect(invokeMock).toHaveBeenCalledWith('export_results', {
      format: 'text',
      includeRaw: true,
    })
    expect(savedContent()).toContain('=== WindowsForum Diagnostic Report ===')
    expect(savedContent()).toContain('backend export payload')
  })

  it('saves JSON exports without adding a text header', async () => {
    contextValue = makeContext('json')
    saveMock.mockResolvedValue('/tmp/wf-diagnostics.json')
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_system_info') return Promise.resolve(contextValue.systemInfo)
      if (cmd === 'get_available_tasks') return Promise.resolve([])
      if (cmd === 'export_results') return Promise.resolve('{"ok":true}')
      if (cmd === 'save_results_to_file') return Promise.resolve()
      return Promise.reject(new Error(`unexpected command ${cmd}`))
    })

    const { result } = renderHook(() => useDiagnostics())

    await act(async () => {
      await result.current.exportResults()
    })

    expect(invokeMock).toHaveBeenCalledWith('export_results', {
      format: 'json',
      includeRaw: true,
    })
    expect(savedContent()).toBe('{"ok":true}')
  })

  it('saves HTML exports without adding a text header', async () => {
    contextValue = makeContext('html')
    saveMock.mockResolvedValue('/tmp/wf-diagnostics.html')
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_system_info') return Promise.resolve(contextValue.systemInfo)
      if (cmd === 'get_available_tasks') return Promise.resolve([])
      if (cmd === 'export_results') return Promise.resolve('<!DOCTYPE html><html></html>')
      if (cmd === 'save_results_to_file') return Promise.resolve()
      return Promise.reject(new Error(`unexpected command ${cmd}`))
    })

    const { result } = renderHook(() => useDiagnostics())

    await act(async () => {
      await result.current.exportResults()
    })

    expect(invokeMock).toHaveBeenCalledWith('export_results', {
      format: 'html',
      includeRaw: true,
    })
    expect(savedContent()).toBe('<!DOCTYPE html><html></html>')
  })
})
