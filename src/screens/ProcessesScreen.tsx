import React, { useMemo, useState } from 'react'
import { EmptyState, Skeleton } from '../components/ui'
import { useProcessExplorer, type ProcessSortDirection, type ProcessSortKey } from '../hooks/useProcessExplorer'
import type { ProcessExplorerRow } from '../types/monitoring'
import { formatBytesMb } from './util'

const PAGE_SIZE = 100

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

function PercentCell({ value }: { value: number }) {
  const normalized = Math.max(0, Math.min(100, value))
  return (
    <div className="proc-percent">
      <span>{value.toFixed(1)}%</span>
      <div className={`proc-bar ${value > 50 ? 'warn' : ''} ${value > 80 ? 'danger' : ''}`} aria-hidden="true">
        <span style={{ width: `${normalized}%` }} />
      </div>
    </div>
  )
}

interface SortHeadingProps {
  field: ProcessSortKey
  children: React.ReactNode
  active: ProcessSortKey
  direction: ProcessSortDirection
  onSort: (field: ProcessSortKey) => void
  className?: string
}

function SortHeading({ field, children, active, direction, onSort, className }: SortHeadingProps) {
  return (
    <th className={className} aria-sort={active === field ? (direction === 'asc' ? 'ascending' : 'descending') : 'none'}>
      <button className="th-sort" onClick={() => onSort(field)}>
        {children}
        {active === field && <span className="sort-arrow" aria-hidden="true">{direction === 'asc' ? '↑' : '↓'}</span>}
      </button>
    </th>
  )
}

export const ProcessesScreen: React.FC = () => {
  const [sortBy, setSortBy] = useState<ProcessSortKey>('cpu_percent')
  const [sortDirection, setSortDirection] = useState<ProcessSortDirection>('desc')
  const [search, setSearch] = useState('')
  const [offset, setOffset] = useState(0)
  const [selected, setSelected] = useState<ProcessExplorerRow | null>(null)

  const query = useMemo(() => ({ search, sortBy, sortDirection, offset, limit: PAGE_SIZE }), [offset, search, sortBy, sortDirection])
  const { page, isLoading, isRefreshing, isPaused, error, refresh, togglePaused } = useProcessExplorer(query)
  const selectedProcess = selected
    ? page?.items.find(item => item.pid === selected.pid && item.start_time === selected.start_time) ?? null
    : null

  const setSort = (field: ProcessSortKey) => {
    setOffset(0)
    if (field === sortBy) {
      setSortDirection(current => current === 'desc' ? 'asc' : 'desc')
    } else {
      setSortBy(field)
      setSortDirection(field === 'name' || field === 'status' ? 'asc' : 'desc')
    }
  }

  const total = page?.total ?? 0
  // Process counts can shrink between requests; the backend returns the new
  // last valid page. Drive controls from that effective offset rather than a
  // now-stale requested offset.
  const pageOffset = page?.offset ?? offset
  const first = total === 0 ? 0 : pageOffset + 1
  const last = Math.min(total, pageOffset + (page?.items.length ?? 0))
  const canPrevious = pageOffset > 0
  const canNext = pageOffset + PAGE_SIZE < total

  return (
    <>
      <div className="screen-toolbar process-toolbar">
        <div className="row-gap-12 process-toolbar-main">
          <label className="sr-only" htmlFor="process-filter">Filter processes</label>
          <input
            id="process-filter"
            className="field-input filter-input"
            type="search"
            placeholder="Filter processes…"
            value={search}
            onChange={event => {
              setSearch(event.target.value)
              setOffset(0)
              setSelected(null)
            }}
          />
          <span className="process-summary" aria-live="polite">
            {error ? 'Process data unavailable' : `Showing ${first}–${last} of ${total} processes`}
          </span>
        </div>
        <div className="row-gap-12">
          <button className="btn" onClick={() => void refresh()} disabled={isRefreshing}>
            <i className={`fa-solid fa-arrows-rotate ${isRefreshing ? 'fa-spin' : ''}`} aria-hidden="true" /> Refresh
          </button>
          <button className="btn" onClick={togglePaused}>
            <i className={`fa-solid ${isPaused ? 'fa-play' : 'fa-pause'}`} aria-hidden="true" /> {isPaused ? 'Resume' : 'Pause'}
          </button>
        </div>
      </div>

      <div className="scrollable screen-pad process-screen-scroll">
        {error && page && (
          <div className="inline-error" role="alert">
            <span><i className="fa-solid fa-triangle-exclamation" /> {error}</span>
            <button className="btn" onClick={() => void refresh()}>Try again</button>
          </div>
        )}
        {error && !page ? (
          <EmptyState
            icon="fa-triangle-exclamation"
            title="Processes could not be loaded"
            sub={error}
            actions={<button className="btn primary" onClick={() => void refresh()}>Try again</button>}
          />
        ) : (
          <div className="process-layout">
            <div className="wf-block process-table-block">
              <div className="process-table-scroll" tabIndex={0} aria-label="Running processes table; scroll horizontally for more columns">
                <table className="proc-table">
                  <thead>
                    <tr>
                      <SortHeading field="name" active={sortBy} direction={sortDirection} onSort={setSort} className="process-col-name">Process</SortHeading>
                      <SortHeading field="pid" active={sortBy} direction={sortDirection} onSort={setSort} className="process-col-pid">PID</SortHeading>
                      <SortHeading field="cpu_percent" active={sortBy} direction={sortDirection} onSort={setSort} className="process-col-cpu">CPU</SortHeading>
                      <SortHeading field="memory_percent" active={sortBy} direction={sortDirection} onSort={setSort} className="process-col-memory">Memory</SortHeading>
                      <SortHeading field="status" active={sortBy} direction={sortDirection} onSort={setSort} className="process-col-status">Status</SortHeading>
                      <SortHeading field="thread_count" active={sortBy} direction={sortDirection} onSort={setSort} className="process-col-threads">Threads</SortHeading>
                    </tr>
                  </thead>
                  <tbody>
                    {isLoading && !page && (
                      <tr><td colSpan={6} className="process-loading"><Skeleton variant="text" count={8} height={22} /></td></tr>
                    )}
                    {!isLoading && page?.items.length === 0 && (
                      <tr><td colSpan={6} className="process-empty">No processes match “{search}”.</td></tr>
                    )}
                    {page?.items.map(process => {
                      const isSelected = selected?.pid === process.pid && selected.start_time === process.start_time
                      return (
                        <tr
                          key={`${process.pid}:${process.start_time}`}
                          className={isSelected ? 'selected' : undefined}
                          aria-selected={isSelected}
                          onClick={() => setSelected(process)}
                        >
                          <td className="process-col-name">
                            <button className="process-name-button" onClick={() => setSelected(process)}>
                              <i className={`fa-solid ${procIcon(process.name)} pico`} aria-hidden="true" />
                              <span>{process.name}</span>
                            </button>
                          </td>
                          <td className="num process-col-pid">{process.pid}</td>
                          <td className="num process-col-cpu"><PercentCell value={process.cpu_percent} /></td>
                          <td className="num process-col-memory">
                            <div className="process-memory-cell">
                              <span>{formatBytesMb(process.memory_mb)}</span>
                              <PercentCell value={process.memory_percent} />
                            </div>
                          </td>
                          <td className="process-col-status">{process.status}</td>
                          <td className="num process-col-threads">{process.thread_count}</td>
                        </tr>
                      )
                    })}
                  </tbody>
                </table>
              </div>
              <div className="process-pagination" aria-label="Process pages">
                <button className="btn" disabled={!canPrevious} onClick={() => setOffset(Math.max(0, pageOffset - PAGE_SIZE))}>Previous</button>
                <span>{first}–{last} of {total}</span>
                <button className="btn" disabled={!canNext} onClick={() => setOffset(pageOffset + PAGE_SIZE)}>Next</button>
              </div>
            </div>

            {selectedProcess && (
              <aside className="wf-block process-details" aria-label={`${selectedProcess.name} details`}>
                <div className="section-head">
                  <div>
                    <strong>{selectedProcess.name}</strong>
                    <span>PID {selectedProcess.pid}</span>
                  </div>
                  <button className="icon-btn" aria-label="Close process details" onClick={() => setSelected(null)}><i className="fa-solid fa-xmark" /></button>
                </div>
                <dl className="process-detail-grid">
                  <div><dt>CPU</dt><dd>{selectedProcess.cpu_percent.toFixed(1)}%</dd></div>
                  <div><dt>Memory</dt><dd>{formatBytesMb(selectedProcess.memory_mb)} ({selectedProcess.memory_percent.toFixed(1)}%)</dd></div>
                  <div><dt>Virtual memory</dt><dd>{formatBytesMb(selectedProcess.virtual_memory_mb)}</dd></div>
                  <div><dt>Threads</dt><dd>{selectedProcess.thread_count}</dd></div>
                  <div><dt>Handles</dt><dd>{selectedProcess.handle_count}</dd></div>
                  <div><dt>CPU time</dt><dd>{selectedProcess.cpu_time_secs}s</dd></div>
                  <div><dt>Read</dt><dd>{formatBytesMb(selectedProcess.io_read_bytes / (1024 * 1024))}</dd></div>
                  <div><dt>Written</dt><dd>{formatBytesMb(selectedProcess.io_write_bytes / (1024 * 1024))}</dd></div>
                </dl>
                <p className="muted-note">Path, owner, architecture, and elevation are omitted when Windows does not expose them without an additional privileged query.</p>
              </aside>
            )}
          </div>
        )}
      </div>
    </>
  )
}
