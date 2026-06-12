import React, { createContext, useContext, useState, useEffect, ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { type TabValue, type SettingsData } from '../components'
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

export interface Issue {
  id?: string
  title: string
  description: string
  severity: string
  category: string
  recommendation?: string
  detected: boolean
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
  setResults: (results: Record<string, TaskResult>) => void
  isRunning: boolean
  setIsRunning: (running: boolean) => void
  currentProgress: number
  setCurrentProgress: (progress: number) => void
  currentTaskName: string
  setCurrentTaskName: (name: string) => void
  // Per-task status of the current scan, keyed by task id (drives the
  // per-category progress chips in the scanning hero)
  taskStatuses: Record<string, 'running' | 'done'>
  setTaskStatuses: React.Dispatch<React.SetStateAction<Record<string, 'running' | 'done'>>>
  isMonitoringActive: boolean
  setIsMonitoringActive: (active: boolean) => void
  showComparison: boolean
  setShowComparison: (show: boolean) => void
  searchQuery: string
  setSearchQuery: (query: string) => void
  filteredResults: Record<string, TaskResult>
  setFilteredResults: (results: Record<string, TaskResult>) => void
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
  // Deep-link from the command palette into a diagnostic's detail pane
  pendingDetailTaskId: string | null
  setPendingDetailTaskId: (id: string | null) => void
}

const AppContext = createContext<AppContextType | undefined>(undefined)

export const useAppContext = () => {
  const context = useContext(AppContext)
  if (!context) {
    throw new Error('useAppContext must be used within an AppProvider')
  }
  return context
}

interface AppProviderProps {
  children: ReactNode
}

export const AppProvider: React.FC<AppProviderProps> = ({ children }) => {
  const [selectedTab, setSelectedTabInternal] = useState<TabValue>('diagnostics')
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null)
  const [availableTasks, setAvailableTasks] = useState<DiagnosticTask[]>([])
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [results, setResults] = useState<Record<string, TaskResult>>({})
  const [isRunning, setIsRunning] = useState(false)
  const [currentProgress, setCurrentProgress] = useState(0)
  const [currentTaskName, setCurrentTaskName] = useState('')
  const [taskStatuses, setTaskStatuses] = useState<Record<string, 'running' | 'done'>>({})
  const [isMonitoringActive, setIsMonitoringActive] = useState(false)

  // Wrapper to stop monitoring when leaving the monitoring tab
  const setSelectedTab = (tab: TabValue) => {
    if (selectedTab === 'monitoring' && tab !== 'monitoring' && isMonitoringActive) {
      setIsMonitoringActive(false)
    }
    setSelectedTabInternal(tab)
  }
  const [showComparison, setShowComparison] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const [filteredResults, setFilteredResults] = useState<Record<string, TaskResult>>({})
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
  })
  const [settingsLoaded, setSettingsLoaded] = useState(false)

  // NavRail state - initialize from localStorage
  const [navRailCollapsed, setNavRailCollapsedInternal] = useState(() => {
    const saved = localStorage.getItem('navRailCollapsed')
    return saved ? JSON.parse(saved) : false
  })

  const setNavRailCollapsed = (collapsed: boolean) => {
    setNavRailCollapsedInternal(collapsed)
    localStorage.setItem('navRailCollapsed', JSON.stringify(collapsed))
  }

  // Task to open in the diagnostics detail pane on next render — set by the
  // command palette to deep-link into a specific result, consumed (and
  // cleared) by DiagnosticsScreen
  const [pendingDetailTaskId, setPendingDetailTaskId] = useState<string | null>(null)

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

      // Also store API key in keyring if provided (for AI service compatibility)
      if (newSettings.openAiApiKey) {
        try {
          await invoke('store_api_key', { key: newSettings.openAiApiKey })
          logger.debug('AppContext', 'API key stored in keyring')
        } catch (keyError) {
          logger.warn('AppContext', 'Failed to store API key in keyring', String(keyError))
          // Don't throw - settings are still saved, just keyring failed
        }
      } else if (settings.openAiApiKey && !newSettings.openAiApiKey) {
        // API key was cleared
        try {
          await invoke('clear_api_key')
          logger.debug('AppContext', 'API key cleared from keyring')
        } catch (keyError) {
          logger.warn('AppContext', 'Failed to clear API key from keyring', String(keyError))
        }
      }

      setSettings(newSettings)
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
    taskStatuses,
    setTaskStatuses,
    isMonitoringActive,
    setIsMonitoringActive,
    showComparison,
    setShowComparison,
    searchQuery,
    setSearchQuery,
    filteredResults,
    setFilteredResults,
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
    pendingDetailTaskId,
    setPendingDetailTaskId,
  }

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>
}