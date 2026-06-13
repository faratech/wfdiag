import React, { ReactNode } from 'react'

/**
 * Minimal markdown renderer for AI chat responses. Supports paragraphs,
 * **bold**, `inline code`, fenced code blocks, unordered/ordered lists, and
 * ### headings — rendered as React elements with NO dangerouslySetInnerHTML.
 *
 * Deliberately not a markdown library: AI output can embed adversarial HTML
 * via prompt-injected diagnostic data, so string-to-HTML rendering would need
 * a sanitizer on top. If fidelity needs grow, swap the implementation behind
 * this signature for marked + DOMPurify.
 */

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
    // Group consecutive lines into paragraphs / lists / headings
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

    for (const line of lines) {
      const trimmed = line.trim()
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
