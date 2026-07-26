import { useMemo } from 'react'
import { useAppContext } from '../contexts/AppContext'
import { useTheme } from '../contexts/ThemeContext'
import { useScanner } from './useScanner'
import { useDiagnostics } from './useDiagnostics'
import { NAV_TAB_ICON } from '../ui/diagnostic-icons'
import { taskIcon } from '../ui/diagnostic-icons'
import type { TabValue } from '../components/types'

export interface Command {
  id: string
  title: string
  section: 'Navigate' | 'Scan' | 'Report' | 'App' | 'Diagnostics'
  /** FontAwesome icon name without the fa-solid prefix */
  icon: string
  /** Display-only shortcut hint, e.g. "Ctrl+1" */
  shortcut?: string
  /** Extra text included in fuzzy matching */
  keywords?: string
  enabled?: boolean
  run: () => void
}

const NAV_TABS: { id: TabValue; label: string }[] = [
  { id: 'diagnostics', label: 'Diagnostics' },
  { id: 'monitoring', label: 'Live Monitor' },
  { id: 'processes', label: 'Processes' },
  { id: 'ai', label: 'AI Analysis' },
  { id: 'issues', label: 'Issues' },
  { id: 'history', label: 'History' },
]

/** Registry of everything the command palette can do, built from existing hooks. */
export function useCommands(): Command[] {
  const {
    setSelectedTab, availableTasks, results, isRunning, systemInfo,
    setShowSettings, setShowAbout, navRailCollapsed, setNavRailCollapsed,
    setSelectedDiagnosticId, settings, saveSettings,
  } = useAppContext()
  const { setThemeMode, isDark } = useTheme()
  const { rerunDiagnostic, runQuickScan, runFullScan, stopScan } = useScanner()
  const { copyToClipboard, exportResults, shareToWindowsForum, emailReport, generateSupportPackage } = useDiagnostics()

  const hasResults = Object.keys(results).length > 0

  return useMemo(() => {
    const commands: Command[] = []

    NAV_TABS.forEach((t, i) => {
      commands.push({
        id: `nav:${t.id}`,
        title: `Go to ${t.label}`,
        section: 'Navigate',
        icon: NAV_TAB_ICON[t.id],
        shortcut: `Ctrl+${i + 1}`,
        keywords: t.id,
        run: () => setSelectedTab(t.id),
      })
    })

    commands.push(
      {
        id: 'scan:quick',
        title: 'Run Quick Scan',
        section: 'Scan',
        icon: 'fa-bolt',
        shortcut: 'Ctrl+Shift+Q',
        enabled: !isRunning,
        run: () => { runQuickScan() },
      },
      {
        id: 'scan:full',
        title: 'Run Full Scan',
        section: 'Scan',
        icon: 'fa-list-check',
        shortcut: 'Ctrl+Shift+F',
        enabled: !isRunning,
        run: () => { runFullScan() },
      },
    )
    if (isRunning) {
      commands.push({
        id: 'scan:stop',
        title: 'Stop Scan',
        section: 'Scan',
        icon: 'fa-stop',
        run: () => stopScan(),
      })
    }

    const reportActions: [string, string, string, () => void][] = [
      ['report:copy', 'Copy Report to Clipboard', 'fa-copy', () => { copyToClipboard() }],
      ['report:export', 'Export Report…', 'fa-file-export', () => { exportResults() }],
      ['report:share', 'Share to WindowsForum', 'fa-share-nodes', () => { shareToWindowsForum() }],
      ['report:email', 'Email Report', 'fa-envelope', () => { emailReport() }],
      ['report:package', 'Generate Support Package', 'fa-box-archive', () => { generateSupportPackage() }],
    ]
    for (const [id, title, icon, run] of reportActions) {
      commands.push({ id, title, section: 'Report', icon, enabled: hasResults, keywords: 'report results', run })
    }

    commands.push(
      {
        id: 'app:theme',
        title: `Switch to ${isDark ? 'Light' : 'Dark'} Theme`,
        section: 'App',
        icon: isDark ? 'fa-sun' : 'fa-moon',
        keywords: 'theme dark light appearance',
        run: () => {
          const nextTheme = isDark ? 'light' : 'dark'
          setThemeMode(nextTheme)
          void saveSettings({ ...settings, theme: nextTheme })
        },
      },
      {
        id: 'app:settings',
        title: 'Open Settings',
        section: 'App',
        icon: 'fa-gear',
        keywords: 'preferences options api key',
        run: () => setShowSettings(true),
      },
      {
        id: 'app:about',
        title: 'About WindowsForum Diagnostics',
        section: 'App',
        icon: 'fa-circle-info',
        keywords: 'version',
        run: () => setShowAbout(true),
      },
      {
        id: 'app:rail',
        title: navRailCollapsed ? 'Expand Navigation Rail' : 'Collapse Navigation Rail',
        section: 'App',
        icon: navRailCollapsed ? 'fa-angles-right' : 'fa-angles-left',
        keywords: 'sidebar nav',
        run: () => setNavRailCollapsed(!navRailCollapsed),
      },
    )

    for (const task of availableTasks) {
      const adminBlocked = task.admin_required && !systemInfo?.is_admin
      if (results[task.id]) {
        commands.push({
          id: `view:${task.id}`,
          title: `View Result: ${task.name}`,
          section: 'Diagnostics',
          icon: taskIcon(task.id, task.category),
          keywords: `${task.category} ${task.id}`,
          run: () => {
            setSelectedDiagnosticId(task.id)
            setSelectedTab('diagnostics')
          },
        })
      }
      commands.push({
        id: `run:${task.id}`,
        title: `Run: ${task.name}`,
        section: 'Diagnostics',
        icon: taskIcon(task.id, task.category),
        keywords: `${task.category} ${task.id}`,
        enabled: !isRunning && !adminBlocked,
        run: () => { void rerunDiagnostic(task.id) },
      })
    }

    return commands
  }, [
    availableTasks, results, isRunning, hasResults, isDark, navRailCollapsed, systemInfo,
    setSelectedTab, setShowSettings, setShowAbout, setNavRailCollapsed, setSelectedDiagnosticId,
    settings, saveSettings, setThemeMode, rerunDiagnostic, runQuickScan, runFullScan, stopScan,
    copyToClipboard, exportResults, shareToWindowsForum, emailReport, generateSupportPackage,
  ])
}
