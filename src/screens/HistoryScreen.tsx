import React, { useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useScanHistory } from '../hooks/useScanHistory'
import { useComparison, TaskChange } from '../hooks/useComparison'
import { useJsonDiff, JsonDifference } from '../hooks/useJsonDiff'
import { useToast } from '../contexts/ToastContext'
import { Skeleton, EmptyState, Modal, Button } from '../components/ui'

interface TaskTrend {
  task_id: string
  failed: number
  seen_in: number
  scans_considered: number
}

const preStyle: React.CSSProperties = {
  margin: 0,
  padding: 8,
  fontSize: 11,
  fontFamily: 'var(--wf-font-mono)',
  whiteSpace: 'pre-wrap',
  wordBreak: 'break-word',
  maxHeight: 180,
  overflow: 'auto',
  background: 'rgba(0,0,0,0.04)',
  borderRadius: 4,
}

/** Side-by-side previous/current output, plus a field-level change list when both sides parse as JSON. */
const TaskDiffDetail: React.FC<{ change: TaskChange }> = ({ change }) => {
  const { findJsonDifferences, formatDifference } = useJsonDiff()
  const diffs: JsonDifference[] | null = useMemo(
    () => findJsonDifferences(change.previous_output, change.current_output),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [change.previous_output, change.current_output]
  )

  return (
    <div style={{ marginTop: 6 }}>
      {diffs && diffs.length > 0 && (
        <div style={{ marginBottom: 8 }}>
          {diffs.slice(0, 12).map((d, i) => (
            <div key={i} style={{ fontSize: 11, fontFamily: 'var(--wf-font-mono)', padding: '2px 0', color: d.type === 'added' ? 'var(--ok-fg)' : d.type === 'removed' ? 'var(--err-fg)' : 'var(--wf-text)' }}>
              {formatDifference(d)}
            </div>
          ))}
          {diffs.length > 12 && (
            <div style={{ fontSize: 11, color: 'var(--wf-text-muted)' }}>…and {diffs.length - 12} more changes</div>
          )}
        </div>
      )}
      <div className="diff-cols">
        <div>
          <div style={{ fontSize: 10, textTransform: 'uppercase', letterSpacing: '0.04em', color: 'var(--wf-text-muted)', fontWeight: 700, marginBottom: 4 }}>Previous</div>
          <pre style={preStyle}>{change.previous_output || '(empty)'}</pre>
        </div>
        <div>
          <div style={{ fontSize: 10, textTransform: 'uppercase', letterSpacing: '0.04em', color: 'var(--wf-text-muted)', fontWeight: 700, marginBottom: 4 }}>Current</div>
          <pre style={preStyle}>{change.current_output || '(empty)'}</pre>
        </div>
      </div>
    </div>
  )
}

type ChangeKind = 'regressed' | 'recovered' | 'changed'

const kindTag: Record<ChangeKind, { cls: string; label: string }> = {
  regressed: { cls: 'tag error', label: 'regressed' },
  recovered: { cls: 'tag success', label: 'recovered' },
  changed: { cls: 'tag info', label: 'changed' },
}

export const HistoryScreen: React.FC = () => {
  const { scans, loading, refreshScans } = useScanHistory()
  const { comparison, loading: cmpLoading, error: cmpError, compareScans, clearComparison } = useComparison()
  const { showSuccess, showError } = useToast()
  const [selected, setSelected] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [expandedTask, setExpandedTask] = useState<string | null>(null)
  const [trends, setTrends] = useState<Map<string, TaskTrend>>(new Map())
  const [labelDraft, setLabelDraft] = useState('')
  const [editingLabel, setEditingLabel] = useState(false)
  const [confirmClearOpen, setConfirmClearOpen] = useState(false)
  const [clearingHistory, setClearingHistory] = useState(false)

  const current = scans[0]

  useEffect(() => {
    invoke<TaskTrend[]>('get_task_trends', { limit: 10 })
      .then(list => setTrends(new Map(list.map(t => [t.task_id, t]))))
      .catch(() => setTrends(new Map())) // trends are decorative — never block the screen
  }, [scans.length])

  const filteredScans = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return scans
    return scans.filter(s =>
      (s.tags || []).some(t => t.toLowerCase().includes(q)) ||
      new Date(s.timestamp).toLocaleString().toLowerCase().includes(q) ||
      s.computer_name.toLowerCase().includes(q)
    )
  }, [scans, query])

  const select = (id: string) => {
    setSelected(id)
    setExpandedTask(null)
    setEditingLabel(false)
    clearComparison()
    if (current && id !== current.id) {
      // compareScans never rejects — failures land in cmpError, rendered below
      void compareScans(current.id, id)
    }
  }

  const clearHistory = async () => {
    setClearingHistory(true)
    try {
      await invoke('clear_scan_history')
      showSuccess('History cleared')
      await refreshScans()
      setSelected(null)
      clearComparison()
      setConfirmClearOpen(false)
    } catch (e) {
      showError('Failed to clear history', String(e))
    } finally {
      setClearingHistory(false)
    }
  }

  const selectedScan = scans.find(s => s.id === selected)

  const saveLabel = async () => {
    if (!selectedScan) return
    const label = labelDraft.trim()
    try {
      await invoke('update_scan_tags', { scanId: selectedScan.id, tags: label ? [label] : [] })
      setEditingLabel(false)
      showSuccess(label ? `Labeled "${label}"` : 'Label removed')
      await refreshScans()
    } catch (e) {
      showError('Failed to save label', String(e))
    }
  }

  // Unified changed-task list for the diff panel
  const changes: Array<{ kind: ChangeKind; change: TaskChange }> = comparison
    ? [
        ...comparison.new_failures.map(c => ({ kind: 'regressed' as ChangeKind, change: c })),
        ...comparison.new_successes.map(c => ({ kind: 'recovered' as ChangeKind, change: c })),
        ...comparison.status_unchanged.filter(c => c.output_changed).map(c => ({ kind: 'changed' as ChangeKind, change: c })),
      ]
    : []

  const trendBadge = (taskId: string) => {
    const t = trends.get(taskId)
    if (!t || t.failed < 2) return null
    return (
      <span className="tag warning" style={{ marginLeft: 6, padding: '0 6px' }} title={`This task failed in ${t.failed} of the last ${t.scans_considered} scans`}>
        {t.failed}/{t.scans_considered} fails
      </span>
    )
  }

  return (
    <>
      <div className="row-gap-12 screen-toolbar" style={{ justifyContent: 'space-between' }}>
        <div className="row-gap-12">
          <span style={{ fontSize: 13, color: 'var(--wf-text-muted)' }}>
            <strong style={{ color: 'var(--wf-text)' }}>{filteredScans.length}</strong>
            {query ? ` of ${scans.length}` : ''} scans
          </span>
          <input
            className="field-input filter-input"
            type="search"
            placeholder="Filter by label, date, machine…"
            value={query}
            onChange={e => setQuery(e.target.value)}
            aria-label="Filter scan history"
          />
        </div>
        <div className="row-gap-12">
          <button className="btn" onClick={() => refreshScans()}><i className="fa-solid fa-arrows-rotate" /> Refresh</button>
          <button className="btn danger" onClick={() => setConfirmClearOpen(true)} disabled={scans.length === 0}><i className="fa-solid fa-trash" /> Clear history</button>
        </div>
      </div>

      <div className="scrollable screen-pad history-grid">
        <div className="wf-block cq-block">
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
            {filteredScans.map(h => (
              <div key={h.id} className="history-row" style={{ background: selected === h.id ? 'rgba(15,108,189,0.12)' : 'transparent' }} onClick={() => select(h.id)}>
                <span className={`status-dot ${h.failure_count > 0 ? 'warning' : 'success'}`} />
                <span className="ts">{new Date(h.timestamp).toLocaleString()}</span>
                <span className="name">
                  {h.tags?.[0] || 'Scan'}
                  {current && h.id === current.id && <span className="tag" style={{ marginLeft: 6, padding: '0 6px' }}>Latest</span>}
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
            {!loading && scans.length > 0 && filteredScans.length === 0 && (
              <EmptyState icon="fa-magnifying-glass" title="No scans match" sub={`Nothing in the history matches "${query}".`} />
            )}
          </div>
        </div>

        <div className="wf-block">
          <header className="wf-block-header"><span className="accent-bar" /><span>Diff vs latest</span></header>
          <div style={{ padding: 14 }}>
            {!selectedScan && <div style={{ fontSize: 13, color: 'var(--wf-text-muted)' }}>Select a scan to compare it against the latest.</div>}
            {selectedScan && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10, fontSize: 12 }}>
                <span style={{ color: 'var(--wf-text-muted)' }}>Label:</span>
                {editingLabel ? (
                  <>
                    <input
                      className="field-input"
                      style={{ flex: 1, minWidth: 100, maxWidth: 200 }}
                      value={labelDraft}
                      autoFocus
                      onChange={e => setLabelDraft(e.target.value)}
                      onKeyDown={e => { if (e.key === 'Enter') saveLabel(); if (e.key === 'Escape') setEditingLabel(false) }}
                      aria-label="Scan label"
                    />
                    <button className="btn" style={{ padding: '2px 10px' }} onClick={saveLabel}>Save</button>
                  </>
                ) : (
                  <>
                    <strong>{selectedScan.tags?.[0] || 'Scan'}</strong>
                    <button
                      className="btn ghost"
                      style={{ padding: '2px 8px' }}
                      aria-label="Edit scan label"
                      onClick={() => { setLabelDraft(selectedScan.tags?.[0] || ''); setEditingLabel(true) }}
                    >
                      <i className="fa-solid fa-pen" />
                    </button>
                  </>
                )}
              </div>
            )}
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

                {comparison.total_changes === 0 && (
                  <div style={{ marginTop: 12, padding: 12, borderRadius: 6, background: 'var(--ok-bg)', color: 'var(--ok-fg)', fontSize: 13 }}>
                    <i className="fa-solid fa-circle-check" style={{ marginRight: 6 }} />
                    No drift — both scans produced identical results.
                  </div>
                )}

                {changes.map(({ kind, change }) => {
                  const open = expandedTask === `${kind}:${change.task_id}`
                  return (
                    <div key={`${kind}:${change.task_id}`} style={{ marginTop: 8, fontSize: 12 }}>
                      <button
                        className="btn ghost"
                        style={{ padding: '4px 6px', width: '100%', justifyContent: 'flex-start', textAlign: 'left' }}
                        aria-expanded={open}
                        onClick={() => setExpandedTask(open ? null : `${kind}:${change.task_id}`)}
                      >
                        <i className={`fa-solid ${open ? 'fa-chevron-down' : 'fa-chevron-right'}`} style={{ fontSize: 10, marginRight: 6 }} />
                        <span className={kindTag[kind].cls} style={{ marginRight: 6 }}>{kindTag[kind].label}</span>
                        {change.task_name}
                        {trendBadge(change.task_id)}
                      </button>
                      {open && <TaskDiffDetail change={change} />}
                    </div>
                  )
                })}
              </>
            )}
          </div>
        </div>
      </div>

      <Modal
        open={confirmClearOpen}
        onClose={() => setConfirmClearOpen(false)}
        title="Clear Scan History"
        width={420}
        footer={
          <>
            <Button onClick={() => setConfirmClearOpen(false)} disabled={clearingHistory}>Cancel</Button>
            <Button variant="danger" icon="fa-trash" onClick={() => { void clearHistory() }} disabled={clearingHistory}>
              {clearingHistory ? 'Clearing...' : 'Clear history'}
            </Button>
          </>
        }
      >
        <p style={{ marginTop: 0 }}>
          This permanently deletes {scans.length} saved scan{scans.length === 1 ? '' : 's'} and
          removes all comparison history.
        </p>
      </Modal>
    </>
  )
}
