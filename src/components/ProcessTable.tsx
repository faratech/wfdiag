import React, { useState } from 'react'
import {
  Table,
  TableHeader,
  TableHeaderCell,
  TableBody,
  TableRow,
  TableCell,
  TableCellLayout,
  Badge,
  Button,
  SearchBox,
  tokens,
  makeStyles,
  shorthands,
  Caption1,
  Body1Strong,
  Tooltip
} from '@fluentui/react-components'
import {
  ArrowUp16Regular,
  ArrowDown16Regular,
  Apps16Regular,
  Delete16Regular,
  Info16Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  container: {
    backgroundColor: 'rgba(30, 41, 59, 0.6)',
    backdropFilter: 'blur(10px)',
    ...shorthands.border('1px', 'solid', 'rgba(255, 255, 255, 0.1)'),
    ...shorthands.borderRadius(tokens.borderRadiusLarge),
    ...shorthands.padding(tokens.spacingVerticalL),
  },
  header: {
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginBottom: tokens.spacingVerticalL,
  },
  searchBox: {
    width: '300px',
  },
  table: {
    backgroundColor: 'transparent',
  },
  tableHeader: {
    backgroundColor: 'rgba(30, 41, 59, 0.5)',
  },
  tableRow: {
    ':hover': {
      backgroundColor: 'rgba(59, 130, 246, 0.1)',
    }
  },
  cpuHigh: {
    color: tokens.colorPaletteRedForeground1,
    fontWeight: tokens.fontWeightSemibold,
  },
  cpuMedium: {
    color: tokens.colorPaletteYellowForeground1,
  },
  cpuLow: {
    color: tokens.colorPaletteGreenForeground1,
  },
  memoryBar: {
    width: '100px',
    height: '8px',
    backgroundColor: 'rgba(255, 255, 255, 0.1)',
    ...shorthands.borderRadius(tokens.borderRadiusSmall),
    ...shorthands.overflow('hidden'),
  },
  memoryFill: {
    height: '100%',
    transition: 'width 0.3s ease',
  },
  actions: {
    display: 'flex',
    ...shorthands.gap(tokens.spacingHorizontalXS),
  },
  processIcon: {
    width: '24px',
    height: '24px',
    ...shorthands.borderRadius(tokens.borderRadiusSmall),
    backgroundColor: 'rgba(59, 130, 246, 0.2)',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
  }
})

export interface ProcessInfo {
  pid: number
  name: string
  cpu_percent: number
  memory_percent: number
  memory_mb: number
  status?: string
  username?: string
  create_time?: number
}

export interface ProcessTableProps {
  processes: ProcessInfo[]
  onKillProcess?: (pid: number) => void
  onRefresh?: () => void
  title?: string
}

export const ProcessTable: React.FC<ProcessTableProps> = ({
  processes,
  onKillProcess,
  onRefresh,
  title = 'Top Processes'
}) => {
  const styles = useStyles()
  const [searchQuery, setSearchQuery] = useState('')
  const [sortConfig, setSortConfig] = useState<{
    key: keyof ProcessInfo
    direction: 'asc' | 'desc'
  }>({ key: 'cpu_percent', direction: 'desc' })

  const handleSort = (key: keyof ProcessInfo) => {
    setSortConfig(prev => ({
      key,
      direction: prev.key === key && prev.direction === 'desc' ? 'asc' : 'desc'
    }))
  }

  const filteredProcesses = processes.filter(process =>
    process.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
    process.pid.toString().includes(searchQuery)
  )

  const sortedProcesses = [...filteredProcesses].sort((a, b) => {
    const aVal = a[sortConfig.key]
    const bVal = b[sortConfig.key]
    const modifier = sortConfig.direction === 'asc' ? 1 : -1

    if (typeof aVal === 'string' && typeof bVal === 'string') {
      return aVal.localeCompare(bVal) * modifier
    }

    return ((aVal as number) - (bVal as number)) * modifier
  })

  const getCpuStyle = (cpu: number) => {
    if (cpu > 50) return styles.cpuHigh
    if (cpu > 20) return styles.cpuMedium
    return styles.cpuLow
  }

  const getMemoryColor = (memory: number) => {
    if (memory > 50) return '#EF4444'
    if (memory > 20) return '#F59E0B'
    return '#10B981'
  }

  const formatMemory = (mb: number) => {
    if (mb < 1024) return `${mb.toFixed(1)} MB`
    return `${(mb / 1024).toFixed(2)} GB`
  }

  const SortIcon: React.FC<{ column: keyof ProcessInfo }> = ({ column }) => {
    if (sortConfig.key !== column) return null
    return sortConfig.direction === 'desc' ?
      <ArrowDown16Regular /> :
      <ArrowUp16Regular />
  }

  return (
    <div className={styles.container}>
      <div className={styles.header}>
        <Body1Strong>{title}</Body1Strong>
        <div style={{ display: 'flex', gap: tokens.spacingHorizontalM }}>
          <SearchBox
            className={styles.searchBox}
            placeholder="Search processes..."
            value={searchQuery}
            onChange={(_, data) => setSearchQuery(data?.value || '')}
            size="small"
          />
          {onRefresh && (
            <Button
              appearance="secondary"
              onClick={onRefresh}
              size="small"
            >
              Refresh
            </Button>
          )}
        </div>
      </div>

      <Table
        className={styles.table}
        size="small"
      >
        <TableHeader className={styles.tableHeader}>
          <TableRow>
            <TableHeaderCell>
              <TableCellLayout>
                Process
              </TableCellLayout>
            </TableHeaderCell>
            <TableHeaderCell>
              <Button
                appearance="transparent"
                icon={<SortIcon column="pid" />}
                iconPosition="after"
                onClick={() => handleSort('pid')}
                size="small"
              >
                PID
              </Button>
            </TableHeaderCell>
            <TableHeaderCell>
              <Button
                appearance="transparent"
                icon={<SortIcon column="cpu_percent" />}
                iconPosition="after"
                onClick={() => handleSort('cpu_percent')}
                size="small"
              >
                CPU %
              </Button>
            </TableHeaderCell>
            <TableHeaderCell>
              <Button
                appearance="transparent"
                icon={<SortIcon column="memory_percent" />}
                iconPosition="after"
                onClick={() => handleSort('memory_percent')}
                size="small"
              >
                Memory
              </Button>
            </TableHeaderCell>
            {sortedProcesses[0]?.status && (
              <TableHeaderCell>Status</TableHeaderCell>
            )}
            {onKillProcess && (
              <TableHeaderCell>Actions</TableHeaderCell>
            )}
          </TableRow>
        </TableHeader>
        <TableBody>
          {sortedProcesses.map(process => (
            <TableRow key={process.pid} className={styles.tableRow}>
              <TableCell>
                <TableCellLayout
                  media={
                    <div className={styles.processIcon}>
                      <Apps16Regular />
                    </div>
                  }
                >
                  <Tooltip content={process.name} relationship="label">
                    <Caption1 style={{
                      maxWidth: '200px',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap'
                    }}>
                      {process.name}
                    </Caption1>
                  </Tooltip>
                </TableCellLayout>
              </TableCell>
              <TableCell>
                <Caption1>{process.pid}</Caption1>
              </TableCell>
              <TableCell>
                <Caption1 className={getCpuStyle(process.cpu_percent)}>
                  {process.cpu_percent.toFixed(1)}%
                </Caption1>
              </TableCell>
              <TableCell>
                <div>
                  <Caption1>{formatMemory(process.memory_mb)}</Caption1>
                  <div className={styles.memoryBar}>
                    <div
                      className={styles.memoryFill}
                      style={{
                        width: `${Math.min(process.memory_percent, 100)}%`,
                        backgroundColor: getMemoryColor(process.memory_percent)
                      }}
                    />
                  </div>
                  <Caption1 style={{ fontSize: tokens.fontSizeBase100 }}>
                    {process.memory_percent.toFixed(1)}%
                  </Caption1>
                </div>
              </TableCell>
              {process.status && (
                <TableCell>
                  <Badge
                    appearance="filled"
                    color={process.status === 'running' ? 'success' : 'subtle'}
                    size="small"
                  >
                    {process.status}
                  </Badge>
                </TableCell>
              )}
              {onKillProcess && (
                <TableCell>
                  <div className={styles.actions}>
                    <Tooltip content="Process Information" relationship="label">
                      <Button
                        appearance="subtle"
                        icon={<Info16Regular />}
                        size="small"
                      />
                    </Tooltip>
                    <Tooltip content="Terminate Process" relationship="label">
                      <Button
                        appearance="subtle"
                        icon={<Delete16Regular />}
                        onClick={() => onKillProcess(process.pid)}
                        size="small"
                        style={{ color: tokens.colorPaletteRedForeground1 }}
                      />
                    </Tooltip>
                  </div>
                </TableCell>
              )}
            </TableRow>
          ))}
        </TableBody>
      </Table>

      {sortedProcesses.length === 0 && (
        <div style={{
          textAlign: 'center',
          padding: tokens.spacingVerticalXXL,
          color: tokens.colorNeutralForeground3
        }}>
          <Caption1>
            {searchQuery ? `No processes found matching "${searchQuery}"` : 'No processes to display'}
          </Caption1>
        </div>
      )}
    </div>
  )
}