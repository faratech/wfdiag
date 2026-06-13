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

  it('renders a GFM pipe table', () => {
    const out = html('| Component | Status |\n|-----------|--------|\n| CPU | OK |\n| Memory | Warning |')
    expect(out).toBe(
      '<table class="md-table">' +
        '<thead><tr><th>Component</th><th>Status</th></tr></thead>' +
        '<tbody>' +
        '<tr><td>CPU</td><td>OK</td></tr>' +
        '<tr><td>Memory</td><td>Warning</td></tr>' +
        '</tbody>' +
        '</table>'
    )
  })

  it('applies column alignment from the delimiter row', () => {
    const out = html('| L | C | R |\n| :--- | :--: | ---: |\n| a | b | c |')
    expect(out).toContain('<th style="text-align: center;">C</th>')
    expect(out).toContain('<td style="text-align: right;">c</td>')
  })

  it('keeps spaces in cells and pads ragged rows', () => {
    const out = html('| Name | Value |\n|---|---|\n| CPU Usage | 91% |\n| OnlyOne |')
    // a real space inside a cell must survive
    expect(out).toContain('<td>CPU Usage</td>')
    // a short row is padded to the header width with an empty cell
    expect(out).toContain('<tr><td>OnlyOne</td><td></td></tr>')
  })

  it('does not treat a non-table pipe line as a table', () => {
    expect(html('a | b without a delimiter row')).toBe('<p>a | b without a delimiter row</p>')
  })
})
