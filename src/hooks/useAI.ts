import { useCallback } from 'react'
import { useAIContext, AIProvider } from '../contexts/AIContext'

/**
 * Convenience hook for AI operations
 * Provides simplified access to AI context with additional helpers
 */
export const useAI = () => {
  const context = useAIContext()

  /**
   * Request AI interpretation for a diagnostic
   * Returns cached result if available, otherwise fetches from backend
   */
  const requestDiagnosticInterpretation = useCallback(
    async (taskId: string, taskName: string, output: string): Promise<string | null> => {
      if (!context.aiEnabled || !context.isAIAvailable) {
        return null
      }

      try {
        return await context.analyzeDiagnostic(taskId, taskName, output)
      } catch {
        return null
      }
    },
    [context]
  )

  /**
   * Get cached interpretation for a diagnostic (doesn't trigger fetch)
   */
  const getCachedDiagnosticInterpretation = useCallback(
    (taskId: string): string | null => {
      const cacheKey = `diagnostic:${taskId}`
      return context.interpretations[cacheKey] ?? null
    },
    [context.interpretations]
  )

  /**
   * Check if a diagnostic interpretation is currently loading
   */
  const isDiagnosticLoading = useCallback(
    (taskId: string): boolean => {
      const cacheKey = `diagnostic:${taskId}`
      return context.isAnalyzing[cacheKey] ?? false
    },
    [context.isAnalyzing]
  )

  /**
   * Get error for a diagnostic interpretation
   */
  const getDiagnosticError = useCallback(
    (taskId: string): string | null => {
      const cacheKey = `diagnostic:${taskId}`
      return context.errors[cacheKey] ?? null
    },
    [context.errors]
  )

  /**
   * Request AI interpretation for a section
   */
  const requestSectionInterpretation = useCallback(
    async (sectionName: string, sectionData: string): Promise<string | null> => {
      if (!context.aiEnabled || !context.isAIAvailable) {
        return null
      }

      try {
        return await context.analyzeSection(sectionName, sectionData)
      } catch {
        return null
      }
    },
    [context]
  )

  /**
   * Get cached section interpretation
   */
  const getCachedSectionInterpretation = useCallback(
    (sectionName: string): string | null => {
      const cacheKey = `section:${sectionName}`
      return context.interpretations[cacheKey] ?? null
    },
    [context.interpretations]
  )

  /**
   * Check if a section interpretation is currently loading
   */
  const isSectionLoading = useCallback(
    (sectionName: string): boolean => {
      const cacheKey = `section:${sectionName}`
      return context.isAnalyzing[cacheKey] ?? false
    },
    [context.isAnalyzing]
  )

  /**
   * Get provider display name
   */
  const getProviderDisplayName = useCallback((provider: AIProvider): string => {
    switch (provider) {
      case 'openai':
        return 'OpenAI'
      case 'phi_silica':
        return 'Phi Silica (Local)'
      default:
        return 'None'
    }
  }, [])

  /**
   * Check if any AI operation is in progress
   */
  const isAnyAnalysisInProgress = Object.values(context.isAnalyzing).some(v => v)

  return {
    // Status
    aiStatus: context.aiStatus,
    isAIAvailable: context.isAIAvailable,
    activeProvider: context.activeProvider,
    isLoading: context.isLoading,

    // Settings
    aiEnabled: context.aiEnabled,
    setAIEnabled: context.setAIEnabled,
    preferredProvider: context.preferredProvider,
    setPreferredProvider: context.setPreferredProvider,

    // Diagnostic operations
    requestDiagnosticInterpretation,
    getCachedDiagnosticInterpretation,
    isDiagnosticLoading,
    getDiagnosticError,

    // Section operations
    requestSectionInterpretation,
    getCachedSectionInterpretation,
    isSectionLoading,

    // Health operations
    explainHealth: context.explainHealth,

    // Utilities
    getProviderDisplayName,
    isAnyAnalysisInProgress,

    // Cache management
    clearCache: context.clearCache,

    // Session
    sessionId: context.sessionId,
    setSessionId: context.setSessionId,

    // Refresh
    refreshStatus: context.refreshStatus
  }
}
