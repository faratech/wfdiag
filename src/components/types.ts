// Shared lightweight types (decoupled from Fluent UI).

export type TabValue = 'diagnostics' | 'monitoring' | 'processes' | 'ai' | 'issues' | 'history'

/** Wire ids of the selectable AI providers (pinned by backend tests) */
export type AIProviderId =
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

export type AIExecutionClass =
  | 'on_device'
  | 'local_server'
  | 'subscription_cloud'
  | 'api_cloud'

export interface AIProviderUse {
  providerId: AIProviderId
  executionClass: AIExecutionClass
  fallbackFrom?: AIProviderId
  /** Configured model id or alias sent to the provider. */
  requestedModel?: string
  /** Concrete model ids reported by the provider for this operation. */
  actualModels?: string[]
}

export interface ChatContextRef {
  kind: 'issue' | 'diagnostic' | 'scan'
  id: string
}

/**
 * A chat handoff may carry detailed context to the backend without dumping it
 * into the visible user bubble. Strings remain accepted by the chat hook for
 * compatibility with older callers.
 */
export interface ChatPrompt {
  query: string
  displayText?: string
  contextRefs?: ChatContextRef[]
  /**
   * Frontend-only preparation state. The chat hook deliberately strips this
   * field before invoking the backend, so a question can survive navigation
   * while the app gathers the scan evidence it needs.
   */
  scanGate?: {
    requestId: string
    kind: 'quick' | 'full'
    status: 'queued' | 'waiting' | 'running' | 'ready' | 'failed'
    /** True after AppContext has visibly joined another in-flight scan. */
    observedRunning?: boolean
    reason?: string
    error?: string
  }
}

export interface SettingsData {
  openAiApiKey?: string
  /** OpenAI model override; empty uses the app default (gpt-5-nano) */
  openAiModel?: string
  /** Anthropic (Claude) API key — stored in the OS secret store, never the settings file */
  anthropicApiKey?: string
  /** Anthropic model override; empty uses the app default (claude-sonnet-5) */
  anthropicModel?: string
  /** Google Gemini API key */
  geminiApiKey?: string
  /** Gemini model override; empty uses the app default (gemini-3.6-flash) */
  geminiModel?: string
  /** DeepSeek API key */
  deepseekApiKey?: string
  /** DeepSeek model override; empty uses the app default (deepseek-v4-flash) */
  deepseekModel?: string
  /** Base URL of a custom OpenAI-compatible endpoint (OpenRouter, Groq, …) */
  customEndpoint?: string
  /** API key for the custom endpoint (optional — local proxies may not need one) */
  customApiKey?: string
  /** Credential presence flags returned by the backend; secret values are never loaded. */
  openAiApiKeySet?: boolean
  anthropicApiKeySet?: boolean
  geminiApiKeySet?: boolean
  deepseekApiKeySet?: boolean
  customApiKeySet?: boolean
  /** Model id on the custom endpoint (required for the provider to be usable) */
  customModel?: string
  /** Ollama endpoint; empty auto-discovers http://127.0.0.1:11434 */
  ollamaEndpoint?: string
  /** Ollama model; empty uses the first locally pulled model */
  ollamaModel?: string
  /**
   * Path to the OpenAI Codex CLI executable (ChatGPT subscription bridge —
   * the CLI owns sign-in, no API key). Empty auto-detects it on PATH.
   */
  codexCliPath?: string
  /** Codex model override; empty uses the CLI's default */
  codexModel?: string
  /**
   * Path to the Claude Code CLI executable (Claude subscription bridge —
   * the CLI owns sign-in, no API key). Empty auto-detects it on PATH.
   */
  claudeCliPath?: string
  /** Claude Code model override; empty uses the CLI's default */
  claudeModel?: string
  autoSave?: boolean
  scanOnStartup?: boolean
  maxConcurrentTasks?: number
  exportFormat?: 'json' | 'text' | 'html'
  theme?: 'dark' | 'light' | 'auto'
  showNotifications?: boolean
  customExportPath?: string
  retainHistory?: boolean
  historyLimit?: number
  /** Enable AI-powered insights throughout the app */
  aiEnabled?: boolean
  /** Preferred AI provider ('auto' routes local-first) */
  preferredAIProvider?: 'auto' | AIProviderId
  /** Allow optional web grounding for providers that support it. Off by default. */
  networkGroundingEnabled?: boolean
  /** What Auto should do before moving from private/local execution to cloud. */
  cloudFallbackPolicy?: 'ask' | 'allow' | 'never'
  /** Custom task IDs for Quick Scan (if empty, uses default set) */
  quickScanTasks?: string[]
  /**
   * Base URL of a local OpenAI-compatible endpoint (e.g. Foundry Local).
   * Leave empty to auto-discover via the foundry CLI (its port is dynamic).
   */
  localAiEndpoint?: string
  /** Foundry Local model override; empty uses the app default (phi-4-mini) */
  localAiModel?: string
  /**
   * Microsoft-issued Limited Access Feature token for Phi Silica. With an
   * approved token the supported WinRT path works without the DLL bypass.
   */
  phiSilicaLafToken?: string
  /** Closing the main window hides to the system tray instead of exiting */
  closeToTray?: boolean
}
