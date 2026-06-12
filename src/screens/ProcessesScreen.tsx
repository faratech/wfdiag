import React, { useMemo, useState } from 'react'
import { useMonitoring } from '../hooks/useMonitoring'
import type { ProcessInfo } from '../types/monitoring'
import { Skeleton } from '../components/ui'
import { formatBytesMb } from './util'

type SortKey = 'name' | 'pid' | 'cpu_percent' | 'memory_mb' | 'thread_count' | 'user'
type SortDir = 'asc' | 'desc'

export const ProcessesScreen: React.FC = () => {
  const { processes, isActive, toggle, refresh } = useMonitoring({ autoStart: true, componentName: 'ProcessesScreen' })
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

  const cols: [SortKey, string][] = [
    ['name', 'Process'], ['pid', 'PID'], ['cpu_percent', 'CPU'],
    ['memory_mb', 'Memory'], ['thread_count', 'Threads'], ['user', 'User'],
  ]

  return (
    <>
      <div className="row-gap-12" style={{ padding: '0 24px 12px', justifyContent: 'space-between' }}>
        <div className="row-gap-12">
          <input
            className="field-input"
            type="text"
            placeholder="Filter processes…"
            aria-label="Filter processes"
            value={search}
            onChange={e => setSearch(e.target.value)}
            style={{ width: 240 }}
          />
          <span style={{ fontSize: 12, color: 'var(--wf-text-muted)' }}>
            Showing {sorted.length} of {processes.length} · {totalCpu.toFixed(1)}% CPU · {(totalMem / 1024).toFixed(1)} GB RAM
          </span>
        </div>
        <div className="row-gap-12">
          <button className="btn" onClick={() => refresh()}><i className="fa-solid fa-arrows-rotate" /> Refresh</button>
          <button className="btn" onClick={() => toggle()}><i className={`fa-solid ${isActive ? 'fa-pause' : 'fa-play'}`} /> {isActive ? 'Pause' : 'Resume'}</button>
        </div>
      </div>

      <div className="scrollable" style={{ padding: '0 24px 24px' }}>
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
                  <td colSpan={6} style={{ padding: 14 }}>
                    <Skeleton variant="text" count={8} height={22} />
                  </td>
                </tr>
              )}
              {sorted.map((p: ProcessInfo) => (
                <tr key={p.pid}>
                  <td>
                    <div className="pname">
                      <span className="pico">{(p.name[0] || '?').toUpperCase()}</span>
                      <div>
                        <div style={{ fontWeight: 600 }}>{p.name}</div>
                        <div style={{ fontSize: 11, color: 'var(--wf-text-muted)' }}>{p.command || p.exe_path || p.status}</div>
                      </div>
                    </div>
                  </td>
                  <td className="num">{p.pid}</td>
                  <td className="num">
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8, justifyContent: 'flex-end' }}>
                      <span>{p.cpu_percent.toFixed(1)}%</span>
                      <div className={`proc-bar ${p.cpu_percent > 10 ? 'warn' : ''} ${p.cpu_percent > 25 ? 'danger' : ''}`}><span style={{ width: `${Math.min(100, p.cpu_percent * 4)}%` }} /></div>
                    </div>
                  </td>
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
