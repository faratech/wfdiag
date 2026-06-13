// Shared lightweight types (decoupled from Fluent UI).

export type TabValue = 'diagnostics' | 'monitoring' | 'processes' | 'ai' | 'issues' | 'history'

/** Wire ids of the selectable AI providers (pinned by backend tests) */
export type AIProviderId =
  | 'openai'
  | 'phi_silica'
  | 'foundry_local'
  | 'ollama'
  | 'custom_openai'
  | 'anthropic'
  | 'gemini'
  | 'deepseek'

export interface SettingsData {
  openAiApiKey?: string
  /** Anthropic (Claude) API key — stored in the OS secret store, never the settings file */
  anthropicApiKey?: string
  /** Anthropic model override; empty uses the app default (claude-sonnet-4-6) */
  anthropicModel?: string
  /** Google Gemini API key */
  geminiApiKey?: string
  /** Gemini model override; empty uses the app default (gemini-2.5-flash) */
  geminiModel?: string
  /** DeepSeek API key */
  deepseekApiKey?: string
  /** DeepSeek model override; empty uses the app default (deepseek-chat) */
  deepseekModel?: string
  /** Base URL of a custom OpenAI-compatible endpoint (OpenRouter, Groq, …) */
  customEndpoint?: string
  /** API key for the custom endpoint (optional — local proxies may not need one) */
  customApiKey?: string
  /** Model id on the custom endpoint (required for the provider to be usable) */
  customModel?: string
  /** Ollama endpoint; empty auto-discovers http://127.0.0.1:11434 */
  ollamaEndpoint?: string
  /** Ollama model; empty uses the first locally pulled model */
  ollamaModel?: string
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
  /** Custom task IDs for Quick Scan (if empty, uses default set) */
  quickScanTasks?: string[]
  /**
   * Base URL of a local OpenAI-compatible endpoint (e.g. Foundry Local).
   * Leave empty to auto-discover via the foundry CLI (its port is dynamic).
   */
  localAiEndpoint?: string
  /**
   * Microsoft-issued Limited Access Feature token for Phi Silica. With an
   * approved token the supported WinRT path works without the DLL bypass.
   */
  phiSilicaLafToken?: string
  /** Closing the main window hides to the system tray instead of exiting */
  closeToTray?: boolean
}
