import React, { useState } from 'react'
import {
  Text,
  makeStyles,
  shorthands,
  tokens,
  Button,
  Badge,
  ProgressBar,
  Tab,
  TabList
} from '@fluentui/react-components'
import {
  CheckmarkCircle20Filled,
  Warning20Filled,
  ErrorCircle20Filled,
  ChevronDown20Regular,
  ChevronRight20Regular,
  Copy16Regular,
  Code20Regular,
  SlideText20Regular
} from '@fluentui/react-icons'
import { AIInterpretationPanel } from './AIInterpretationPanel'

const useStyles = makeStyles({
  card: {
    backgroundColor: tokens.colorNeutralBackground1,
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke2),
    marginBottom: tokens.spacingVerticalM,
    transition: 'all 0.2s ease',
    ':hover': {
      backgroundColor: tokens.colorNeutralBackground1Hover,
      ...shorthands.borderColor(tokens.colorNeutralStroke1),
    },
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
  },
  cardPrimary: {
    borderLeft: `4px solid ${tokens.colorBrandStroke1}`,
  },
  cardError: {
    borderLeft: `4px solid ${tokens.colorPaletteRedBorder2}`,
    backgroundColor: tokens.colorPaletteRedBackground2,
  },
  cardWarning: {
    borderLeft: `4px solid ${tokens.colorPaletteYellowBorder2}`,
    backgroundColor: tokens.colorPaletteYellowBackground2,
  },
  cardHighlighted: {
    boxShadow: `0 0 0 3px ${tokens.colorBrandStroke1}, ${tokens.shadow16}`,
    transform: 'scale(1.01)',
    transition: 'all 0.3s ease',
  },
  highlightMark: {
    backgroundColor: tokens.colorPaletteYellowBackground2,
    color: tokens.colorNeutralForeground1,
    padding: '0 2px',
    borderRadius: '2px',
    fontWeight: 600,
  },
  header: {
    display: 'flex',
    alignItems: 'center',
    padding: tokens.spacingHorizontalM,
    cursor: 'pointer',
    gap: tokens.spacingHorizontalM,
  },
  titleContainer: {
    flex: 1,
    display: 'flex',
    flexDirection: 'column',
  },
  title: {
    fontSize: tokens.fontSizeBase300,
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground1,
  },
  meta: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
    marginTop: '2px',
  },
  preview: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground2,
    marginTop: '4px',
    maxWidth: '400px',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap' as const,
  },
  statusIndicator: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    marginRight: tokens.spacingHorizontalS,
    flexShrink: 0,
  },
  content: {
    padding: `0 ${tokens.spacingHorizontalM} ${tokens.spacingVerticalM} ${tokens.spacingHorizontalM}`,
    borderTop: `1px solid ${tokens.colorNeutralStroke2}`,
    marginTop: tokens.spacingVerticalS,
    paddingTop: tokens.spacingVerticalM,
  },
  sectionTitle: {
    fontSize: tokens.fontSizeBase200,
    fontWeight: 700,
    color: tokens.colorNeutralForeground3,
    textTransform: 'uppercase',
    letterSpacing: '0.05em',
    marginBottom: tokens.spacingVerticalXS,
    marginTop: tokens.spacingVerticalM,
    display: 'block',
  },
  description: {
    fontSize: tokens.fontSizeBase300,
    color: tokens.colorNeutralForeground2,
    lineHeight: 1.5,
    marginBottom: tokens.spacingVerticalM,
  },
  outputContainer: {
    backgroundColor: tokens.colorNeutralBackgroundAlpha,
    padding: tokens.spacingHorizontalM,
    borderRadius: tokens.borderRadiusMedium,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    overflowX: 'auto',
  },
  summaryContainer: {
    display: 'flex',
    flexDirection: 'column',
    gap: tokens.spacingVerticalM,
  },
  summaryGrid: {
    display: 'grid',
    gridTemplateColumns: 'repeat(auto-fill, minmax(200px, 1fr))',
    gap: tokens.spacingHorizontalL,
  },
  summaryItem: {
    display: 'flex',
    flexDirection: 'column',
    gap: tokens.spacingVerticalXS,
  },
  summaryLabel: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
  },
  summaryValue: {
    fontSize: tokens.fontSizeBase300,
    color: tokens.colorNeutralForeground1,
    fontWeight: tokens.fontWeightSemibold,
    wordBreak: 'break-word',
  },
  dataGrid: {
    display: 'grid',
    gridTemplateColumns: 'minmax(120px, auto) 1fr',
    columnGap: tokens.spacingHorizontalM,
    rowGap: tokens.spacingVerticalXS,
    alignItems: 'start',
  },
  dataLabel: {
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase200,
    fontFamily: 'Consolas, monospace',
  },
  dataValue: {
    color: tokens.colorNeutralForeground1,
    fontSize: tokens.fontSizeBase200,
    fontFamily: 'Consolas, monospace',
    wordBreak: 'break-word',
  },
  aiInsight: {
    background: `linear-gradient(90deg, ${tokens.colorBrandBackground2} 0%, transparent 100%)`,
    borderLeft: `2px solid ${tokens.colorBrandStroke1}`,
    padding: tokens.spacingHorizontalM,
    marginTop: tokens.spacingVerticalM,
    borderRadius: `0 ${tokens.borderRadiusMedium} ${tokens.borderRadiusMedium} 0`,
  },
  aiTitle: {
    display: 'flex',
    alignItems: 'center',
    gap: tokens.spacingHorizontalXS,
    color: tokens.colorBrandForeground1,
    fontWeight: 600,
    fontSize: tokens.fontSizeBase200,
    marginBottom: tokens.spacingVerticalXS,
  }
})

export interface DiagnosticCardProps {
  /** Unique task ID for AI caching */
  taskId?: string
  title: string
  description?: string
  status: 'verified' | 'monitor' | 'action_required'
  importance: 'primary' | 'secondary' | 'informational'
  executionTime?: number
  output?: any
  error?: string
  category?: string
  onCopyOutput?: (output: string) => void
  isHighlighted?: boolean
  highlightText?: string
}

export const DiagnosticCard: React.FC<DiagnosticCardProps> = ({
  taskId,
  title,
  description,
  status,
  importance,
  executionTime,
  output,
  error,
  category,
  onCopyOutput,
  isHighlighted = false,
  highlightText = ''
}) => {
  const styles = useStyles()
  const [expanded, setExpanded] = useState(true)  // Expanded by default
  const [viewMode, setViewMode] = useState<'summary' | 'raw'>('summary')

  const isSuccess = status === 'verified'
  const isError = status === 'action_required'
  const isWarning = status === 'monitor'

  const getCardStyle = () => {
    const highlight = isHighlighted ? ` ${styles.cardHighlighted}` : ''
    if (isError) return `${styles.card} ${styles.cardError}${highlight}`
    if (isWarning) return `${styles.card} ${styles.cardWarning}${highlight}`
    if (importance === 'primary') return `${styles.card} ${styles.cardPrimary}${highlight}`
    return `${styles.card}${highlight}`
  }

  // Highlight matching text in a string
  const highlightMatchingText = (text: string): React.ReactNode => {
    if (!highlightText || !text) return text
    const lowerText = text.toLowerCase()
    const lowerHighlight = highlightText.toLowerCase()
    const index = lowerText.indexOf(lowerHighlight)
    if (index === -1) return text

    const before = text.substring(0, index)
    const match = text.substring(index, index + highlightText.length)
    const after = text.substring(index + highlightText.length)

    return (
      <>
        {before}
        <mark className={styles.highlightMark}>{match}</mark>
        {after}
      </>
    )
  }

  const getStatusIcon = () => {
    if (isError) return <ErrorCircle20Filled primaryFill={tokens.colorPaletteRedForeground1} />
    if (isWarning) return <Warning20Filled primaryFill={tokens.colorPaletteYellowForeground1} />
    return <CheckmarkCircle20Filled primaryFill={tokens.colorPaletteGreenForeground1} />
  }

  // Helper to safely parse output if it's a JSON string
  const getParsedOutput = () => {
    if (typeof output === 'string') {
      try {
        return JSON.parse(output)
      } catch {
        return output
      }
    }
    return output
  }

  const parsedData = getParsedOutput()

  // Generate a brief one-line preview for the header
  const getQuickSummary = (): string => {
    if (error) return error.substring(0, 60) + (error.length > 60 ? '...' : '')
    if (!parsedData) return 'No data'

    try {
      // Handle arrays - show count
      if (Array.isArray(parsedData)) {
        if (parsedData.length === 0) return 'No items found'
        const firstItem = parsedData[0]
        if (firstItem?.Name || firstItem?.Caption || firstItem?.DisplayName) {
          return `${parsedData.length} items: ${firstItem.DisplayName || firstItem.Name || firstItem.Caption}`
        }
        return `${parsedData.length} items`
      }

      // Handle specific data patterns
      if (parsedData.status) return parsedData.status
      if (parsedData.message) return parsedData.message
      if (parsedData.cpu_performance && parsedData.memory_performance) {
        return `Memory: ${parsedData.memory_performance.UsedPercent || 0}% used, CPU: ${parsedData.cpu_performance.NumberOfLogicalProcessors || '?'} cores`
      }
      if (parsedData.count !== undefined && parsedData.tasks) {
        return `${parsedData.count} scheduled tasks`
      }
      if (parsedData.apps) {
        return `${parsedData.apps.length} Store apps`
      }
      if (parsedData.disks || parsedData.disk_drives) {
        const disks = parsedData.disks || parsedData.disk_drives
        return `${disks.length} disk(s) - ${disks[0]?.HealthStatusText || 'OK'}`
      }

      // Fallback: show first key-value or type
      const keys = Object.keys(parsedData)
      if (keys.length > 0 && keys.length < 5) {
        const firstValue = parsedData[keys[0]]
        if (typeof firstValue === 'string' || typeof firstValue === 'number') {
          return `${keys[0]}: ${String(firstValue).substring(0, 40)}`
        }
      }
      return `${keys.length} properties`
    } catch {
      return 'Data available'
    }
  }

  // Specialized Renderers
  const renderDiskSummary = (data: any) => {
    // Handle both direct array (Logical Disks) and object wrapper (Disk Check)
    const disks = Array.isArray(data) ? data : (Array.isArray(data?.disk_drives) ? data.disk_drives : null)
    
    if (!disks || disks.length === 0) return null
    
    return (
      <div className={styles.summaryContainer}>
        {disks.map((disk: any, i: number) => {
          // Fallback for missing size info (common in some WMI queries or readers)
          if (!disk.Size && !disk.TotalSize) {
             return (
                <div key={i} style={{ marginBottom: tokens.spacingVerticalS }}>
                  <Text weight="semibold" block>{disk.DeviceID || disk.Model || 'Unknown Disk'}</Text>
                  <Text size={200} style={{ color: tokens.colorNeutralForeground3 }}>{disk.InterfaceType || disk.MediaType || 'Storage Device'}</Text>
                </div>
             )
          }

          const size = parseInt(disk.Size || disk.TotalSize || '0')
          const free = parseInt(disk.FreeSpace || '0')
          const used = size - free
          const percentUsed = size > 0 ? (used / size) * 100 : 0
          
          return (
            <div key={i} style={{ marginBottom: tokens.spacingVerticalS }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                <Text weight="semibold">{disk.DeviceID} ({disk.FileSystem || 'Unknown'})</Text>
                <Text>{(free / 1024 / 1024 / 1024).toFixed(1)} GB free of {(size / 1024 / 1024 / 1024).toFixed(1)} GB</Text>
              </div>
              <ProgressBar 
                value={percentUsed / 100} 
                color={percentUsed > 90 ? 'error' : percentUsed > 75 ? 'warning' : 'brand'}
              />
            </div>
          )
        })}
      </div>
    )
  }

  const renderBatterySummary = (data: any) => {
    if (!data?.battery_summary?.batteries) return null
    
    const health = data.battery_summary.battery_health_percentage
    
    return (
      <div className={styles.summaryContainer}>
        <div style={{ display: 'flex', gap: tokens.spacingHorizontalL, alignItems: 'center' }}>
           {health !== undefined && (
             <div style={{ textAlign: 'center' }}>
               <Text size={500} block weight="bold" style={{ color: health > 80 ? tokens.colorPaletteGreenForeground1 : tokens.colorPaletteYellowForeground1 }}>
                 {health}%
               </Text>
               <Text size={200} style={{ color: tokens.colorNeutralForeground3 }}>Health</Text>
             </div>
           )}
           <div className={styles.summaryGrid} style={{ flex: 1 }}>
             {data.battery_summary.batteries.map((b: any, i: number) => (
               <div key={i} className={styles.summaryItem}>
                 <span className={styles.summaryLabel}>{b.property}</span>
                 <span className={styles.summaryValue}>{b.value}</span>
               </div>
             ))}
           </div>
        </div>
      </div>
    )
  }

  const renderSystemInfoSummary = (data: any) => {
    if (!data?.os_version || !data?.computer_system) return null
    
    const os = data.os_version
    const sys = data.computer_system
    
    return (
      <div className={styles.summaryGrid}>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>OS Version</span>
          <span className={styles.summaryValue}>{os.Caption || os.windows_version}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Build</span>
          <span className={styles.summaryValue}>{os.BuildNumber}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>System Model</span>
          <span className={styles.summaryValue}>{sys.Model}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Manufacturer</span>
          <span className={styles.summaryValue}>{sys.Manufacturer}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Total Memory</span>
          <span className={styles.summaryValue}>{sys.TotalPhysicalMemory ? `${(parseInt(sys.TotalPhysicalMemory) / 1024 / 1024 / 1024).toFixed(1)} GB` : 'N/A'}</span>
        </div>
      </div>
    )
  }
  
  const renderNetworkSummary = (data: any) => {
    if (!Array.isArray(data)) return null
    const activeAdapters = data.filter((a: any) => a.IPEnabled === true)
    
    if (activeAdapters.length === 0) return <Text>No active network adapters found.</Text>

    return (
      <div className={styles.summaryContainer}>
        {activeAdapters.map((adapter: any, i: number) => (
          <div key={i} style={{ padding: tokens.spacingHorizontalS, borderLeft: `2px solid ${tokens.colorBrandStroke1}`, backgroundColor: tokens.colorNeutralBackgroundAlpha }}>
            <Text weight="semibold" block>{adapter.Description}</Text>
            <div className={styles.summaryGrid} style={{ marginTop: tokens.spacingVerticalS }}>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>IPv4 Address</span>
                <span className={styles.summaryValue}>{Array.isArray(adapter.IPAddress) ? adapter.IPAddress[0] : (adapter.IPAddress || 'N/A')}</span>
              </div>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>MAC Address</span>
                <span className={styles.summaryValue}>{adapter.MACAddress || 'N/A'}</span>
              </div>
               <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>DHCP Enabled</span>
                <span className={styles.summaryValue}>{adapter.DHCPEnabled ? 'Yes' : 'No'}</span>
              </div>
            </div>
          </div>
        ))}
      </div>
    )
  }

  const renderProcessorSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return null
    const cpu = data[0]
    return (
      <div className={styles.summaryGrid}>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Name</span>
          <span className={styles.summaryValue}>{cpu.Name}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Cores</span>
          <span className={styles.summaryValue}>{cpu.NumberOfCores || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Logical Processors</span>
          <span className={styles.summaryValue}>{cpu.NumberOfLogicalProcessors || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Clock Speed</span>
          <span className={styles.summaryValue}>{cpu.MaxClockSpeed ? `${(cpu.MaxClockSpeed / 1000).toFixed(2)} GHz` : 'N/A'}</span>
        </div>
      </div>
    )
  }

  const renderMemorySummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return null
    
    // Calculate total capacity
    let totalCapacity = 0
    data.forEach((dimm: any) => {
      if (dimm.Capacity) {
        totalCapacity += parseInt(dimm.Capacity)
      }
    })
    
    return (
      <div className={styles.summaryContainer}>
        <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalL, marginBottom: tokens.spacingVerticalM }}>
          <div style={{ textAlign: 'center' }}>
            <Text size={600} block weight="bold">{(totalCapacity / 1024 / 1024 / 1024).toFixed(0)} GB</Text>
            <Text size={200} style={{ color: tokens.colorNeutralForeground3 }}>Total RAM</Text>
          </div>
          <div style={{ flex: 1 }}>
            <Text block weight="semibold" style={{ marginBottom: tokens.spacingVerticalS }}>Installed Modules</Text>
            {data.map((dimm: any, i: number) => (
              <div key={i} style={{ display: 'flex', justifyContent: 'space-between', borderBottom: `1px solid ${tokens.colorNeutralStroke2}`, padding: tokens.spacingVerticalXS }}>
                <Text>{dimm.Manufacturer || 'Unknown'} {dimm.PartNumber}</Text>
                <Text>{dimm.Capacity ? `${(parseInt(dimm.Capacity) / 1024 / 1024 / 1024).toFixed(0)} GB` : 'N/A'} @ {dimm.Speed || '?'} MHz</Text>
              </div>
            ))}
          </div>
        </div>
      </div>
    )
  }

  const renderGraphicsSummary = (data: any) => {
    if (!data?.video_controllers) return null
    
    return (
      <div className={styles.summaryContainer}>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>DirectX Version</span>
          <span className={styles.summaryValue}>{data.directx_version || 'Unknown'}</span>
        </div>
        
        {Array.isArray(data.video_controllers) && data.video_controllers.map((gpu: any, i: number) => (
          <div key={i} style={{ padding: tokens.spacingHorizontalS, borderLeft: `2px solid ${tokens.colorBrandStroke1}`, backgroundColor: tokens.colorNeutralBackgroundAlpha }}>
            <Text weight="semibold" block>{gpu.Name}</Text>
            <div className={styles.summaryGrid} style={{ marginTop: tokens.spacingVerticalS }}>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Driver Version</span>
                <span className={styles.summaryValue}>{gpu.DriverVersion}</span>
              </div>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>VRAM</span>
                <span className={styles.summaryValue}>
                  {/* Handle 0 bytes as Shared/Integrated for SoC/iGPU */}
                  {gpu.AdapterRAM && parseInt(gpu.AdapterRAM) > 0 
                    ? `${(parseInt(gpu.AdapterRAM) / 1024 / 1024 / 1024).toFixed(1)} GB` 
                    : 'Shared / Integrated'}
                </span>
              </div>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Resolution</span>
                <span className={styles.summaryValue}>
                  {gpu.CurrentHorizontalResolution && gpu.CurrentVerticalResolution 
                    ? `${gpu.CurrentHorizontalResolution}x${gpu.CurrentVerticalResolution} @ ${gpu.CurrentRefreshRate}Hz` 
                    : 'N/A'}
                </span>
              </div>
            </div>
          </div>
        ))}
      </div>
    )
  }

  const renderEventLogSummary = (data: any) => {
    if (!Array.isArray(data)) return null
    
    // Count errors by source
    const errorsBySource: Record<string, number> = {}
    data.forEach((event: any) => {
      const source = event.SourceName || 'Unknown'
      errorsBySource[source] = (errorsBySource[source] || 0) + 1
    })
    
    const topSources = Object.entries(errorsBySource)
      .sort(([, a], [, b]) => b - a)
      .slice(0, 5)
      
    return (
      <div className={styles.summaryContainer}>
        <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalM }}>
          <Badge appearance="filled" color="danger">{data.length}</Badge>
          <Text weight="semibold">Recent Critical/Error Events</Text>
        </div>
        
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 100px', gap: tokens.spacingHorizontalM, marginTop: tokens.spacingVerticalS }}>
          <Text weight="semibold" style={{ color: tokens.colorNeutralForeground3 }}>Source</Text>
          <Text weight="semibold" style={{ color: tokens.colorNeutralForeground3 }}>Count</Text>
          
          {topSources.map(([source, count], i) => (
            <React.Fragment key={i}>
              <Text>{source}</Text>
              <Text>{count}</Text>
            </React.Fragment>
          ))}
        </div>
        
        <Text size={200} style={{ color: tokens.colorNeutralForeground3, marginTop: tokens.spacingVerticalS }}>
          Review raw data for specific event IDs and messages.
        </Text>
      </div>
    )
  }

  const renderServicesSummary = (data: any) => {
    if (!Array.isArray(data)) return null
    
    const running = data.filter((s: any) => s.State === 'Running').length
    const stopped = data.filter((s: any) => s.State === 'Stopped').length
    
    // Find stopped services that are set to auto start
    const stoppedAuto = data.filter((s: any) => s.State === 'Stopped' && s.StartMode === 'Auto').slice(0, 5)
    
    return (
      <div className={styles.summaryContainer}>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Running</span>
            <span className={styles.summaryValue} style={{ color: tokens.colorPaletteGreenForeground1 }}>{running}</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Stopped</span>
            <span className={styles.summaryValue} style={{ color: tokens.colorNeutralForeground3 }}>{stopped}</span>
          </div>
        </div>
        
        {stoppedAuto.length > 0 && (
          <div style={{ marginTop: tokens.spacingVerticalM }}>
            <Text weight="semibold" style={{ color: tokens.colorPaletteYellowForeground1 }}>Warning: Stopped Automatic Services</Text>
            <ul style={{ marginTop: tokens.spacingVerticalXS, paddingLeft: tokens.spacingHorizontalL }}>
              {stoppedAuto.map((s: any, i: number) => (
                <li key={i}><Text>{s.DisplayName || s.Name}</Text></li>
              ))}
            </ul>
          </div>
        )}
      </div>
    )
  }

  const renderWindowsUpdateSummary = (data: any) => {
    if (!data?.installed_updates && !data?.hotfix_details) return null
    
    const updates = data.installed_updates || (Array.isArray(data.hotfix_details) ? data.hotfix_details : [data.hotfix_details])
    if (!Array.isArray(updates)) return null
    
    const recent = updates.slice(0, 5)
    
    return (
      <div className={styles.summaryContainer}>
        <Text weight="semibold">Most Recent Updates</Text>
        {recent.map((update: any, i: number) => (
          <div key={i} className={styles.summaryItem} style={{ borderBottom: `1px solid ${tokens.colorNeutralStroke2}`, paddingBottom: tokens.spacingVerticalXS }}>
            <div style={{ display: 'flex', justifyContent: 'space-between' }}>
              <span className={styles.summaryValue}>{update.HotFixID}</span>
              <span className={styles.summaryLabel}>{update.InstalledOn}</span>
            </div>
            <span className={styles.summaryLabel}>{update.Description}</span>
          </div>
        ))}
      </div>
    )
  }

  const renderProcessesSummary = (data: any) => {
    if (!Array.isArray(data)) return null

    const topMemory = [...data].sort((a, b) => parseInt(b.WorkingSetSize || 0) - parseInt(a.WorkingSetSize || 0)).slice(0, 5)

    return (
      <div className={styles.summaryContainer}>
        <Text weight="semibold">Top Memory Consumers</Text>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 100px', gap: tokens.spacingHorizontalM }}>
          {topMemory.map((proc: any, i: number) => (
            <React.Fragment key={i}>
              <Text>{proc.Name}</Text>
              <Text>{proc.WorkingSetSize ? `${(parseInt(proc.WorkingSetSize) / 1024 / 1024).toFixed(0)} MB` : 'N/A'}</Text>
            </React.Fragment>
          ))}
        </div>
        <div style={{ marginTop: tokens.spacingVerticalM }}>
           <Text>Total Processes: {data.length}</Text>
        </div>
      </div>
    )
  }

  const renderBiosSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return null
    const bios = data[0]
    return (
      <div className={styles.summaryGrid}>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Manufacturer</span>
          <span className={styles.summaryValue}>{bios.Manufacturer || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Version</span>
          <span className={styles.summaryValue}>{bios.SMBIOSBIOSVersion || bios.Version || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Release Date</span>
          <span className={styles.summaryValue}>{bios.ReleaseDate ? bios.ReleaseDate.substring(0, 8) : 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>SMBIOS Version</span>
          <span className={styles.summaryValue}>{bios.SMBIOSMajorVersion ? `${bios.SMBIOSMajorVersion}.${bios.SMBIOSMinorVersion}` : 'N/A'}</span>
        </div>
      </div>
    )
  }

  const renderMotherboardSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return null
    const board = data[0]
    return (
      <div className={styles.summaryGrid}>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Manufacturer</span>
          <span className={styles.summaryValue}>{board.Manufacturer || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Product</span>
          <span className={styles.summaryValue}>{board.Product || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Serial Number</span>
          <span className={styles.summaryValue}>{board.SerialNumber || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Version</span>
          <span className={styles.summaryValue}>{board.Version || 'N/A'}</span>
        </div>
      </div>
    )
  }

  const renderDiskDrivesSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return null
    return (
      <div className={styles.summaryContainer}>
        {data.map((disk: any, i: number) => (
          <div key={i} style={{ padding: tokens.spacingHorizontalS, borderLeft: `2px solid ${tokens.colorBrandStroke1}`, backgroundColor: tokens.colorNeutralBackgroundAlpha, marginBottom: tokens.spacingVerticalS }}>
            <Text weight="semibold" block>{disk.Model || disk.Caption || 'Unknown Disk'}</Text>
            <div className={styles.summaryGrid} style={{ marginTop: tokens.spacingVerticalS }}>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Size</span>
                <span className={styles.summaryValue}>{disk.Size ? `${(parseInt(disk.Size) / 1024 / 1024 / 1024).toFixed(0)} GB` : 'N/A'}</span>
              </div>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Interface</span>
                <span className={styles.summaryValue}>{disk.InterfaceType || 'N/A'}</span>
              </div>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Media Type</span>
                <span className={styles.summaryValue}>{disk.MediaType || 'N/A'}</span>
              </div>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Partitions</span>
                <span className={styles.summaryValue}>{disk.Partitions || 'N/A'}</span>
              </div>
            </div>
          </div>
        ))}
      </div>
    )
  }

  const renderDiskPartitionsSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return null
    return (
      <div className={styles.summaryContainer}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 100px 100px 80px', gap: tokens.spacingHorizontalS, fontWeight: 600, marginBottom: tokens.spacingVerticalS, color: tokens.colorNeutralForeground3 }}>
          <Text>Name</Text>
          <Text>Size</Text>
          <Text>Type</Text>
          <Text>Boot</Text>
        </div>
        {data.slice(0, 10).map((part: any, i: number) => (
          <div key={i} style={{ display: 'grid', gridTemplateColumns: '1fr 100px 100px 80px', gap: tokens.spacingHorizontalS, padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
            <Text>{part.Name || part.DeviceID || 'Unknown'}</Text>
            <Text>{part.Size ? `${(parseInt(part.Size) / 1024 / 1024 / 1024).toFixed(1)} GB` : 'N/A'}</Text>
            <Text>{part.Type || 'N/A'}</Text>
            <Badge appearance="tint" color={part.BootPartition ? 'success' : 'subtle'} size="small">{part.BootPartition ? 'Yes' : 'No'}</Badge>
          </div>
        ))}
        {data.length > 10 && <Text size={200} style={{ marginTop: tokens.spacingVerticalS, color: tokens.colorNeutralForeground3 }}>...and {data.length - 10} more partitions</Text>}
      </div>
    )
  }

  const renderEnvironmentSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return null
    const systemVars = data.filter((v: any) => v.SystemVariable === true || v.UserName === '<SYSTEM>')
    const importantVars = ['PATH', 'TEMP', 'TMP', 'USERPROFILE', 'APPDATA', 'ProgramFiles', 'SystemRoot', 'ComSpec']
    const highlighted = data.filter((v: any) => importantVars.some(iv => v.Name?.toUpperCase().includes(iv)))

    return (
      <div className={styles.summaryContainer}>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Total Variables</span>
            <span className={styles.summaryValue}>{data.length}</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>System Variables</span>
            <span className={styles.summaryValue}>{systemVars.length}</span>
          </div>
        </div>
        {highlighted.length > 0 && (
          <div style={{ marginTop: tokens.spacingVerticalM }}>
            <Text weight="semibold" block style={{ marginBottom: tokens.spacingVerticalS }}>Key Variables</Text>
            {highlighted.slice(0, 5).map((v: any, i: number) => (
              <div key={i} style={{ display: 'flex', gap: tokens.spacingHorizontalM, padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
                <Text style={{ minWidth: 120, color: tokens.colorNeutralForeground3 }}>{v.Name}</Text>
                <Text style={{ wordBreak: 'break-all' }}>{v.VariableValue?.substring(0, 100)}{v.VariableValue?.length > 100 ? '...' : ''}</Text>
              </div>
            ))}
          </div>
        )}
      </div>
    )
  }

  const renderStartupSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return <Text>No startup commands found.</Text>
    return (
      <div className={styles.summaryContainer}>
        <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalM, marginBottom: tokens.spacingVerticalM }}>
          <Badge appearance="filled" color="informative">{data.length}</Badge>
          <Text weight="semibold">Startup Programs</Text>
        </div>
        {data.slice(0, 8).map((item: any, i: number) => (
          <div key={i} style={{ padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
            <Text weight="semibold" block>{item.Name || item.Caption || 'Unknown'}</Text>
            <Text size={200} style={{ color: tokens.colorNeutralForeground3 }}>{item.Command?.substring(0, 80)}{item.Command?.length > 80 ? '...' : ''}</Text>
          </div>
        ))}
        {data.length > 8 && <Text size={200} style={{ marginTop: tokens.spacingVerticalS, color: tokens.colorNeutralForeground3 }}>...and {data.length - 8} more</Text>}
      </div>
    )
  }

  const renderInstalledProgramsSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return <Text>No installed programs found.</Text>
    const sorted = [...data].sort((a, b) => (a.DisplayName || '').localeCompare(b.DisplayName || ''))
    return (
      <div className={styles.summaryContainer}>
        <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalM, marginBottom: tokens.spacingVerticalM }}>
          <Badge appearance="filled" color="informative">{data.length}</Badge>
          <Text weight="semibold">Installed Programs</Text>
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 100px 120px', gap: tokens.spacingHorizontalS, fontWeight: 600, marginBottom: tokens.spacingVerticalS, color: tokens.colorNeutralForeground3 }}>
          <Text>Name</Text>
          <Text>Version</Text>
          <Text>Publisher</Text>
        </div>
        {sorted.slice(0, 10).map((prog: any, i: number) => (
          <div key={i} style={{ display: 'grid', gridTemplateColumns: '1fr 100px 120px', gap: tokens.spacingHorizontalS, padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
            <Text style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{prog.DisplayName}</Text>
            <Text size={200}>{prog.DisplayVersion || '-'}</Text>
            <Text size={200} style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{prog.Publisher || '-'}</Text>
          </div>
        ))}
        {data.length > 10 && <Text size={200} style={{ marginTop: tokens.spacingVerticalS, color: tokens.colorNeutralForeground3 }}>...and {data.length - 10} more programs</Text>}
      </div>
    )
  }

  const renderDriverListSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return null
    const signed = data.filter((d: any) => d.IsSigned === true)
    const unsigned = data.filter((d: any) => d.IsSigned === false)

    return (
      <div className={styles.summaryContainer}>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Total Drivers</span>
            <span className={styles.summaryValue}>{data.length}</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Signed</span>
            <span className={styles.summaryValue} style={{ color: tokens.colorPaletteGreenForeground1 }}>{signed.length}</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Unsigned</span>
            <span className={styles.summaryValue} style={{ color: unsigned.length > 0 ? tokens.colorPaletteYellowForeground1 : undefined }}>{unsigned.length}</span>
          </div>
        </div>
        {unsigned.length > 0 && (
          <div style={{ marginTop: tokens.spacingVerticalM }}>
            <Text weight="semibold" style={{ color: tokens.colorPaletteYellowForeground1 }}>Unsigned Drivers</Text>
            <ul style={{ marginTop: tokens.spacingVerticalXS, paddingLeft: tokens.spacingHorizontalL }}>
              {unsigned.slice(0, 5).map((d: any, i: number) => (
                <li key={i}><Text>{d.DeviceName || d.Name}</Text></li>
              ))}
            </ul>
          </div>
        )}
      </div>
    )
  }

  const renderScheduledTasksSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return <Text>No scheduled tasks found.</Text>
    const running = data.filter((t: any) => t.State === 'Running')
    const ready = data.filter((t: any) => t.State === 'Ready')

    return (
      <div className={styles.summaryContainer}>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Total Tasks</span>
            <span className={styles.summaryValue}>{data.length}</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Running</span>
            <span className={styles.summaryValue} style={{ color: tokens.colorPaletteGreenForeground1 }}>{running.length}</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Ready</span>
            <span className={styles.summaryValue}>{ready.length}</span>
          </div>
        </div>
        <div style={{ marginTop: tokens.spacingVerticalM }}>
          <Text weight="semibold" block style={{ marginBottom: tokens.spacingVerticalS }}>Recent Tasks</Text>
          {data.slice(0, 6).map((task: any, i: number) => (
            <div key={i} style={{ display: 'flex', justifyContent: 'space-between', padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
              <Text style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1 }}>{task.TaskName}</Text>
              <Badge appearance="tint" color={task.State === 'Running' ? 'success' : task.State === 'Ready' ? 'informative' : 'subtle'} size="small">{task.State}</Badge>
            </div>
          ))}
        </div>
      </div>
    )
  }

  const renderDismHealthSummary = (data: any) => {
    if (!data) return null
    const isHealthy = data.status === 'Healthy'
    const isRepairable = data.repairable === true

    return (
      <div className={styles.summaryContainer}>
        <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalM, marginBottom: tokens.spacingVerticalM }}>
          <Badge appearance="filled" color={isHealthy ? 'success' : isRepairable ? 'warning' : 'danger'} size="large">
            {data.status || 'Unknown'}
          </Badge>
          <Text>{data.message || 'Windows image status'}</Text>
        </div>
        {data.error && (
          <div style={{ padding: tokens.spacingHorizontalS, backgroundColor: tokens.colorPaletteYellowBackground2, borderRadius: tokens.borderRadiusSmall }}>
            <Text>{data.error}</Text>
            {data.suggestion && <Text size={200} block style={{ marginTop: tokens.spacingVerticalXS }}>{data.suggestion}</Text>}
          </div>
        )}
      </div>
    )
  }

  const renderMinidumpsSummary = (data: any) => {
    const dumps = data?.dumps || []
    if (dumps.length === 0) {
      return (
        <div className={styles.summaryContainer}>
          <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalM }}>
            <Badge appearance="filled" color="success">0</Badge>
            <Text weight="semibold">No BSOD crashes detected</Text>
          </div>
          <Text size={200} style={{ color: tokens.colorNeutralForeground3, marginTop: tokens.spacingVerticalS }}>{data?.message || 'No minidump files found'}</Text>
        </div>
      )
    }

    return (
      <div className={styles.summaryContainer}>
        <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalM, marginBottom: tokens.spacingVerticalM }}>
          <Badge appearance="filled" color="danger">{dumps.length}</Badge>
          <Text weight="semibold">BSOD Crash Dumps Found</Text>
        </div>
        {dumps.slice(0, 5).map((dump: any, i: number) => (
          <div key={i} style={{ display: 'flex', justifyContent: 'space-between', padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
            <Text>{dump.filename}</Text>
            <Text size={200} style={{ color: tokens.colorNeutralForeground3 }}>{dump.size ? `${(dump.size / 1024).toFixed(0)} KB` : ''}</Text>
          </div>
        ))}
        {data.can_copy && <Text size={200} style={{ marginTop: tokens.spacingVerticalS, color: tokens.colorNeutralForeground3 }}>Dumps can be copied to Desktop for analysis</Text>}
      </div>
    )
  }

  const renderHostsFileSummary = (data: any) => {
    if (!data) return null
    const entries = data.entries || []
    const customEntries = entries.filter((e: any) => e.ip !== '127.0.0.1' && e.ip !== '::1')

    return (
      <div className={styles.summaryContainer}>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Total Entries</span>
            <span className={styles.summaryValue}>{entries.length}</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Custom Entries</span>
            <span className={styles.summaryValue}>{customEntries.length}</span>
          </div>
        </div>
        {entries.length > 0 && (
          <div style={{ marginTop: tokens.spacingVerticalM }}>
            <div style={{ display: 'grid', gridTemplateColumns: '120px 1fr', gap: tokens.spacingHorizontalM, fontWeight: 600, marginBottom: tokens.spacingVerticalS, color: tokens.colorNeutralForeground3 }}>
              <Text>IP Address</Text>
              <Text>Hostname</Text>
            </div>
            {entries.slice(0, 8).map((entry: any, i: number) => (
              <div key={i} style={{ display: 'grid', gridTemplateColumns: '120px 1fr', gap: tokens.spacingHorizontalM, padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
                <Text style={{ fontFamily: 'Consolas, monospace' }}>{entry.ip}</Text>
                <Text>{entry.hostname}</Text>
              </div>
            ))}
          </div>
        )}
        {data.error && <Text style={{ color: tokens.colorPaletteRedForeground1 }}>{data.error}</Text>}
      </div>
    )
  }

  const renderDomainSummary = (data: any) => {
    if (!data) return null
    const isJoined = data.PartOfDomain === true
    const roleMap: Record<number, string> = {
      0: 'Standalone Workstation',
      1: 'Member Workstation',
      2: 'Standalone Server',
      3: 'Member Server',
      4: 'Backup Domain Controller',
      5: 'Primary Domain Controller'
    }

    return (
      <div className={styles.summaryContainer}>
        <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalM, marginBottom: tokens.spacingVerticalM }}>
          <Badge appearance="filled" color={isJoined ? 'success' : 'informative'}>
            {isJoined ? 'Domain Joined' : 'Workgroup'}
          </Badge>
        </div>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Domain/Workgroup</span>
            <span className={styles.summaryValue}>{data.Domain || 'N/A'}</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Role</span>
            <span className={styles.summaryValue}>{roleMap[data.DomainRole] || `Role ${data.DomainRole}`}</span>
          </div>
        </div>
      </div>
    )
  }

  const renderFirewallSummary = (data: any) => {
    if (!Array.isArray(data)) return <Text>Firewall information unavailable.</Text>
    if (data.length === 0) return <Text>No firewall products detected via WMI SecurityCenter.</Text>

    return (
      <div className={styles.summaryContainer}>
        {data.map((fw: any, i: number) => (
          <div key={i} style={{ padding: tokens.spacingHorizontalS, borderLeft: `2px solid ${tokens.colorBrandStroke1}`, backgroundColor: tokens.colorNeutralBackgroundAlpha, marginBottom: tokens.spacingVerticalS }}>
            <Text weight="semibold" block>{fw.displayName || 'Unknown Firewall'}</Text>
            <div className={styles.summaryGrid} style={{ marginTop: tokens.spacingVerticalS }}>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>State</span>
                <Badge appearance="tint" color={fw.productState ? 'success' : 'subtle'} size="small">
                  {fw.productState ? 'Active' : 'Unknown'}
                </Badge>
              </div>
            </div>
          </div>
        ))}
      </div>
    )
  }

  const renderPerformanceSummary = (data: any) => {
    if (!data) return null

    return (
      <div className={styles.summaryContainer}>
        {data.cpu_performance && (
          <div style={{ marginBottom: tokens.spacingVerticalM }}>
            <Text weight="semibold" block style={{ marginBottom: tokens.spacingVerticalS }}>CPU</Text>
            <div className={styles.summaryGrid}>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Processor</span>
                <span className={styles.summaryValue}>{data.cpu_performance.Name || 'N/A'}</span>
              </div>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Load</span>
                <span className={styles.summaryValue}>{data.cpu_performance.LoadPercentage || data.cpu_performance.PercentProcessorTime || '0'}%</span>
              </div>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Cores</span>
                <span className={styles.summaryValue}>{data.cpu_performance.NumberOfCores || 'N/A'}</span>
              </div>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Logical CPUs</span>
                <span className={styles.summaryValue}>{data.cpu_performance.NumberOfLogicalProcessors || 'N/A'}</span>
              </div>
            </div>
          </div>
        )}
        {data.memory_performance && (
          <div style={{ marginBottom: tokens.spacingVerticalM }}>
            <Text weight="semibold" block style={{ marginBottom: tokens.spacingVerticalS }}>Memory</Text>
            <div className={styles.summaryGrid}>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Used</span>
                <span className={styles.summaryValue}>{data.memory_performance.UsedPercent || '0'}%</span>
              </div>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Available</span>
                <span className={styles.summaryValue}>{data.memory_performance.AvailableMBytes ? `${data.memory_performance.AvailableMBytes} MB` : 'N/A'}</span>
              </div>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Total</span>
                <span className={styles.summaryValue}>{data.memory_performance.TotalMBytes ? `${(data.memory_performance.TotalMBytes / 1024).toFixed(1)} GB` : 'N/A'}</span>
              </div>
            </div>
          </div>
        )}
        {data.disk_performance && (
          <div>
            <Text weight="semibold" block style={{ marginBottom: tokens.spacingVerticalS }}>Disk</Text>
            {Array.isArray(data.disk_performance) ? (
              data.disk_performance.map((disk: any, i: number) => (
                <div key={i} style={{ display: 'flex', justifyContent: 'space-between', padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
                  <Text>{disk.DeviceID}</Text>
                  <Text>{disk.FreeSpace && disk.Size ? `${(parseInt(disk.FreeSpace) / 1024 / 1024 / 1024).toFixed(0)} GB free of ${(parseInt(disk.Size) / 1024 / 1024 / 1024).toFixed(0)} GB` : 'N/A'}</Text>
                </div>
              ))
            ) : (
              <div className={styles.summaryGrid}>
                <div className={styles.summaryItem}>
                  <span className={styles.summaryLabel}>Disk Time</span>
                  <span className={styles.summaryValue}>{data.disk_performance.PercentDiskTime || '0'}%</span>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    )
  }

  const renderComputerSystemSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return null
    const sys = data[0]
    return (
      <div className={styles.summaryGrid}>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Computer Name</span>
          <span className={styles.summaryValue}>{sys.Name || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Manufacturer</span>
          <span className={styles.summaryValue}>{sys.Manufacturer || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Model</span>
          <span className={styles.summaryValue}>{sys.Model || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>System Type</span>
          <span className={styles.summaryValue}>{sys.SystemType || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Total Memory</span>
          <span className={styles.summaryValue}>{sys.TotalPhysicalMemory ? `${(parseInt(sys.TotalPhysicalMemory) / 1024 / 1024 / 1024).toFixed(1)} GB` : 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Domain</span>
          <span className={styles.summaryValue}>{sys.Domain || 'N/A'}</span>
        </div>
      </div>
    )
  }

  const renderOsSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return null
    const os = data[0]
    return (
      <div className={styles.summaryGrid}>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>OS Name</span>
          <span className={styles.summaryValue}>{os.Caption || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Version</span>
          <span className={styles.summaryValue}>{os.Version || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Build</span>
          <span className={styles.summaryValue}>{os.BuildNumber || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Architecture</span>
          <span className={styles.summaryValue}>{os.OSArchitecture || 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Install Date</span>
          <span className={styles.summaryValue}>{os.InstallDate ? os.InstallDate.substring(0, 8) : 'N/A'}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Last Boot</span>
          <span className={styles.summaryValue}>{os.LastBootUpTime ? os.LastBootUpTime.substring(0, 14) : 'N/A'}</span>
        </div>
      </div>
    )
  }

  const renderPrintersSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return <Text>No printers found.</Text>
    const defaultPrinter = data.find((p: any) => p.Default === true)

    return (
      <div className={styles.summaryContainer}>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Total Printers</span>
            <span className={styles.summaryValue}>{data.length}</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Default Printer</span>
            <span className={styles.summaryValue}>{defaultPrinter?.Name || 'None'}</span>
          </div>
        </div>
        <div style={{ marginTop: tokens.spacingVerticalM }}>
          {data.slice(0, 5).map((printer: any, i: number) => (
            <div key={i} style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
              <Text>{printer.Name}</Text>
              <div style={{ display: 'flex', gap: tokens.spacingHorizontalS }}>
                {printer.Default && <Badge appearance="tint" color="brand" size="small">Default</Badge>}
                <Badge appearance="tint" color={printer.WorkOffline ? 'warning' : 'success'} size="small">
                  {printer.WorkOffline ? 'Offline' : 'Ready'}
                </Badge>
              </div>
            </div>
          ))}
        </div>
      </div>
    )
  }

  const renderStoreAppsSummary = (data: any) => {
    if (!Array.isArray(data) && data?.raw_output) {
      return <Text>Store apps data available in raw format.</Text>
    }
    if (!Array.isArray(data) || data.length === 0) return <Text>No Store apps found.</Text>

    // Filter to show only interesting apps (not framework packages)
    const userApps = data.filter((app: any) =>
      !app.Name?.includes('Microsoft.NET') &&
      !app.Name?.includes('Microsoft.VCLibs') &&
      !app.Name?.includes('Microsoft.UI') &&
      !app.Name?.includes('.Framework')
    )

    return (
      <div className={styles.summaryContainer}>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Total Packages</span>
            <span className={styles.summaryValue}>{data.length}</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>User Apps</span>
            <span className={styles.summaryValue}>{userApps.length}</span>
          </div>
        </div>
        <div style={{ marginTop: tokens.spacingVerticalM }}>
          <Text weight="semibold" block style={{ marginBottom: tokens.spacingVerticalS }}>Installed Apps</Text>
          {userApps.slice(0, 8).map((app: any, i: number) => (
            <div key={i} style={{ display: 'flex', justifyContent: 'space-between', padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
              <Text style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1 }}>{app.Name}</Text>
              <Text size={200} style={{ color: tokens.colorNeutralForeground3 }}>{app.Version}</Text>
            </div>
          ))}
          {userApps.length > 8 && <Text size={200} style={{ marginTop: tokens.spacingVerticalS, color: tokens.colorNeutralForeground3 }}>...and {userApps.length - 8} more</Text>}
        </div>
      </div>
    )
  }

  const renderChkdskSummary = (data: any) => {
    if (!data) return null
    const isHealthy = data.status === 'Healthy'
    const hasErrors = data.errors_found === true
    const disks = data.disks || data.disk_drives || []

    return (
      <div className={styles.summaryContainer}>
        <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalM, marginBottom: tokens.spacingVerticalM }}>
          <Badge appearance="filled" color={isHealthy ? 'success' : hasErrors ? 'danger' : 'warning'} size="large">
            {data.status || 'Unknown'}
          </Badge>
          <Text>{data.message || 'Disk health status'}</Text>
        </div>
        {data.note && (
          <div style={{ padding: tokens.spacingHorizontalS, backgroundColor: tokens.colorPaletteYellowBackground2, borderRadius: tokens.borderRadiusSmall, marginBottom: tokens.spacingVerticalM }}>
            <Text size={200}>{data.note}</Text>
          </div>
        )}
        {disks.length > 0 && (
          <div style={{ marginTop: tokens.spacingVerticalM }}>
            {disks.map((disk: any, i: number) => (
              <div key={i} style={{ padding: tokens.spacingHorizontalS, borderLeft: `2px solid ${disk.HealthStatusText === 'Healthy' || disk.HealthStatusText === 'OK' ? tokens.colorPaletteGreenBorder2 : tokens.colorPaletteYellowBorder2}`, backgroundColor: tokens.colorNeutralBackgroundAlpha, marginBottom: tokens.spacingVerticalS }}>
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                  <Text weight="semibold">{disk.FriendlyName || disk.Model || 'Disk'}</Text>
                  <Badge appearance="tint" color={disk.HealthStatusText === 'Healthy' || disk.HealthStatusText === 'OK' ? 'success' : 'warning'} size="small">
                    {disk.HealthStatusText || 'Unknown'}
                  </Badge>
                </div>
                <div className={styles.summaryGrid} style={{ marginTop: tokens.spacingVerticalS }}>
                  <div className={styles.summaryItem}>
                    <span className={styles.summaryLabel}>Size</span>
                    <span className={styles.summaryValue}>{disk.Size ? `${(parseInt(disk.Size) / 1024 / 1024 / 1024).toFixed(0)} GB` : 'N/A'}</span>
                  </div>
                  <div className={styles.summaryItem}>
                    <span className={styles.summaryLabel}>Type</span>
                    <span className={styles.summaryValue}>{disk.MediaTypeText || disk.MediaType || 'N/A'}</span>
                  </div>
                  <div className={styles.summaryItem}>
                    <span className={styles.summaryLabel}>Bus</span>
                    <span className={styles.summaryValue}>{disk.BusTypeText || disk.InterfaceType || 'N/A'}</span>
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    )
  }

  const renderDriverVerifierSummary = (data: any) => {
    if (!data) return null
    const isEnabled = data.enabled === true

    return (
      <div className={styles.summaryContainer}>
        <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalM, marginBottom: tokens.spacingVerticalM }}>
          <Badge appearance="filled" color={isEnabled ? 'warning' : 'informative'}>
            {isEnabled ? 'Active' : 'Inactive'}
          </Badge>
          <Text>{data.status || 'Driver Verifier status'}</Text>
        </div>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Verifier Flags</span>
            <span className={styles.summaryValue}>{data.verifier_flags || '0x00000000'}</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Boot Mode</span>
            <span className={styles.summaryValue}>{data.boot_mode || 'N/A'}</span>
          </div>
        </div>
        {data.enabled_flags && data.enabled_flags.length > 0 && (
          <div style={{ marginTop: tokens.spacingVerticalM }}>
            <Text weight="semibold" block style={{ marginBottom: tokens.spacingVerticalS }}>Active Verification Flags</Text>
            <ul style={{ paddingLeft: tokens.spacingHorizontalL, margin: 0 }}>
              {data.enabled_flags.map((flag: string, i: number) => (
                <li key={i}><Text size={200}>{flag}</Text></li>
              ))}
            </ul>
          </div>
        )}
        {data.verified_drivers && data.verified_drivers.length > 0 && (
          <div style={{ marginTop: tokens.spacingVerticalM }}>
            <Text weight="semibold" block style={{ marginBottom: tokens.spacingVerticalS }}>Verified Drivers</Text>
            <ul style={{ paddingLeft: tokens.spacingHorizontalL, margin: 0 }}>
              {data.verified_drivers.map((driver: string, i: number) => (
                <li key={i}><Text>{driver}</Text></li>
              ))}
            </ul>
          </div>
        )}
        {!isEnabled && (
          <Text size={200} style={{ marginTop: tokens.spacingVerticalM, color: tokens.colorNeutralForeground3 }}>
            Driver Verifier is not monitoring any drivers. This is normal for most systems.
          </Text>
        )}
        {data.error && <Text style={{ color: tokens.colorPaletteRedForeground1 }}>{data.error}</Text>}
      </div>
    )
  }

  const renderFragmentationSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return null

    return (
      <div className={styles.summaryContainer}>
        {data.map((drive: any, i: number) => {
          const percent = drive.fragmentation_percent
          return (
            <div key={i} style={{ marginBottom: tokens.spacingVerticalM }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                <Text weight="semibold">{drive.drive}</Text>
                <Text>{percent !== null ? `${percent}% fragmented` : drive.status}</Text>
              </div>
              {percent !== null && (
                <ProgressBar
                  value={percent / 100}
                  color={percent > 30 ? 'error' : percent > 15 ? 'warning' : 'brand'}
                />
              )}
            </div>
          )
        })}
      </div>
    )
  }

  const renderDmaChannelsSummary = (data: any) => {
    if (!Array.isArray(data)) {
      return <Text>No DMA channels configured. This is normal for modern systems using MSI/MSI-X.</Text>
    }
    if (data.length === 0) {
      return <Text>No DMA channels in use. Modern systems typically use MSI/MSI-X interrupts instead.</Text>
    }

    return (
      <div className={styles.summaryContainer}>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>DMA Channels</span>
            <span className={styles.summaryValue}>{data.length}</span>
          </div>
        </div>
        <div style={{ marginTop: tokens.spacingVerticalM }}>
          {data.slice(0, 10).map((channel: any, i: number) => (
            <div key={i} style={{ display: 'flex', justifyContent: 'space-between', padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
              <Text>Channel {channel.DMAChannel ?? channel.Channel ?? i}</Text>
              <Text size={200} style={{ color: tokens.colorNeutralForeground3 }}>{channel.Caption || channel.Name || 'In use'}</Text>
            </div>
          ))}
        </div>
      </div>
    )
  }

  const renderDeviceMemorySummary = (data: any) => {
    if (!Array.isArray(data)) {
      return <Text>Device memory information unavailable.</Text>
    }
    if (data.length === 0) {
      return <Text>No device memory address ranges reported.</Text>
    }

    return (
      <div className={styles.summaryContainer}>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Memory Ranges</span>
            <span className={styles.summaryValue}>{data.length}</span>
          </div>
        </div>
        <div style={{ marginTop: tokens.spacingVerticalM }}>
          <Text weight="semibold" block style={{ marginBottom: tokens.spacingVerticalS }}>Allocated Ranges</Text>
          {data.slice(0, 8).map((mem: any, i: number) => (
            <div key={i} style={{ padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
              <Text style={{ fontFamily: 'Consolas, monospace' }}>{mem.StartingAddress || mem.Name || `Range ${i}`}</Text>
              {mem.Caption && <Text size={200} block style={{ color: tokens.colorNeutralForeground3 }}>{mem.Caption}</Text>}
            </div>
          ))}
          {data.length > 8 && <Text size={200} style={{ marginTop: tokens.spacingVerticalS, color: tokens.colorNeutralForeground3 }}>...and {data.length - 8} more ranges</Text>}
        </div>
      </div>
    )
  }

  const renderIrqSummary = (data: any) => {
    if (!Array.isArray(data)) {
      return <Text>IRQ information unavailable.</Text>
    }
    if (data.length === 0) {
      return <Text>No legacy IRQ resources in use. Modern devices use MSI/MSI-X interrupts.</Text>
    }

    return (
      <div className={styles.summaryContainer}>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>IRQ Resources</span>
            <span className={styles.summaryValue}>{data.length}</span>
          </div>
        </div>
        <div style={{ marginTop: tokens.spacingVerticalM }}>
          <div style={{ display: 'grid', gridTemplateColumns: '60px 1fr', gap: tokens.spacingHorizontalS, fontWeight: 600, marginBottom: tokens.spacingVerticalS, color: tokens.colorNeutralForeground3 }}>
            <Text>IRQ</Text>
            <Text>Device</Text>
          </div>
          {data.slice(0, 12).map((irq: any, i: number) => (
            <div key={i} style={{ display: 'grid', gridTemplateColumns: '60px 1fr', gap: tokens.spacingHorizontalS, padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
              <Text style={{ fontFamily: 'Consolas, monospace' }}>{irq.IRQNumber ?? irq.IRQ ?? '-'}</Text>
              <Text size={200}>{irq.Caption || irq.Name || 'Unknown device'}</Text>
            </div>
          ))}
          {data.length > 12 && <Text size={200} style={{ marginTop: tokens.spacingVerticalS, color: tokens.colorNeutralForeground3 }}>...and {data.length - 12} more</Text>}
        </div>
      </div>
    )
  }

  const renderSystemDevicesSummary = (data: any) => {
    if (!Array.isArray(data)) {
      return <Text>System devices information unavailable.</Text>
    }
    if (data.length === 0) {
      return <Text>No system devices reported.</Text>
    }

    // Group by type if possible
    const devices = data.slice(0, 15)

    return (
      <div className={styles.summaryContainer}>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Total Devices</span>
            <span className={styles.summaryValue}>{data.length}</span>
          </div>
        </div>
        <div style={{ marginTop: tokens.spacingVerticalM }}>
          {devices.map((device: any, i: number) => (
            <div key={i} style={{ padding: `${tokens.spacingVerticalXS} 0`, borderBottom: `1px solid ${tokens.colorNeutralStroke2}` }}>
              <Text>{device.Caption || device.Name || device.PartComponent || `Device ${i}`}</Text>
              {device.Description && device.Description !== device.Caption && (
                <Text size={200} block style={{ color: tokens.colorNeutralForeground3 }}>{device.Description}</Text>
              )}
            </div>
          ))}
          {data.length > 15 && <Text size={200} style={{ marginTop: tokens.spacingVerticalS, color: tokens.colorNeutralForeground3 }}>...and {data.length - 15} more devices</Text>}
        </div>
      </div>
    )
  }

  const renderSystemDriversSummary = (data: any) => {
    if (!Array.isArray(data) || data.length === 0) return null
    const running = data.filter((d: any) => d.State === 'Running')
    const stopped = data.filter((d: any) => d.State === 'Stopped')

    return (
      <div className={styles.summaryContainer}>
        <div className={styles.summaryGrid}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Total Drivers</span>
            <span className={styles.summaryValue}>{data.length}</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Running</span>
            <span className={styles.summaryValue} style={{ color: tokens.colorPaletteGreenForeground1 }}>{running.length}</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryLabel}>Stopped</span>
            <span className={styles.summaryValue}>{stopped.length}</span>
          </div>
        </div>
      </div>
    )
  }

  const renderSummary = () => {
    if (!parsedData) return <Text>No data available.</Text>

    // Match by exact task title or content patterns
    // System category
    if (title.includes('Computer System')) return renderComputerSystemSummary(parsedData)
    if (title.includes('Operating System')) return renderOsSummary(parsedData)
    if (title.includes('BIOS')) return renderBiosSummary(parsedData)
    if (title.includes('Motherboard')) return renderMotherboardSummary(parsedData)
    if (title.includes('System Information') && parsedData.os_version) return renderSystemInfoSummary(parsedData)
    if (title.includes('Environment Variables')) return renderEnvironmentSummary(parsedData)
    if (title.includes('Startup Commands')) return renderStartupSummary(parsedData)
    if (title.includes('Domain Registration')) return renderDomainSummary(parsedData)
    if (title.includes('Scheduled Tasks')) return renderScheduledTasksSummary(parsedData)
    if (title.includes('DISM Health')) return renderDismHealthSummary(parsedData)

    // Hardware category
    if (title.includes('Processor')) return renderProcessorSummary(parsedData)
    if (title.includes('Physical Memory')) return renderMemorySummary(parsedData)
    if (title.includes('Battery') && parsedData.battery_summary) return renderBatterySummary(parsedData)
    if (title.includes('Printers')) return renderPrintersSummary(parsedData)
    if (title.includes('DMA Channel')) return renderDmaChannelsSummary(parsedData)
    if (title.includes('Device Memory') || title.includes('Memory Address')) return renderDeviceMemorySummary(parsedData)
    if (title.includes('IRQ')) return renderIrqSummary(parsedData)
    if (title.includes('System Devices')) return renderSystemDevicesSummary(parsedData)

    // Storage category
    if (title.includes('Disk Drives')) return renderDiskDrivesSummary(parsedData)
    if (title.includes('Disk Partitions')) return renderDiskPartitionsSummary(parsedData)
    if (title.includes('Logical Disks')) return renderDiskSummary(parsedData)
    if (title.includes('Disk Fragmentation')) return renderFragmentationSummary(parsedData)
    if (title.includes('Disk Check')) return renderChkdskSummary(parsedData)

    // Network category
    if (title.includes('Network') || title.includes('IP Configuration')) return renderNetworkSummary(parsedData)
    if (title.includes('HOSTS File')) return renderHostsFileSummary(parsedData)
    if (title.includes('Firewall')) return renderFirewallSummary(parsedData)

    // Drivers category
    if (title.includes('System Drivers')) return renderSystemDriversSummary(parsedData)
    if (title.includes('Driver List')) return renderDriverListSummary(parsedData)
    if (title.includes('Driver Verifier')) return renderDriverVerifierSummary(parsedData)

    // Graphics category
    if (title.includes('DirectX') || title.includes('Graphics')) return renderGraphicsSummary(parsedData)

    // Software category
    if (title.includes('Installed Programs')) return renderInstalledProgramsSummary(parsedData)
    if (title.includes('Store Apps')) return renderStoreAppsSummary(parsedData)

    // Logs category
    if (title.includes('Event Logs')) return renderEventLogSummary(parsedData)
    if (title.includes('Windows Update')) return renderWindowsUpdateSummary(parsedData)

    // Debug category
    if (title.includes('Minidump') || title.includes('BSOD')) return renderMinidumpsSummary(parsedData)

    // Performance category
    if (title.includes('Performance')) return renderPerformanceSummary(parsedData)

    // System services/processes
    if (title.includes('Services')) return renderServicesSummary(parsedData)
    if (title.includes('Running Processes')) return renderProcessesSummary(parsedData)

    // Default fallback for simple objects (key-value pairs)
    if (typeof parsedData === 'object' && !Array.isArray(parsedData) && Object.keys(parsedData).length < 15) {
       return (
         <div className={styles.summaryGrid}>
           {Object.entries(parsedData).map(([k, v]) => {
             if (typeof v === 'object') return null // Skip nested objects in simple summary
             return (
               <div key={k} className={styles.summaryItem}>
                 <span className={styles.summaryLabel}>{k.replace(/_/g, ' ')}</span>
                 <span className={styles.summaryValue}>{String(v)}</span>
               </div>
             )
           })}
         </div>
       )
    }

    // Fallback if no specific summary available
    return (
      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', padding: tokens.spacingVerticalL, gap: tokens.spacingVerticalS }}>
        <Text style={{ color: tokens.colorNeutralForeground3 }}>Detailed structured data available.</Text>
        <Button onClick={() => setViewMode('raw')}>View Raw Data</Button>
      </div>
    )
  }

  // Recursive renderer for structured data (Raw View)
  const renderData = (data: any, depth = 0): React.ReactNode => {
    if (data === null || data === undefined) return <span style={{ color: tokens.colorNeutralForegroundDisabled }}>null</span>

    if (typeof data === 'boolean') {
      return (
        <Badge
          appearance="tint"
          color={data ? 'success' : 'subtle'}
          size="small"
        >
          {data ? 'True' : 'False'}
        </Badge>
      )
    }

    if (typeof data === 'string') {
      if (data.trim().startsWith('{') || data.trim().startsWith('[')) {
        try {
          const parsed = JSON.parse(data)
          return renderData(parsed, depth)
        } catch { /* ignore */ }
      }
      return <span className={styles.dataValue}>{data}</span>
    }

    if (typeof data === 'number') {
      return <span className={styles.dataValue} style={{ color: tokens.colorPaletteBlueForeground2 }}>{data}</span>
    }

    if (Array.isArray(data)) {
      if (data.length === 0) return <span style={{ color: tokens.colorNeutralForegroundDisabled }}>[]</span>
      return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: tokens.spacingVerticalXS, paddingLeft: depth > 0 ? tokens.spacingHorizontalM : 0 }}>
          {data.map((item, index) => (
            <div key={index} style={{ borderLeft: `2px solid ${tokens.colorNeutralStroke2}`, paddingLeft: tokens.spacingHorizontalS }}>
              {renderData(item, depth + 1)}
            </div>
          ))}
        </div>
      )
    }

    if (typeof data === 'object') {
      const keys = Object.keys(data)
      if (keys.length === 0) return <span style={{ color: tokens.colorNeutralForegroundDisabled }}>{`{}`}</span>
      return (
        <div className={styles.dataGrid} style={{ paddingLeft: depth > 0 ? tokens.spacingHorizontalM : 0 }}>
          {keys.map(key => (
            <React.Fragment key={key}>
              <div className={styles.dataLabel}>{key.replace(/_/g, ' ')}:</div>
              <div>{renderData(data[key], depth + 1)}</div>
            </React.Fragment>
          ))}
        </div>
      )
    }

    return String(data)
  }

  const formatRawOutput = (data: any) => {
    if (typeof data === 'string') return data
    return JSON.stringify(data, null, 2)
  }

  return (
    <div className={getCardStyle()}>
      <div className={styles.header} onClick={() => setExpanded(!expanded)}>
        <div className={styles.titleContainer}>
          <Text className={styles.title}>{highlightText ? highlightMatchingText(title) : title}</Text>
          <Text className={styles.meta}>
            {importance !== 'primary' && `${category} • `}
            {executionTime ? `${executionTime}ms` : ''}
          </Text>
          {!expanded && (
            <Text className={styles.preview}>
              {highlightText ? highlightMatchingText(getQuickSummary()) : getQuickSummary()}
            </Text>
          )}
        </div>
        <div className={styles.statusIndicator}>
          {getStatusIcon()}
        </div>
        {expanded ? <ChevronDown20Regular /> : <ChevronRight20Regular />}
      </div>

      {expanded && (
        <div className={styles.content}>
          {description && (
            <>
              <Text className={styles.sectionTitle}>Objective</Text>
              <Text className={styles.description}>{description}</Text>
            </>
          )}

          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: tokens.spacingVerticalS, marginTop: tokens.spacingVerticalM }}>
             <Text className={styles.sectionTitle} style={{ marginTop: 0, marginBottom: 0 }}>Analysis Result</Text>
             <TabList 
               selectedValue={viewMode} 
               onTabSelect={(_, data) => setViewMode(data.value as 'summary' | 'raw')}
               size="small"
             >
               <Tab icon={<SlideText20Regular />} value="summary">Summary</Tab>
               <Tab icon={<Code20Regular />} value="raw">Raw Data</Tab>
             </TabList>
          </div>
          
          <div className={styles.outputContainer}>
            {error ? (
              <div style={{ color: tokens.colorPaletteRedForeground1, fontFamily: 'monospace' }}>
                Error: {error}
              </div>
            ) : (
              viewMode === 'summary' ? renderSummary() : renderData(parsedData)
            )}
          </div>

          <AIInterpretationPanel
            taskId={taskId ?? title.toLowerCase().replace(/\s+/g, '_')}
            taskName={title}
            output={formatRawOutput(output)}
            isSuccess={isSuccess}
          />

          {onCopyOutput && output && (
            <Button
              appearance="subtle"
              icon={<Copy16Regular />}
              size="small"
              onClick={(e) => {
                e.stopPropagation()
                onCopyOutput(formatRawOutput(output))
              }}
              style={{ marginTop: tokens.spacingVerticalM }}
            >
              Copy Raw Data
            </Button>
          )}
        </div>
      )}
    </div>
  )
}