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
  /** Preferred AI provider: auto-detect, openai, or phi_silica */
  preferredAIProvider?: 'auto' | 'openai' | 'phi_silica'
  /** Custom task IDs for Quick Scan (if empty, uses default set) */
  quickScanTasks?: string[]
}
