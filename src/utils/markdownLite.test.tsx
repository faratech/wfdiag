import React from 'react'
import { describe, it, expect } from 'vitest'
import { render } from '@testing-library/react'
import { renderMarkdownLite } from './markdownLite'

function html(text: string): string {
  const { container } = render(<>{renderMarkdownLite(text)}</>)
  return container.innerHTML
}

describe('renderMarkdownLite', () => {
  it('renders paragraphs', () => {
    expect(html('first\n\nsecond')).toBe('<p>first</p><p>second</p>')
  })

  it('renders bold and inline code', () => {
    expect(html('a **bold** and `code` here')).toBe(
      '<p>a <strong>bold</strong> and <code>code</code> here</p>'
    )
  })

  it('renders unordered and ordered lists', () => {
    expect(html('- one\n- two')).toBe('<ul><li>one</li><li>two</li></ul>')
    expect(html('1. one\n2. two')).toBe('<ol><li>one</li><li>two</li></ol>')
  })

  it('renders headings', () => {
    expect(html('### Title\nbody')).toBe('<h4>Title</h4><p>body</p>')
  })

  it('renders fenced code blocks verbatim', () => {
    expect(html('before\n```\nlet x = 1\n```\nafter')).toBe(
      '<p>before</p><pre>let x = 1</pre><p>after</p>'
    )
  })

  it('escapes embedded HTML instead of rendering it', () => {
    const out = html('try <img src=x onerror=alert(1)> this')
    expect(out).not.toContain('<img')
    expect(out).toContain('&lt;img')
  })
})
