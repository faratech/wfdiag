import React, { ReactNode } from 'react'

/**
 * Minimal markdown renderer for AI chat responses. Supports paragraphs,
 * **bold**, `inline code`, fenced code blocks, unordered/ordered lists,
 * ### headings and GFM pipe tables — rendered as React elements with NO
 * dangerouslySetInnerHTML.
 *
 * Deliberately not a markdown library: AI output can embed adversarial HTML
 * via prompt-injected diagnostic data, so string-to-HTML rendering would need
 * a sanitizer on top. If fidelity needs grow, swap the implementation behind
 * this signature for marked + DOMPurify.
 */

type Align = 'left' | 'right' | 'center' | undefined

// Inline pass: `code` first (its content is verbatim), then **bold**
function renderInline(text: string): ReactNode[] {
  const out: ReactNode[] = []
  let key = 0
  const codeParts = text.split(/(`[^`]+`)/g)
  for (const part of codeParts) {
    if (part.startsWith('`') && part.endsWith('`') && part.length > 2) {
      out.push(<code key={key++}>{part.slice(1, -1)}</code>)
      continue
    }
    const boldParts = part.split(/(\*\*[^*]+\*\*)/g)
    for (const seg of boldParts) {
      if (seg.startsWith('**') && seg.endsWith('**') && seg.length > 4) {
        out.push(<strong key={key++}>{seg.slice(2, -2)}</strong>)
      } else if (seg) {
        out.push(<React.Fragment key={key++}>{seg}</React.Fragment>)
      }
    }
  }
  return out
}

// Split a table row into trimmed cells. Drops the optional outer pipes and
// honours `\|` as a literal pipe inside a cell. Scans char-by-char so a cell
// containing spaces (or any other text) is never corrupted.
function splitRow(line: string): string[] {
  let s = line.trim()
  if (s.startsWith('|')) s = s.slice(1)
  if (s.endsWith('|')) s = s.slice(0, -1)
  const cells: string[] = []
  let cur = ''
  for (let i = 0; i < s.length; i++) {
    if (s[i] === '\\' && s[i + 1] === '|') {
      cur += '|'
      i++
    } else if (s[i] === '|') {
      cells.push(cur.trim())
      cur = ''
    } else {
      cur += s[i]
    }
  }
  cells.push(cur.trim())
  return cells
}

// A GFM delimiter row, e.g. `|---|:--:|`. Requires a pipe (so a bare `---`
// horizontal rule is never mistaken for a one-column table) and dashes.
function isDelimiterRow(line: string): boolean {
  const t = line.trim()
  if (!t.includes('|') || !t.includes('-')) return false
  const cells = splitRow(t)
  return cells.length > 0 && cells.every(cell => /^:?-+:?$/.test(cell))
}

function alignOf(cell: string): Align {
  const left = cell.startsWith(':')
  const right = cell.endsWith(':')
  if (left && right) return 'center'
  if (right) return 'right'
  if (left) return 'left'
  return undefined
}

export function renderMarkdownLite(text: string): ReactNode[] {
  const out: ReactNode[] = []
  let key = 0
  // Split out fenced code blocks first; everything between fences is verbatim
  const blocks = text.split(/```(?:\w*\n)?/)
  blocks.forEach((block, blockIdx) => {
    if (blockIdx % 2 === 1) {
      out.push(<pre key={key++}>{block.replace(/\n$/, '')}</pre>)
      return
    }
    // Group consecutive lines into paragraphs / lists / headings / tables
    const lines = block.split('\n')
    let para: string[] = []
    let list: { ordered: boolean; items: string[] } | null = null

    const flushPara = () => {
      if (para.length) {
        out.push(<p key={key++}>{renderInline(para.join(' '))}</p>)
        para = []
      }
    }
    const flushList = () => {
      if (list) {
        const items = list.items.map((item, i) => <li key={i}>{renderInline(item)}</li>)
        out.push(list.ordered ? <ol key={key++}>{items}</ol> : <ul key={key++}>{items}</ul>)
        list = null
      }
    }

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i]
      const trimmed = line.trim()

      // Table: a header row (has a pipe) immediately followed by a delimiter row
      if (trimmed.includes('|') && i + 1 < lines.length && isDelimiterRow(lines[i + 1])) {
        flushPara()
        flushList()
        const headers = splitRow(trimmed)
        const aligns = splitRow(lines[i + 1]).map(alignOf)
        const rows: string[][] = []
        let j = i + 2
        // Body rows run until a blank line or a line without a pipe
        while (j < lines.length && lines[j].trim() && lines[j].includes('|')) {
          rows.push(splitRow(lines[j]))
          j++
        }
        const cellStyle = (ci: number) => (aligns[ci] ? { textAlign: aligns[ci] } : undefined)
        out.push(
          <table key={key++} className="md-table">
            <thead>
              <tr>
                {headers.map((h, ci) => (
                  <th key={ci} style={cellStyle(ci)}>{renderInline(h)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, ri) => (
                <tr key={ri}>
                  {/* Map against the header so ragged rows pad/truncate cleanly */}
                  {headers.map((_, ci) => (
                    <td key={ci} style={cellStyle(ci)}>{renderInline(row[ci] ?? '')}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        )
        i = j - 1
        continue
      }

      const heading = /^(#{1,4})\s+(.*)/.exec(trimmed)
      const bullet = /^[-*]\s+(.*)/.exec(trimmed)
      const numbered = /^\d+[.)]\s+(.*)/.exec(trimmed)

      if (!trimmed) {
        flushPara()
        flushList()
      } else if (heading) {
        flushPara()
        flushList()
        out.push(<h4 key={key++}>{renderInline(heading[2])}</h4>)
      } else if (bullet) {
        flushPara()
        if (!list || list.ordered) {
          flushList()
          list = { ordered: false, items: [] }
        }
        list.items.push(bullet[1])
      } else if (numbered) {
        flushPara()
        if (!list || !list.ordered) {
          flushList()
          list = { ordered: true, items: [] }
        }
        list.items.push(numbered[1])
      } else {
        flushList()
        para.push(trimmed)
      }
    }
    flushPara()
    flushList()
  })
  return out
}
