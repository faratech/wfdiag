import React, { useEffect, useState, useRef } from 'react';
import {
  Card,
  Title3,
  Caption1,
  ProgressBar,
  makeStyles,
  tokens,
  Badge,
  Divider,
  Text,
  Button,
} from '@fluentui/react-components';
import {
  DesktopRegular,
  Memory16Regular,
  HardDriveRegular,
  NetworkCheckRegular,
  AppsListDetailRegular,
  ArrowDownloadRegular,
  ArrowUploadRegular,
  ArrowLeftRegular,
} from '@fluentui/react-icons';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import {
  Chart as ChartJS,
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  BarElement,
  Title,
  Tooltip,
  Legend,
  Filler,
} from 'chart.js';
import { Line, Bar } from 'react-chartjs-2';

// Register Chart.js components
ChartJS.register(
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  BarElement,
  Title,
  Tooltip,
  Legend,
  Filler
);

const useStyles = makeStyles({
  container: {
    display: 'grid',
    gridTemplateColumns: 'repeat(auto-fill, minmax(320px, 1fr))',
    gap: tokens.spacingVerticalM,
    padding: tokens.spacingVerticalM,
  },
  card: {
    padding: tokens.spacingVerticalM,
  },
  chartCard: {
    padding: tokens.spacingVerticalM,
    gridColumn: 'span 2',
  },
  fullWidthCard: {
    padding: tokens.spacingVerticalM,
    gridColumn: '1 / -1',
  },
  header: {
    display: 'flex',
    alignItems: 'center',
    marginBottom: tokens.spacingVerticalS,
    gap: tokens.spacingHorizontalS,
  },
  metric: {
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginBottom: tokens.spacingVerticalXS,
  },
  processTable: {
    width: '100%',
    borderCollapse: 'collapse',
    fontSize: tokens.fontSizeBase200,
  },
  processHeader: {
    textAlign: 'left',
    padding: tokens.spacingVerticalXS,
    borderBottom: `1px solid ${tokens.colorNeutralStroke1}`,
    fontWeight: tokens.fontWeightSemibold,
  },
  processCell: {
    padding: tokens.spacingVerticalXS,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  networkStats: {
    display: 'flex',
    gap: tokens.spacingHorizontalL,
    marginTop: tokens.spacingVerticalS,
  },
  networkMetric: {
    display: 'flex',
    alignItems: 'center',
    gap: tokens.spacingHorizontalXS,
  },
});

interface DiskInfo {
  name: string;
  mount_point: string;
  total_gb: number;
  used_gb: number;
  available_gb: number;
  utilization: number;
  file_system: string;
  disk_type: string;
}

interface SystemStats {
  cpu_utilization: number;
  per_cpu_utilization: number[];
  cpu_frequency: number;
  memory_total_gb: number;
  memory_used_gb: number;
  memory_available_gb: number;
  memory_utilization: number;
  swap_total_gb: number;
  swap_used_gb: number;
  swap_utilization: number;
  disk_utilization: number;
  disk_read_bytes: number;
  disk_write_bytes: number;
  disks: DiskInfo[];
  network_upload_kb: number;
  network_download_kb: number;
  top_processes: ProcessInfo[];
  timestamp: number;
}

interface ProcessInfo {
  pid: number;
  name: string;
  cpu_percent: number;
  memory_percent: number;
  memory_mb: number;
  virtual_memory_mb: number;
  disk_read_bytes: number;
  disk_write_bytes: number;
  status: string;
  start_time: number;
  command: string;
}

interface NetworkConnection {
  protocol: string;
  local_addr: string;
  remote_addr: string;
  status: string;
}

interface SystemMonitoringProps {
  isActive: boolean;
  onToggle: (active: boolean) => void;
  onBack?: () => void;
}

const MAX_DATA_POINTS = 60; // 60 seconds of history (1 second intervals)

// Helper function to format bytes
const formatBytes = (bytes: number): string => {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
};

export const SystemMonitoring: React.FC<SystemMonitoringProps> = ({ isActive, onToggle, onBack }) => {
  const styles = useStyles();
  const [stats, setStats] = useState<SystemStats | null>(null);
  const [cpuHistory, setCpuHistory] = useState<number[]>([]);
  const [memoryHistory, setMemoryHistory] = useState<number[]>([]);
  const [networkUploadHistory, setNetworkUploadHistory] = useState<number[]>([]);
  const [networkDownloadHistory, setNetworkDownloadHistory] = useState<number[]>([]);
  const [timeLabels, setTimeLabels] = useState<string[]>([]);
  const [networkConnections, setNetworkConnections] = useState<NetworkConnection[]>([]);
  const [showNetworkConnections, setShowNetworkConnections] = useState(false);
  const unlistenRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    const setupMonitoring = async () => {
      if (isActive) {
        // Start monitoring
        try {
          await invoke('start_monitoring');
          
          // Listen for system stats events
          unlistenRef.current = await listen<SystemStats>('system-stats', (event) => {
            const newStats = event.payload;
            setStats(newStats);
            
            // Update history arrays
            setCpuHistory(prev => {
              const updated = [...prev, newStats.cpu_utilization];
              return updated.slice(-MAX_DATA_POINTS);
            });
            
            setMemoryHistory(prev => {
              const updated = [...prev, newStats.memory_utilization];
              return updated.slice(-MAX_DATA_POINTS);
            });
            
            setNetworkUploadHistory(prev => {
              const updated = [...prev, newStats.network_upload_kb];
              return updated.slice(-MAX_DATA_POINTS);
            });
            
            setNetworkDownloadHistory(prev => {
              const updated = [...prev, newStats.network_download_kb];
              return updated.slice(-MAX_DATA_POINTS);
            });
            
            setTimeLabels(prev => {
              const updated = [...prev, new Date().toLocaleTimeString()];
              return updated.slice(-MAX_DATA_POINTS);
            });
          });
        } catch (error) {
          console.error('Failed to start monitoring:', error);
          onToggle(false);
        }
      } else {
        // Stop monitoring
        if (unlistenRef.current) {
          unlistenRef.current();
          unlistenRef.current = null;
        }
        try {
          await invoke('stop_monitoring');
        } catch (error) {
          console.error('Failed to stop monitoring:', error);
        }
      }
    };

    setupMonitoring();

    return () => {
      if (unlistenRef.current) {
        unlistenRef.current();
      }
    };
  }, [isActive, onToggle]);

  const cpuChartData = {
    labels: timeLabels,
    datasets: [
      {
        label: 'CPU Usage %',
        data: cpuHistory,
        borderColor: tokens.colorBrandForeground1,
        backgroundColor: `${tokens.colorBrandBackground2}40`,
        fill: true,
        tension: 0.4,
      },
    ],
  };

  const memoryChartData = {
    labels: timeLabels,
    datasets: [
      {
        label: 'Memory Usage %',
        data: memoryHistory,
        borderColor: tokens.colorPaletteGreenForeground1,
        backgroundColor: `${tokens.colorPaletteGreenBackground2}40`,
        fill: true,
        tension: 0.4,
      },
    ],
  };

  const networkChartData = {
    labels: timeLabels,
    datasets: [
      {
        label: 'Upload KB/s',
        data: networkUploadHistory,
        borderColor: tokens.colorPaletteBlueForeground2,
        backgroundColor: `${tokens.colorPaletteBlueBackground2}40`,
        fill: true,
        tension: 0.4,
      },
      {
        label: 'Download KB/s',
        data: networkDownloadHistory,
        borderColor: tokens.colorPalettePurpleForeground2,
        backgroundColor: `${tokens.colorPalettePurpleBackground2}40`,
        fill: true,
        tension: 0.4,
      },
    ],
  };

  const cpuCoreData = {
    labels: stats?.per_cpu_utilization.map((_, i) => `Core ${i}`) || [],
    datasets: [
      {
        label: 'CPU Core Usage %',
        data: stats?.per_cpu_utilization || [],
        backgroundColor: stats?.per_cpu_utilization.map(val => 
          val > 80 ? tokens.colorPaletteRedBackground3 :
          val > 50 ? tokens.colorPaletteYellowBackground3 :
          tokens.colorPaletteGreenBackground3
        ) || [],
        borderWidth: 0,
      },
    ],
  };

  const chartOptions = {
    responsive: true,
    maintainAspectRatio: false,
    plugins: {
      legend: {
        display: true,
        position: 'top' as const,
      },
    },
    scales: {
      y: {
        beginAtZero: true,
        max: 100,
      },
    },
  };

  const networkChartOptions = {
    ...chartOptions,
    scales: {
      y: {
        beginAtZero: true,
      },
    },
  };

  if (!isActive || !stats) {
    return (
      <div style={{ padding: tokens.spacingVerticalL, textAlign: 'center' }}>
        <Text>System monitoring is not active. Enable it from the toolbar to see real-time stats.</Text>
      </div>
    );
  }

  return (
    <div>
      {/* Header with back button */}
      <div style={{ 
        padding: tokens.spacingVerticalM, 
        display: 'flex', 
        alignItems: 'center', 
        gap: tokens.spacingHorizontalM,
        borderBottom: `1px solid ${tokens.colorNeutralStroke1}`
      }}>
        {onBack && (
          <Button 
            appearance="secondary" 
            icon={<ArrowLeftRegular />} 
            onClick={() => {
              onToggle(false); // Stop monitoring
              onBack(); // Navigate back
            }}
          >
            Back to Home
          </Button>
        )}
        <Title3 style={{ flex: 1 }}>Real-time System Monitor</Title3>
        <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalS }}>
          <Text>Auto-refresh: </Text>
          <Badge appearance="filled" color="success">Active</Badge>
        </div>
      </div>
      
      <div className={styles.container}>
      {/* CPU Card */}
      <Card className={styles.card}>
        <div className={styles.header}>
          <DesktopRegular fontSize={24} />
          <Title3>CPU</Title3>
        </div>
        <div className={styles.metric}>
          <Text>Usage</Text>
          <Badge appearance="filled" color={stats.cpu_utilization > 80 ? 'danger' : stats.cpu_utilization > 50 ? 'warning' : 'success'}>
            {stats.cpu_utilization.toFixed(1)}%
          </Badge>
        </div>
        <ProgressBar value={stats.cpu_utilization / 100} />
        <Caption1 style={{ marginTop: tokens.spacingVerticalXS }}>
          Frequency: {stats.cpu_frequency} MHz
        </Caption1>
      </Card>

      {/* Memory Card */}
      <Card className={styles.card}>
        <div className={styles.header}>
          <Memory16Regular fontSize={24} />
          <Title3>Memory</Title3>
        </div>
        <div className={styles.metric}>
          <Text>Usage</Text>
          <Badge appearance="filled" color={stats.memory_utilization > 80 ? 'danger' : stats.memory_utilization > 50 ? 'warning' : 'success'}>
            {stats.memory_utilization.toFixed(1)}%
          </Badge>
        </div>
        <ProgressBar value={stats.memory_utilization / 100} />
        <Caption1 style={{ marginTop: tokens.spacingVerticalXS }}>
          {stats.memory_used_gb.toFixed(1)} GB / {stats.memory_total_gb.toFixed(1)} GB
        </Caption1>
        <Divider style={{ margin: `${tokens.spacingVerticalS} 0` }} />
        <Text size={200}>Swap: {stats.swap_utilization.toFixed(1)}%</Text>
      </Card>

      {/* Disk Cards - Show all physical disks */}
      {stats.disks && stats.disks.map((disk, index) => (
        <Card key={index} className={styles.card}>
          <div className={styles.header}>
            <HardDriveRegular fontSize={24} />
            <div style={{ flex: 1 }}>
              <Title3>{disk.mount_point}</Title3>
              <Caption1>{disk.name || 'Local Disk'} • {disk.disk_type} • {disk.file_system}</Caption1>
            </div>
          </div>
          <div className={styles.metric}>
            <Text>Usage</Text>
            <Badge appearance="filled" color={disk.utilization > 80 ? 'danger' : disk.utilization > 50 ? 'warning' : 'success'}>
              {disk.utilization.toFixed(1)}%
            </Badge>
          </div>
          <ProgressBar value={disk.utilization / 100} />
          <Caption1 style={{ marginTop: tokens.spacingVerticalXS }}>
            {disk.used_gb.toFixed(1)} GB / {disk.total_gb.toFixed(1)} GB
            <span style={{ opacity: 0.7, marginLeft: 8 }}>
              ({disk.available_gb.toFixed(1)} GB free)
            </span>
          </Caption1>
        </Card>
      ))}

      {/* Network Card */}
      <Card className={styles.card}>
        <div className={styles.header}>
          <NetworkCheckRegular fontSize={24} />
          <Title3>Network</Title3>
        </div>
        <div className={styles.networkStats}>
          <div className={styles.networkMetric}>
            <ArrowUploadRegular />
            <Text>{stats.network_upload_kb.toFixed(1)} KB/s</Text>
          </div>
          <div className={styles.networkMetric}>
            <ArrowDownloadRegular />
            <Text>{stats.network_download_kb.toFixed(1)} KB/s</Text>
          </div>
        </div>
      </Card>

      {/* CPU History Chart */}
      <Card className={styles.chartCard}>
        <Title3 style={{ marginBottom: tokens.spacingVerticalM }}>CPU Usage History</Title3>
        <div style={{ height: '200px' }}>
          <Line data={cpuChartData} options={chartOptions} />
        </div>
      </Card>

      {/* Memory History Chart */}
      <Card className={styles.chartCard}>
        <Title3 style={{ marginBottom: tokens.spacingVerticalM }}>Memory Usage History</Title3>
        <div style={{ height: '200px' }}>
          <Line data={memoryChartData} options={chartOptions} />
        </div>
      </Card>

      {/* CPU Cores Chart */}
      <Card className={styles.chartCard}>
        <Title3 style={{ marginBottom: tokens.spacingVerticalM }}>CPU Cores</Title3>
        <div style={{ height: '200px' }}>
          <Bar data={cpuCoreData} options={chartOptions} />
        </div>
      </Card>

      {/* Network Chart */}
      <Card className={styles.chartCard}>
        <Title3 style={{ marginBottom: tokens.spacingVerticalM }}>Network Activity</Title3>
        <div style={{ height: '200px' }}>
          <Line data={networkChartData} options={networkChartOptions} />
        </div>
      </Card>

      {/* Network Connections */}
      <Card className={styles.fullWidthCard}>
        <div className={styles.header}>
          <NetworkCheckRegular fontSize={24} />
          <Title3>Network Connections</Title3>
          <Button 
            appearance="secondary" 
            size="small"
            onClick={async () => {
              try {
                const connections = await invoke<NetworkConnection[]>('get_network_connections');
                setNetworkConnections(connections);
                setShowNetworkConnections(true);
              } catch (error) {
                console.error('Failed to get network connections:', error);
              }
            }}
          >
            Refresh Connections
          </Button>
        </div>
        {showNetworkConnections && networkConnections.length > 0 && (
          <div style={{ overflowX: 'auto', marginTop: tokens.spacingVerticalM }}>
            <table className={styles.processTable}>
              <thead>
                <tr>
                  <th className={styles.processHeader}>Protocol</th>
                  <th className={styles.processHeader}>Local Address</th>
                  <th className={styles.processHeader}>Remote Address</th>
                  <th className={styles.processHeader}>Status</th>
                </tr>
              </thead>
              <tbody>
                {networkConnections.slice(0, 20).map((conn, index) => (
                  <tr key={index}>
                    <td className={styles.processCell}>
                      <Badge appearance="tint" size="small">
                        {conn.protocol}
                      </Badge>
                    </td>
                    <td className={styles.processCell}>
                      <Text size={200}>{conn.local_addr}</Text>
                    </td>
                    <td className={styles.processCell}>
                      <Text size={200}>{conn.remote_addr}</Text>
                    </td>
                    <td className={styles.processCell}>
                      <Badge 
                        appearance="tint" 
                        color={
                          conn.status === 'ESTABLISHED' ? 'success' :
                          conn.status === 'LISTENING' ? 'brand' :
                          conn.status === 'TIME_WAIT' ? 'warning' :
                          'subtle'
                        }
                        size="small"
                      >
                        {conn.status}
                      </Badge>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {networkConnections.length > 20 && (
              <Text size={200} style={{ marginTop: tokens.spacingVerticalS }}>
                Showing first 20 of {networkConnections.length} connections
              </Text>
            )}
          </div>
        )}
        {showNetworkConnections && networkConnections.length === 0 && (
          <Text>No active network connections found</Text>
        )}
      </Card>

      {/* Top Processes */}
      <Card className={styles.fullWidthCard}>
        <div className={styles.header}>
          <AppsListDetailRegular fontSize={24} />
          <Title3>Top Processes (by CPU Usage)</Title3>
        </div>
        <div style={{ overflowX: 'auto' }}>
          <table className={styles.processTable}>
            <thead>
              <tr>
                <th className={styles.processHeader}>PID</th>
                <th className={styles.processHeader}>Name</th>
                <th className={styles.processHeader}>Status</th>
                <th className={styles.processHeader}>CPU %</th>
                <th className={styles.processHeader}>Memory</th>
                <th className={styles.processHeader}>Disk I/O</th>
                <th className={styles.processHeader} style={{ maxWidth: '300px' }}>Command</th>
              </tr>
            </thead>
            <tbody>
              {stats.top_processes.map((process) => (
                <tr key={process.pid}>
                  <td className={styles.processCell}>{process.pid}</td>
                  <td className={styles.processCell} title={process.name}>
                    {process.name.length > 20 ? process.name.substring(0, 20) + '...' : process.name}
                  </td>
                  <td className={styles.processCell}>
                    <Badge 
                      appearance="tint" 
                      color={process.status === 'Running' ? 'success' : 'subtle'}
                      size="small"
                    >
                      {process.status}
                    </Badge>
                  </td>
                  <td className={styles.processCell}>
                    <Badge 
                      appearance="filled" 
                      color={process.cpu_percent > 50 ? 'danger' : process.cpu_percent > 20 ? 'warning' : 'success'}
                    >
                      {process.cpu_percent.toFixed(1)}%
                    </Badge>
                  </td>
                  <td className={styles.processCell}>
                    <div>
                      <Text size={200}>{process.memory_mb.toFixed(1)} MB</Text>
                      <Badge 
                        appearance="tint" 
                        color={process.memory_percent > 50 ? 'danger' : process.memory_percent > 20 ? 'warning' : 'brand'}
                        size="small"
                        style={{ marginLeft: '8px' }}
                      >
                        {process.memory_percent.toFixed(1)}%
                      </Badge>
                    </div>
                  </td>
                  <td className={styles.processCell}>
                    <div style={{ fontSize: '11px' }}>
                      {process.disk_read_bytes > 0 || process.disk_write_bytes > 0 ? (
                        <>
                          <div>R: {formatBytes(process.disk_read_bytes)}</div>
                          <div>W: {formatBytes(process.disk_write_bytes)}</div>
                        </>
                      ) : (
                        <Text size={100}>-</Text>
                      )}
                    </div>
                  </td>
                  <td className={styles.processCell} style={{ maxWidth: '300px', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={process.command}>
                    <Text size={100}>{process.command}</Text>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Card>
    </div>
    </div>
  );
};