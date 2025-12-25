import React, { createContext, useContext, useState, useEffect, ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { type TabValue, type SettingsData } from '../components'

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
  })
  const [settingsLoaded, setSettingsLoaded] = useState(false)

  // Load settings from backend on startup
  useEffect(() => {
    const loadSettingsFromBackend = async () => {
      try {
        const savedSettings = await invoke<SettingsData>('load_settings')
        setSettings(prev => ({ ...prev, ...savedSettings }))
        console.log('Settings loaded from backend:', savedSettings)
      } catch (error) {
        console.error('Failed to load settings:', error)
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
      setSettings(newSettings)
      console.log('Settings saved to backend:', newSettings)
    } catch (error) {
      console.error('Failed to save settings:', error)
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
  }

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>
}