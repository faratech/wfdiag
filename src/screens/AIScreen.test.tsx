import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { AIScreen } from './AIScreen'

let appContext: Record<string, unknown>
let aiContext: Record<string, unknown>
let workspace: Record<string, unknown>

vi.mock('../contexts/AppContext', () => ({
  useAppContext: () => appContext,
}))
vi.mock('../contexts/AIContext', () => ({
  useAIContext: () => aiContext,
}))
vi.mock('../contexts/AIWorkspaceContext', () => ({
  useAIWorkspace: () => workspace,
}))

function scanReportState(overrides: Record<string, unknown> = {}) {
  return {
    report: '',
    generating: false,
    cancelling: false,
    error: null,
    hasResults: true,
    aiEnabled: true,
    lastProviderUse: null,
    generate: vi.fn(),
    cancel: vi.fn(),
    copy: vi.fn(),
    ...overrides,
  }
}

function chatState(overrides: Record<string, unknown> = {}) {
  return {
    messages: [],
    send: vi.fn(),
    stop: vi.fn(),
    resolveFallback: vi.fn(),
    newConversation: vi.fn(),
    isStreaming: false,
    pendingFallback: null,
    lastProviderUse: { providerId: 'openai', executionClass: 'api_cloud' },
    aiEnabled: true,
    ...overrides,
  }
}

beforeEach(() => {
  appContext = {
    setShowSettings: vi.fn(),
    pendingChatPrompt: null,
    setPendingChatPrompt: vi.fn(),
    pendingScanReport: false,
    setPendingScanReport: vi.fn(),
    aiMode: 'assistant',
    setAIMode: vi.fn(),
    isRunning: false,
    currentProgress: 0,
    currentTaskName: '',
  }
  aiContext = {
    aiStatus: { providers: [{ id: 'openai', available: true, supports_tools: true }] },
    activeProvider: 'openai',
    isAIAvailable: true,
    isLoading: false,
  }
  workspace = {
    chat: chatState(),
    scanReport: scanReportState(),
    queuePrompt: vi.fn(),
    retryPendingScan: vi.fn(),
    cancelPendingPrompt: vi.fn(),
    acceptFullScanRequest: vi.fn(),
    dismissFullScanRequest: vi.fn(),
    reportPreparation: { status: 'idle' },
    retryReportPreparation: vi.fn(),
    cancelReportPreparation: vi.fn(),
  }
})

describe('AIScreen workspace', () => {
  it('offers only Assistant and Scan Report modes with a compact actual-provider indicator', () => {
    render(<AIScreen />)
    expect(screen.getAllByRole('tab').map(tab => tab.textContent?.trim())).toEqual(['Assistant', 'Scan Report'])
    expect(screen.getByRole('status', { name: /OpenAI, API cloud/ })).toBeInTheDocument()
    expect(screen.queryByText('Providers')).not.toBeInTheDocument()
    expect(document.querySelectorAll('.chat-msgs')).toHaveLength(1)
  })

  it('shows a useful configuration state when no provider is available', () => {
    aiContext = { aiStatus: { providers: [] }, activeProvider: 'none', isAIAvailable: false, isLoading: false }
    workspace = { chat: chatState({ lastProviderUse: null }), scanReport: scanReportState() }
    render(<AIScreen />)
    expect(screen.getByRole('heading', { name: 'Connect an AI provider' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Configure AI' })).toBeInTheDocument()
  })

  it('renders Scan Report as a dedicated mode instead of a side panel', () => {
    appContext.aiMode = 'report'
    render(<AIScreen />)
    expect(screen.getByRole('tab', { name: /Scan Report/ })).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByRole('heading', { name: /Turn scan data into a clear plan/ })).toBeInTheDocument()
    expect(screen.queryByLabelText('AI conversation')).not.toBeVisible()
  })

  it('shows persistent scan preparation without dispatching from the screen layer', () => {
    const send = vi.fn().mockResolvedValue(undefined)
    const prompt = {
      displayText: 'Help me fix “Low disk space”.',
      query: 'Explain issue disk_space_low using logical_disk.',
      contextRefs: [{ kind: 'issue', id: 'disk_space_low' }],
      scanGate: { requestId: 'scan-1', kind: 'quick', status: 'running' },
    }
    appContext.pendingChatPrompt = prompt
    appContext.isRunning = true
    appContext.currentProgress = 42
    appContext.currentTaskName = 'Disk information'
    workspace = {
      ...workspace,
      chat: chatState({ send }),
    }
    render(<AIScreen />)
    expect(screen.getByText('Running Quick Scan before asking')).toBeInTheDocument()
    expect(screen.getByText(/42% · Disk information/)).toBeInTheDocument()
    expect(screen.getByLabelText('Message the AI assistant')).toBeDisabled()
    expect(send).not.toHaveBeenCalled()
  })

  it('routes suggested questions through the lifetime preparation queue', () => {
    const queuePrompt = vi.fn()
    workspace = { ...workspace, queuePrompt }
    render(<AIScreen />)
    fireEvent.click(screen.getByRole('button', { name: 'Summarize my latest scan' }))
    expect(queuePrompt).toHaveBeenCalledWith('Summarize my latest scan')
  })

  it('blocks click, Enter, and form submission while provider status is loading', () => {
    const send = vi.fn()
    aiContext = {
      aiStatus: { providers: [{ id: 'openai', available: true, supports_tools: true }] },
      activeProvider: 'openai',
      isAIAvailable: true,
      isLoading: true,
    }
    workspace = { chat: chatState({ send }), scanReport: scanReportState() }
    render(<AIScreen />)

    const input = screen.getByLabelText('Message the AI assistant') as HTMLTextAreaElement
    const form = input.closest('form')!
    const sendButton = screen.getByRole('button', { name: /send/i })
    expect(input).toBeDisabled()
    expect(sendButton).toBeDisabled()

    fireEvent.change(input, { target: { value: 'Do not send yet' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    fireEvent.submit(form)
    fireEvent.click(sendButton)
    expect(send).not.toHaveBeenCalled()
  })

  it('does not present the previous provider as current during a provider transition', () => {
    aiContext = {
      aiStatus: null,
      activeProvider: 'none',
      isAIAvailable: false,
      isLoading: true,
    }
    workspace = {
      chat: chatState({ lastProviderUse: { providerId: 'codex_cli', executionClass: 'subscription_cloud' } }),
      scanReport: scanReportState(),
    }

    const view = render(<AIScreen />)
    expect(screen.getByRole('status', { name: /Checking AI provider, Please wait/ })).toBeInTheDocument()
    expect(screen.queryByRole('status', { name: /ChatGPT via Codex, Subscription cloud/ })).not.toBeInTheDocument()

    aiContext = {
      aiStatus: { providers: [{ id: 'phi_silica', available: true, supports_tools: false }] },
      activeProvider: 'phi_silica',
      isAIAvailable: true,
      isLoading: false,
    }
    view.rerender(<AIScreen />)

    expect(screen.getByRole('status', { name: /Phi Silica, On device/ })).toBeInTheDocument()
  })

  it('disables report generation actions while provider status is loading', () => {
    const generate = vi.fn()
    appContext.aiMode = 'report'
    aiContext = {
      aiStatus: null,
      activeProvider: 'none',
      isAIAvailable: false,
      isLoading: true,
    }
    workspace = {
      chat: chatState(),
      scanReport: scanReportState({ report: 'Existing report', generate }),
    }
    const view = render(<AIScreen />)

    const regenerate = screen.getByRole('button', { name: 'Regenerate report' })
    expect(regenerate).toBeDisabled()
    fireEvent.click(regenerate)
    expect(generate).not.toHaveBeenCalled()

    workspace = {
      chat: chatState(),
      scanReport: scanReportState({ generate }),
    }
    view.rerender(<AIScreen />)
    expect(screen.getByRole('heading', { name: /Checking AI provider/ })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Generate report/ })).not.toBeInTheDocument()
  })

  it('leaves a pending chat handoff untouched while provider status changes', () => {
    const send = vi.fn().mockResolvedValue(undefined)
    const setPendingChatPrompt = vi.fn()
    const prompt = 'Explain the latest result.'
    appContext.pendingChatPrompt = prompt
    appContext.setPendingChatPrompt = setPendingChatPrompt
    aiContext = {
      aiStatus: { providers: [{ id: 'openai', available: true, supports_tools: true }] },
      activeProvider: 'openai',
      isAIAvailable: true,
      isLoading: true,
    }
    workspace = { ...workspace, chat: chatState({ send }) }

    const view = render(<AIScreen />)
    expect(send).not.toHaveBeenCalled()
    expect(setPendingChatPrompt).not.toHaveBeenCalled()

    aiContext = { ...aiContext, isLoading: false }
    view.rerender(<AIScreen />)

    expect(send).not.toHaveBeenCalled()
    expect(setPendingChatPrompt).not.toHaveBeenCalled()
    expect(screen.getByLabelText('Message the AI assistant')).toBeDisabled()
  })

  it('requires explicit consent for a requested Full Scan and waits for the turn to finish', () => {
    const acceptFullScanRequest = vi.fn()
    workspace = {
      ...workspace,
      chat: chatState({
        pendingFullScan: {
          sessionId: 's1', messageId: 'm1', sourceScanId: 'scan-quick', kind: 'full',
          reason: 'Event logs need broader coverage.', question: 'Why did the PC crash?',
        },
        isStreaming: true,
      }),
      acceptFullScanRequest,
    }
    const view = render(<AIScreen />)
    const runFull = screen.getByRole('button', { name: 'Run Full Scan' })
    expect(runFull).toBeDisabled()

    workspace = {
      ...workspace,
      chat: chatState({
        pendingFullScan: {
          sessionId: 's1', messageId: 'm1', sourceScanId: 'scan-quick', kind: 'full',
          reason: 'Event logs need broader coverage.', question: 'Why did the PC crash?',
        },
        isStreaming: false,
      }),
      acceptFullScanRequest,
    }
    view.rerender(<AIScreen />)
    fireEvent.click(screen.getByRole('button', { name: 'Run Full Scan' }))
    expect(acceptFullScanRequest).toHaveBeenCalledOnce()
  })

  it('offers Quick Scan and automatic generation when report mode has no data', () => {
    const setPendingScanReport = vi.fn()
    appContext.aiMode = 'report'
    appContext.setPendingScanReport = setPendingScanReport
    workspace = {
      ...workspace,
      scanReport: scanReportState({ hasResults: false }),
      reportPreparation: { status: 'idle' },
    }
    render(<AIScreen />)
    fireEvent.click(screen.getByRole('button', { name: 'Run Quick Scan & Generate' }))
    expect(setPendingScanReport).toHaveBeenCalledWith(true)
  })

  it('keeps a pending report handoff until provider discovery completes', async () => {
    const generate = vi.fn().mockResolvedValue(undefined)
    const setPendingScanReport = vi.fn()
    appContext.pendingScanReport = true
    appContext.setPendingScanReport = setPendingScanReport
    aiContext = { aiStatus: null, activeProvider: 'none', isAIAvailable: false, isLoading: true }
    workspace = { chat: chatState({ lastProviderUse: null }), scanReport: scanReportState({ generate }) }

    const view = render(<AIScreen />)
    expect(generate).not.toHaveBeenCalled()
    expect(setPendingScanReport).not.toHaveBeenCalled()

    aiContext = {
      aiStatus: { providers: [{ id: 'openai', available: true, supports_tools: true }] },
      activeProvider: 'openai',
      isAIAvailable: true,
      isLoading: false,
    }
    view.rerender(<AIScreen />)

    await waitFor(() => expect(generate).toHaveBeenCalledOnce())
    expect(setPendingScanReport).toHaveBeenCalledWith(false)
  })
})
