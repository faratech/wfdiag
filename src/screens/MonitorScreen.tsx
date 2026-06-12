import React, { useState, useCallback } from 'react'
import { useMonitoring } from '../hooks/useMonitoring'
import type { SystemStats } from '../types/monitoring'
import { Skeleton } from '../components/ui'
import { ChartCard } from './ChartCard'

const HISTORY = 60

interface Series { cpu: number[]; mem: number[]; disk: number[]; net: number[]; npu: number[] }

export const MonitorScreen: React.FC = () => {
  const [series, setSeries] = useState<Series>({ cpu: [], mem: [], disk: [], net: [], npu: [] })
  // Last non-null sample, kept so the header/cards don't blank out when the
  // monitoring hook momentarily reports null stats (e.g. while paused)
  const [lastStats, setLastStats] = useState<SystemStats | null>(null)

  const onStats = useCallback((s: SystemStats) => {
    setLastStats(s)
    const push = (arr: number[], v: number) => {
      const next = [...arr, v]
      return next.length > HISTORY ? next.slice(next.length - HISTORY) : next
    }
    setSeries(prev => ({
      cpu: push(prev.cpu, s.cpu_utilization),
      mem: push(prev.mem, s.memory_utilization),
      disk: push(prev.disk, s.disk_utilization),
      net: push(prev.net, (s.network_upload_kb + s.network_download_kb) / 1024),
      npu: push(prev.npu, s.npu_utilization ?? 0),
    }))
  }, [])

  const { isActive, stats, toggle, refresh } = useMonitoring({ autoStart: true, onStats, componentName: 'MonitorScreen' })

  const last = stats || lastStats
  const netMax = Math.max(2, ...series.net) * 1.2 || 2

  return (
    <>
      <div className="row-gap-12" style={{ padding: '0 24px 12px', justifyContent: 'space-between' }}>
        <div className="row-gap-12">
          <span className={`tag ${isActive ? 'success' : 'neutral'}`}>
            <span className="status-dot" style={{ background: isActive ? 'var(--ok-fg)' : 'var(--wf-text-muted)', animation: isActive ? 'pulse 1.4s ease-in-out infinite' : 'none' }} />
            {isActive ? 'Live · sampling' : 'Paused'}
          </span>
          {last && (
            <span style={{ fontSize: 12, color: 'var(--wf-text-muted)' }}>
              {last.per_cpu_utilization?.length || 0} threads · {last.memory_total_gb.toFixed(1)} GB RAM
              {last.npu_available ? ` · NPU: ${last.npu_name ?? 'present'}` : ''}
            </span>
          )}
        </div>
        <div className="row-gap-12">
          <button className="btn" onClick={() => toggle()}>
            <i className={`fa-solid ${isActive ? 'fa-pause' : 'fa-play'}`} /> {isActive ? 'Pause' : 'Resume'}
          </button>
          <button className="btn" onClick={() => refresh()}><i className="fa-solid fa-arrows-rotate" /> Refresh</button>
        </div>
      </div>

      <div className="scrollable" style={{ padding: '0 24px 24px' }}>
        {!last && (
          <div className="charts-grid" aria-hidden="true">
            {Array.from({ length: 4 }, (_, i) => (
              <Skeleton key={i} variant="block" height={150} />
            ))}
          </div>
        )}
        {last && (
        <div className="charts-grid">
          <ChartCard title="CPU" value={(last?.cpu_utilization ?? 0).toFixed(1)} sub="%" color="var(--wf-paletteColor1)" series={series.cpu} max={100} hint={last?.cpu_frequency ? `${(last.cpu_frequency / 1000).toFixed(2)} GHz` : 'Processor utilization'} />
          <ChartCard title="Memory" value={(last?.memory_utilization ?? 0).toFixed(1)} sub="%" color="#0e8fb8" series={series.mem} max={100} hint={last ? `${last.memory_used_gb.toFixed(1)} / ${last.memory_total_gb.toFixed(1)} GB used` : ''} />
          <ChartCard title="Disk" value={(last?.disk_utilization ?? 0).toFixed(1)} sub="%" color="#7a4cc9" series={series.disk} max={100} hint="Active disk time" />
          <ChartCard title="Network" value={(((last?.network_upload_kb ?? 0) + (last?.network_download_kb ?? 0)) / 1024).toFixed(2)} sub="MB/s" color="#0f8a5a" series={series.net} max={netMax} hint="Up + down throughput" />
          {last?.npu_available && (
            <ChartCard title="NPU" value={(last?.npu_utilization ?? 0).toFixed(1)} sub="%" color="#c98a00" series={series.npu} max={100} hint={last?.npu_name ?? 'Neural processing unit'} />
          )}
        </div>
        )}
      </div>
    </>
  )
}
