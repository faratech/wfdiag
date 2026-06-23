import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { SettingsDialog } from './SettingsDialog'

const invokeMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

beforeEach(() => {
  vi.clearAllMocks()
  invokeMock.mockResolvedValue([])
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    setTimeout(cb, 0)
    return 0
  })
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('SettingsDialog provider setup seeding', () => {
  it('opens the configured Anthropic pane when Active AI is Auto', () => {
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{ preferredAIProvider: 'auto', anthropicApiKey: 'sk-ant-test' }}
      />
    )

    expect((screen.getByLabelText('Provider to configure') as HTMLSelectElement).value).toBe('anthropic')
    expect(screen.getByText('Anthropic API key')).toBeInTheDocument()
  })

  it('opens the configured custom endpoint pane when Active AI is Auto', () => {
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{
          preferredAIProvider: 'auto',
          openAiApiKey: 'sk-openai-test',
          customEndpoint: 'https://openrouter.ai/api',
          customModel: 'anthropic/claude-haiku-4-5',
        }}
      />
    )

    expect((screen.getByLabelText('Provider to configure') as HTMLSelectElement).value).toBe('custom_openai')
    expect(screen.getByText('Endpoint URL')).toBeInTheDocument()
  })

  it('shows an inline error when settings fail to save', async () => {
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn().mockRejectedValue(new Error('DPAPI unavailable'))}
        settings={{ preferredAIProvider: 'auto' }}
      />
    )

    fireEvent.click(screen.getByRole('button', { name: /save/i }))

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('Settings were not saved: DPAPI unavailable')
    })
  })
})
