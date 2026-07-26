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

function modelSuggestions(ariaLabel: string): string[] {
  const input = screen.getByLabelText(ariaLabel)
  if (input.getAttribute('aria-expanded') !== 'true') fireEvent.focus(input)
  const listId = input.getAttribute('aria-controls')
  if (!listId) return []
  return Array.from(document.getElementById(listId)?.querySelectorAll('[role="option"]') ?? [])
    .map(option => option.getAttribute('data-model-id') || '')
}

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

beforeEach(() => {
  vi.clearAllMocks()
  invokeMock.mockResolvedValue({ models: [] })
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

    const active = screen.getByLabelText('AI provider') as HTMLSelectElement
    expect(active.querySelector<HTMLOptionElement>('option[value="phi_silica"]')).toBeDisabled()
    expect(screen.queryByLabelText('Provider to set up')).not.toBeInTheDocument()

    fireEvent.change(active, { target: { value: 'auto' } })
    const configure = screen.getByLabelText('Provider to set up') as HTMLSelectElement
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

    const active = screen.getByLabelText('AI provider') as HTMLSelectElement
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

    const active = screen.getByLabelText('AI provider') as HTMLSelectElement
    expect(active.querySelector<HTMLOptionElement>('option[value="phi_silica"]')).toBeDisabled()
    expect(screen.queryByLabelText('Provider to set up')).not.toBeInTheDocument()
    expect(screen.getByText('LAF token')).toBeInTheDocument()
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

    expect((screen.getByLabelText('Provider to set up') as HTMLSelectElement).value).toBe('anthropic')
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

    expect((screen.getByLabelText('Provider to set up') as HTMLSelectElement).value).toBe('custom_openai')
    expect(screen.getByText('Endpoint URL')).toBeInTheDocument()
  })

  it('uses one provider selector and synchronizes a concrete choice with its setup pane', async () => {
    const onSave = vi.fn().mockResolvedValue(undefined)
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={onSave}
        settings={{ preferredAIProvider: 'auto', openAiApiKeySet: true }}
      />
    )

    const provider = screen.getByLabelText('AI provider') as HTMLSelectElement
    expect(provider).toHaveValue('auto')
    expect(screen.getByLabelText('Provider to set up')).toBeInTheDocument()

    fireEvent.change(provider, { target: { value: 'anthropic' } })

    expect(provider).toHaveValue('anthropic')
    expect(screen.queryByLabelText('Provider to set up')).not.toBeInTheDocument()
    expect(screen.getByText('Anthropic API key')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /^save$/i }))
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(expect.objectContaining({
      preferredAIProvider: 'anthropic',
    })))
  })

  it('keeps provider setup navigation available under Auto without changing activation', () => {
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{ preferredAIProvider: 'auto' }}
      />
    )

    const provider = screen.getByLabelText('AI provider')
    const setup = screen.getByLabelText('Provider to set up')
    fireEvent.change(setup, { target: { value: 'gemini' } })

    expect(provider).toHaveValue('auto')
    expect(screen.getByText('Gemini API key')).toBeInTheDocument()
    expect(screen.getByLabelText('Provider to set up')).toHaveValue('gemini')
  })

  it('fills the editable OpenAI model picker from the live model list', async () => {
    invokeMock.mockImplementation(async (cmd: unknown) => {
      if (cmd === 'ai_list_models') {
        return {
          models: [
            { id: 'gpt-5.6-sol', label: 'GPT-5.6 Sol' },
            { id: 'gpt-5.6-luna', label: 'GPT-5.6 Luna' },
          ],
          defaultModel: 'gpt-5.6-luna',
        }
      }
      return { models: [] }
    })
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{ preferredAIProvider: 'openai', openAiApiKey: 'sk-test' }}
      />
    )

    const input = screen.getByLabelText('OpenAI model') as HTMLInputElement
    await waitFor(() => {
      expect(modelSuggestions('OpenAI model')).toContain('gpt-5.6-sol')
    })
    expect(input).toHaveAttribute('placeholder', 'Default (gpt-5.6-luna)')
    expect(invokeMock).toHaveBeenCalledWith('ai_list_models', {
      provider: 'openai',
      apiKey: 'sk-test',
      endpoint: undefined,
      cliPath: undefined,
    })
  })

  it('uses credential-presence flags without loading a secret into the UI', async () => {
    invokeMock.mockImplementation(async (cmd: unknown) => {
      if (cmd === 'ai_list_models') return { models: [{ id: 'gpt-5.6-luna' }] }
      return { models: [] }
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

    expect((screen.getByLabelText('Provider to set up') as HTMLSelectElement).value).toBe('openai')
    expect(screen.getByText('Configured')).toBeInTheDocument()
    expect(screen.getByLabelText('OpenAI API key')).toHaveValue('')
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('ai_list_models', {
      provider: 'openai', apiKey: undefined, endpoint: undefined, cliPath: undefined,
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

    expect((screen.getByLabelText('Provider to set up') as HTMLSelectElement).value).toBe('codex_cli')
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

  it('preserves a saved model that is absent from the latest catalog and accepts manual IDs', async () => {
    invokeMock.mockImplementation(async (cmd: unknown) => {
      if (cmd === 'ai_list_models') return { models: [{ id: 'gpt-5.6-sol' }] }
      return { models: [] }
    })
    const onSave = vi.fn().mockResolvedValue(undefined)
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={onSave}
        settings={{
          preferredAIProvider: 'openai',
          openAiApiKey: 'sk-test',
          openAiModel: 'account-private-model',
        }}
      />
    )

    const input = screen.getByLabelText('OpenAI model')
    expect(input).toHaveValue('account-private-model')
    await waitFor(() => expect(modelSuggestions('OpenAI model')).toEqual(['gpt-5.6-sol']))
    expect(input).toHaveValue('account-private-model')

    fireEvent.change(input, { target: { value: 'future-model-id' } })
    fireEvent.click(screen.getByRole('button', { name: /^save$/i }))
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(expect.objectContaining({
      openAiModel: 'future-model-id',
    })))
  })

  it('retains the last successful catalog and marks it stale when refresh fails', async () => {
    let catalogCalls = 0
    invokeMock.mockImplementation(async (cmd: unknown) => {
      if (cmd === 'ai_list_models') {
        catalogCalls += 1
        if (catalogCalls === 1) return { models: [{ id: 'gpt-5.6-terra' }] }
        throw new Error('provider unavailable')
      }
      return { models: [] }
    })
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{ preferredAIProvider: 'openai', openAiApiKey: 'sk-test' }}
      />
    )

    await waitFor(() => expect(modelSuggestions('OpenAI model')).toContain('gpt-5.6-terra'))
    fireEvent.click(screen.getByRole('button', { name: 'Refresh OpenAI model list' }))

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent(
        'Could not refresh models: provider unavailable. Showing previously loaded results.'
      )
    })
    expect(modelSuggestions('OpenAI model')).toContain('gpt-5.6-terra')
  })

  it('passes an unsaved CLI path when loading Codex models', async () => {
    invokeMock.mockImplementation(async (cmd: unknown) => {
      if (cmd === 'ai_bridge_status') return { installed: true, signedIn: true }
      if (cmd === 'ai_list_models') return { models: [{ id: 'gpt-5.6-sol' }] }
      return { models: [] }
    })
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{
          preferredAIProvider: 'codex_cli',
          codexCliPath: 'C:\\Tools\\codex.exe',
          codexModel: 'gpt-5.6-sol',
        }}
      />
    )

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('ai_list_models', {
      provider: 'codex_cli',
      apiKey: undefined,
      endpoint: undefined,
      cliPath: 'C:\\Tools\\codex.exe',
    }))
    expect(screen.getByLabelText('Codex model')).toHaveValue('gpt-5.6-sol')
  })

  it('shows every Claude model, version, label, and description despite a saved alias', async () => {
    invokeMock.mockImplementation(async (cmd: unknown) => {
      if (cmd === 'ai_bridge_status') return { installed: true, signedIn: true }
      if (cmd === 'ai_list_models') {
        return {
          defaultModel: 'sonnet',
          models: [
            { id: 'opus', label: 'Opus (latest)', description: 'Resolves to Claude Opus 5' },
            { id: 'claude-opus-5', label: 'Claude Opus 5', description: 'Pinned Opus 5 version' },
            { id: 'fable', label: 'Fable (latest)', description: 'Resolves to Claude Fable 5' },
            { id: 'claude-fable-5', label: 'Claude Fable 5', description: 'Pinned Fable 5 version' },
          ],
        }
      }
      return { models: [] }
    })
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{ preferredAIProvider: 'claude_code', claudeModel: 'opus' }}
      />
    )

    const input = screen.getByLabelText('Claude Code model')
    expect(input).toHaveValue('opus')
    await waitFor(() => expect(modelSuggestions('Claude Code model')).toEqual([
      'opus',
      'claude-opus-5',
      'fable',
      'claude-fable-5',
    ]))

    const listId = input.getAttribute('aria-controls')
    const listbox = document.getElementById(listId!)
    const fableAlias = listbox?.querySelector('[data-model-id="fable"]')
    const fableVersion = listbox?.querySelector('[data-model-id="claude-fable-5"]')
    expect(fableAlias).toHaveTextContent('Fable (latest)')
    expect(fableAlias).toHaveTextContent('Resolves to Claude Fable 5')
    expect(fableVersion).toHaveTextContent('Claude Fable 5')
    expect(fableVersion).toHaveTextContent('claude-fable-5')
    expect(fableVersion).toHaveTextContent('Pinned Fable 5 version')
    expect(input.closest('.model-picker')?.querySelector('.model-picker-selection'))
      .toHaveTextContent('Resolves to Claude Opus 5')

    fireEvent.keyDown(input, { key: 'End' })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(input).toHaveValue('claude-fable-5')
    expect(input).toHaveAttribute('aria-expanded', 'false')
    expect(screen.getByText('Pinned Fable 5 version')).toBeInTheDocument()
  })

  it('shows newly discovered Gemini entries while preserving a saved old model ID', async () => {
    invokeMock.mockImplementation(async (cmd: unknown) => {
      if (cmd === 'ai_list_models') {
        return {
          defaultModel: 'gemini-4-flash',
          models: [
            { id: 'gemini-4-pro', label: 'Gemini 4 Pro', description: 'Latest quality model' },
            { id: 'gemini-4-flash', label: 'Gemini 4 Flash', description: 'Latest fast model' },
          ],
        }
      }
      return { models: [] }
    })
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        onSave={vi.fn()}
        settings={{
          preferredAIProvider: 'gemini',
          geminiApiKeySet: true,
          geminiModel: 'gemini-3.5-flash',
        }}
      />
    )

    const input = screen.getByLabelText('Gemini model')
    expect(input).toHaveValue('gemini-3.5-flash')
    await waitFor(() => expect(modelSuggestions('Gemini model')).toEqual([
      'gemini-4-pro',
      'gemini-4-flash',
    ]))
    expect(input).toHaveValue('gemini-3.5-flash')
    expect(screen.getByText('Custom or unavailable model')).toBeInTheDocument()
    expect(screen.getByText('gemini-3.5-flash')).toBeInTheDocument()
    expect(screen.getByRole('option', { name: /Gemini 4 Pro.*gemini-4-pro.*Latest quality model/ })).toBeInTheDocument()
    expect(screen.getByRole('option', { name: /Gemini 4 Flash.*gemini-4-flash.*Latest fast model/ })).toBeInTheDocument()
  })
})
