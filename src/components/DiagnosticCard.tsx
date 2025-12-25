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
  Stethoscope20Regular,
  Code20Regular,
  SlideText20Regular
} from '@fluentui/react-icons'

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
  header: {
    display: 'flex',
    alignItems: 'center',
    padding: tokens.spacingHorizontalM,
    cursor: 'pointer',
    gap: tokens.spacingHorizontalM,
  },
  statusIcon: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
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
  statusBadge: {
    marginRight: tokens.spacingHorizontalM,
    textTransform: 'uppercase',
    fontWeight: 700,
    fontSize: '10px',
    letterSpacing: '0.05em',
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
  title: string
  description?: string
  status: 'verified' | 'monitor' | 'action_required'
  importance: 'primary' | 'secondary' | 'informational'
  executionTime?: number
  output?: any
  error?: string
  category?: string
  onCopyOutput?: (output: string) => void
}

export const DiagnosticCard: React.FC<DiagnosticCardProps> = ({
  title,
  description,
  status,
  importance,
  executionTime,
  output,
  error,
  category,
  onCopyOutput
}) => {
  const styles = useStyles()
  const [expanded, setExpanded] = useState(false)
  const [viewMode, setViewMode] = useState<'summary' | 'raw'>('summary')

  const isSuccess = status === 'verified'
  const isError = status === 'action_required'
  const isWarning = status === 'monitor'

  const getCardStyle = () => {
    if (isError) return `${styles.card} ${styles.cardError}`
    if (isWarning) return `${styles.card} ${styles.cardWarning}`
    if (importance === 'primary') return `${styles.card} ${styles.cardPrimary}`
    return styles.card
  }

  const getStatusIcon = () => {
    if (isError) return <ErrorCircle20Filled primaryFill={tokens.colorPaletteRedForeground1} />
    if (isWarning) return <Warning20Filled primaryFill={tokens.colorPaletteYellowForeground1} />
    return <CheckmarkCircle20Filled primaryFill={tokens.colorPaletteGreenForeground1} />
  }

  const getStatusText = () => {
    if (isError) return 'Action Required'
    if (isWarning) return 'Monitor'
    return 'Verified'
  }

  const getStatusBadgeColor = () => {
    if (isError) return 'danger'
    if (isWarning) return 'warning'
    return 'success'
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

  // Specialized Renderers
  const renderDiskSummary = (data: any) => {
    if (!Array.isArray(data)) return null
    
    return (
      <div className={styles.summaryContainer}>
        {data.map((disk: any, i: number) => {
          if (!disk.Size) return null
          const size = parseInt(disk.Size)
          const free = parseInt(disk.FreeSpace)
          const used = size - free
          const percentUsed = (used / size) * 100
          
          return (
            <div key={i} style={{ marginBottom: tokens.spacingVerticalS }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                <Text weight="semibold">{disk.DeviceID} ({disk.FileSystem})</Text>
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
                <span className={styles.summaryValue}>{Array.isArray(adapter.IPAddress) ? adapter.IPAddress[0] : adapter.IPAddress}</span>
              </div>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>MAC Address</span>
                <span className={styles.summaryValue}>{adapter.MACAddress}</span>
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
          <span className={styles.summaryValue}>{cpu.NumberOfCores}</span>
        </div>
        <div className={styles.summaryItem}>
          <span className={styles.summaryLabel}>Logical Processors</span>
          <span className={styles.summaryValue}>{cpu.NumberOfLogicalProcessors}</span>
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
                <Text>{(parseInt(dimm.Capacity) / 1024 / 1024 / 1024).toFixed(0)} GB @ {dimm.Speed} MHz</Text>
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
                <span className={styles.summaryValue}>{gpu.AdapterRAM ? `${(parseInt(gpu.AdapterRAM) / 1024 / 1024 / 1024).toFixed(1)} GB` : 'N/A'}</span>
              </div>
              <div className={styles.summaryItem}>
                <span className={styles.summaryLabel}>Resolution</span>
                <span className={styles.summaryValue}>{gpu.CurrentHorizontalResolution}x{gpu.CurrentVerticalResolution} @ {gpu.CurrentRefreshRate}Hz</span>
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

  const renderSummary = () => {
    if (!parsedData) return <Text>No data available.</Text>

    // Match by exact task ID or title content
    if (title.includes('Logical Disks') || category === 'Storage') return renderDiskSummary(parsedData)
    if (title.includes('Battery') && parsedData.battery_summary) return renderBatterySummary(parsedData)
    if (title.includes('System Information') && parsedData.os_version) return renderSystemInfoSummary(parsedData)
    if (title.includes('Network') || title.includes('IP Configuration')) return renderNetworkSummary(parsedData)
    if (title.includes('Processor')) return renderProcessorSummary(parsedData)
    if (title.includes('Physical Memory')) return renderMemorySummary(parsedData)
    if (title.includes('DirectX') || title.includes('Graphics')) return renderGraphicsSummary(parsedData)
    if (title.includes('Event Logs')) return renderEventLogSummary(parsedData)
    if (title.includes('Services')) return renderServicesSummary(parsedData)
    if (title.includes('Windows Update')) return renderWindowsUpdateSummary(parsedData)
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
        <div className={styles.statusIcon}>{getStatusIcon()}</div>
        <div className={styles.titleContainer}>
          <Text className={styles.title}>{title}</Text>
          <Text className={styles.meta}>
            {importance !== 'primary' && `${category} • `}
            {executionTime ? `${executionTime}ms` : ''}
          </Text>
        </div>
        <Badge appearance="outline" color={getStatusBadgeColor()} className={styles.statusBadge}>
          {getStatusText()}
        </Badge>
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

          <div className={styles.aiInsight}>
            <div className={styles.aiTitle}>
              <Stethoscope20Regular />
              AI Interpretation
            </div>
            <Text className={styles.description} style={{ marginBottom: 0 }}>
              {isSuccess 
                ? "This component is operating within normal parameters. No anomalies detected in current configuration."
                : "This component reported an unexpected state. Review the output above for specific error codes."
              }
            </Text>
          </div>

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