import React, { useMemo, useState } from 'react'
import { useMonitoring } from '../hooks/useMonitoring'
import type { ProcessInfo } from '../types/monitoring'
import { Skeleton } from '../components/ui'
import { formatBytesMb } from './util'

type SortKey =
  | 'name'
  | 'pid'
  | 'cpu_percent'
  | 'gpu_percent'
  | 'gpu_memory_mb'
  | 'npu_percent'
  | 'npu_memory_mb'
  | 'memory_mb'
  | 'thread_count'
  | 'user'
type SortDir = 'asc' | 'desc'

/** Muted per-process glyph, keyed off the executable name. */
function procIcon(name: string): string {
  const n = name.toLowerCase()
  if (n.includes('edge') || n.includes('chrome') || n.includes('firefox') || n.includes('browser') || n.includes('opera')) return 'fa-globe'
  if (n.includes('code') || n.includes('devenv') || n.includes('studio')) return 'fa-code'
  if (n.includes('explorer')) return 'fa-folder-open'
  if (n.includes('svchost') || n.includes('services') || n.startsWith('svc')) return 'fa-gears'
  if (n.includes('teams') || n.includes('slack') || n.includes('discord') || n.includes('zoom')) return 'fa-users'
  if (n.includes('defender') || n.includes('msmpeng') || n.includes('security') || n.includes('antimal')) return 'fa-shield-halved'
  if (n.includes('dwm') || n.includes('desktop')) return 'fa-layer-group'
  if (n.includes('audio')) return 'fa-volume-high'
  if (n.includes('onedrive') || n.includes('dropbox') || n.includes('cloud')) return 'fa-cloud'
  if (n.includes('search') || n.includes('index')) return 'fa-magnifying-glass'
  if (n === 'system' || n.includes('kernel') || n.includes('ntoskrnl') || n.includes('registry')) return 'fa-microchip'
  if (n.includes('python') || n.includes('node') || n.includes('java') || n.includes('powershell') || n.includes('cmd')) return 'fa-terminal'
  if (n.includes('wfdiag') || n.includes('diagnostic')) return 'fa-stethoscope'
  return 'fa-window-maximize'
}

export const ProcessesScreen: React.FC = () => {
  const { processes, stats, isActive, isLoading, toggle, refresh } = useMonitoring({
    autoStart: true,
    componentName: 'ProcessesScreen',
    includeProcessAdapterStats: true,
  })
  const [sortBy, setSortBy] = useState<SortKey>('cpu_percent')
  const [sortDir, setSortDir] = useState<SortDir>('desc')
  const [search, setSearch] = useState('')

  const setSort = (key: SortKey) => {
    if (key === sortBy) {
      setSortDir(d => (d === 'desc' ? 'asc' : 'desc'))
    } else {
      setSortBy(key)
      // Numeric columns read best descending, text columns ascending
      setSortDir(key === 'name' || key === 'user' ? 'asc' : 'desc')
    }
  }

  const sorted = useMemo(() => {
    let list = [...processes]
    if (search.trim()) {
      const q = search.toLowerCase()
      list = list.filter(p => p.name.toLowerCase().includes(q) || (p.command || '').toLowerCase().includes(q))
    }
    const dir = sortDir === 'desc' ? 1 : -1
    list.sort((a, b) => {
      const av = a[sortBy] as number | string
      const bv = b[sortBy] as number | string
      const cmp = typeof bv === 'number' ? (bv as number) - (av as number) : String(bv).localeCompare(String(av))
      return cmp * dir
    })
    return list
  }, [processes, sortBy, sortDir, search])

  const totalCpu = processes.reduce((s, p) => s + p.cpu_percent, 0)
  const totalMem = processes.reduce((s, p) => s + p.memory_mb, 0)
  const showGpu = Boolean(stats?.gpu_available || processes.some(p => p.gpu_percent > 0 || p.gpu_memory_mb > 0))
  const showNpu = Boolean(stats?.npu_available || processes.some(p => p.npu_percent > 0 || p.npu_memory_mb > 0))

  const cols: [SortKey, string][] = [
    ['name', 'Process'],
    ['pid', 'PID'],
    ['cpu_percent', 'CPU'],
    ...(showGpu ? [['gpu_percent', 'GPU'], ['gpu_memory_mb', 'GPU Mem']] as [SortKey, string][] : []),
    ...(showNpu ? [['npu_percent', 'NPU'], ['npu_memory_mb', 'NPU Mem']] as [SortKey, string][] : []),
    ['memory_mb', 'Memory'],
    ['thread_count', 'Threads'],
    ['user', 'User'],
  ]
  const summary = [
    `Showing ${sorted.length} of ${processes.length}`,
    `${totalCpu.toFixed(1)}% CPU`,
    `${(totalMem / 1024).toFixed(1)} GB RAM`,
    ...(showGpu ? [`${(stats?.gpu_utilization ?? 0).toFixed(1)}% GPU`] : []),
    ...(showNpu ? [`${(stats?.npu_utilization ?? 0).toFixed(1)}% NPU`] : []),
  ].join(' · ')

  const renderPercentCell = (value: number) => (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, justifyContent: 'flex-end' }}>
      <span>{value.toFixed(1)}%</span>
      <div className={`proc-bar ${value > 10 ? 'warn' : ''} ${value > 25 ? 'danger' : ''}`}>
        <span style={{ width: `${Math.min(100, value * 4)}%` }} />
      </div>
    </div>
  )

  return (
    <>
      <div className="row-gap-12 screen-toolbar" style={{ justifyContent: 'space-between' }}>
        <div className="row-gap-12">
          <input
            className="field-input filter-input"
            type="text"
            placeholder="Filter processes…"
            aria-label="Filter processes"
            value={search}
            onChange={e => setSearch(e.target.value)}
          />
          <span style={{ fontSize: 12, color: 'var(--wf-text-muted)' }}>
            {summary}
          </span>
        </div>
        <div className="row-gap-12">
          <button className="btn" onClick={() => refresh()}><i className="fa-solid fa-arrows-rotate" /> Refresh</button>
          <button className="btn" onClick={() => toggle()} disabled={isLoading}><i className={`fa-solid ${isActive ? 'fa-pause' : 'fa-play'}`} /> {isActive ? 'Pause' : 'Resume'}</button>
        </div>
      </div>

      <div className="scrollable screen-pad">
        <div className="wf-block">
          <table className="proc-table">
            <thead>
              <tr>
                {cols.map(([k, label]) => (
                  <th key={k} aria-sort={sortBy === k ? (sortDir === 'asc' ? 'ascending' : 'descending') : undefined}>
                    <button className="th-sort" onClick={() => setSort(k)}>
                      {label}{sortBy === k && <span className="sort-arrow">{sortDir === 'asc' ? '↑' : '↓'}</span>}
                    </button>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {processes.length === 0 && (
                <tr>
                  <td colSpan={cols.length} style={{ padding: 14 }}>
                    <Skeleton variant="text" count={8} height={22} />
                  </td>
                </tr>
              )}
              {sorted.map((p: ProcessInfo) => (
                <tr key={p.pid}>
                  <td>
                    <div className="pname">
                      <i className={`fa-solid ${procIcon(p.name)} pico`} aria-hidden="true" />
                      <div>
                        <div>{p.name}</div>
                        <div style={{ fontSize: 11, color: 'var(--wf-text-muted)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{p.command || p.exe_path || p.status}</div>
                      </div>
                    </div>
                  </td>
                  <td className="num">{p.pid}</td>
                  <td className="num">{renderPercentCell(p.cpu_percent)}</td>
                  {showGpu && (
                    <>
                      <td className="num">{renderPercentCell(p.gpu_percent)}</td>
                      <td className="num">{formatBytesMb(p.gpu_memory_mb)}</td>
                    </>
                  )}
                  {showNpu && (
                    <>
                      <td className="num">{renderPercentCell(p.npu_percent)}</td>
                      <td className="num">{formatBytesMb(p.npu_memory_mb)}</td>
                    </>
                  )}
                  <td className="num">
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8, justifyContent: 'flex-end' }}>
                      <span>{formatBytesMb(p.memory_mb)}</span>
                      <div className="proc-bar"><span style={{ width: `${Math.min(100, p.memory_mb / 20)}%` }} /></div>
                    </div>
                  </td>
                  <td className="num">{p.thread_count}</td>
                  <td>{p.user}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </>
  )
}
