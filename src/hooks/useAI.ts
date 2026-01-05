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

  // ============================================================================
  // Monitoring Analysis
  // ============================================================================
  const MONITORING_CACHE_KEY = '__monitoring_analysis__'
  const MONITORING_PROMPT_TEMPLATE = `Analyze these Windows system monitoring stats. Provide a brief assessment (3-4 sentences) of:
1. Overall system health
2. Any concerns or bottlenecks
3. Recommendations if any issues are detected

System Stats:
`

  const requestMonitoringAnalysis = useCallback(
    async (statsJson: string, forceRefresh = false): Promise<string | null> => {
      if (!context.aiEnabled || !context.isAIAvailable) {
        return null
      }
      try {
        const prompt = MONITORING_PROMPT_TEMPLATE + statsJson
        return await context.analyzeGeneric(MONITORING_CACHE_KEY, prompt, forceRefresh)
      } catch {
        return null
      }
    },
    [context]
  )

  const getCachedMonitoringAnalysis = useCallback((): string | null => {
    return context.interpretations[MONITORING_CACHE_KEY] ?? null
  }, [context.interpretations])

  const isMonitoringAnalysisLoading = useCallback((): boolean => {
    return context.isAnalyzing[MONITORING_CACHE_KEY] ?? false
  }, [context.isAnalyzing])

  const getMonitoringAnalysisError = useCallback((): string | null => {
    return context.errors[MONITORING_CACHE_KEY] ?? null
  }, [context.errors])

  // ============================================================================
  // Process Analysis
  // ============================================================================
  const PROCESS_CACHE_KEY = '__process_analysis__'
  const PROCESS_PROMPT_TEMPLATE = `Analyze these running Windows processes. Provide a brief assessment (3-4 sentences) of:
1. High resource consumers that may need attention
2. Any suspicious or unnecessary processes
3. Optimization suggestions

Top Processes:
`

  const requestProcessAnalysis = useCallback(
    async (processesJson: string, forceRefresh = false): Promise<string | null> => {
      if (!context.aiEnabled || !context.isAIAvailable) {
        return null
      }
      try {
        const prompt = PROCESS_PROMPT_TEMPLATE + processesJson
        return await context.analyzeGeneric(PROCESS_CACHE_KEY, prompt, forceRefresh)
      } catch {
        return null
      }
    },
    [context]
  )

  const getCachedProcessAnalysis = useCallback((): string | null => {
    return context.interpretations[PROCESS_CACHE_KEY] ?? null
  }, [context.interpretations])

  const isProcessAnalysisLoading = useCallback((): boolean => {
    return context.isAnalyzing[PROCESS_CACHE_KEY] ?? false
  }, [context.isAnalyzing])

  const getProcessAnalysisError = useCallback((): string | null => {
    return context.errors[PROCESS_CACHE_KEY] ?? null
  }, [context.errors])

  // ============================================================================
  // Comparison Analysis
  // ============================================================================
  const COMPARISON_CACHE_KEY = '__comparison_analysis__'
  const COMPARISON_PROMPT_TEMPLATE = `Compare these two Windows diagnostic scans. Provide a brief assessment (3-4 sentences) of:
1. What has improved since the previous scan
2. What has degraded or needs attention
3. Overall health trend

Scan Comparison:
`

  const requestComparisonAnalysis = useCallback(
    async (comparisonJson: string, forceRefresh = false): Promise<string | null> => {
      if (!context.aiEnabled || !context.isAIAvailable) {
        return null
      }
      try {
        const prompt = COMPARISON_PROMPT_TEMPLATE + comparisonJson
        return await context.analyzeGeneric(COMPARISON_CACHE_KEY, prompt, forceRefresh)
      } catch {
        return null
      }
    },
    [context]
  )

  const getCachedComparisonAnalysis = useCallback((): string | null => {
    return context.interpretations[COMPARISON_CACHE_KEY] ?? null
  }, [context.interpretations])

  const isComparisonAnalysisLoading = useCallback((): boolean => {
    return context.isAnalyzing[COMPARISON_CACHE_KEY] ?? false
  }, [context.isAnalyzing])

  const getComparisonAnalysisError = useCallback((): string | null => {
    return context.errors[COMPARISON_CACHE_KEY] ?? null
  }, [context.errors])

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

    // Monitoring analysis
    requestMonitoringAnalysis,
    getCachedMonitoringAnalysis,
    isMonitoringAnalysisLoading,
    getMonitoringAnalysisError,

    // Process analysis
    requestProcessAnalysis,
    getCachedProcessAnalysis,
    isProcessAnalysisLoading,
    getProcessAnalysisError,

    // Comparison analysis
    requestComparisonAnalysis,
    getCachedComparisonAnalysis,
    isComparisonAnalysisLoading,
    getComparisonAnalysisError,

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
