import React, { useMemo, useState } from 'react'
import { useAppContext } from '../contexts/AppContext'
import { useScanner } from '../hooks/useScanner'
import { formatDuration } from './util'
import { DiagnosticDetail, type DiagItem } from './DiagnosticDetail'

export const DiagnosticsScreen: React.FC = () => {
  const { availableTasks, results, isRunning, currentProgress, currentTaskName, scanStartTime, scanEndTime, taskStatuses, selectedDiagnosticId, setSelectedDiagnosticId, setPendingScanReport, setSelectedTab, systemInfo } = useAppContext()
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
  const machine = systemInfo?.computer_name || 'this PC'

  if (isRunning) {
    // r=53 → circumference ≈ 333; the progress arc's dash length scales with %.
    const dash = (currentProgress * 3.33).toFixed(1)
    return (
      <div className="scan-hero scanning">
        <div className="scan-ring">
          <svg width="124" height="124" viewBox="0 0 124 124">
            <circle className="ring-track" cx="62" cy="62" r="53" fill="none" strokeWidth="6" />
            <circle className="ring-prog" cx="62" cy="62" r="53" fill="none" strokeWidth="6" strokeLinecap="round" strokeDasharray={`${dash} 333`} transform="rotate(-90 62 62)" />
          </svg>
          <div className="pct">{Math.round(currentProgress)}%</div>
        </div>
        <h2 className="scan-title">Scanning {machine}…</h2>
        <div className="task-line">{currentTaskName || 'Starting…'}</div>
        <div className="progress-mini"><span style={{ width: `${currentProgress}%` }} /></div>
        {categoryProgress.length > 0 && (
          <div className="scan-cats">
            {categoryProgress.map(([cat, p]) => (
              <span key={cat} className={`cat-pill ${p.done === p.total ? 'done' : ''}`}>{cat} {p.done}/{p.total}</span>
            ))}
          </div>
        )}
        <button className="btn" style={{ marginTop: 24, height: 33 }} onClick={stopScan}><i className="fa-solid fa-stop" /> Stop scan</button>
      </div>
    )
  }

  if (completed.length === 0) {
    return (
      <div className="scan-hero">
        <div className="hero-tile"><i className="fa-solid fa-stethoscope" /></div>
        <h1>Ready to diagnose</h1>
        <p className="sub">Run a Quick Scan to inventory this PC. Checks are read-only, finish in seconds, and never leave this machine.</p>
        <div className="hero-actions">
          <button className="btn primary" onClick={() => runQuickScan()}><i className="fa-solid fa-bolt" /> Quick Scan</button>
          <button className="btn" onClick={() => runFullScan()}><i className="fa-solid fa-list-check" /> Full Scan</button>
        </div>
      </div>
    )
  }

  return (
    <div className="results-wrap">
      <div className="stat-row">
        <div className="st">
          <div className="label">Passed</div>
          <div className="value"><span className="accent-n">{passed}</span><span className="unit"> / {completed.length}</span></div>
        </div>
        <div className={`st ${failed > 0 ? 'failed' : ''}`}>
          <div className="label">Failed</div>
          <div className="value">{failed}</div>
        </div>
        <div className="st">
          <div className="label">Duration</div>
          <div className="value">{durationMs > 0 ? (durationMs / 1000).toFixed(1) : '—'}{durationMs > 0 && <span className="unit">s</span>}</div>
        </div>
        <div className="spacer" />
        <button
          className="btn"
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
            <input type="text" placeholder="Filter diagnostics…" value={searchTerm} onChange={e => setSearchTerm(e.target.value)} />
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
    </div>
  )
}
