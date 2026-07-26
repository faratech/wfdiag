import React, { createContext, useContext, useState, useEffect, ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { type TabValue, type SettingsData } from '../components'
import type { ChatPrompt } from '../components/types'
import * as logger from '../utils/logger'

export interface SystemInfo {
  computer_name: string
  os_version: string
  is_admin: boolean
}

export interface DiagnosticTask {
  id: string
  name: string
  description: string
  category: string
  admin_required: boolean
}

export interface TaskResult {
  success: boolean
  output: string
  error?: string
  duration_ms: number
}

export interface RemediationSummary {
  id: string
  label: string
  description: string
  tier: 'open_tool' | 'auto_safe' | 'repair'
  admin_required: boolean
  requires_restart: boolean
  long_running: boolean
  maintenance: boolean
  batch_eligible: boolean
  cancellable: boolean
}

export interface ActionRequest {
  remediationId: string
  issueId?: string
}

export interface ActionPreview {
  remediation: RemediationSummary
  issueId?: string
  steps: string[]
}

export interface ActionProposal {
  proposalId: string
  approvalScope: 'exact' | 'batch'
  actions: ActionPreview[]
  scanFingerprint: string
  catalogFingerprint: string
  createdAtMs: number
  expiresAtMs: number
}

export interface ActionStepResult {
  action: string
  status: 'succeeded' | 'already_satisfied' | 'failed' | 'cancelled'
  detail?: string
}

export interface ActionFixResult {
  success: boolean
  message: string
  actions_taken: string[]
  requires_restart: boolean
  completion_status: 'succeeded' | 'partial' | 'failed' | 'cancelled'
  steps: ActionStepResult[]
}

export interface ActionItemRun {
  remediationId: string
  label: string
  cancellable: boolean
  status: 'pending' | 'running' | 'succeeded' | 'partial' | 'failed' | 'cancelled' | 'skipped'
  result?: ActionFixResult
  error?: string
}

export interface ActionRunSummary {
  runId: string
  proposalId: string
  authorizationId: string
  status: 'running' | 'cancel_requested' | 'succeeded' | 'partial' | 'failed' | 'cancelled'
  actions: ActionItemRun[]
  currentIndex?: number | null
  approvedAtMs: number
  completedAtMs?: number
  scanFingerprint: string
  catalogFingerprint: string
}

export interface Issue {
  id?: string
  title: string
  description: string
  severity: string
  category: string
  recommendation?: string
  detected: boolean
  /** unknown means the source check could not establish a trustworthy result. */
  status?: 'detected' | 'ok' | 'unknown' | 'skipped'
  /** Diagnostic tasks this issue was derived from (used by "Ask AI") */
  source_tasks?: string[]
  /** The vetted remediation for this issue, when one applies */
  remediation?: RemediationSummary
}

// Global search result type
interface AppContextType {
  // State
  selectedTab: TabValue
  setSelectedTab: (tab: TabValue) => void
  systemInfo: SystemInfo | null
  setSystemInfo: (info: SystemInfo | null) => void
  availableTasks: DiagnosticTask[]
  setAvailableTasks: (tasks: DiagnosticTask[]) => void
  sessionId: string | null
  setSessionId: (id: string | null) => void
  results: Record<string, TaskResult>
  setResults: React.Dispatch<React.SetStateAction<Record<string, TaskResult>>>
  isRunning: boolean
  setIsRunning: (running: boolean) => void
  currentProgress: number
  setCurrentProgress: (progress: number) => void
  currentTaskName: string
  setCurrentTaskName: (name: string) => void
  diagnosticsError: string | null
  setDiagnosticsError: (message: string | null) => void
  // Per-task status of the current scan, keyed by task id (drives the
  // per-category progress chips in the scanning hero)
  taskStatuses: Record<string, 'running' | 'done'>
  setTaskStatuses: React.Dispatch<React.SetStateAction<Record<string, 'running' | 'done'>>>
  scanStartTime: number
  setScanStartTime: (time: number) => void
  scanEndTime: number
  setScanEndTime: (time: number) => void
  issues: Issue[]
  setIssues: (issues: Issue[]) => void
  fixingIssue: string | null
  setFixingIssue: (id: string | null) => void
  showSettings: boolean
  setShowSettings: (show: boolean) => void
  showAbout: boolean
  setShowAbout: (show: boolean) => void
  settings: SettingsData
  setSettings: (settings: SettingsData) => void
  saveSettings: (settings: SettingsData) => Promise<void>
  settingsLoaded: boolean
  // NavRail state
  navRailCollapsed: boolean
  setNavRailCollapsed: (collapsed: boolean) => void
  // The diagnostic selected in the detail pane; also the target of the command
  // palette's "View Result" deep-link (set directly, so no transient flag)
  selectedDiagnosticId: string | null
  setSelectedDiagnosticId: (id: string | null) => void
  // Deep-link from "Explain this scan" into the AI screen's report panel
  pendingScanReport: boolean
  setPendingScanReport: (pending: boolean) => void
  aiMode: 'assistant' | 'report'
  setAIMode: (mode: 'assistant' | 'report') => void
  // Deep-link from an issue's "Ask AI" into the agentic chat (pre-seeded prompt)
  pendingChatPrompt: ChatPrompt | string | null
  setPendingChatPrompt: React.Dispatch<React.SetStateAction<ChatPrompt | string | null>>
}

const AppContext = createContext<AppContextType | undefined>(undefined)

export const useAppContext = () => {
  const context = useContext(AppContext)
  if (!context) {
    throw new Error('useAppContext must be used within an AppProvider')
  }
  return context
}

function loadNavRailCollapsed(): boolean {
  const saved = localStorage.getItem('navRailCollapsed')
  if (!saved) return false
  try {
    return JSON.parse(saved) === true
  } catch {
    return false
  }
}

interface AppProviderProps {
  children: ReactNode
}

export const AppProvider: React.FC<AppProviderProps> = ({ children }) => {
  const [selectedTab, setSelectedTab] = useState<TabValue>('diagnostics')
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null)
  const [availableTasks, setAvailableTasks] = useState<DiagnosticTask[]>([])
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [results, setResults] = useState<Record<string, TaskResult>>({})
  const [isRunning, setIsRunning] = useState(false)
  const [currentProgress, setCurrentProgress] = useState(0)
  const [currentTaskName, setCurrentTaskName] = useState('')
  const [diagnosticsError, setDiagnosticsError] = useState<string | null>(null)
  const [taskStatuses, setTaskStatuses] = useState<Record<string, 'running' | 'done'>>({})
  const [scanStartTime, setScanStartTime] = useState<number>(0)
  const [scanEndTime, setScanEndTime] = useState<number>(0)
  const [issues, setIssues] = useState<Issue[]>([])
  const [fixingIssue, setFixingIssue] = useState<string | null>(null)
  const [showSettings, setShowSettings] = useState(false)
  const [showAbout, setShowAbout] = useState(false)
  const [settings, setSettings] = useState<SettingsData>({
    autoSave: true,
    scanOnStartup: false,
    maxConcurrentTasks: 5,
    exportFormat: 'text',
    theme: 'dark',
    showNotifications: true,
    retainHistory: true,
    historyLimit: 30,
    aiEnabled: true,
    preferredAIProvider: 'auto',
    networkGroundingEnabled: false,
    cloudFallbackPolicy: 'ask',
  })
  const [settingsLoaded, setSettingsLoaded] = useState(false)

  // NavRail state - initialize from localStorage
  const [navRailCollapsed, setNavRailCollapsedInternal] = useState(loadNavRailCollapsed)

  const setNavRailCollapsed = (collapsed: boolean) => {
    setNavRailCollapsedInternal(collapsed)
    localStorage.setItem('navRailCollapsed', JSON.stringify(collapsed))
  }

  // Task to open in the diagnostics detail pane on next render — set by the
  // command palette to deep-link into a specific result, consumed (and
  // cleared) by DiagnosticsScreen
  const [selectedDiagnosticId, setSelectedDiagnosticId] = useState<string | null>(null)

  // "Explain this scan" pressed — consumed (and cleared) by ScanReportPanel,
  // which auto-generates the report when this is set
  const [pendingScanReport, setPendingScanReport] = useState(false)
  const [aiMode, setAIMode] = useState<'assistant' | 'report'>('assistant')

  // An issue's "Ask AI" pressed — consumed (and cleared) by AIScreen, which
  // sends it into the agentic chat
  const [pendingChatPrompt, setPendingChatPrompt] = useState<ChatPrompt | string | null>(null)

  // Load settings from backend on startup
  useEffect(() => {
    const loadSettingsFromBackend = async () => {
      try {
        const savedSettings = await invoke<SettingsData>('load_settings')
        setSettings(prev => ({ ...prev, ...savedSettings }))
        // Don't log the settings object itself — it can contain the API key
        logger.debug('AppContext', 'Settings loaded from backend')
      } catch (error) {
        logger.error('AppContext', 'Failed to load settings', String(error))
      } finally {
        setSettingsLoaded(true)
      }
    }
    loadSettingsFromBackend()
  }, [])

  // Save settings to backend
  const saveSettings = async (newSettings: SettingsData) => {
    try {
      await invoke('save_settings', { settings: newSettings })
      // Credentials are write-only IPC inputs. Once the backend has persisted
      // them, retain only presence flags in webview memory.
      const {
        openAiApiKey,
        anthropicApiKey,
        geminiApiKey,
        deepseekApiKey,
        customApiKey,
        ...nonSecretSettings
      } = newSettings
      setSettings({
        ...nonSecretSettings,
        openAiApiKeySet: openAiApiKey === undefined ? newSettings.openAiApiKeySet : openAiApiKey.trim().length > 0,
        anthropicApiKeySet: anthropicApiKey === undefined ? newSettings.anthropicApiKeySet : anthropicApiKey.trim().length > 0,
        geminiApiKeySet: geminiApiKey === undefined ? newSettings.geminiApiKeySet : geminiApiKey.trim().length > 0,
        deepseekApiKeySet: deepseekApiKey === undefined ? newSettings.deepseekApiKeySet : deepseekApiKey.trim().length > 0,
        customApiKeySet: customApiKey === undefined ? newSettings.customApiKeySet : customApiKey.trim().length > 0,
      })
      logger.debug('AppContext', 'Settings saved to backend')
    } catch (error) {
      logger.error('AppContext', 'Failed to save settings', String(error))
      throw error
    }
  }

  const value: AppContextType = {
    selectedTab,
    setSelectedTab,
    systemInfo,
    setSystemInfo,
    availableTasks,
    setAvailableTasks,
    sessionId,
    setSessionId,
    results,
    setResults,
    isRunning,
    setIsRunning,
    currentProgress,
    setCurrentProgress,
    currentTaskName,
    setCurrentTaskName,
    diagnosticsError,
    setDiagnosticsError,
    taskStatuses,
    setTaskStatuses,
    scanStartTime,
    setScanStartTime,
    scanEndTime,
    setScanEndTime,
    issues,
    setIssues,
    fixingIssue,
    setFixingIssue,
    showSettings,
    setShowSettings,
    showAbout,
    setShowAbout,
    settings,
    setSettings,
    saveSettings,
    settingsLoaded,
    navRailCollapsed,
    setNavRailCollapsed,
    selectedDiagnosticId,
    setSelectedDiagnosticId,
    pendingScanReport,
    setPendingScanReport,
    aiMode,
    setAIMode,
    pendingChatPrompt,
    setPendingChatPrompt,
  }

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>
}
