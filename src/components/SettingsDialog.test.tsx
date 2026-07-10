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

  it('fills the OpenAI model dropdown from the live model list', async () => {
    invokeMock.mockImplementation(async (cmd: unknown) => {
      if (cmd === 'ai_list_models') return ['gpt-5.2', 'gpt-5-nano']
      return []
    })
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{ preferredAIProvider: 'openai', openAiApiKey: 'sk-test' }}
      />
    )

    const select = screen.getByLabelText('OpenAI model') as HTMLSelectElement
    await waitFor(() => {
      expect(Array.from(select.options).map(o => o.value)).toContain('gpt-5.2')
    })
    expect(invokeMock).toHaveBeenCalledWith('ai_list_models', {
      provider: 'openai',
      apiKey: 'sk-test',
      endpoint: undefined,
    })
  })

  it('opens the Codex CLI pane when a codex path is configured and Active AI is Auto', async () => {
    invokeMock.mockImplementation(async (cmd: unknown) => {
      if (cmd === 'ai_bridge_status') return { installed: true, signedIn: true, path: 'C:\\Tools\\codex.exe' }
      return []
    })
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{ preferredAIProvider: 'auto', codexCliPath: 'C:\\Tools\\codex.exe' }}
      />
    )

    expect((screen.getByLabelText('Provider to configure') as HTMLSelectElement).value).toBe('codex_cli')
    expect(screen.getByText('ChatGPT account')).toBeInTheDocument()
    await waitFor(() => expect(screen.getByText('Signed in')).toBeInTheDocument())
  })

  it('opens the Claude Code pane and drives sign-in through the bridge commands', async () => {
    invokeMock.mockImplementation(async (cmd: unknown, args?: unknown) => {
      const provider = (args as { provider?: string } | undefined)?.provider
      if (cmd === 'ai_bridge_status' && provider === 'claude_code') return { installed: true, signedIn: false }
      if (cmd === 'ai_bridge_sign_in' && provider === 'claude_code') return { installed: true, signedIn: true }
      return []
    })
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{ preferredAIProvider: 'claude_code' }}
      />
    )

    expect(screen.getByText('Claude account')).toBeInTheDocument()
    const signIn = await screen.findByRole('button', { name: /sign in with claude/i })
    fireEvent.click(signIn)

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('ai_bridge_sign_in', { provider: 'claude_code' })
      expect(screen.getByText('Signed in')).toBeInTheDocument()
    })
  })

  it('drives the Codex CLI sign-in through the bridge commands', async () => {
    invokeMock.mockImplementation(async (cmd: unknown) => {
      if (cmd === 'ai_bridge_status') return { installed: true, signedIn: false }
      if (cmd === 'ai_bridge_sign_in') return { installed: true, signedIn: true }
      return []
    })
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{ preferredAIProvider: 'codex_cli' }}
      />
    )

    const signIn = await screen.findByRole('button', { name: /sign in with chatgpt/i })
    fireEvent.click(signIn)

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('ai_bridge_sign_in', { provider: 'codex_cli' })
      expect(screen.getByText('Signed in')).toBeInTheDocument()
    })
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
