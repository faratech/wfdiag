import React, { useEffect, useMemo, useRef, useState } from 'react'
import { useCommands, type Command } from '../hooks/useCommands'
import { fuzzyScore } from '../utils/fuzzy'
import { Kbd } from './ui'

interface CommandPaletteProps {
  open: boolean
  onClose: () => void
}

interface Scored {
  command: Command
  score: number
  indices: number[]
}

const SECTION_ORDER: Command['section'][] = ['Navigate', 'Scan', 'Report', 'App', 'Diagnostics']
const MAX_RESULTS = 14

/** Title with fuzzy-matched characters highlighted. */
const Highlighted: React.FC<{ text: string; indices: number[] }> = ({ text, indices }) => {
  if (indices.length === 0) return <>{text}</>
  const set = new Set(indices)
  return (
    <>
      {text.split('').map((ch, i) =>
        set.has(i) ? <span key={i} className="palette-match">{ch}</span> : ch
      )}
    </>
  )
}

// Mounted only while open (the wrapper below gates on it), so query/selection
// start fresh every time — no in-render reset of leftover state.
const CommandPaletteBody: React.FC<{ onClose: () => void }> = ({ onClose }) => {
  const commands = useCommands()
  const [query, setQuery] = useState('')
  const [activeIdx, setActiveIdx] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const restoreRef = useRef<HTMLElement | null>(null)

  // Capture the element to restore focus to, then focus the input — once, on
  // mount (i.e. when the palette opens).
  useEffect(() => {
    restoreRef.current = document.activeElement as HTMLElement | null
    requestAnimationFrame(() => inputRef.current?.focus())
  }, [])

  const close = () => {
    onClose()
    restoreRef.current?.focus()
  }

  const matches: Scored[] = useMemo(() => {
    const q = query.trim()
    if (!q) {
      // Default view: navigation and scan actions
      return commands
        .filter(c => c.section === 'Navigate' || c.section === 'Scan')
        .map(c => ({ command: c, score: 0, indices: [] }))
    }
    return commands
      .map((c): Scored | null => {
        const titleMatch = fuzzyScore(q, c.title)
        if (titleMatch) return { command: c, score: titleMatch.score, indices: titleMatch.indices }
        // Keyword hits rank below title hits and aren't highlighted
        const kwMatch = c.keywords ? fuzzyScore(q, c.keywords) : null
        return kwMatch ? { command: c, score: kwMatch.score * 0.6, indices: [] } : null
      })
      .filter((s): s is Scored => s !== null)
      .sort((a, b) => b.score - a.score)
      .slice(0, MAX_RESULTS)
  }, [commands, query])

  // Group while preserving score order inside each section
  const grouped = useMemo(() => {
    const bySection = new Map<Command['section'], Scored[]>()
    for (const m of matches) {
      const list = bySection.get(m.command.section) || []
      list.push(m)
      bySection.set(m.command.section, list)
    }
    return SECTION_ORDER.filter(s => bySection.has(s)).map(s => [s, bySection.get(s)!] as const)
  }, [matches])

  const flat = useMemo(() => grouped.flatMap(([, items]) => items), [grouped])
  const active = flat[Math.min(activeIdx, flat.length - 1)]

  useEffect(() => {
    const el = listRef.current?.querySelector('.palette-item.active')
    el?.scrollIntoView({ block: 'nearest' })
  }, [activeIdx, matches])

  const runCommand = (c: Command) => {
    if (c.enabled === false) return
    close()
    c.run()
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.stopPropagation()
      close()
    } else if (e.key === 'ArrowDown') {
      e.preventDefault()
      setActiveIdx(i => Math.min(i + 1, flat.length - 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setActiveIdx(i => Math.max(i - 1, 0))
    } else if (e.key === 'Enter' && active) {
      e.preventDefault()
      runCommand(active.command)
    }
  }

  return (
    <div className="palette-overlay" onClick={close}>
      <div
        className="palette"
        role="dialog"
        aria-label="Command palette"
        onClick={e => e.stopPropagation()}
        onKeyDown={handleKeyDown}
      >
        <div className="palette-input">
          <i className="fa-solid fa-magnifying-glass" aria-hidden="true" />
          <input
            ref={inputRef}
            type="text"
            placeholder="Search commands, screens and diagnostics…"
            value={query}
            onChange={e => { setQuery(e.target.value); setActiveIdx(0) }}
            role="combobox"
            aria-expanded="true"
            aria-controls="palette-listbox"
            aria-activedescendant={active ? `palette-item-${active.command.id}` : undefined}
          />
          <Kbd keys="Esc" />
        </div>
        <div className="palette-list" ref={listRef} role="listbox" id="palette-listbox">
          {flat.length === 0 && <div className="palette-empty">No matching commands</div>}
          {grouped.map(([section, items]) => (
            <React.Fragment key={section}>
              <div className="palette-section">{section}</div>
              {items.map(m => (
                <button
                  key={m.command.id}
                  id={`palette-item-${m.command.id}`}
                  className={`palette-item ${active?.command.id === m.command.id ? 'active' : ''}`}
                  role="option"
                  aria-selected={active?.command.id === m.command.id}
                  disabled={m.command.enabled === false}
                  onMouseEnter={() => setActiveIdx(flat.indexOf(m))}
                  onClick={() => runCommand(m.command)}
                >
                  <i className={`fa-solid ${m.command.icon} pi-icon`} aria-hidden="true" />
                  <span className="pi-label"><Highlighted text={m.command.title} indices={m.indices} /></span>
                  {m.command.shortcut && <span className="pi-hint"><Kbd keys={m.command.shortcut} /></span>}
                </button>
              ))}
            </React.Fragment>
          ))}
        </div>
        <div className="palette-foot">
          <span><Kbd keys="↑" /><Kbd keys="↓" /> navigate</span>
          <span><Kbd keys="Enter" /> run</span>
          <span><Kbd keys="Esc" /> close</span>
        </div>
      </div>
    </div>
  )
}

export const CommandPalette: React.FC<CommandPaletteProps> = ({ open, onClose }) =>
  open ? <CommandPaletteBody onClose={onClose} /> : null
