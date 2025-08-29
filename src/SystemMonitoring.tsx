import React, { useEffect, useState, useRef } from 'react';
import {
  ProgressBar,
  Text,
  Button,
  Badge,
} from '@fluentui/react-components';
import {
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
import './styles.css';

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
  const [stats, setStats] = useState<SystemStats | null>(null);
  const [cpuHistory, setCpuHistory] = useState<number[]>([]);
  const [memoryHistory, setMemoryHistory] = useState<number[]>([]);
  const [networkUploadHistory, setNetworkUploadHistory] = useState<number[]>([]);
  const [networkDownloadHistory, setNetworkDownloadHistory] = useState<number[]>([]);
  const [timeLabels, setTimeLabels] = useState<string[]>([]);
  const [networkConnections, setNetworkConnections] = useState<NetworkConnection[]>([]);
  const [showNetworkConnections, setShowNetworkConnections] = useState(false);
  const unlistenRef = useRef<(() => void) | null>(null);

  // Auto-start monitoring when component mounts
  useEffect(() => {
    if (!isActive) {
      onToggle(true);
    }
  }, []);

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
        borderColor: '#3b82f6',
        backgroundColor: 'rgba(59, 130, 246, 0.1)',
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
        borderColor: '#10b981',
        backgroundColor: 'rgba(16, 185, 129, 0.1)',
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
        borderColor: '#8b5cf6',
        backgroundColor: 'rgba(139, 92, 246, 0.1)',
        fill: true,
        tension: 0.4,
      },
      {
        label: 'Download KB/s',
        data: networkDownloadHistory,
        borderColor: '#f59e0b',
        backgroundColor: 'rgba(245, 158, 11, 0.1)',
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
          val > 80 ? 'rgba(239, 68, 68, 0.8)' :
          val > 50 ? 'rgba(245, 158, 11, 0.8)' :
          'rgba(16, 185, 129, 0.8)'
        ) || [],
        borderColor: stats?.per_cpu_utilization.map(val => 
          val > 80 ? '#ef4444' :
          val > 50 ? '#f59e0b' :
          '#10b981'
        ) || [],
        borderWidth: 1,
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
        labels: {
          color: '#94a3b8',
          font: {
            family: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto',
          },
        },
      },
      tooltip: {
        backgroundColor: 'rgba(30, 41, 59, 0.95)',
        borderColor: 'rgba(59, 130, 246, 0.3)',
        borderWidth: 1,
        titleColor: '#f1f5f9',
        bodyColor: '#94a3b8',
      },
    },
    scales: {
      y: {
        beginAtZero: true,
        max: 100,
        grid: {
          color: 'rgba(148, 163, 184, 0.1)',
        },
        ticks: {
          color: '#94a3b8',
        },
      },
      x: {
        grid: {
          color: 'rgba(148, 163, 184, 0.1)',
        },
        ticks: {
          color: '#94a3b8',
        },
      },
    },
  };

  const networkChartOptions = {
    ...chartOptions,
    scales: {
      ...chartOptions.scales,
      y: {
        ...chartOptions.scales.y,
        beginAtZero: true,
        max: undefined,
      },
    },
  };

  if (!isActive || !stats) {
    return (
      <div className="glass-card" style={{ 
        padding: 48, 
        textAlign: 'center',
        background: 'linear-gradient(135deg, rgba(59, 130, 246, 0.1), rgba(139, 92, 246, 0.1))',
        border: '1px solid rgba(59, 130, 246, 0.3)',
      }}>
        <i className="fas fa-chart-line" style={{ 
          fontSize: 64, 
          background: 'linear-gradient(135deg, #3b82f6, #8b5cf6)',
          WebkitBackgroundClip: 'text',
          WebkitTextFillColor: 'transparent',
          marginBottom: 24 
        }}></i>
        <Text size={500} weight="bold" block style={{ marginBottom: 12, color: '#f1f5f9' }}>
          System Monitoring {!isActive ? 'Inactive' : 'Starting...'}
        </Text>
        <Text size={300} block style={{ marginBottom: 24, color: '#94a3b8' }}>
          {!isActive ? 'Click the button below to start real-time system monitoring' : 'Initializing monitoring services...'}
        </Text>
        {!isActive && (
          <Button 
            appearance="primary" 
            onClick={() => onToggle(true)} 
            size="large"
            style={{
              background: 'linear-gradient(135deg, #3b82f6, #8b5cf6)',
              border: 'none',
              padding: '12px 32px',
            }}
          >
            <i className="fas fa-play" style={{ marginRight: 8 }}></i>
            Start Monitoring
          </Button>
        )}
      </div>
    );
  }

  return (
    <div>
      {/* Header with gradient background */}
      <div className="glass-card" style={{ 
        padding: 16, 
        marginBottom: 24,
        background: 'linear-gradient(135deg, rgba(59, 130, 246, 0.1), rgba(139, 92, 246, 0.1))',
        display: 'flex', 
        alignItems: 'center', 
        gap: 16,
      }}>
        {onBack && (
          <Button 
            appearance="secondary" 
            icon={<ArrowLeftRegular />} 
            onClick={() => {
              onToggle(false);
              onBack();
            }}
            style={{
              background: 'rgba(30, 41, 59, 0.8)',
              border: '1px solid rgba(255, 255, 255, 0.1)',
              color: '#f1f5f9',
            }}
          >
            Back to Home
          </Button>
        )}
        <div style={{ flex: 1 }}>
          <Text size={500} weight="bold" style={{ color: '#f1f5f9' }}>
            <i className="fas fa-chart-line" style={{ marginRight: 12, color: '#3b82f6' }}></i>
            Real-time System Monitor
          </Text>
        </div>
        <Button 
          appearance={isActive ? "secondary" : "primary"}
          onClick={() => onToggle(!isActive)}
          style={{
            background: isActive ? 'rgba(239, 68, 68, 0.8)' : 'linear-gradient(135deg, #3b82f6, #8b5cf6)',
            border: 'none',
            color: 'white',
          }}
        >
          <i className={isActive ? "fas fa-stop" : "fas fa-play"} style={{ marginRight: 8 }}></i>
          {isActive ? 'Stop Monitoring' : 'Start Monitoring'}
        </Button>
        <div className={`status-badge ${isActive ? 'success' : 'warning'}`}>
          <i className="fas fa-circle icon-pulse" style={{ fontSize: 8 }}></i>
          {isActive ? 'Live' : 'Inactive'}
        </div>
      </div>
      
      <div style={{ 
        display: 'grid', 
        gridTemplateColumns: 'repeat(auto-fill, minmax(320px, 1fr))', 
        gap: 24,
      }}>
        {/* CPU Card */}
        <div className="glass-card" style={{ padding: 20 }}>
          <div style={{ display: 'flex', alignItems: 'center', marginBottom: 16, gap: 12 }}>
            <div className="category-icon">
              <i className="fas fa-microchip"></i>
            </div>
            <Text size={400} weight="semibold" style={{ color: '#f1f5f9' }}>CPU</Text>
          </div>
          <div style={{ marginBottom: 12 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 8 }}>
              <Text size={200} style={{ color: '#94a3b8' }}>Usage</Text>
              <Badge 
                appearance="filled" 
                style={{
                  background: stats.cpu_utilization > 80 ? '#ef4444' : 
                             stats.cpu_utilization > 50 ? '#f59e0b' : '#10b981',
                }}
              >
                {stats.cpu_utilization.toFixed(1)}%
              </Badge>
            </div>
            <ProgressBar 
              value={stats.cpu_utilization / 100} 
              style={{ height: 8 }}
            />
          </div>
          <Text size={200} style={{ color: '#94a3b8' }}>
            <i className="fas fa-tachometer-alt" style={{ marginRight: 6 }}></i>
            Frequency: {stats.cpu_frequency} MHz
          </Text>
        </div>

        {/* Memory Card */}
        <div className="glass-card" style={{ padding: 20 }}>
          <div style={{ display: 'flex', alignItems: 'center', marginBottom: 16, gap: 12 }}>
            <div className="category-icon" style={{ background: 'linear-gradient(135deg, #10b981, #3b82f6)' }}>
              <i className="fas fa-memory"></i>
            </div>
            <Text size={400} weight="semibold" style={{ color: '#f1f5f9' }}>Memory</Text>
          </div>
          <div style={{ marginBottom: 12 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 8 }}>
              <Text size={200} style={{ color: '#94a3b8' }}>Usage</Text>
              <Badge 
                appearance="filled"
                style={{
                  background: stats.memory_utilization > 80 ? '#ef4444' : 
                             stats.memory_utilization > 50 ? '#f59e0b' : '#10b981',
                }}
              >
                {stats.memory_utilization.toFixed(1)}%
              </Badge>
            </div>
            <ProgressBar 
              value={stats.memory_utilization / 100}
              style={{ height: 8 }}
            />
          </div>
          <Text size={200} style={{ color: '#94a3b8' }}>
            <i className="fas fa-database" style={{ marginRight: 6 }}></i>
            {stats.memory_used_gb.toFixed(1)} GB / {stats.memory_total_gb.toFixed(1)} GB
          </Text>
          <div style={{ marginTop: 8, paddingTop: 8, borderTop: '1px solid rgba(255, 255, 255, 0.1)' }}>
            <Text size={200} style={{ color: '#94a3b8' }}>
              <i className="fas fa-exchange-alt" style={{ marginRight: 6 }}></i>
              Swap: {stats.swap_utilization.toFixed(1)}%
            </Text>
          </div>
        </div>

        {/* Disk Cards */}
        {stats.disks && stats.disks.map((disk, index) => (
          <div key={index} className="glass-card" style={{ padding: 20 }}>
            <div style={{ display: 'flex', alignItems: 'center', marginBottom: 16, gap: 12 }}>
              <div className="category-icon" style={{ background: 'linear-gradient(135deg, #f59e0b, #ef4444)' }}>
                <i className="fas fa-hdd"></i>
              </div>
              <div style={{ flex: 1 }}>
                <Text size={400} weight="semibold" style={{ color: '#f1f5f9', marginLeft: 4 }}>{disk.mount_point}</Text>
                <Text size={100} style={{ color: '#94a3b8' }}>
                  {disk.name || 'Local Disk'} • {disk.disk_type} • {disk.file_system}
                </Text>
              </div>
            </div>
            <div style={{ marginBottom: 12 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 8 }}>
                <Text size={200} style={{ color: '#94a3b8' }}>Usage</Text>
                <Badge 
                  appearance="filled"
                  style={{
                    background: disk.utilization > 80 ? '#ef4444' : 
                               disk.utilization > 50 ? '#f59e0b' : '#10b981',
                  }}
                >
                  {disk.utilization.toFixed(1)}%
                </Badge>
              </div>
              <ProgressBar 
                value={disk.utilization / 100}
                style={{ height: 8 }}
              />
            </div>
            <Text size={200} style={{ color: '#94a3b8' }}>
              <i className="fas fa-chart-pie" style={{ marginRight: 6 }}></i>
              {disk.used_gb.toFixed(1)} GB / {disk.total_gb.toFixed(1)} GB
              <span style={{ opacity: 0.7, marginLeft: 8 }}>
                ({disk.available_gb.toFixed(1)} GB free)
              </span>
            </Text>
          </div>
        ))}

        {/* Network Card */}
        <div className="glass-card" style={{ padding: 20 }}>
          <div style={{ display: 'flex', alignItems: 'center', marginBottom: 16, gap: 12 }}>
            <div className="category-icon" style={{ background: 'linear-gradient(135deg, #8b5cf6, #ec4899)' }}>
              <i className="fas fa-network-wired"></i>
            </div>
            <Text size={400} weight="semibold" style={{ color: '#f1f5f9' }}>Network</Text>
          </div>
          <div style={{ display: 'flex', gap: 24, marginTop: 16 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <i className="fas fa-upload" style={{ color: '#8b5cf6' }}></i>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                <Text size={100} style={{ color: '#94a3b8' }}>Upload</Text>
                <Text size={300} weight="semibold" style={{ color: '#f1f5f9' }}>
                  {stats.network_upload_kb.toFixed(1)} KB/s
                </Text>
              </div>
            </div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <i className="fas fa-download" style={{ color: '#f59e0b' }}></i>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                <Text size={100} style={{ color: '#94a3b8' }}>Download</Text>
                <Text size={300} weight="semibold" style={{ color: '#f1f5f9' }}>
                  {stats.network_download_kb.toFixed(1)} KB/s
                </Text>
              </div>
            </div>
          </div>
        </div>

        {/* CPU History Chart */}
        <div className="glass-card" style={{ gridColumn: 'span 2', padding: 20 }}>
          <Text size={400} weight="semibold" style={{ marginBottom: 16, color: '#f1f5f9', display: 'block' }}>
            <i className="fas fa-chart-area" style={{ marginRight: 8, color: '#3b82f6' }}></i>
            CPU Usage History
          </Text>
          <div style={{ height: '200px' }}>
            <Line data={cpuChartData} options={chartOptions} />
          </div>
        </div>

        {/* Memory History Chart */}
        <div className="glass-card" style={{ gridColumn: 'span 2', padding: 20 }}>
          <Text size={400} weight="semibold" style={{ marginBottom: 16, color: '#f1f5f9', display: 'block' }}>
            <i className="fas fa-chart-area" style={{ marginRight: 8, color: '#10b981' }}></i>
            Memory Usage History
          </Text>
          <div style={{ height: '200px' }}>
            <Line data={memoryChartData} options={chartOptions} />
          </div>
        </div>

        {/* CPU Cores Chart */}
        <div className="glass-card" style={{ gridColumn: 'span 2', padding: 20 }}>
          <Text size={400} weight="semibold" style={{ marginBottom: 16, color: '#f1f5f9', display: 'block' }}>
            <i className="fas fa-chart-bar" style={{ marginRight: 8, color: '#8b5cf6' }}></i>
            CPU Cores
          </Text>
          <div style={{ height: '200px' }}>
            <Bar data={cpuCoreData} options={chartOptions} />
          </div>
        </div>

        {/* Network Chart */}
        <div className="glass-card" style={{ gridColumn: 'span 2', padding: 20 }}>
          <Text size={400} weight="semibold" style={{ marginBottom: 16, color: '#f1f5f9', display: 'block' }}>
            <i className="fas fa-chart-line" style={{ marginRight: 8, color: '#f59e0b' }}></i>
            Network Activity
          </Text>
          <div style={{ height: '200px' }}>
            <Line data={networkChartData} options={networkChartOptions} />
          </div>
        </div>

        {/* Network Connections */}
        <div className="glass-card" style={{ gridColumn: '1 / -1', padding: 20 }}>
          <div style={{ display: 'flex', alignItems: 'center', marginBottom: 16, gap: 12 }}>
            <Text size={400} weight="semibold" style={{ flex: 1, color: '#f1f5f9' }}>
              <i className="fas fa-plug" style={{ marginRight: 8, color: '#3b82f6' }}></i>
              Network Connections
            </Text>
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
              style={{
                background: 'rgba(59, 130, 246, 0.1)',
                border: '1px solid rgba(59, 130, 246, 0.3)',
                color: '#60a5fa',
              }}
            >
              <i className="fas fa-sync" style={{ marginRight: 6 }}></i>
              Refresh Connections
            </Button>
          </div>
          {showNetworkConnections && networkConnections.length > 0 && (
            <div style={{ overflowX: 'auto' }}>
              <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.1)', color: '#94a3b8' }}>Protocol</th>
                    <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.1)', color: '#94a3b8' }}>Local Address</th>
                    <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.1)', color: '#94a3b8' }}>Remote Address</th>
                    <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.1)', color: '#94a3b8' }}>Status</th>
                  </tr>
                </thead>
                <tbody>
                  {networkConnections.slice(0, 20).map((conn, index) => (
                    <tr key={index}>
                      <td style={{ padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.05)' }}>
                        <Badge appearance="tint" size="small" style={{ background: 'rgba(59, 130, 246, 0.1)', color: '#60a5fa' }}>
                          {conn.protocol}
                        </Badge>
                      </td>
                      <td style={{ padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.05)', color: '#f1f5f9', fontSize: 12 }}>
                        {conn.local_addr}
                      </td>
                      <td style={{ padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.05)', color: '#f1f5f9', fontSize: 12 }}>
                        {conn.remote_addr}
                      </td>
                      <td style={{ padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.05)' }}>
                        <Badge 
                          appearance="tint" 
                          size="small"
                          style={{
                            background: conn.status === 'ESTABLISHED' ? 'rgba(16, 185, 129, 0.1)' :
                                       conn.status === 'LISTENING' ? 'rgba(59, 130, 246, 0.1)' :
                                       conn.status === 'TIME_WAIT' ? 'rgba(245, 158, 11, 0.1)' :
                                       'rgba(148, 163, 184, 0.1)',
                            color: conn.status === 'ESTABLISHED' ? '#10b981' :
                                  conn.status === 'LISTENING' ? '#3b82f6' :
                                  conn.status === 'TIME_WAIT' ? '#f59e0b' :
                                  '#94a3b8',
                          }}
                        >
                          {conn.status}
                        </Badge>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {networkConnections.length > 20 && (
                <Text size={200} style={{ marginTop: 12, color: '#94a3b8' }}>
                  Showing first 20 of {networkConnections.length} connections
                </Text>
              )}
            </div>
          )}
          {showNetworkConnections && networkConnections.length === 0 && (
            <Text style={{ color: '#94a3b8' }}>No active network connections found</Text>
          )}
        </div>

        {/* Top Processes */}
        <div className="glass-card" style={{ gridColumn: '1 / -1', padding: 20 }}>
          <Text size={400} weight="semibold" style={{ marginBottom: 16, color: '#f1f5f9', display: 'block' }}>
            <i className="fas fa-list-ul" style={{ marginRight: 8, color: '#8b5cf6' }}></i>
            Top Processes (by CPU Usage)
          </Text>
          <div style={{ overflowX: 'auto' }}>
            <table style={{ width: '100%', borderCollapse: 'collapse' }}>
              <thead>
                <tr>
                  <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.1)', color: '#94a3b8' }}>PID</th>
                  <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.1)', color: '#94a3b8' }}>Name</th>
                  <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.1)', color: '#94a3b8' }}>Status</th>
                  <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.1)', color: '#94a3b8' }}>CPU %</th>
                  <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.1)', color: '#94a3b8' }}>Memory</th>
                  <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.1)', color: '#94a3b8' }}>Disk I/O</th>
                  <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.1)', color: '#94a3b8', maxWidth: 300 }}>Command</th>
                </tr>
              </thead>
              <tbody>
                {stats.top_processes.map((process) => (
                  <tr key={process.pid}>
                    <td style={{ padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.05)', color: '#f1f5f9', fontSize: 12 }}>
                      {process.pid}
                    </td>
                    <td style={{ padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.05)', color: '#f1f5f9', fontSize: 12 }} title={process.name}>
                      {process.name.length > 20 ? process.name.substring(0, 20) + '...' : process.name}
                    </td>
                    <td style={{ padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.05)' }}>
                      <Badge 
                        appearance="tint" 
                        size="small"
                        style={{
                          background: process.status === 'Running' ? 'rgba(16, 185, 129, 0.1)' : 'rgba(148, 163, 184, 0.1)',
                          color: process.status === 'Running' ? '#10b981' : '#94a3b8',
                        }}
                      >
                        {process.status}
                      </Badge>
                    </td>
                    <td style={{ padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.05)' }}>
                      <Badge 
                        appearance="filled"
                        style={{
                          background: process.cpu_percent > 50 ? '#ef4444' : 
                                     process.cpu_percent > 20 ? '#f59e0b' : '#10b981',
                        }}
                      >
                        {process.cpu_percent.toFixed(1)}%
                      </Badge>
                    </td>
                    <td style={{ padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.05)' }}>
                      <div>
                        <Text size={200} style={{ color: '#f1f5f9' }}>{process.memory_mb.toFixed(1)} MB</Text>
                        <Badge 
                          appearance="tint" 
                          size="small"
                          style={{
                            marginLeft: 8,
                            background: process.memory_percent > 50 ? 'rgba(239, 68, 68, 0.1)' : 
                                       process.memory_percent > 20 ? 'rgba(245, 158, 11, 0.1)' : 
                                       'rgba(59, 130, 246, 0.1)',
                            color: process.memory_percent > 50 ? '#ef4444' : 
                                  process.memory_percent > 20 ? '#f59e0b' : '#3b82f6',
                          }}
                        >
                          {process.memory_percent.toFixed(1)}%
                        </Badge>
                      </div>
                    </td>
                    <td style={{ padding: 8, borderBottom: '1px solid rgba(255, 255, 255, 0.05)', fontSize: 11, color: '#94a3b8' }}>
                      {process.disk_read_bytes > 0 || process.disk_write_bytes > 0 ? (
                        <>
                          <div>R: {formatBytes(process.disk_read_bytes)}</div>
                          <div>W: {formatBytes(process.disk_write_bytes)}</div>
                        </>
                      ) : (
                        <Text size={100} style={{ color: '#475569' }}>-</Text>
                      )}
                    </td>
                    <td style={{ 
                      padding: 8, 
                      borderBottom: '1px solid rgba(255, 255, 255, 0.05)', 
                      maxWidth: 300, 
                      overflow: 'hidden', 
                      textOverflow: 'ellipsis', 
                      whiteSpace: 'nowrap',
                      color: '#94a3b8',
                      fontSize: 11
                    }} title={process.command}>
                      {process.command}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      </div>
    </div>
  );
};