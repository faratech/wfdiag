import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen } from '@testing-library/react'
import { DiagnosticDetail, type DiagItem } from './DiagnosticDetail'

let contextValue: Record<string, unknown>

vi.mock('../contexts/AIContext', () => ({
  useAIContext: () => contextValue,
  diagnosticCacheKey: (taskId: string, output: string) => `${taskId}:${output.length}`,
}))

function item(): DiagItem {
  return {
    id: 'os_info',
    name: 'OS Info',
    category: 'System',
    description: '',
    admin_required: false,
    result: { success: true, output: '{}', duration_ms: 5 },
  }
}

function baseContext(grounding: unknown) {
  const cacheKey = 'os_info:2'
  return {
    analyzeDiagnostic: vi.fn(),
    isAnalyzing: {},
    interpretations: { [cacheKey]: 'Looks fine.' },
    analysisMeta: { [cacheKey]: { provider_used: 'openai', cached: false, grounding } },
    errors: {},
    isAIAvailable: true,
    aiEnabled: true,
    activeProvider: 'openai',
  }
}

beforeEach(() => {
  contextValue = baseContext(undefined)
})

describe('DiagnosticDetail grounding source links', () => {
  it('renders an http(s) grounding source as a clickable link', () => {
    contextValue = baseContext({
      enabled: true,
      query: 'q',
      source_count: 1,
      sources: [{ source: 'WindowsForum', title: 'Relevant KB article', url: 'https://example.com/kb' }],
    })

    render(<DiagnosticDetail item={item()} />)

    const link = screen.getByRole('link', { name: 'Relevant KB article' })
    expect(link).toHaveAttribute('href', 'https://example.com/kb')
  })

  it('does not render a javascript: grounding source url as a clickable link', () => {
    contextValue = baseContext({
      enabled: true,
      query: 'q',
      source_count: 1,
      sources: [{ source: 'WindowsForum', title: 'Suspicious source', url: 'javascript:alert(1)' }],
    })

    render(<DiagnosticDetail item={item()} />)

    expect(screen.queryByRole('link', { name: 'Suspicious source' })).toBeNull()
    expect(screen.getByText('Suspicious source')).toBeInTheDocument()
  })

  it('does not render a data: grounding source url as a clickable link', () => {
    contextValue = baseContext({
      enabled: true,
      query: 'q',
      source_count: 1,
      sources: [{ source: 'WindowsForum', title: 'Data URI source', url: 'data:text/html,<script>alert(1)</script>' }],
    })

    render(<DiagnosticDetail item={item()} />)

    expect(screen.queryByRole('link', { name: 'Data URI source' })).toBeNull()
    expect(screen.getByText('Data URI source')).toBeInTheDocument()
  })
})
