import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { SettingsDialog } from './SettingsDialog'
import type { AIProviderStatus } from '../contexts/AIContext'

const invokeMock = vi.fn()

function phiStatus(overrides: Partial<AIProviderStatus> = {}): AIProviderStatus {
  return {
    preferred_provider: 'codex_cli',
    openai_available: false,
    openai_api_key_set: false,
    phi_silica_available: false,
    phi_silica_ready: false,
    phi_silica_message: 'Phi Silica requires the Microsoft Store version of this app.',
    active_provider: 'codex_cli',
    providers: [],
    ...overrides,
  }
}

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
  it('disables unavailable Phi as Active AI, keeps its setup pane accessible, and shows the backend reason', () => {
    const reason = 'Phi Silica requires the Microsoft Store version of this app.'
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{ preferredAIProvider: 'codex_cli' }}
        aiStatus={phiStatus({ phi_silica_message: reason })}
      />
    )

    const active = screen.getByLabelText('Active AI provider') as HTMLSelectElement
    const configure = screen.getByLabelText('Provider to configure') as HTMLSelectElement
    expect(active.querySelector<HTMLOptionElement>('option[value="phi_silica"]')).toBeDisabled()
    expect(configure.querySelector<HTMLOptionElement>('option[value="phi_silica"]')).not.toBeDisabled()
    expect(screen.getByText(reason)).toBeInTheDocument()

    fireEvent.change(configure, { target: { value: 'phi_silica' } })
    expect(screen.getByText('LAF token')).toBeInTheDocument()
  })

  it('also disables Phi when the package is supported but its model is not ready', () => {
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{ preferredAIProvider: 'codex_cli' }}
        aiStatus={phiStatus({
          phi_silica_available: true,
          phi_silica_ready: false,
          phi_silica_message: 'Phi Silica is still preparing its model.',
        })}
      />
    )

    const active = screen.getByLabelText('Active AI provider') as HTMLSelectElement
    expect(active.querySelector<HTMLOptionElement>('option[value="phi_silica"]')).toBeDisabled()
    expect(screen.getByText('Phi Silica is still preparing its model.')).toBeInTheDocument()
  })

  it('rejects saving an already-selected unavailable Phi preference', async () => {
    const onSave = vi.fn()
    const reason = 'Phi Silica requires the Microsoft Store version of this app.'
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={onSave}
        settings={{ preferredAIProvider: 'phi_silica' }}
        aiStatus={phiStatus({ phi_silica_message: reason })}
      />
    )

    fireEvent.click(screen.getByRole('button', { name: /^save$/i }))

    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent(reason))
    expect(onSave).not.toHaveBeenCalled()
  })

  it('blocks selecting or saving Phi while provider status is still unknown', async () => {
    const onSave = vi.fn()
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={onSave}
        settings={{ preferredAIProvider: 'phi_silica' }}
        aiStatus={null}
        aiStatusLoading
      />
    )

    const active = screen.getByLabelText('Active AI provider') as HTMLSelectElement
    const configure = screen.getByLabelText('Provider to configure') as HTMLSelectElement
    expect(active.querySelector<HTMLOptionElement>('option[value="phi_silica"]')).toBeDisabled()
    expect(configure.querySelector<HTMLOptionElement>('option[value="phi_silica"]')).not.toBeDisabled()
    expect(screen.getByText(/Checking whether Phi Silica is available/)).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /^save$/i }))
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent(/Wait for the check to finish/))
    expect(onSave).not.toHaveBeenCalled()
  })

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

  it('uses credential-presence flags without loading a secret into the UI', async () => {
    invokeMock.mockImplementation(async (cmd: unknown) => {
      if (cmd === 'ai_list_models') return ['gpt-5-nano']
      return []
    })
    const onSave = vi.fn().mockResolvedValue(undefined)
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={onSave}
        settings={{ preferredAIProvider: 'auto', openAiApiKeySet: true, cloudFallbackPolicy: 'ask' }}
      />
    )

    expect((screen.getByLabelText('Provider to configure') as HTMLSelectElement).value).toBe('openai')
    expect(screen.getByText('Configured')).toBeInTheDocument()
    expect(screen.getByLabelText('OpenAI API key')).toHaveValue('')
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('ai_list_models', {
      provider: 'openai', apiKey: undefined, endpoint: undefined,
    }))

    fireEvent.change(screen.getByLabelText('Cloud fallback policy'), { target: { value: 'never' } })
    fireEvent.click(screen.getByRole('button', { name: /save/i }))
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(expect.objectContaining({
      openAiApiKeySet: true,
      cloudFallbackPolicy: 'never',
    })))
    expect(onSave.mock.calls[0][0].openAiApiKey).toBeUndefined()
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
