import React, { createContext, useContext, useState, useEffect, useCallback, ReactNode, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useAppContext } from './AppContext'

// Types matching the Rust backend
export type AIProvider = 'none' | 'openai' | 'phi_silica'
export type AIProviderPreference = 'auto' | 'openai' | 'phi_silica'

export interface AIProviderStatus {
  preferred_provider: AIProvider
  openai_available: boolean
  openai_api_key_set: boolean
  phi_silica_available: boolean
  phi_silica_ready: boolean
  phi_silica_message?: string
  active_provider: AIProvider
}

export interface AIResponse {
  interpretation: string
  provider_used: AIProvider
  cached: boolean
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
  analyzeDiagnostic: (taskId: string, taskName: string, output: string) => Promise<string>
  analyzeSection: (sectionName: string, sectionData: string) => Promise<string>
  explainHealth: (metricsData: string) => Promise<string>

  // Loading states per context_id
  isAnalyzing: Record<string, boolean>
  interpretations: Record<string, string>
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
  const [errors, setErrors] = useState<Record<string, string>>({})

  // Track previous settings to detect changes
  const prevSettingsRef = useRef<{ apiKey?: string; provider?: string }>({})

  // Use settings from AppContext (with defaults)
  const aiEnabled = settings.aiEnabled ?? true
  const preferredProvider: AIProviderPreference = (settings.preferredAIProvider as AIProviderPreference) ?? 'auto'

  // Derived state - AI is available if backend reports provider OR if we have an API key
  const backendAvailable = aiStatus?.active_provider !== 'none'
  const hasSettingsApiKey = !!settings.openAiApiKey
  const isAIAvailable = backendAvailable || hasSettingsApiKey
  const activeProvider: AIProvider = backendAvailable
    ? (aiStatus?.active_provider ?? 'none')
    : (hasSettingsApiKey ? 'openai' : 'none')

  // Load AI status
  const refreshStatus = useCallback(async () => {
    try {
      setIsLoading(true)
      const status = await invoke<AIProviderStatus>('ai_get_status')
      setAiStatus(status)
      console.log('AI status refreshed:', status)
    } catch (error) {
      console.error('Failed to get AI status:', error)
      setAiStatus({
        preferred_provider: 'none',
        openai_available: false,
        openai_api_key_set: false,
        phi_silica_available: false,
        phi_silica_ready: false,
        active_provider: 'none'
      })
    } finally {
      setIsLoading(false)
    }
  }, [])

  // Initial status load once settings are loaded
  useEffect(() => {
    if (settingsLoaded) {
      refreshStatus()
    }
  }, [settingsLoaded, refreshStatus])

  // Refresh status when relevant settings change
  useEffect(() => {
    if (!settingsLoaded) return

    const currentApiKey = settings.openAiApiKey || ''
    const currentProvider = settings.preferredAIProvider || 'auto'
    const prevApiKey = prevSettingsRef.current.apiKey || ''
    const prevProvider = prevSettingsRef.current.provider || 'auto'

    // Check if API key or provider changed
    if (currentApiKey !== prevApiKey || currentProvider !== prevProvider) {
      console.log('AI settings changed, refreshing status...')
      prevSettingsRef.current = { apiKey: currentApiKey, provider: currentProvider }

      // Update backend preference if it changed
      if (currentProvider !== prevProvider) {
        invoke('ai_set_preference', { preference: currentProvider }).catch(console.error)
      }

      // Refresh status after a short delay to allow keyring update to complete
      const timer = setTimeout(() => {
        refreshStatus()
      }, 100)

      return () => clearTimeout(timer)
    }
  }, [settings.openAiApiKey, settings.preferredAIProvider, settingsLoaded, refreshStatus])

  // Dummy setters for backwards compatibility (settings are managed via AppContext now)
  const setAiEnabled = useCallback((_enabled: boolean) => {
    console.warn('setAiEnabled is deprecated - use Settings dialog to change AI settings')
  }, [])

  const setPreferredProvider = useCallback(async (_provider: AIProviderPreference) => {
    console.warn('setPreferredProvider is deprecated - use Settings dialog to change AI settings')
  }, [])

  // Check if we have an API key from settings
  const hasApiKey = !!settings.openAiApiKey

  // Analysis functions
  const analyzeDiagnostic = useCallback(async (
    taskId: string,
    taskName: string,
    output: string
  ): Promise<string> => {
    // Allow if AI is enabled and either backend says available OR we have an API key
    if (!aiEnabled || (!isAIAvailable && !hasApiKey)) {
      return ''
    }

    // Check if already cached
    const cacheKey = `diagnostic:${taskId}`
    if (interpretations[cacheKey]) {
      return interpretations[cacheKey]
    }

    // Set loading state
    setIsAnalyzing(prev => ({ ...prev, [cacheKey]: true }))
    setErrors(prev => {
      const { [cacheKey]: _, ...rest } = prev
      return rest
    })

    try {
      const response = await invoke<AIResponse>('ai_analyze_diagnostic', {
        taskId,
        taskName,
        diagnosticOutput: output,
        sessionId,
        apiKey: settings.openAiApiKey || null
      })

      const interpretation = response.interpretation
      setInterpretations(prev => ({ ...prev, [cacheKey]: interpretation }))
      return interpretation
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error)
      setErrors(prev => ({ ...prev, [cacheKey]: errorMsg }))
      throw error
    } finally {
      setIsAnalyzing(prev => ({ ...prev, [cacheKey]: false }))
    }
  }, [aiEnabled, isAIAvailable, hasApiKey, interpretations, sessionId, settings.openAiApiKey])

  const analyzeSection = useCallback(async (
    sectionName: string,
    sectionData: string
  ): Promise<string> => {
    if (!aiEnabled || (!isAIAvailable && !hasApiKey)) {
      return ''
    }

    const cacheKey = `section:${sectionName}`
    if (interpretations[cacheKey]) {
      return interpretations[cacheKey]
    }

    setIsAnalyzing(prev => ({ ...prev, [cacheKey]: true }))
    setErrors(prev => {
      const { [cacheKey]: _, ...rest } = prev
      return rest
    })

    try {
      const response = await invoke<AIResponse>('ai_analyze_section', {
        sectionName,
        sectionData,
        sessionId,
        apiKey: settings.openAiApiKey || null
      })

      const interpretation = response.interpretation
      setInterpretations(prev => ({ ...prev, [cacheKey]: interpretation }))
      return interpretation
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error)
      setErrors(prev => ({ ...prev, [cacheKey]: errorMsg }))
      throw error
    } finally {
      setIsAnalyzing(prev => ({ ...prev, [cacheKey]: false }))
    }
  }, [aiEnabled, isAIAvailable, hasApiKey, interpretations, sessionId, settings.openAiApiKey])

  const explainHealth = useCallback(async (metricsData: string): Promise<string> => {
    if (!aiEnabled || (!isAIAvailable && !hasApiKey)) {
      return ''
    }

    const cacheKey = 'health:explanation'
    if (interpretations[cacheKey]) {
      return interpretations[cacheKey]
    }

    setIsAnalyzing(prev => ({ ...prev, [cacheKey]: true }))
    setErrors(prev => {
      const { [cacheKey]: _, ...rest } = prev
      return rest
    })

    try {
      const response = await invoke<AIResponse>('ai_explain_health', {
        metricsData,
        sessionId,
        apiKey: settings.openAiApiKey || null
      })

      const interpretation = response.interpretation
      setInterpretations(prev => ({ ...prev, [cacheKey]: interpretation }))
      return interpretation
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error)
      setErrors(prev => ({ ...prev, [cacheKey]: errorMsg }))
      throw error
    } finally {
      setIsAnalyzing(prev => ({ ...prev, [cacheKey]: false }))
    }
  }, [aiEnabled, isAIAvailable, hasApiKey, interpretations, sessionId, settings.openAiApiKey])

  // Cache management
  const clearCache = useCallback(async (targetSessionId?: string) => {
    try {
      await invoke('ai_clear_cache', { sessionId: targetSessionId })

      // Clear local interpretations
      if (targetSessionId) {
        // Clear only for that session - but we only have one session in memory
        if (targetSessionId === sessionId) {
          setInterpretations({})
          setErrors({})
        }
      } else {
        setInterpretations({})
        setErrors({})
      }
    } catch (error) {
      console.error('Failed to clear AI cache:', error)
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
    isAnalyzing,
    interpretations,
    errors,
    clearCache,
    sessionId,
    setSessionId,
    refreshStatus
  }

  return <AIContext.Provider value={value}>{children}</AIContext.Provider>
}
