import React, { useEffect, useState, useRef, useCallback, useMemo } from 'react';
import {
  ProgressBar,
  Text,
  Button,
  Badge,
  makeStyles,
  shorthands,
  tokens,
} from '@fluentui/react-components';
import {
  HeartPulse20Regular,
  Play20Regular,
  Stop20Regular,
} from '@fluentui/react-icons';
import { PageHeader, AIAnalysisPanel } from './components';
import { useAI } from './hooks/useAI';
import * as logger from './utils/logger';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import type { SystemStats, NetworkConnection } from './types/monitoring';

const useStyles = makeStyles({
  container: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalL),
  },
  headerActions: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
  },
  statusBadge: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
  },
});
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

// Sanitize data to prevent prototype pollution
const sanitizeChartData = (data: any): any => {
  if (typeof data !== 'object' || data === null) return data;
  if (Array.isArray(data)) return data.map(sanitizeChartData);

  const sanitized: any = {};
  for (const key in data) {
    if (data.hasOwnProperty(key) && !['__proto__', 'constructor', 'prototype'].includes(key)) {
      sanitized[key] = sanitizeChartData(data[key]);
    }
  }
  return sanitized;
};

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

// Types imported from ./types/monitoring

interface SystemMonitoringProps {
  isActive: boolean;
  onToggle: (active: boolean) => void;
}

const MAX_DATA_POINTS = 60; // 60 seconds of history (1 second intervals)

export const SystemMonitoring: React.FC<SystemMonitoringProps> = ({ isActive, onToggle }) => {
  const styles = useStyles();
  const [stats, setStats] = useState<SystemStats | null>(null);

  // AI analysis hooks
  const {
    requestMonitoringAnalysis,
    getCachedMonitoringAnalysis,
    isMonitoringAnalysisLoading,
    getMonitoringAnalysisError,
  } = useAI();
  const [cpuHistory, setCpuHistory] = useState<number[]>([]);
  const [memoryHistory, setMemoryHistory] = useState<number[]>([]);
  const [networkUploadHistory, setNetworkUploadHistory] = useState<number[]>([]);
  const [networkDownloadHistory, setNetworkDownloadHistory] = useState<number[]>([]);
  const [timeLabels, setTimeLabels] = useState<string[]>([]);
  const [networkConnections, setNetworkConnections] = useState<NetworkConnection[]>([]);
  const [showNetworkConnections, setShowNetworkConnections] = useState(false);
  const unlistenRef = useRef<(() => void) | null>(null);
  // Track if we've auto-started monitoring on mount
  const hasAutoStarted = useRef(false);

  // Clear all monitoring data
  const clearMonitoringData = () => {
    setStats(null);
    setCpuHistory([]);
    setMemoryHistory([]);
    setNetworkUploadHistory([]);
    setNetworkDownloadHistory([]);
    setTimeLabels([]);
    setNetworkConnections([]);
    setShowNetworkConnections(false);
  };

  // AI analysis handlers
  const handleMonitoringAnalysis = useCallback(() => {
    if (!stats) return;
    const statsJson = JSON.stringify({
      cpu_utilization: stats.cpu_utilization,
      cpu_frequency: stats.cpu_frequency,
      memory_utilization: stats.memory_utilization,
      memory_used_gb: stats.memory_used_gb,
      memory_total_gb: stats.memory_total_gb,
      swap_utilization: stats.swap_utilization,
      disks: stats.disks.map(d => ({
        name: d.name,
        mount_point: d.mount_point,
        utilization: d.utilization,
        used_gb: d.used_gb,
        total_gb: d.total_gb,
      })),
      network_upload_kb: stats.network_upload_kb,
      network_download_kb: stats.network_download_kb,
      npu_available: stats.npu_available,
      npu_name: stats.npu_name,
      npu_utilization: stats.npu_utilization,
    }, null, 2);
    requestMonitoringAnalysis(statsJson);
  }, [stats, requestMonitoringAnalysis]);

  // Stop monitoring and clear data when page becomes hidden
  useEffect(() => {
    const handleVisibilityChange = () => {
      if (document.hidden && isActive) {
        logger.info('SystemMonitoring', 'Page hidden, stopping monitoring');
        onToggle(false);
        clearMonitoringData();
      }
    };

    document.addEventListener('visibilitychange', handleVisibilityChange);
    return () => {
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  }, [isActive, onToggle]);

  // Auto-start monitoring when component mounts
  useEffect(() => {
    if (!isActive && !hasAutoStarted.current) {
      hasAutoStarted.current = true;
      onToggle(true);
    }
    // Clear data when component unmounts (tab switch)
    return () => {
      clearMonitoringData();
    };
  }, [isActive, onToggle]);

  useEffect(() => {
    let isMounted = true;

    const setupMonitoring = async () => {
      if (isActive) {
        // Start monitoring
        try {
          await invoke('start_monitoring');

          // Only set up listener if component is still mounted
          if (!isMounted) {
            await invoke('stop_monitoring').catch(err =>
              logger.error('SystemMonitoring', 'Failed to stop monitoring after unmount', err)
            );
            return;
          }

          // Listen for system stats events
          const unlisten = await listen<SystemStats>('system-stats', (event) => {
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

          unlistenRef.current = unlisten;
        } catch (error) {
          logger.error('SystemMonitoring', 'Failed to start monitoring', error);
          if (isMounted) {
            onToggle(false);
          }
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
          logger.error('SystemMonitoring', 'Failed to stop monitoring', error);
        }
      }
    };

    setupMonitoring();

    return () => {
      isMounted = false;

      // Clean up event listener
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }

      // Stop monitoring on unmount
      if (isActive) {
        invoke('stop_monitoring').catch(error =>
          logger.error('SystemMonitoring', 'Failed to stop monitoring on cleanup', error)
        );
      }
    };
  }, [isActive, onToggle]);

  // Memoize chart data to prevent unnecessary re-renders
  const cpuChartData = useMemo(() => sanitizeChartData({
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
  }), [timeLabels, cpuHistory]);

  const memoryChartData = useMemo(() => sanitizeChartData({
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
  }), [timeLabels, memoryHistory]);

  const networkChartData = useMemo(() => sanitizeChartData({
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
  }), [timeLabels, networkUploadHistory, networkDownloadHistory]);

  const cpuCoreData = useMemo(() => sanitizeChartData({
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
  }), [stats?.per_cpu_utilization]);

  // Get computed theme colors for charts - memoized to avoid repeated DOM queries
  const getThemeColor = useCallback((varName: string, fallback: string) => {
    if (typeof document !== 'undefined') {
      return getComputedStyle(document.documentElement).getPropertyValue(varName).trim() || fallback;
    }
    return fallback;
  }, []);

  // Memoize chart options to prevent unnecessary re-renders
  const chartOptions = useMemo(() => ({
    responsive: true,
    maintainAspectRatio: false,
    interaction: {
      mode: 'index' as const,
      intersect: false,
    },
    plugins: {
      legend: {
        display: true,
        position: 'top' as const,
        labels: {
          color: getThemeColor('--theme-foreground-2', '#64748b'),
          font: {
            family: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto',
          },
        },
      },
      tooltip: {
        backgroundColor: getThemeColor('--theme-background-3', 'rgba(30, 41, 59, 0.95)'),
        borderColor: 'rgba(59, 130, 246, 0.3)',
        borderWidth: 1,
        titleColor: getThemeColor('--theme-foreground', '#1a1d24'),
        bodyColor: getThemeColor('--theme-foreground-2', '#64748b'),
      },
    },
    scales: {
      y: {
        beginAtZero: true,
        max: 100,
        grid: {
          color: getThemeColor('--theme-stroke-2', 'rgba(148, 163, 184, 0.2)'),
        },
        ticks: {
          color: getThemeColor('--theme-foreground-2', '#64748b'),
        },
      },
      x: {
        grid: {
          color: getThemeColor('--theme-stroke-2', 'rgba(148, 163, 184, 0.2)'),
        },
        ticks: {
          color: getThemeColor('--theme-foreground-2', '#64748b'),
        },
      },
    },
  }), [getThemeColor]);

  const networkChartOptions = useMemo(() => ({
    ...chartOptions,
    scales: {
      ...chartOptions.scales,
      y: {
        ...chartOptions.scales.y,
        beginAtZero: true,
        max: undefined,
      },
    },
  }), [chartOptions]);

  const headerActions = (
    <div className={styles.headerActions}>
      <Button
        appearance={isActive ? "secondary" : "primary"}
        onClick={() => onToggle(!isActive)}
        icon={isActive ? <Stop20Regular /> : <Play20Regular />}
      >
        {isActive ? 'Stop Monitoring' : 'Start Monitoring'}
      </Button>
      <Badge
        appearance="filled"
        color={isActive ? 'success' : 'warning'}
        size="large"
      >
        <span className={styles.statusBadge}>
          <span style={{
            width: 6,
            height: 6,
            borderRadius: '50%',
            backgroundColor: isActive ? tokens.colorPaletteGreenForeground1 : tokens.colorPaletteYellowForeground1,
          }} />
          {isActive ? 'Live' : 'Inactive'}
        </span>
      </Badge>
    </div>
  );

  if (!isActive || !stats) {
    return (
      <div className={styles.container}>
        <PageHeader
          title="System Monitor"
          description={!isActive ? 'Real-time system performance monitoring' : 'Initializing monitoring services...'}
          icon={<HeartPulse20Regular />}
          actions={headerActions}
        />
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
          <Text size={500} weight="bold" block style={{ marginBottom: 12, color: 'var(--theme-foreground)' }}>
            System Monitoring {!isActive ? 'Inactive' : 'Starting...'}
          </Text>
          <Text size={300} block style={{ marginBottom: 24, color: 'var(--theme-foreground-2)' }}>
            {!isActive ? 'Click the button above to start real-time system monitoring' : 'Initializing monitoring services...'}
          </Text>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.container}>
      {/* Page Header */}
      <PageHeader
        title="System Monitor"
        description="Real-time system performance monitoring"
        icon={<HeartPulse20Regular />}
        actions={headerActions}
      />
      
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
            <Text size={400} weight="semibold" style={{ color: 'var(--theme-foreground)' }}>CPU</Text>
          </div>
          <div style={{ marginBottom: 12 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 8 }}>
              <Text size={200} style={{ color: 'var(--theme-foreground-2)' }}>Usage</Text>
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
          <Text size={200} style={{ color: 'var(--theme-foreground-2)' }}>
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
            <Text size={400} weight="semibold" style={{ color: 'var(--theme-foreground)' }}>Memory</Text>
          </div>
          <div style={{ marginBottom: 12 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 8 }}>
              <Text size={200} style={{ color: 'var(--theme-foreground-2)' }}>Usage</Text>
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
          <Text size={200} style={{ color: 'var(--theme-foreground-2)' }}>
            <i className="fas fa-database" style={{ marginRight: 6 }}></i>
            {stats.memory_used_gb.toFixed(1)} GB / {stats.memory_total_gb.toFixed(1)} GB
          </Text>
          <div style={{ marginTop: 8, paddingTop: 8, borderTop: '1px solid var(--theme-stroke-2)' }}>
            <Text size={200} style={{ color: 'var(--theme-foreground-2)' }}>
              <i className="fas fa-exchange-alt" style={{ marginRight: 6 }}></i>
              Swap: {stats.swap_utilization.toFixed(1)}%
            </Text>
          </div>
        </div>

        {/* NPU Card - only show if NPU is detected */}
        {stats.npu_available && (
          <div className="glass-card" style={{ padding: 20 }}>
            <div style={{ display: 'flex', alignItems: 'center', marginBottom: 16, gap: 12 }}>
              <div className="category-icon" style={{ background: 'linear-gradient(135deg, #8b5cf6, #ec4899)' }}>
                <i className="fas fa-brain"></i>
              </div>
              <Text size={400} weight="semibold" style={{ color: 'var(--theme-foreground)' }}>NPU</Text>
            </div>
            <div style={{ marginBottom: 12 }}>
              <div style={{ display: 'flex', alignItems: 'flex-start', marginBottom: 12 }}>
                <i className="fas fa-microchip" style={{ marginRight: 8, marginTop: 2, color: 'var(--theme-foreground-2)' }}></i>
                <Text size={200} style={{ color: 'var(--theme-foreground-2)', wordBreak: 'break-word' }}>
                  {stats.npu_name}
                </Text>
              </div>
              {stats.npu_utilization !== null ? (
                <div>
                  <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 8 }}>
                    <Text size={200} style={{ color: 'var(--theme-foreground-2)' }}>Usage</Text>
                    <Badge
                      appearance="filled"
                      style={{
                        background: stats.npu_utilization > 80 ? '#ef4444' :
                                   stats.npu_utilization > 50 ? '#f59e0b' : '#10b981',
                      }}
                    >
                      {stats.npu_utilization.toFixed(1)}%
                    </Badge>
                  </div>
                  <ProgressBar
                    value={stats.npu_utilization / 100}
                    style={{ height: 8 }}
                  />
                </div>
              ) : (
                <div style={{
                  display: 'flex',
                  alignItems: 'center',
                  padding: '8px 12px',
                  background: 'rgba(100, 116, 139, 0.1)',
                  borderRadius: 6,
                  border: '1px solid rgba(100, 116, 139, 0.2)'
                }}>
                  <i className="fas fa-info-circle" style={{ marginRight: 8, color: 'var(--theme-foreground-3)' }}></i>
                  <Text size={200} style={{ color: 'var(--theme-foreground-3)' }}>
                    {stats.npu_name?.toLowerCase().includes('qualcomm') || stats.npu_name?.toLowerCase().includes('hexagon')
                      ? 'Qualcomm NPU metrics not exposed via Windows APIs'
                      : 'Usage metrics not available'}
                  </Text>
                </div>
              )}
            </div>
          </div>
        )}

        {/* Disk Cards */}
        {stats.disks && stats.disks.map((disk, index) => (
          <div key={index} className="glass-card" style={{ padding: 20 }}>
            <div style={{ display: 'flex', alignItems: 'center', marginBottom: 16, gap: 12 }}>
              <div className="category-icon" style={{ background: 'linear-gradient(135deg, #f59e0b, #ef4444)' }}>
                <i className="fas fa-hdd"></i>
              </div>
              <div style={{ flex: 1 }}>
                <Text size={400} weight="semibold" style={{ color: 'var(--theme-foreground)', marginLeft: 4 }}>{disk.mount_point}</Text>
                <Text size={100} style={{ color: 'var(--theme-foreground-2)' }}>
                  {disk.name || 'Local Disk'} • {disk.disk_type} • {disk.file_system}
                </Text>
              </div>
            </div>
            <div style={{ marginBottom: 12 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 8 }}>
                <Text size={200} style={{ color: 'var(--theme-foreground-2)' }}>Usage</Text>
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
            <Text size={200} style={{ color: 'var(--theme-foreground-2)' }}>
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
            <Text size={400} weight="semibold" style={{ color: 'var(--theme-foreground)' }}>Network</Text>
          </div>
          <div style={{ display: 'flex', gap: 24, marginTop: 16 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <i className="fas fa-upload" style={{ color: '#8b5cf6' }}></i>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                <Text size={100} style={{ color: 'var(--theme-foreground-2)' }}>Upload</Text>
                <Text size={300} weight="semibold" style={{ color: 'var(--theme-foreground)' }}>
                  {stats.network_upload_kb.toFixed(1)} KB/s
                </Text>
              </div>
            </div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <i className="fas fa-download" style={{ color: '#f59e0b' }}></i>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                <Text size={100} style={{ color: 'var(--theme-foreground-2)' }}>Download</Text>
                <Text size={300} weight="semibold" style={{ color: 'var(--theme-foreground)' }}>
                  {stats.network_download_kb.toFixed(1)} KB/s
                </Text>
              </div>
            </div>
          </div>
        </div>

        {/* CPU History Chart */}
        <div className="glass-card" style={{ gridColumn: 'span 2', padding: 20 }}>
          <Text size={400} weight="semibold" style={{ marginBottom: 16, color: 'var(--theme-foreground)', display: 'block' }}>
            <i className="fas fa-chart-area" style={{ marginRight: 8, color: '#3b82f6' }}></i>
            CPU Usage History
          </Text>
          <div style={{ height: '200px' }}>
            <Line data={cpuChartData} options={chartOptions} />
          </div>
        </div>

        {/* Memory History Chart */}
        <div className="glass-card" style={{ gridColumn: 'span 2', padding: 20 }}>
          <Text size={400} weight="semibold" style={{ marginBottom: 16, color: 'var(--theme-foreground)', display: 'block' }}>
            <i className="fas fa-chart-area" style={{ marginRight: 8, color: '#10b981' }}></i>
            Memory Usage History
          </Text>
          <div style={{ height: '200px' }}>
            <Line data={memoryChartData} options={chartOptions} />
          </div>
        </div>

        {/* CPU Cores Chart */}
        <div className="glass-card" style={{ gridColumn: 'span 2', padding: 20 }}>
          <Text size={400} weight="semibold" style={{ marginBottom: 16, color: 'var(--theme-foreground)', display: 'block' }}>
            <i className="fas fa-chart-bar" style={{ marginRight: 8, color: '#8b5cf6' }}></i>
            CPU Cores
          </Text>
          <div style={{ height: '200px' }}>
            <Bar data={cpuCoreData} options={chartOptions} />
          </div>
        </div>

        {/* Network Chart */}
        <div className="glass-card" style={{ gridColumn: 'span 2', padding: 20 }}>
          <Text size={400} weight="semibold" style={{ marginBottom: 16, color: 'var(--theme-foreground)', display: 'block' }}>
            <i className="fas fa-chart-line" style={{ marginRight: 8, color: '#f59e0b' }}></i>
            Network Activity
          </Text>
          <div style={{ height: '200px' }}>
            <Line data={networkChartData} options={networkChartOptions} />
          </div>
        </div>

        {/* AI System Health Analysis */}
        <div style={{ gridColumn: '1 / -1' }}>
          <AIAnalysisPanel
            cacheKey="__monitoring_analysis__"
            title="AI System Health Analysis"
            hasData={!!stats}
            cachedResult={getCachedMonitoringAnalysis()}
            isLoading={isMonitoringAnalysisLoading()}
            error={getMonitoringAnalysisError()}
            onAnalyze={handleMonitoringAnalysis}
            analyzeButtonText="Analyze System"
            noDataMessage="Start monitoring to collect system data for AI analysis."
            readyMessage="Click to get AI-powered analysis of your system health, performance, and recommendations."
          />
        </div>

        {/* Network Connections */}
        <div className="glass-card" style={{ gridColumn: '1 / -1', padding: 20 }}>
          <div style={{ display: 'flex', alignItems: 'center', marginBottom: 16, gap: 12 }}>
            <Text size={400} weight="semibold" style={{ flex: 1, color: 'var(--theme-foreground)' }}>
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
                  logger.error('SystemMonitoring', 'Failed to get network connections', error);
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
                    <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid var(--theme-stroke-2)', color: 'var(--theme-foreground-2)' }}>Protocol</th>
                    <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid var(--theme-stroke-2)', color: 'var(--theme-foreground-2)' }}>Local Address</th>
                    <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid var(--theme-stroke-2)', color: 'var(--theme-foreground-2)' }}>Remote Address</th>
                    <th style={{ textAlign: 'left', padding: 8, borderBottom: '1px solid var(--theme-stroke-2)', color: 'var(--theme-foreground-2)' }}>Status</th>
                  </tr>
                </thead>
                <tbody>
                  {networkConnections.slice(0, 20).map((conn, index) => (
                    <tr key={index}>
                      <td style={{ padding: 8, borderBottom: '1px solid var(--theme-stroke-2)' }}>
                        <Badge appearance="tint" size="small" style={{ background: 'rgba(59, 130, 246, 0.1)', color: '#60a5fa' }}>
                          {conn.protocol}
                        </Badge>
                      </td>
                      <td style={{ padding: 8, borderBottom: '1px solid var(--theme-stroke-2)', color: 'var(--theme-foreground)', fontSize: 12 }}>
                        {conn.local_addr}
                      </td>
                      <td style={{ padding: 8, borderBottom: '1px solid var(--theme-stroke-2)', color: 'var(--theme-foreground)', fontSize: 12 }}>
                        {conn.remote_addr}
                      </td>
                      <td style={{ padding: 8, borderBottom: '1px solid var(--theme-stroke-2)' }}>
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
                <Text size={200} style={{ marginTop: 12, color: 'var(--theme-foreground-2)' }}>
                  Showing first 20 of {networkConnections.length} connections
                </Text>
              )}
            </div>
          )}
          {showNetworkConnections && networkConnections.length === 0 && (
            <Text style={{ color: 'var(--theme-foreground-2)' }}>No active network connections found</Text>
          )}
        </div>

      </div>
    </div>
  );
};