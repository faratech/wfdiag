// Shared lightweight types (decoupled from Fluent UI).

export type TabValue = 'diagnostics' | 'monitoring' | 'processes' | 'ai' | 'issues' | 'history'

export interface SettingsData {
  openAiApiKey?: string
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
  /** Preferred AI provider: auto-detect, openai, phi_silica, or foundry_local */
  preferredAIProvider?: 'auto' | 'openai' | 'phi_silica' | 'foundry_local'
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
}
