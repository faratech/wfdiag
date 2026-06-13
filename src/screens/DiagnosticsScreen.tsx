import React, { useMemo, useState } from 'react'
import { useAppContext } from '../contexts/AppContext'
import { useScanner } from '../hooks/useScanner'
import { taskIcon } from '../ui/diagnostic-icons'
import { formatDuration } from './util'
import { DiagnosticDetail, type DiagItem } from './DiagnosticDetail'

export const DiagnosticsScreen: React.FC = () => {
  const { availableTasks, results, isRunning, currentProgress, currentTaskName, scanStartTime, scanEndTime, taskStatuses, selectedDiagnosticId, setSelectedDiagnosticId, setPendingScanReport, setSelectedTab } = useAppContext()
  const { runQuickScan, runFullScan, stopScan } = useScanner()

  const [searchTerm, setSearchTerm] = useState('')

  // The selected diagnostic lives in AppContext so the command palette's "View
  // Result" deep-link can set it directly (no transient flag, no effect). When
  // nothing valid is selected we fall back to the first completed check below.

  // Per-category progress for the scanning hero ("Hardware 3/5"), derived from
  // the live task-progress events; only categories with started tasks appear
  const categoryProgress = useMemo(() => {
    if (!isRunning) return []
    const byCat: Record<string, { done: number; total: number }> = {}
    for (const task of availableTasks) {
      const status = taskStatuses[task.id]
      if (!status) continue
      const cat = (byCat[task.category] = byCat[task.category] || { done: 0, total: 0 })
      cat.total++
      if (status === 'done') cat.done++
    }
    return Object.entries(byCat)
  }, [isRunning, availableTasks, taskStatuses])

  const completed: DiagItem[] = useMemo(
    () => availableTasks.filter(t => results[t.id]).map(t => ({ ...t, result: results[t.id] })),
    [availableTasks, results]
  )

  const filtered = useMemo(() => {
    if (!searchTerm.trim()) return completed
    const q = searchTerm.toLowerCase()
    return completed.filter(t => t.name.toLowerCase().includes(q) || t.category.toLowerCase().includes(q))
  }, [completed, searchTerm])

  const grouped = useMemo(() => {
    const out: Record<string, DiagItem[]> = {}
    filtered.forEach(t => { (out[t.category] = out[t.category] || []).push(t) })
    return out
  }, [filtered])

  const passed = completed.filter(t => t.result.success).length
  const failed = completed.filter(t => !t.result.success).length
  const durationMs = scanEndTime > 0 ? scanEndTime - scanStartTime : 0
  const selected = completed.find(t => t.id === selectedDiagnosticId) || completed[0]

  if (isRunning) {
    return (
      <div className="scan-hero scanning">
        <div className="ring" style={{ '--p': currentProgress } as React.CSSProperties}>
          <span className="pct">{Math.round(currentProgress)}%</span>
        </div>
        <h1>Running diagnostic sweep…</h1>
        <p className="sub">Collecting hardware inventory, driver telemetry, storage health, network state, security posture and event logs.</p>
        <div className="progress-mini"><span style={{ width: `${currentProgress}%` }} /></div>
        {categoryProgress.length > 0 && (
          <div className="scan-cats">
            {categoryProgress.map(([cat, p]) => (
              <span key={cat} className={`tag ${p.done === p.total ? 'done' : 'neutral'}`}>
                {cat} {p.done}/{p.total}
              </span>
            ))}
          </div>
        )}
        <div className="task-line">{currentTaskName || 'Starting…'}</div>
        <button className="btn" onClick={stopScan}><i className="fa-solid fa-stop" /> Stop scan</button>
      </div>
    )
  }

  if (completed.length === 0) {
    return (
      <div className="scan-hero">
        <div style={{ width: 72, height: 72, borderRadius: 16, background: 'rgba(15,108,189,0.12)', display: 'grid', placeItems: 'center', marginBottom: 18 }}>
          <i className="fa-solid fa-stethoscope" style={{ fontSize: 32, color: 'var(--wf-paletteColor1)' }} />
        </div>
        <h1>Ready to diagnose</h1>
        <p className="sub">Run a Quick Scan to inventory your system. Most reads complete in seconds and never leave this machine.</p>
        <div className="row-gap-12">
          <button className="btn primary" onClick={() => runQuickScan()}><i className="fa-solid fa-bolt" /> Quick Scan</button>
          <button className="btn" onClick={() => runFullScan()}><i className="fa-solid fa-list-check" /> Full Scan</button>
        </div>
      </div>
    )
  }

  return (
    <>
      <div className="stat-strip screen-strip">
        <div className="stat-card passed">
          <div className="label">Passed</div>
          <div className="value">{passed}<span className="ch-sub"> / {completed.length}</span></div>
          <i className="fa-solid fa-circle-check icon" />
        </div>
        <div className="stat-card failed">
          <div className="label">Failed</div>
          <div className="value">{failed}</div>
          <i className="fa-solid fa-circle-xmark icon" />
        </div>
        <div className="stat-card brand">
          <div className="label">Checks</div>
          <div className="value">{completed.length}</div>
          <i className="fa-solid fa-list-check icon" />
        </div>
        <div className="stat-card brand">
          <div className="label">Duration</div>
          <div className="value">{durationMs > 0 ? (durationMs / 1000).toFixed(1) : '—'}<span className="ch-sub">{durationMs > 0 ? 's' : ''}</span></div>
          <i className="fa-solid fa-stopwatch icon" />
        </div>
        <button
          className="btn"
          style={{ alignSelf: 'center', whiteSpace: 'nowrap' }}
          title="Generate an AI health report for this scan"
          onClick={() => { setPendingScanReport(true); setSelectedTab('ai') }}
        >
          <i className="fa-solid fa-wand-magic-sparkles" /> Explain this scan
        </button>
      </div>

      <div className="split-pane">
        <div className="diag-list">
          <div className="diag-list-header">
            <span>{filtered.length} of {completed.length} diagnostics</span>
            {failed > 0 && <span className="failed-count">{failed} failed</span>}
          </div>
          <div className="diag-list-search">
            <input type="text" placeholder="Filter…" value={searchTerm} onChange={e => setSearchTerm(e.target.value)} />
          </div>
          <div className="diag-list-scroll">
            {Object.entries(grouped).map(([cat, items]) => (
              <div key={cat}>
                <div className="diag-cat"><span>{cat}</span><span className="cat-bar" /><span>{items.length}</span></div>
                {items.map(item => (
                  <div
                    key={item.id}
                    className={`diag-item ${(selected?.id === item.id) ? 'selected' : ''}`}
                    onClick={() => setSelectedDiagnosticId(item.id)}
                  >
                    <span className={`status-dot ${item.result.success ? 'success' : 'error'}`} />
                    <i className={`fa-solid ${taskIcon(item.id, item.category)}`} style={{ width: 13, color: 'var(--wf-text-muted)', fontSize: 12 }} />
                    <span className="name">{item.name}</span>
                    {item.result.duration_ms > 0 && <span className="duration">{formatDuration(item.result.duration_ms)}</span>}
                  </div>
                ))}
              </div>
            ))}
          </div>
        </div>
        {selected && <DiagnosticDetail key={selected.id} item={selected} />}
      </div>
    </>
  )
}
