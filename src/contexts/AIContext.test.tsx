import React from 'react'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { renderHook, act, waitFor } from '@testing-library/react'
import { AIProvider, useAIContext, diagnosticCacheKey, type AIResponse } from './AIContext'

const invokeMock = vi.fn()
let appContextValue: {
  settings: Record<string, unknown>
  settingsLoaded: boolean
}

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

vi.mock('./AppContext', () => ({
  useAppContext: () => appContextValue,
}))

const okStatus = {
  preferred_provider: 'openai',
  openai_available: true,
  openai_api_key_set: true,
  phi_silica_available: false,
  phi_silica_ready: false,
  active_provider: 'openai',
}

function aiResponse(interpretation: string): AIResponse {
  return { interpretation, provider_used: 'openai', cached: false }
}

function deferred<T>() {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

function analyzeCalls() {
  return invokeMock.mock.calls.filter(c => c[0] === 'ai_analyze_diagnostic')
}

const wrapper: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <AIProvider>{children}</AIProvider>
)

beforeEach(() => {
  vi.clearAllMocks()
  vi.useRealTimers()
  appContextValue = {
    settings: { openAiApiKey: 'sk-test', preferredAIProvider: 'auto' },
    settingsLoaded: true,
  }
})

afterEach(() => {
  vi.useRealTimers()
})

describe('AIContext analysis dedup and cache', () => {
  it('returns an empty provider list when status fetch fails', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'ai_get_status') throw new Error('status unavailable')
      return undefined
    })

    const { result } = renderHook(() => useAIContext(), { wrapper })

    await waitFor(() => {
      expect(result.current.aiStatus?.active_provider).toBe('none')
      expect(result.current.aiStatus?.providers).toEqual([])
    })
  })

  it('deduplicates concurrent requests for the same content', async () => {
    const pending = deferred<AIResponse>()
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'ai_get_status') return okStatus
      if (cmd === 'ai_analyze_diagnostic') return pending.promise
      return undefined
    })

    const { result } = renderHook(() => useAIContext(), { wrapper })

    let p1!: Promise<string>
    let p2!: Promise<string>
    act(() => {
      p1 = result.current.analyzeDiagnostic('task1', 'Task One', 'output data')
      p2 = result.current.analyzeDiagnostic('task1', 'Task One', 'output data')
    })

    pending.resolve(aiResponse('analysis result'))
    let r1 = ''
    let r2 = ''
    await act(async () => {
      ;[r1, r2] = await Promise.all([p1, p2])
    })

    expect(analyzeCalls()).toHaveLength(1)
    expect(r1).toBe('analysis result')
    expect(r2).toBe('analysis result')
  })

  it('serves repeat requests from the cache without a backend call', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'ai_get_status') return okStatus
      if (cmd === 'ai_analyze_diagnostic') return aiResponse('cached analysis')
      return undefined
    })

    const { result } = renderHook(() => useAIContext(), { wrapper })

    await act(async () => {
      await result.current.analyzeDiagnostic('task1', 'Task One', 'output data')
    })

    // Wait for the interpretation to land in state, then ask again
    await waitFor(() => {
      expect(Object.keys(result.current.interpretations)).toHaveLength(1)
    })

    let second = ''
    await act(async () => {
      second = await result.current.analyzeDiagnostic('task1', 'Task One', 'output data')
    })

    expect(second).toBe('cached analysis')
    expect(analyzeCalls()).toHaveLength(1)
  })

  it('stores results under the exported diagnosticCacheKey (DiagnosticDetail contract)', async () => {
    // Regression: DiagnosticDetail once looked up `diagnostic:${id}` while the
    // context keyed by content hash — the "Interpret does nothing" bug. The
    // exported helper and the internal key must always agree.
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'ai_get_status') return okStatus
      if (cmd === 'ai_analyze_diagnostic') return aiResponse('keyed analysis')
      return undefined
    })

    const { result } = renderHook(() => useAIContext(), { wrapper })

    await act(async () => {
      await result.current.analyzeDiagnostic('os_info', 'OS Info', '{"build": 26100}')
    })

    const key = diagnosticCacheKey('os_info', '{"build": 26100}')
    await waitFor(() => {
      expect(result.current.interpretations[key]).toBe('keyed analysis')
    })
    // And the loading flag uses the same key space
    expect(result.current.isAnalyzing[key]).toBe(false)
  })

  it('re-analyzes when the same task produces different output', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'ai_get_status') return okStatus
      if (cmd === 'ai_analyze_diagnostic') return aiResponse('analysis')
      return undefined
    })

    const { result } = renderHook(() => useAIContext(), { wrapper })

    await act(async () => {
      await result.current.analyzeDiagnostic('task1', 'Task One', 'first output')
    })
    await act(async () => {
      await result.current.analyzeDiagnostic('task1', 'Task One', 'second output')
    })

    // Content hash differs, so the cache must not be reused
    expect(analyzeCalls()).toHaveLength(2)
  })

  it('records the error and clears the loading flag on failure', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'ai_get_status') return okStatus
      if (cmd === 'ai_analyze_diagnostic') throw new Error('rate limited')
      return undefined
    })

    const { result } = renderHook(() => useAIContext(), { wrapper })

    await act(async () => {
      await expect(
        result.current.analyzeDiagnostic('task1', 'Task One', 'output data')
      ).rejects.toThrow('rate limited')
    })

    await waitFor(() => {
      const errorValues = Object.values(result.current.errors)
      expect(errorValues.some(e => e.includes('rate limited'))).toBe(true)
      expect(Object.values(result.current.isAnalyzing).every(v => v === false)).toBe(true)
    })

    // A retry after failure must hit the backend again (errors are not cached)
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'ai_get_status') return okStatus
      if (cmd === 'ai_analyze_diagnostic') return aiResponse('recovered')
      return undefined
    })

    let retried = ''
    await act(async () => {
      retried = await result.current.analyzeDiagnostic('task1', 'Task One', 'output data')
    })
    expect(retried).toBe('recovered')
  })

  it('refreshes provider status when a non-OpenAI provider setting changes', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'ai_get_status') return okStatus
      return undefined
    })

    const { rerender } = renderHook(() => useAIContext(), { wrapper })
    await waitFor(() => expect(invokeMock.mock.calls.filter(c => c[0] === 'ai_get_status')).toHaveLength(2))

    appContextValue = {
      settings: {
        openAiApiKey: 'sk-test',
        preferredAIProvider: 'auto',
        anthropicApiKey: 'sk-ant-new',
      },
      settingsLoaded: true,
    }
    rerender()

    await waitFor(() => {
      expect(invokeMock.mock.calls.filter(c => c[0] === 'ai_get_status').length).toBeGreaterThanOrEqual(3)
    })
  })

  it('clears local interpretations when provider configuration changes', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'ai_get_status') return okStatus
      if (cmd === 'ai_analyze_diagnostic') return aiResponse('old provider answer')
      return undefined
    })

    const { result, rerender } = renderHook(() => useAIContext(), { wrapper })

    await act(async () => {
      await result.current.analyzeDiagnostic('task1', 'Task One', 'output data')
    })

    await waitFor(() => {
      expect(Object.keys(result.current.interpretations)).toHaveLength(1)
    })

    appContextValue = {
      settings: {
        openAiApiKey: 'sk-test',
        preferredAIProvider: 'auto',
        anthropicModel: 'claude-opus-4-8',
      },
      settingsLoaded: true,
    }
    rerender()

    await waitFor(() => {
      expect(Object.keys(result.current.interpretations)).toHaveLength(0)
    })
  })

  it('does not enable AI actions for an explicit unavailable provider just because OpenAI has a key', async () => {
    appContextValue = {
      settings: { openAiApiKey: 'sk-test', preferredAIProvider: 'anthropic' },
      settingsLoaded: true,
    }
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'ai_get_status') return okStatus
      if (cmd === 'ai_analyze_diagnostic') return aiResponse('should not run')
      return undefined
    })

    const { result } = renderHook(() => useAIContext(), { wrapper })

    await waitFor(() => expect(result.current.activeProvider).toBe('none'))
    expect(result.current.isAIAvailable).toBe(false)

    let response = 'not empty'
    await act(async () => {
      response = await result.current.analyzeDiagnostic('task1', 'Task One', 'output data')
    })

    expect(response).toBe('')
    expect(analyzeCalls()).toHaveLength(0)
  })
})
