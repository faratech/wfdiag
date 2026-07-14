import React, { createContext, useContext, useState, useEffect, useCallback, ReactNode, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useAppContext } from './AppContext'
import * as logger from '../utils/logger'

// Stable, fast (djb2) hash of the analyzed content. Folded into AI cache keys so that a
// re-scan producing different output for the same task/section yields a NEW key and is
// re-analyzed, instead of returning the previous scan's stale interpretation.
function hashContent(input: string): string {
  let hash = 5381
  for (let i = 0; i < input.length; i++) {
    hash = ((hash << 5) + hash + input.charCodeAt(i)) | 0
  }
  return (hash >>> 0).toString(36)
}

// The ONLY way to build keys into interpretations/isAnalyzing/errors. Components
// must use these instead of hand-writing `diagnostic:${id}` — a hand-written key
// without the content hash silently never matches (the "Interpret this diagnostic
// does nothing" bug).
export const diagnosticCacheKey = (taskId: string, output: string) =>
  `diagnostic:${taskId}:${hashContent(output)}`
export const sectionCacheKey = (sectionName: string, sectionData: string) =>
  `section:${sectionName}:${hashContent(sectionData)}`

// Types matching the Rust backend (wire strings pinned by backend tests)
export type AIProvider =
  | 'none'
  | 'openai'
  | 'phi_silica'
  | 'foundry_local'
  | 'ollama'
  | 'custom_openai'
  | 'codex_cli'
  | 'claude_code'
  | 'anthropic'
  | 'gemini'
  | 'deepseek'
export type AIProviderPreference = 'auto' | Exclude<AIProvider, 'none'>

export interface ProviderInfo {
  id: Exclude<AIProvider, 'none'>
  available: boolean
  configured: boolean
  model?: string
  endpoint?: string
  supports_tools: boolean
  supports_streaming: boolean
}

export interface AIProviderStatus {
  preferred_provider: AIProvider
  openai_available: boolean
  openai_api_key_set: boolean
  phi_silica_available: boolean
  phi_silica_ready: boolean
  phi_silica_message?: string
  foundry_local_available?: boolean
  foundry_local_endpoint?: string
  active_provider: AIProvider
  /** One row per real provider, in Auto routing order (2.5.0+) */
  providers?: ProviderInfo[]
}

export interface GroundingTraceSource {
  source: string
  title: string
  url?: string
}

export interface GroundingTrace {
  enabled: boolean
  query: string
  source_count: number
  sources: GroundingTraceSource[]
  error?: string
}

export interface AIAnalysisMeta {
  provider_used: AIProvider
  cached: boolean
  grounding?: GroundingTrace
}

export interface AIResponse {
  interpretation: string
  provider_used: AIProvider
  cached: boolean
  grounding?: GroundingTrace
  error?: string
}

interface AIContextType {
  // Status
  aiStatus: AIProviderStatus | null
  isAIAvailable: boolean
  activeProvider: AIProvider
  isLoading: boolean

  // Settings
  aiEnabled: boolean
  setAIEnabled: (enabled: boolean) => void
  preferredProvider: AIProviderPreference
  setPreferredProvider: (provider: AIProviderPreference) => void

  // Analysis functions (on-demand)
  analyzeDiagnostic: (taskId: string, taskName: string, output: string, forceRefresh?: boolean) => Promise<string>
  analyzeSection: (sectionName: string, sectionData: string) => Promise<string>
  explainHealth: (metricsData: string) => Promise<string>
  analyzeGeneric: (cacheKey: string, prompt: string, forceRefresh?: boolean) => Promise<string>
  prioritizeIssues: (issuesJson: string, forceRefresh?: boolean) => Promise<string>

  // Loading states per context_id
  isAnalyzing: Record<string, boolean>
  interpretations: Record<string, string>
  analysisMeta: Record<string, AIAnalysisMeta>
  errors: Record<string, string>

  // Cache management
  clearCache: (sessionId?: string) => Promise<void>

  // Session management
  sessionId: string
  setSessionId: (id: string) => void

  // Refresh status
  refreshStatus: () => Promise<void>
}

const AIContext = createContext<AIContextType | undefined>(undefined)

export const useAIContext = () => {
  const context = useContext(AIContext)
  if (!context) {
    throw new Error('useAIContext must be used within an AIProvider')
  }
  return context
}

interface AIProviderProps {
  children: ReactNode
}

export const AIProvider: React.FC<AIProviderProps> = ({ children }) => {
  // Get settings from AppContext
  const { settings, settingsLoaded } = useAppContext()

  // State
  const [aiStatus, setAiStatus] = useState<AIProviderStatus | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [sessionId, setSessionId] = useState(() => `session_${Date.now()}`)

  // Analysis state
  const [isAnalyzing, setIsAnalyzing] = useState<Record<string, boolean>>({})
  const [interpretations, setInterpretations] = useState<Record<string, string>>({})
  const [analysisMeta, setAnalysisMeta] = useState<Record<string, AIAnalysisMeta>>({})
  const [errors, setErrors] = useState<Record<string, string>>({})

  // Track previous settings to detect changes
  const prevSettingsRef = useRef<{ availabilityKey?: string; provider?: string }>({})
  const analysisGenerationRef = useRef(0)
  // Provider changes and status reads form one ordered control-plane
  // transaction. A slow status response for the old provider must never
  // overwrite a newer selection, and a status read must not race ahead of the
  // backend preference update it is meant to describe.
  const statusRequestGenerationRef = useRef(0)
  const preferenceSyncRef = useRef<Promise<void>>(Promise.resolve())

  // Use settings from AppContext (with defaults)
  const aiEnabled = settings.aiEnabled ?? true
  const preferredProvider: AIProviderPreference = (settings.preferredAIProvider as AIProviderPreference) ?? 'auto'

  // Derived state used by the frontend mirror below. The OpenAI key in settings
  // counts while status is refreshing because one-shot calls pass it to the backend.
  const backendAvailable = !!aiStatus && aiStatus.active_provider !== 'none'
  const hasSettingsApiKey = !!settings.openAiApiKey || !!settings.openAiApiKeySet

  // Determine the active provider — a frontend mirror of the backend's
  // route_provider(): explicit preference never falls back; Auto walks the
  // local-first chain. The settings OpenAI key counts as OpenAI availability
  // (it is sent with every analyze call), which is why this can't just read
  // aiStatus.active_provider.
  const activeProvider: AIProvider = (() => {
    const available = (id: Exclude<AIProvider, 'none'>): boolean => {
      if (id === 'openai' && hasSettingsApiKey) return true
      const row = aiStatus?.providers?.find(p => p.id === id)
      if (row) return row.available
      // Legacy fields (status from a pre-2.5 backend during dev reloads)
      if (id === 'phi_silica') return !!aiStatus?.phi_silica_available
      if (id === 'foundry_local') return !!aiStatus?.foundry_local_available
      if (id === 'openai') return !!aiStatus?.openai_available
      return false
    }
    if (preferredProvider !== 'auto') {
      return available(preferredProvider) ? preferredProvider : 'none'
    }
    const autoOrder: Exclude<AIProvider, 'none'>[] = [
      'phi_silica', 'foundry_local', 'ollama', 'custom_openai', 'codex_cli', 'claude_code', 'openai', 'anthropic', 'gemini', 'deepseek',
    ]
    for (const id of autoOrder) {
      if (available(id)) return id
    }
    return backendAvailable ? aiStatus!.active_provider : 'none'
  })()
  const isAIAvailable = activeProvider !== 'none'

  // Pure status fetch: returns a safe fallback on error and never calls
  // setState. The generation-guarded callers below decide whether a response
  // is still current before publishing it.
  const loadStatus = useCallback(async (): Promise<AIProviderStatus> => {
    try {
      const status = await invoke<AIProviderStatus>('ai_get_status')
      logger.debug('AIContext', 'AI status refreshed', status)
      return status
    } catch (error) {
      logger.error('AIContext', 'Failed to get AI status', String(error))
      return {
        preferred_provider: 'none',
        openai_available: false,
        openai_api_key_set: false,
        phi_silica_available: false,
        phi_silica_ready: false,
        active_provider: 'none',
        providers: [],
      }
    }
  }, [])

  // User-initiated refreshes join any provider update already in flight and
  // supersede older status requests. Only the newest request may publish.
  const refreshStatus = useCallback(async () => {
    const generation = ++statusRequestGenerationRef.current
    setIsLoading(true)
    await preferenceSyncRef.current
    if (generation !== statusRequestGenerationRef.current) return
    const status = await loadStatus()
    if (generation !== statusRequestGenerationRef.current) return
    setAiStatus(status)
    setIsLoading(false)
  }, [loadStatus])

  // Refresh status when relevant settings change
  useEffect(() => {
    if (!settingsLoaded) return

    const availabilityKey = [
      settings.openAiApiKey || '',
      settings.openAiApiKeySet ? '1' : '',
      settings.openAiModel || '',
      settings.anthropicApiKey || '',
      settings.anthropicApiKeySet ? '1' : '',
      settings.anthropicModel || '',
      settings.geminiApiKey || '',
      settings.geminiApiKeySet ? '1' : '',
      settings.geminiModel || '',
      settings.deepseekApiKey || '',
      settings.deepseekApiKeySet ? '1' : '',
      settings.deepseekModel || '',
      settings.customApiKey || '',
      settings.customApiKeySet ? '1' : '',
      settings.customEndpoint || '',
      settings.customModel || '',
      settings.ollamaEndpoint || '',
      settings.ollamaModel || '',
      settings.codexCliPath || '',
      settings.codexModel || '',
      settings.claudeCliPath || '',
      settings.claudeModel || '',
      settings.localAiEndpoint || '',
      settings.localAiModel || '',
      settings.phiSilicaLafToken || '',
      settings.networkGroundingEnabled ? '1' : '0',
      settings.cloudFallbackPolicy || 'ask',
    ].join('\u0001')
    const currentProvider = settings.preferredAIProvider || 'auto'
    const prevAvailabilityKey = prevSettingsRef.current.availabilityKey || ''
    const prevProvider = prevSettingsRef.current.provider || 'auto'

    // Check if any setting that can affect provider availability or routing changed
    if (availabilityKey !== prevAvailabilityKey || currentProvider !== prevProvider) {
      logger.debug('AIContext', 'AI settings changed, refreshing status')
      prevSettingsRef.current = { availabilityKey, provider: currentProvider }
      const generation = ++statusRequestGenerationRef.current
      let cancelled = false

      // Do this in the same effect turn as the settings change. Clearing the
      // old snapshot prevents a newly selected provider from being authorized
      // by availability data that belonged to the previous selection.
      setIsLoading(true)
      setAiStatus(null)
      if (prevAvailabilityKey || currentProvider !== prevProvider) {
        analysisGenerationRef.current++
        setIsAnalyzing({})
        setInterpretations({})
        setAnalysisMeta({})
        setErrors({})
        inFlightRef.current.clear()
      }

      // Serialize preference writes. Rapid A -> B -> C changes must reach the
      // backend in that order, and the status fetch for C waits for the whole
      // chain before asking which provider is active.
      if (currentProvider !== prevProvider) {
        preferenceSyncRef.current = preferenceSyncRef.current.then(async () => {
          try {
            await invoke('ai_set_preference', { preference: currentProvider })
          } catch (error) {
            logger.error('AIContext', 'Failed to set AI preference', String(error))
            if (!cancelled && generation === statusRequestGenerationRef.current) {
              const errorMsg = error instanceof Error ? error.message : String(error)
              setErrors((prev) => ({
                ...prev,
                'preference:update': `Failed to update AI preference: ${errorMsg}`,
              }))
            }
          }
        })
      }

      const preferenceSync = preferenceSyncRef.current
      void (async () => {
        await preferenceSync
        if (cancelled || generation !== statusRequestGenerationRef.current) return
        const status = await loadStatus()
        if (cancelled || generation !== statusRequestGenerationRef.current) return
        setAiStatus(status)
        setIsLoading(false)
      })()

      return () => { cancelled = true }
    }
  }, [
    settings.openAiApiKey,
    settings.openAiApiKeySet,
    settings.openAiModel,
    settings.anthropicApiKey,
    settings.anthropicApiKeySet,
    settings.anthropicModel,
    settings.geminiApiKey,
    settings.geminiApiKeySet,
    settings.geminiModel,
    settings.deepseekApiKey,
    settings.deepseekApiKeySet,
    settings.deepseekModel,
    settings.customApiKey,
    settings.customApiKeySet,
    settings.customEndpoint,
    settings.customModel,
    settings.ollamaEndpoint,
    settings.ollamaModel,
    settings.codexCliPath,
    settings.codexModel,
    settings.claudeCliPath,
    settings.claudeModel,
    settings.localAiEndpoint,
    settings.localAiModel,
    settings.phiSilicaLafToken,
    settings.networkGroundingEnabled,
    settings.cloudFallbackPolicy,
    settings.preferredAIProvider,
    settingsLoaded,
    loadStatus,
  ])

  // Dummy setters for backwards compatibility (settings are managed via AppContext now)
  const setAiEnabled = useCallback((_enabled: boolean) => {
    logger.warn('AIContext', 'setAiEnabled is deprecated - use Settings dialog to change AI settings')
  }, [])

  const setPreferredProvider = useCallback(async (_provider: AIProviderPreference) => {
    logger.warn('AIContext', 'setPreferredProvider is deprecated - use Settings dialog to change AI settings')
  }, [])

  // In-flight analysis requests keyed by cacheKey. Concurrent calls for the
  // same content (e.g. a double-clicked Analyze button or two components
  // requesting the same diagnostic) await one backend request instead of
  // firing duplicates.
  const inFlightRef = useRef<Map<string, Promise<string>>>(new Map())

  // Shared body of all analysis functions: dedup against in-flight requests,
  // manage loading/error state, store the interpretation on success.
  const runDedupedAnalysis = useCallback((
    cacheKey: string,
    doInvoke: () => Promise<AIResponse>
  ): Promise<string> => {
    const inFlight = inFlightRef.current.get(cacheKey)
    if (inFlight) {
      return inFlight
    }

    setIsAnalyzing(prev => ({ ...prev, [cacheKey]: true }))
    setErrors(prev => {
      const { [cacheKey]: _, ...rest } = prev
      return rest
    })

    const generation = analysisGenerationRef.current
    // Boxed so the closure below can identify "am I still the current
    // in-flight entry" without TypeScript flagging a self-referential
    // `const request` as used-before-assigned — `self.current` is set
    // synchronously right after the IIFE call, always before the first
    // `await` inside it actually suspends.
    const self: { current?: Promise<string> } = {}
    const request = (async () => {
      try {
        const response = await doInvoke()
        const interpretation = response.interpretation
        if (generation !== analysisGenerationRef.current) {
          return ''
        }
        setInterpretations(prev => ({ ...prev, [cacheKey]: interpretation }))
        setAnalysisMeta(prev => ({
          ...prev,
          [cacheKey]: {
            provider_used: response.provider_used,
            cached: response.cached,
            grounding: response.grounding,
          },
        }))
        return interpretation
      } catch (error) {
        const errorMsg = error instanceof Error ? error.message : String(error)
        if (generation === analysisGenerationRef.current) {
          setErrors(prev => ({ ...prev, [cacheKey]: errorMsg }))
        }
        throw error
      } finally {
        // Only remove OUR OWN entry: if a settings change cleared the map
        // and a newer request already registered under the same cacheKey
        // by the time this (now-stale) request settles, deleting
        // unconditionally would evict that newer request's in-flight entry
        // and let a third caller bypass dedup.
        if (inFlightRef.current.get(cacheKey) === self.current) {
          inFlightRef.current.delete(cacheKey)
        }
        if (generation === analysisGenerationRef.current) {
          setIsAnalyzing(prev => ({ ...prev, [cacheKey]: false }))
        }
      }
    })()
    self.current = request

    inFlightRef.current.set(cacheKey, request)
    return request
  }, [])

  // Analysis functions
  const analyzeDiagnostic = useCallback(async (
    taskId: string,
    taskName: string,
    output: string,
    forceRefresh = false
  ): Promise<string> => {
    // Allow if AI is enabled and either backend says available OR we have an API key
    if (!aiEnabled || !isAIAvailable) {
      return ''
    }

    // Check if already cached (key includes a hash of the output so a re-scan with
    // different data for the same task does not return a stale interpretation).
    const cacheKey = diagnosticCacheKey(taskId, output)
    if (!forceRefresh && interpretations[cacheKey]) {
      return interpretations[cacheKey]
    }

    return runDedupedAnalysis(cacheKey, () =>
      invoke<AIResponse>('ai_analyze_diagnostic', {
        taskId,
        taskName,
        diagnosticOutput: output,
        sessionId,
        forceRefresh
      })
    )
  }, [aiEnabled, isAIAvailable, interpretations, sessionId, runDedupedAnalysis])

  const analyzeSection = useCallback(async (
    sectionName: string,
    sectionData: string
  ): Promise<string> => {
    if (!aiEnabled || !isAIAvailable) {
      return ''
    }

    const cacheKey = sectionCacheKey(sectionName, sectionData)
    if (interpretations[cacheKey]) {
      return interpretations[cacheKey]
    }

    return runDedupedAnalysis(cacheKey, () =>
      invoke<AIResponse>('ai_analyze_section', {
        sectionName,
        sectionData,
        sessionId,
        forceRefresh: false
      })
    )
  }, [aiEnabled, isAIAvailable, interpretations, sessionId, runDedupedAnalysis])

  const explainHealth = useCallback(async (metricsData: string): Promise<string> => {
    if (!aiEnabled || !isAIAvailable) {
      return ''
    }

    const cacheKey = `health:${hashContent(metricsData)}`
    if (interpretations[cacheKey]) {
      return interpretations[cacheKey]
    }

    return runDedupedAnalysis(cacheKey, () =>
      invoke<AIResponse>('ai_explain_health', {
        metricsData,
        sessionId,
        forceRefresh: false
      })
    )
  }, [aiEnabled, isAIAvailable, interpretations, sessionId, runDedupedAnalysis])

  // Generic analysis function for monitoring, processes, comparisons, etc.
  const analyzeGeneric = useCallback(async (
    cacheKey: string,
    prompt: string,
    forceRefresh = false
  ): Promise<string> => {
    if (!aiEnabled || !isAIAvailable) {
      return ''
    }

    // Return cached if not forcing refresh
    if (!forceRefresh && interpretations[cacheKey]) {
      return interpretations[cacheKey]
    }

    return runDedupedAnalysis(cacheKey, () =>
      // Use ai_analyze_section with a special section name for generic analyses
      invoke<AIResponse>('ai_analyze_section', {
        sectionName: cacheKey,
        sectionData: prompt,
        sessionId,
        forceRefresh
      })
    )
  }, [aiEnabled, isAIAvailable, interpretations, sessionId, runDedupedAnalysis])

  // Rank detected issues with the backend's dedicated prioritization prompt.
  // Content-hashed cache key: a re-scan with different issues re-analyzes.
  const prioritizeIssues = useCallback(async (
    issuesJson: string,
    forceRefresh = false
  ): Promise<string> => {
    if (!aiEnabled || !isAIAvailable) {
      return ''
    }
    const cacheKey = `issues:prioritize:${hashContent(issuesJson)}`
    if (!forceRefresh && interpretations[cacheKey]) {
      return interpretations[cacheKey]
    }
    return runDedupedAnalysis(cacheKey, () =>
      invoke<AIResponse>('ai_prioritize_issues', {
        issuesData: issuesJson,
        sessionId,
        forceRefresh
      })
    )
  }, [aiEnabled, isAIAvailable, interpretations, sessionId, runDedupedAnalysis])

  // Cache management
  const clearCache = useCallback(async (targetSessionId?: string) => {
    try {
      await invoke('ai_clear_cache', { sessionId: targetSessionId })

      // Clear local interpretations
      if (targetSessionId) {
        // Clear only for that session - but we only have one session in memory
        if (targetSessionId === sessionId) {
          setInterpretations({})
          setAnalysisMeta({})
          setErrors({})
        }
      } else {
        setInterpretations({})
        setAnalysisMeta({})
        setErrors({})
      }
    } catch (error) {
      logger.error('AIContext', 'Failed to clear AI cache', String(error))
    }
  }, [sessionId])

  const value: AIContextType = {
    aiStatus,
    isAIAvailable,
    activeProvider,
    isLoading,
    aiEnabled,
    setAIEnabled: setAiEnabled,
    preferredProvider,
    setPreferredProvider,
    analyzeDiagnostic,
    analyzeSection,
    explainHealth,
    analyzeGeneric,
    prioritizeIssues,
    isAnalyzing,
    interpretations,
    analysisMeta,
    errors,
    clearCache,
    sessionId,
    setSessionId,
    refreshStatus
  }

  return <AIContext.Provider value={value}>{children}</AIContext.Provider>
}
