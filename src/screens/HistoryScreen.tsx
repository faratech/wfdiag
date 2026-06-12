import React, { useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useScanHistory } from '../hooks/useScanHistory'
import { useComparison } from '../hooks/useComparison'
import { useToast } from '../contexts/ToastContext'
import { Skeleton, EmptyState } from '../components/ui'

export const HistoryScreen: React.FC = () => {
  const { scans, loading, refreshScans } = useScanHistory()
  const { comparison, loading: cmpLoading, error: cmpError, compareScans, clearComparison } = useComparison()
  const { showSuccess, showError } = useToast()
  const [selected, setSelected] = useState<string | null>(null)

  const current = scans[0]

  const select = (id: string) => {
    setSelected(id)
    clearComparison()
    if (current && id !== current.id) {
      // compareScans never rejects — failures land in cmpError, rendered below
      void compareScans(current.id, id)
    }
  }

  const clearHistory = async () => {
    try {
      await invoke('clear_scan_history')
      showSuccess('History cleared')
      await refreshScans()
      setSelected(null)
      clearComparison()
    } catch (e) {
      showError('Failed to clear history', String(e))
    }
  }

  const selectedScan = scans.find(s => s.id === selected)

  return (
    <>
      <div className="row-gap-12" style={{ padding: '0 24px 12px', justifyContent: 'space-between' }}>
        <div className="row-gap-12">
          <span style={{ fontSize: 13, color: 'var(--wf-text-muted)' }}>
            <strong style={{ color: 'var(--wf-text)' }}>{scans.length}</strong> scans
          </span>
        </div>
        <div className="row-gap-12">
          <button className="btn" onClick={() => refreshScans()}><i className="fa-solid fa-arrows-rotate" /> Refresh</button>
          <button className="btn danger" onClick={clearHistory}><i className="fa-solid fa-trash" /> Clear history</button>
        </div>
      </div>

      <div className="scrollable" style={{ padding: '0 24px 24px', display: 'grid', gridTemplateColumns: '1.4fr 1fr', gap: 14 }}>
        <div className="wf-block">
          <header className="wf-block-header">
            <span className="accent-bar" />
            <span>Scan Sessions</span>
            <span className="count">{loading ? 'Loading…' : 'Click to compare vs latest'}</span>
          </header>
          <div>
            <div className="history-row" style={{ background: 'rgba(0,0,0,0.03)', fontSize: 10, textTransform: 'uppercase', letterSpacing: '0.04em', color: 'var(--wf-text-muted)', fontWeight: 700 }}>
              <span></span><span>Timestamp</span><span>Label</span>
              <span style={{ textAlign: 'right' }}>Pass</span>
              <span style={{ textAlign: 'right' }}>Fail</span>
              <span style={{ textAlign: 'right' }}>Time</span>
            </div>
            {loading && scans.length === 0 && (
              <div style={{ padding: 14 }}>
                <Skeleton variant="block" count={4} height={38} />
              </div>
            )}
            {scans.map((h, idx) => (
              <div key={h.id} className="history-row" style={{ background: selected === h.id ? 'rgba(15,108,189,0.12)' : 'transparent' }} onClick={() => select(h.id)}>
                <span className={`status-dot ${h.failure_count > 0 ? 'warning' : 'success'}`} />
                <span className="ts">{new Date(h.timestamp).toLocaleString()}</span>
                <span className="name">
                  {h.tags?.[0] || 'Scan'}
                  {idx === 0 && <span className="tag" style={{ marginLeft: 6, padding: '0 6px' }}>Latest</span>}
                </span>
                <span className="stat passed">{h.success_count}</span>
                <span className="stat failed">{h.failure_count > 0 ? h.failure_count : '—'}</span>
                <span className="stat">{(h.duration_ms / 1000).toFixed(1)}s</span>
              </div>
            ))}
            {!loading && scans.length === 0 && (
              <EmptyState
                icon="fa-clock-rotate-left"
                title="No saved scans yet"
                sub="Run and save a scan to start tracking drift between sessions."
              />
            )}
          </div>
        </div>

        <div className="wf-block">
          <header className="wf-block-header"><span className="accent-bar" /><span>Diff vs latest</span></header>
          <div style={{ padding: 14 }}>
            {!selectedScan && <div style={{ fontSize: 13, color: 'var(--wf-text-muted)' }}>Select a scan to compare it against the latest.</div>}
            {selectedScan && current && selectedScan.id === current.id && (
              <div style={{ fontSize: 13, color: 'var(--wf-text-muted)' }}>This is the latest scan. Select an older scan to see drift.</div>
            )}
            {cmpLoading && <div style={{ fontSize: 13, color: 'var(--wf-text-muted)' }}><i className="fa-solid fa-circle-notch fa-spin" /> Comparing…</div>}
            {cmpError && <div style={{ fontSize: 13, color: 'var(--err-fg)' }}>{cmpError}</div>}
            {comparison && (
              <>
                <div style={{ fontSize: 13, color: 'var(--wf-text-muted)', marginBottom: 12 }}>
                  Comparing <strong style={{ color: 'var(--wf-text)' }}>{new Date(comparison.previous_scan.timestamp).toLocaleString()}</strong> against the latest scan — <strong style={{ color: 'var(--wf-text)' }}>{comparison.total_changes}</strong> changes.
                </div>
                <div className="kv-grid" style={{ gridTemplateColumns: '1fr 90px' }}>
                  <div className="k" style={{ fontFamily: 'inherit' }}>New failures</div>
                  <div className="v" style={{ textAlign: 'right', color: 'var(--err-fg)' }}>{comparison.new_failures.length}</div>
                  <div className="k" style={{ fontFamily: 'inherit' }}>New successes</div>
                  <div className="v" style={{ textAlign: 'right', color: 'var(--ok-fg)' }}>{comparison.new_successes.length}</div>
                  <div className="k" style={{ fontFamily: 'inherit' }}>Output changed</div>
                  <div className="v" style={{ textAlign: 'right' }}>{comparison.status_unchanged.filter(c => c.output_changed).length}</div>
                </div>
                {comparison.new_failures.slice(0, 8).map(c => (
                  <div key={c.task_id} style={{ marginTop: 8, fontSize: 12 }}>
                    <span className="tag error" style={{ marginRight: 6 }}>regressed</span>{c.task_name}
                  </div>
                ))}
              </>
            )}
          </div>
        </div>
      </div>
    </>
  )
}
