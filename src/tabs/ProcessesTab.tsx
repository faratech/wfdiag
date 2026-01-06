import React, { useState, useEffect, useCallback, useMemo } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen, UnlistenFn } from '@tauri-apps/api/event'
import {
  makeStyles,
  tokens,
  shorthands,
  Button,
  Input,
  Switch,
  Text,
  Badge,
  Tooltip,
  Menu,
  MenuTrigger,
  MenuPopover,
  MenuList,
  MenuItemCheckbox,
  Spinner,
} from '@fluentui/react-components'
import {
  Play20Regular,
  Pause20Regular,
  Search20Regular,
  ArrowUp16Regular,
  ArrowDown16Regular,
  ChevronRight16Regular,
  ChevronDown16Regular,
  Settings20Regular,
  ArrowClockwise20Regular,
  Apps24Regular,
} from '@fluentui/react-icons'
import { PageHeader } from '../components'
import type { ProcessInfo, SystemStats } from '../types/monitoring'
import { formatBytes, formatCpuTime } from '../utils/formatters'

// Column definitions matching egui's 15 columns
type SortColumn = 'pid' | 'ppid' | 'name' | 'cpu' | 'mem' | 'virt' | 'res' | 'shr' | 'thr' | 'hndl' | 'pri' | 'time' | 'status' | 'ior' | 'iow'

interface ColumnConfig {
  id: SortColumn
  label: string
  width: number
  visible: boolean
}

const DEFAULT_COLUMNS: ColumnConfig[] = [
  { id: 'pid', label: 'PID', width: 60, visible: true },
  { id: 'ppid', label: 'PPID', width: 60, visible: false },
  { id: 'name', label: 'Name', width: 180, visible: true },
  { id: 'cpu', label: 'CPU%', width: 60, visible: true },
  { id: 'mem', label: 'MEM%', width: 60, visible: true },
  { id: 'virt', label: 'VIRT', width: 70, visible: false },
  { id: 'res', label: 'RES', width: 70, visible: true },
  { id: 'shr', label: 'SHR', width: 70, visible: false },
  { id: 'thr', label: 'THR', width: 50, visible: true },
  { id: 'hndl', label: 'HNDL', width: 60, visible: false },
  { id: 'pri', label: 'PRI', width: 45, visible: false },
  { id: 'time', label: 'TIME+', width: 80, visible: true },
  { id: 'status', label: 'S', width: 35, visible: true },
  { id: 'ior', label: 'IO R', width: 70, visible: false },
  { id: 'iow', label: 'IO W', width: 70, visible: false },
]

const useStyles = makeStyles({
  container: {
    display: 'flex',
    flexDirection: 'column',
    height: '100%',
    ...shorthands.gap(tokens.spacingVerticalM),
  },
  toolbar: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
    flexWrap: 'wrap',
  },
  searchBox: {
    width: '280px',
  },
  toggleGroup: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalS),
  },
  tableContainer: {
    flex: 1,
    overflow: 'auto',
    backgroundColor: 'rgba(30, 41, 59, 0.6)',
    backdropFilter: 'blur(10px)',
    ...shorthands.border('1px', 'solid', 'rgba(255, 255, 255, 0.1)'),
    ...shorthands.borderRadius(tokens.borderRadiusLarge),
  },
  table: {
    width: '100%',
    borderCollapse: 'collapse',
    fontSize: tokens.fontSizeBase200,
  },
  tableHeader: {
    position: 'sticky',
    top: 0,
    backgroundColor: 'rgba(30, 41, 59, 0.95)',
    zIndex: 1,
  },
  headerCell: {
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalS),
    textAlign: 'left',
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground2,
    cursor: 'pointer',
    whiteSpace: 'nowrap',
    userSelect: 'none',
    borderBottom: `1px solid rgba(255, 255, 255, 0.1)`,
    ':hover': {
      backgroundColor: 'rgba(255, 255, 255, 0.05)',
    },
  },
  headerCellContent: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
  },
  row: {
    ':hover': {
      backgroundColor: 'rgba(59, 130, 246, 0.1)',
    },
  },
  rowSelected: {
    backgroundColor: 'rgba(59, 130, 246, 0.2)',
  },
  cell: {
    ...shorthands.padding(tokens.spacingVerticalXS, tokens.spacingHorizontalS),
    borderBottom: '1px solid rgba(255, 255, 255, 0.05)',
    whiteSpace: 'nowrap',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
  },
  nameCell: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
  },
  treeToggle: {
    width: '16px',
    height: '16px',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    cursor: 'pointer',
    opacity: 0.7,
    ':hover': {
      opacity: 1,
    },
  },
  cpuHigh: {
    color: '#ef4444',
    fontWeight: tokens.fontWeightSemibold,
  },
  cpuMedium: {
    color: '#f59e0b',
  },
  cpuLow: {
    color: '#10b981',
  },
  statusBadge: {
    fontSize: '10px',
  },
  stats: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalL),
    color: tokens.colorNeutralForeground2,
    fontSize: tokens.fontSizeBase200,
  },
  emptyState: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    height: '300px',
    color: tokens.colorNeutralForeground3,
    ...shorthands.gap(tokens.spacingVerticalM),
  },
})

export const ProcessesTab: React.FC = () => {
  const styles = useStyles()
  const [processes, setProcesses] = useState<ProcessInfo[]>([])
  const [isMonitoring, setIsMonitoring] = useState(false)
  const [searchFilter, setSearchFilter] = useState('')
  const [sortColumn, setSortColumn] = useState<SortColumn>('cpu')
  const [sortAscending, setSortAscending] = useState(false)
  const [treeView, setTreeView] = useState(false)
  const [showSystemProcesses, setShowSystemProcesses] = useState(true)
  const [columns, setColumns] = useState<ColumnConfig[]>(DEFAULT_COLUMNS)
  const [collapsedPids, setCollapsedPids] = useState<Set<number>>(new Set())
  const [selectedPid, setSelectedPid] = useState<number | null>(null)
  const [loading, setLoading] = useState(false)

  // formatBytes and formatCpuTime imported from ../utils/formatters

  // Start monitoring
  const startMonitoring = useCallback(async () => {
    try {
      setLoading(true)
      await invoke('start_monitoring')
      setIsMonitoring(true)
    } catch (error) {
      console.error('Failed to start monitoring:', error)
    } finally {
      setLoading(false)
    }
  }, [])

  // Stop monitoring
  const stopMonitoring = useCallback(async () => {
    try {
      await invoke('stop_monitoring')
      setIsMonitoring(false)
    } catch (error) {
      console.error('Failed to stop monitoring:', error)
    }
  }, [])

  // Fetch current stats
  const fetchStats = useCallback(async () => {
    try {
      const stats = await invoke<SystemStats>('get_current_stats')
      if (stats?.top_processes) {
        setProcesses(stats.top_processes)
      }
    } catch (error) {
      // Monitoring might not be active
      console.debug('Could not fetch stats:', error)
    }
  }, [])

  // Listen for monitoring events
  useEffect(() => {
    let unsubscribe: UnlistenFn | null = null

    const setupListener = async () => {
      unsubscribe = await listen<SystemStats>('system-stats', (event) => {
        if (event.payload?.top_processes) {
          setProcesses(event.payload.top_processes)
        }
      })
    }

    setupListener()

    return () => {
      if (unsubscribe) {
        unsubscribe()
      }
    }
  }, [])

  // Auto-start monitoring when tab opens
  useEffect(() => {
    startMonitoring()
    return () => {
      // Stop monitoring when leaving the tab
      stopMonitoring()
    }
  }, [startMonitoring, stopMonitoring])

  // Filter and sort processes
  const processedData = useMemo(() => {
    let filtered = [...processes]

    // Apply search filter
    if (searchFilter) {
      const query = searchFilter.toLowerCase()
      filtered = filtered.filter(p =>
        p.name.toLowerCase().includes(query) ||
        p.pid.toString().includes(query) ||
        p.command.toLowerCase().includes(query)
      )
    }

    // Filter system processes
    if (!showSystemProcesses) {
      filtered = filtered.filter(p => p.user !== 'SYSTEM' && p.user !== 'NT AUTHORITY\\SYSTEM')
    }

    // Sort
    filtered.sort((a, b) => {
      let cmp = 0
      switch (sortColumn) {
        case 'pid': cmp = a.pid - b.pid; break
        case 'ppid': cmp = a.parent_pid - b.parent_pid; break
        case 'name': cmp = a.name.localeCompare(b.name); break
        case 'cpu': cmp = a.cpu_percent - b.cpu_percent; break
        case 'mem': cmp = a.memory_percent - b.memory_percent; break
        case 'virt': cmp = a.virtual_memory_mb - b.virtual_memory_mb; break
        case 'res': cmp = a.memory_mb - b.memory_mb; break
        case 'shr': cmp = a.shared_memory_mb - b.shared_memory_mb; break
        case 'thr': cmp = a.thread_count - b.thread_count; break
        case 'hndl': cmp = a.handle_count - b.handle_count; break
        case 'pri': cmp = a.priority - b.priority; break
        case 'time': cmp = a.cpu_time_secs - b.cpu_time_secs; break
        case 'status': cmp = a.status.localeCompare(b.status); break
        case 'ior': cmp = a.io_read_bytes - b.io_read_bytes; break
        case 'iow': cmp = a.io_write_bytes - b.io_write_bytes; break
      }
      return sortAscending ? cmp : -cmp
    })

    // Build tree if needed
    if (treeView) {
      // Group by parent
      const childMap = new Map<number, ProcessInfo[]>()
      const roots: ProcessInfo[] = []

      for (const p of filtered) {
        if (p.parent_pid === 0 || !filtered.some(x => x.pid === p.parent_pid)) {
          roots.push(p)
        } else {
          const siblings = childMap.get(p.parent_pid) || []
          siblings.push(p)
          childMap.set(p.parent_pid, siblings)
        }
      }

      // Build flattened tree
      const result: ProcessInfo[] = []
      const addWithChildren = (p: ProcessInfo, depth: number) => {
        result.push({ ...p, tree_depth: depth })
        if (!collapsedPids.has(p.pid)) {
          const children = childMap.get(p.pid) || []
          for (const child of children) {
            addWithChildren(child, depth + 1)
          }
        }
      }

      for (const root of roots) {
        addWithChildren(root, 0)
      }

      return result
    }

    return filtered
  }, [processes, searchFilter, showSystemProcesses, sortColumn, sortAscending, treeView, collapsedPids])

  // Handle sort click
  const handleSort = (column: SortColumn) => {
    if (sortColumn === column) {
      setSortAscending(!sortAscending)
    } else {
      setSortColumn(column)
      setSortAscending(false)
    }
  }

  // Toggle tree node
  const toggleTreeNode = (pid: number) => {
    setCollapsedPids(prev => {
      const next = new Set(prev)
      if (next.has(pid)) {
        next.delete(pid)
      } else {
        next.add(pid)
      }
      return next
    })
  }


  // Get CPU color class
  const getCpuClass = (cpu: number) => {
    if (cpu > 50) return styles.cpuHigh
    if (cpu > 20) return styles.cpuMedium
    return styles.cpuLow
  }

  // Render cell value
  const renderCell = (process: ProcessInfo, column: SortColumn) => {
    switch (column) {
      case 'pid':
        return process.pid
      case 'ppid':
        return process.parent_pid
      case 'name':
        return (
          <div className={styles.nameCell}>
            {treeView && (
              <>
                {/* Indentation */}
                <span style={{ width: `${(process.tree_depth || 0) * 16}px` }} />
                {/* Toggle button */}
                {processes.some(p => p.parent_pid === process.pid) ? (
                  <span
                    className={styles.treeToggle}
                    onClick={(e) => { e.stopPropagation(); toggleTreeNode(process.pid) }}
                  >
                    {collapsedPids.has(process.pid) ? <ChevronRight16Regular /> : <ChevronDown16Regular />}
                  </span>
                ) : (
                  <span style={{ width: '16px' }} />
                )}
              </>
            )}
            <Tooltip content={process.command || process.name} relationship="label">
              <span>{process.name}</span>
            </Tooltip>
          </div>
        )
      case 'cpu':
        return <span className={getCpuClass(process.cpu_percent)}>{process.cpu_percent.toFixed(1)}%</span>
      case 'mem':
        return <span className={getCpuClass(process.memory_percent)}>{process.memory_percent.toFixed(1)}%</span>
      case 'virt':
        return formatBytes(process.virtual_memory_mb * 1024 * 1024)
      case 'res':
        return `${process.memory_mb.toFixed(1)} MB`
      case 'shr':
        return formatBytes(process.shared_memory_mb * 1024 * 1024)
      case 'thr':
        return process.thread_count
      case 'hndl':
        return process.handle_count
      case 'pri':
        return process.priority
      case 'time':
        return formatCpuTime(process.cpu_time_secs)
      case 'status':
        return (
          <Badge
            appearance="filled"
            color={process.status === 'Running' ? 'success' : 'subtle'}
            size="tiny"
            className={styles.statusBadge}
          >
            {process.status.charAt(0)}
          </Badge>
        )
      case 'ior':
        return formatBytes(process.io_read_bytes)
      case 'iow':
        return formatBytes(process.io_write_bytes)
      default:
        return ''
    }
  }

  const visibleColumns = columns.filter(c => c.visible)

  return (
    <div className={styles.container}>
      <PageHeader
        icon={<Apps24Regular />}
        title="Processes"
        description="Real-time process monitoring with htop-like interface"
      />

      {/* Toolbar */}
      <div className={styles.toolbar}>
        <Button
          appearance={isMonitoring ? 'subtle' : 'primary'}
          icon={isMonitoring ? <Pause20Regular /> : <Play20Regular />}
          onClick={isMonitoring ? stopMonitoring : startMonitoring}
          disabled={loading}
        >
          {loading ? <Spinner size="tiny" /> : (isMonitoring ? 'Stop' : 'Start')}
        </Button>

        <Button
          appearance="subtle"
          icon={<ArrowClockwise20Regular />}
          onClick={fetchStats}
          disabled={!isMonitoring}
        >
          Refresh
        </Button>

        <Input
          className={styles.searchBox}
          placeholder="Filter processes..."
          contentBefore={<Search20Regular />}
          value={searchFilter}
          onChange={(_, data) => setSearchFilter(data.value)}
        />

        <div className={styles.toggleGroup}>
          <Switch
            checked={treeView}
            onChange={(_, data) => setTreeView(data.checked)}
            label="Tree View"
          />
        </div>

        <div className={styles.toggleGroup}>
          <Switch
            checked={showSystemProcesses}
            onChange={(_, data) => setShowSystemProcesses(data.checked)}
            label="System Processes"
          />
        </div>

        <Menu
          checkedValues={{ columns: columns.filter(c => c.visible).map(c => c.id) }}
          onCheckedValueChange={(_, data) => {
            if (data.name === 'columns') {
              setColumns(prev => prev.map(c => ({
                ...c,
                visible: data.checkedItems.includes(c.id)
              })))
            }
          }}
        >
          <MenuTrigger disableButtonEnhancement>
            <Button appearance="subtle" icon={<Settings20Regular />}>
              Columns
            </Button>
          </MenuTrigger>
          <MenuPopover>
            <MenuList>
              {columns.map(col => (
                <MenuItemCheckbox
                  key={col.id}
                  name="columns"
                  value={col.id}
                >
                  {col.label}
                </MenuItemCheckbox>
              ))}
            </MenuList>
          </MenuPopover>
        </Menu>

        <div className={styles.stats}>
          <span>{processedData.length} processes</span>
          {isMonitoring && (
            <Badge appearance="filled" color="success" size="small">Live</Badge>
          )}
        </div>
      </div>

      {/* Process Table */}
      <div className={styles.tableContainer}>
        {processedData.length === 0 && !isMonitoring ? (
          <div className={styles.emptyState}>
            <Apps24Regular style={{ fontSize: 48, opacity: 0.5 }} />
            <Text>Click "Start" to begin monitoring processes</Text>
          </div>
        ) : (
          <table className={styles.table}>
            <thead className={styles.tableHeader}>
              <tr>
                {visibleColumns.map(col => (
                  <th
                    key={col.id}
                    className={styles.headerCell}
                    style={{ width: col.width }}
                    onClick={() => handleSort(col.id)}
                  >
                    <div className={styles.headerCellContent}>
                      {col.label}
                      {sortColumn === col.id && (
                        sortAscending ? <ArrowUp16Regular /> : <ArrowDown16Regular />
                      )}
                    </div>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {processedData.map(process => (
                <tr
                  key={process.pid}
                  className={`${styles.row} ${selectedPid === process.pid ? styles.rowSelected : ''}`}
                  onClick={() => setSelectedPid(process.pid)}
                >
                  {visibleColumns.map(col => (
                    <td
                      key={col.id}
                      className={styles.cell}
                      style={{ width: col.width }}
                    >
                      {renderCell(process, col.id)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  )
}

export default ProcessesTab
