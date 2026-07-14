import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { renderHook, act, waitFor } from '@testing-library/react'

// useAIChat keeps the session id at module scope so conversations survive
// remounts; tests reload the module to start clean.

const invokeMock = vi.fn()
type Handler = (event: { payload: unknown }) => void
let eventHandlers: Map<string, Handler>
let unlistenSpies: Array<ReturnType<typeof vi.fn>>
let listenerRegistrationGate: Promise<void> | null
let mockSettings: Record<string, unknown>
const setSettingsMock = vi.hoisted(() => vi.fn())

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, handler: Handler) => {
    eventHandlers.set(name, handler)
    const unlisten = vi.fn()
    unlistenSpies.push(unlisten)
    return (listenerRegistrationGate ?? Promise.resolve()).then(() => unlisten)
  },
}))
vi.mock('../contexts/AppContext', () => ({
  useAppContext: () => ({ settings: mockSettings, setSettings: setSettingsMock }),
}))

function fire(event: string, payload: unknown) {
  eventHandlers.get(event)?.({ payload })
}

async function loadHook() {
  vi.resetModules()
  const mod = await import('./useAIChat')
  return mod.useAIChat
}

/** Default invoke behavior: new session + send ack on session s1/m1. */
function mockBackend() {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'ai_chat_new_session':
        return Promise.resolve('s1')
      case 'ai_chat_send':
        return Promise.resolve({ sessionId: 's1', messageId: 'm1', provider: 'openai' })
      case 'ai_chat_get_history':
        return Promise.resolve([])
      case 'ai_chat_cancel':
        return Promise.resolve()
      case 'ai_chat_resolve_fallback':
        return Promise.resolve()
      default:
        return Promise.reject(new Error(`unexpected command ${cmd}`))
    }
  })
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

beforeEach(() => {
  eventHandlers = new Map()
  unlistenSpies = []
  listenerRegistrationGate = null
  invokeMock.mockReset()
  mockSettings = { openAiApiKey: 'sk-test', aiEnabled: true }
  setSettingsMock.mockReset()
  mockBackend()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('useAIChat', () => {
  it('refuses every send path while the selected provider is being validated', async () => {
    const useAIChat = await loadHook()
    const { result, rerender } = renderHook(
      ({ ready }) => useAIChat(ready),
      { initialProps: { ready: false } },
    )
    await waitFor(() => expect(eventHandlers.size).toBe(7))

    await act(async () => { await result.current.send('do not dispatch yet') })

    expect(invokeMock).not.toHaveBeenCalledWith('ai_chat_new_session')
    expect(invokeMock).not.toHaveBeenCalledWith('ai_chat_send', expect.anything())
    expect(result.current.messages).toHaveLength(0)

    rerender({ ready: true })
    await act(async () => { await result.current.send('provider is ready now') })
    expect(invokeMock).toHaveBeenCalledWith('ai_chat_send', expect.anything())
  })

  it('re-checks provider readiness after an in-flight session creation', async () => {
    const newSession = deferred<string>()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'ai_chat_new_session') return newSession.promise
      if (cmd === 'ai_chat_get_history') return Promise.resolve([])
      if (cmd === 'ai_chat_cancel') return Promise.resolve()
      return Promise.reject(new Error(`unexpected command ${cmd}`))
    })
    const useAIChat = await loadHook()
    const { result, rerender } = renderHook(
      ({ ready }) => useAIChat(ready),
      { initialProps: { ready: true } },
    )
    await waitFor(() => expect(eventHandlers.size).toBe(7))

    let sendPromise!: Promise<boolean>
    act(() => {
      sendPromise = result.current.send('must not cross the provider transition')
    })
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('ai_chat_new_session'))

    rerender({ ready: false })
    await act(async () => {
      newSession.resolve('s-transition')
      await sendPromise
    })

    expect(invokeMock).not.toHaveBeenCalledWith('ai_chat_send', expect.anything())
    expect(result.current.isStreaming).toBe(false)
    expect(result.current.messages[result.current.messages.length - 1]?.error).toContain('still being validated')
  })

  it('waits for every event listener before starting a chat turn', async () => {
    const gate = deferred<void>()
    listenerRegistrationGate = gate.promise
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())

    let sendPromise!: Promise<boolean>
    act(() => {
      sendPromise = result.current.send('do not miss early events')
    })
    await Promise.resolve()

    expect(eventHandlers.size).toBe(7)
    expect(invokeMock).not.toHaveBeenCalledWith('ai_chat_new_session')
    expect(invokeMock).not.toHaveBeenCalledWith('ai_chat_send', expect.anything())
    expect(result.current.messages).toHaveLength(0)

    await act(async () => {
      gate.resolve()
      await sendPromise
    })

    expect(invokeMock).toHaveBeenCalledWith('ai_chat_send', expect.anything())
    expect(result.current.messages).toHaveLength(2)
  })

  it('sends a message: optimistic user bubble, ack creates the streaming assistant', async () => {
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))

    await act(async () => { await result.current.send('check my disk') })

    expect(invokeMock).toHaveBeenCalledWith('ai_chat_send', {
      sessionId: 's1',
      message: 'check my disk',
      query: 'check my disk',
      displayText: 'check my disk',
      contextRefs: [],
    })
    expect(result.current.messages).toHaveLength(2)
    expect(result.current.messages[0]).toMatchObject({ role: 'user', text: 'check my disk' })
    expect(result.current.messages[1]).toMatchObject({ id: 'm1', role: 'assistant', streaming: true })
    expect(result.current.isStreaming).toBe(true)
  })

  it('accumulates deltas through the flush throttle and finishes on done', async () => {
    vi.useFakeTimers()
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await act(async () => { await vi.runAllTimersAsync() })
    expect(eventHandlers.size).toBe(7)

    await act(async () => { await result.current.send('hello') })

    act(() => {
      fire('ai-chat://delta', { sessionId: 's1', messageId: 'm1', text: 'Your disk ' })
      fire('ai-chat://delta', { sessionId: 's1', messageId: 'm1', text: 'looks healthy.' })
    })
    // Nothing applied until the 80 ms flush fires
    expect(result.current.messages[1].text).toBe('')
    await act(async () => { await vi.advanceTimersByTimeAsync(100) })
    expect(result.current.messages[1].text).toBe('Your disk looks healthy.')

    act(() => {
      fire('ai-chat://done', { sessionId: 's1', messageId: 'm1', finishReason: 'stop', provider: 'openai', toolCallCount: 0 })
    })
    expect(result.current.messages[1].streaming).toBe(false)
    expect(result.current.isStreaming).toBe(false)
  })

  it('keeps early delta, tool and done events that arrive before the send ack resolves', async () => {
    const ack = deferred<{ sessionId: string; messageId: string; provider: string }>()
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'ai_chat_new_session':
          return Promise.resolve('s1')
        case 'ai_chat_send':
          return ack.promise
        case 'ai_chat_get_history':
          return Promise.resolve([])
        default:
          return Promise.reject(new Error(`unexpected command ${cmd}`))
      }
    })

    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))

    let sendPromise!: Promise<boolean>
    act(() => {
      sendPromise = result.current.send('hello')
    })
    await waitFor(() => expect(invokeMock.mock.calls.some(c => c[0] === 'ai_chat_send')).toBe(true))

    act(() => {
      fire('ai-chat://delta', { sessionId: 's1', messageId: 'm1', text: 'Fast answer.' })
      fire('ai-chat://tool', {
        sessionId: 's1', messageId: 'm1', callId: 'c1', tool: 'run_diagnostic',
        argsSummary: 'task_id: os_info', status: 'completed', durationMs: 5,
        resultPreview: 'Windows 11',
      })
      fire('ai-chat://proposal', {
        sessionId: 's1', messageId: 'm1', proposal: {
          proposalId: 'proposal_1', approvalScope: 'exact', actions: [{
            remediation: {
              id: 'flush_dns', label: 'Flush DNS', description: 'Clear the DNS cache',
              tier: 'auto_safe', admin_required: false, requires_restart: false,
              long_running: false, maintenance: true, batch_eligible: true, cancellable: true,
            },
            steps: ['Run ipconfig /flushdns'],
          }],
          scanFingerprint: 'scan', catalogFingerprint: 'catalog', createdAtMs: 1, expiresAtMs: 2,
        },
      })
      fire('ai-chat://done', { sessionId: 's1', messageId: 'm1', finishReason: 'stop', provider: 'openai', toolCallCount: 1 })
    })

    await act(async () => {
      ack.resolve({ sessionId: 's1', messageId: 'm1', provider: 'openai' })
      await sendPromise
    })

    expect(result.current.messages[1]).toMatchObject({
      id: 'm1',
      text: 'Fast answer.',
      streaming: false,
    })
    expect(result.current.messages[1].tools).toEqual([
      expect.objectContaining({ callId: 'c1', status: 'done', resultPreview: 'Windows 11' }),
    ])
    expect(result.current.messages[1].stagedProposals).toEqual([
      expect.objectContaining({ proposalId: 'proposal_1' }),
    ])
    expect(result.current.isStreaming).toBe(false)
  })

  it('prevents duplicate sends before the first request resolves', async () => {
    const ack = deferred<{ sessionId: string; messageId: string; provider: string }>()
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'ai_chat_new_session':
          return Promise.resolve('s1')
        case 'ai_chat_send':
          return ack.promise
        case 'ai_chat_get_history':
          return Promise.resolve([])
        default:
          return Promise.reject(new Error(`unexpected command ${cmd}`))
      }
    })

    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))

    let first!: Promise<boolean>
    let second!: Promise<boolean>
    act(() => {
      first = result.current.send('first')
      second = result.current.send('second')
    })

    await waitFor(() => expect(invokeMock.mock.calls.some(c => c[0] === 'ai_chat_send')).toBe(true))
    const sends = invokeMock.mock.calls.filter(c => c[0] === 'ai_chat_send')
    expect(sends).toHaveLength(1)
    expect(sends[0][1]).toMatchObject({ message: 'first' })

    await act(async () => {
      ack.resolve({ sessionId: 's1', messageId: 'm1', provider: 'openai' })
      await Promise.all([first, second])
    })
  })

  it('tracks tool activity per call id from started to completed', async () => {
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))
    await act(async () => { await result.current.send('hello') })

    act(() => {
      fire('ai-chat://tool', {
        sessionId: 's1', messageId: 'm1', callId: 'c1', tool: 'run_diagnostic',
        argsSummary: 'task_id: logical_disk', status: 'started',
      })
    })
    expect(result.current.messages[1].tools).toEqual([
      expect.objectContaining({ callId: 'c1', status: 'running' }),
    ])

    act(() => {
      fire('ai-chat://tool', {
        sessionId: 's1', messageId: 'm1', callId: 'c1', tool: 'run_diagnostic',
        argsSummary: 'task_id: logical_disk', status: 'completed',
        durationMs: 800, resultPreview: 'C: 80% free',
      })
    })
    expect(result.current.messages[1].tools).toEqual([
      expect.objectContaining({ callId: 'c1', status: 'done', resultPreview: 'C: 80% free' }),
    ])
  })

  it('ignores events for a different chat session', async () => {
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))
    await act(async () => { await result.current.send('hello') })

    act(() => {
      fire('ai-chat://done', { sessionId: 'someone-else', messageId: 'm1', finishReason: 'stop', provider: 'openai', toolCallCount: 0 })
    })
    expect(result.current.isStreaming).toBe(true)
  })

  it('renders backend errors as an error bubble and stops streaming on error event', async () => {
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))
    await act(async () => { await result.current.send('hello') })

    act(() => {
      fire('ai-chat://error', { sessionId: 's1', messageId: 'm1', message: 'rate limited' })
    })
    expect(result.current.messages[1].error).toBe('rate limited')
    expect(result.current.isStreaming).toBe(false)
  })

  it('stop cancels via the backend; new conversation resets state', async () => {
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))
    await act(async () => { await result.current.send('hello') })

    await act(async () => { await result.current.stop() })
    expect(invokeMock).toHaveBeenCalledWith('ai_chat_cancel', { sessionId: 's1' })

    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'ai_chat_new_session' ? Promise.resolve('s2') : Promise.resolve([]))
    await act(async () => { await result.current.newConversation() })
    expect(result.current.messages).toHaveLength(0)
    expect(result.current.sessionId).toBe('s2')
    expect(result.current.isStreaming).toBe(false)
  })

  it('clears the stopping state when a paused fallback turn cancels without a terminal event', async () => {
    const cancellation = deferred<void>()
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'ai_chat_new_session':
          return Promise.resolve('s1')
        case 'ai_chat_send':
          return Promise.resolve({ sessionId: 's1', messageId: 'm1', provider: 'ollama' })
        case 'ai_chat_cancel':
          return cancellation.promise
        case 'ai_chat_get_history':
          return Promise.resolve([])
        default:
          return Promise.reject(new Error(`unexpected command ${cmd}`))
      }
    })
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))
    await act(async () => { await result.current.send('check locally') })
    act(() => {
      fire('ai-chat://fallback-required', {
        sessionId: 's1',
        messageId: 'm1',
        from: { providerId: 'ollama', executionClass: 'local_server' },
        to: { providerId: 'openai', executionClass: 'api_cloud' },
        reason: 'Local provider unavailable',
      })
    })

    let stopPromise!: Promise<void>
    act(() => {
      stopPromise = result.current.stop()
    })
    expect(result.current.isStopping).toBe(true)

    await act(async () => {
      cancellation.resolve()
      await stopPromise
    })
    expect(result.current.pendingFallback).toBeNull()
    expect(result.current.isStopping).toBe(false)
  })

  it('new conversation cancels and ignores an old in-flight send ack', async () => {
    const ack = deferred<{ sessionId: string; messageId: string; provider: string }>()
    let newSessionCalls = 0
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'ai_chat_new_session':
          newSessionCalls += 1
          return Promise.resolve(newSessionCalls === 1 ? 's1' : 's2')
        case 'ai_chat_send':
          return ack.promise
        case 'ai_chat_get_history':
          return Promise.resolve([])
        case 'ai_chat_cancel':
          return Promise.resolve()
        default:
          return Promise.reject(new Error(`unexpected command ${cmd}`))
      }
    })

    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))

    let sendPromise!: Promise<boolean>
    act(() => {
      sendPromise = result.current.send('old prompt')
    })
    await waitFor(() => expect(invokeMock.mock.calls.some(c => c[0] === 'ai_chat_send')).toBe(true))

    await act(async () => {
      await result.current.newConversation()
    })

    expect(invokeMock).toHaveBeenCalledWith('ai_chat_cancel', { sessionId: 's1' })
    expect(result.current.sessionId).toBe('s2')
    expect(result.current.messages).toHaveLength(0)

    await act(async () => {
      ack.resolve({ sessionId: 's1', messageId: 'm1', provider: 'openai' })
      await sendPromise
    })

    expect(result.current.sessionId).toBe('s2')
    expect(result.current.messages).toHaveLength(0)
    expect(result.current.isStreaming).toBe(false)
  })

  it('discards history hydration after the conversation changes', async () => {
    const history = deferred<{
      sessionId: string
      messages: Array<{ id: string; role: 'user' | 'assistant'; text: string; tools: never[] }>
      busy: boolean
    }>()
    let newSessionCalls = 0
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'ai_chat_new_session':
          newSessionCalls += 1
          return Promise.resolve(newSessionCalls === 1 ? 's1' : 's2')
        case 'ai_chat_send':
          return Promise.resolve({ sessionId: 's1', messageId: 'm1', provider: 'openai' })
        case 'ai_chat_get_history':
          return history.promise
        case 'ai_chat_cancel':
          return Promise.resolve()
        default:
          return Promise.reject(new Error(`unexpected command ${cmd}`))
      }
    })

    const useAIChat = await loadHook()
    const first = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))
    await act(async () => { await first.result.current.send('old conversation') })
    act(() => {
      fire('ai-chat://done', {
        sessionId: 's1', messageId: 'm1', finishReason: 'stop', provider: 'openai', toolCallCount: 0,
      })
    })
    first.unmount()

    const second = renderHook(() => useAIChat())
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('ai_chat_get_history', { sessionId: 's1' }))
    await act(async () => { await second.result.current.newConversation() })
    expect(second.result.current.sessionId).toBe('s2')

    await act(async () => {
      history.resolve({
        sessionId: 's1',
        messages: [{ id: 'old', role: 'assistant', text: 'stale answer', tools: [] }],
        busy: false,
      })
      await history.promise
    })

    expect(second.result.current.sessionId).toBe('s2')
    expect(second.result.current.messages).toHaveLength(0)
  })

  it('does not let late history overwrite an optimistic live turn', async () => {
    const history = deferred<{
      sessionId: string
      messages: Array<{ id: string; role: 'user' | 'assistant'; text: string; tools: never[] }>
      busy: boolean
    }>()
    let sendCalls = 0
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'ai_chat_new_session':
          return Promise.resolve('s1')
        case 'ai_chat_send':
          sendCalls += 1
          return Promise.resolve({ sessionId: 's1', messageId: `m${sendCalls}`, provider: 'openai' })
        case 'ai_chat_get_history':
          return history.promise
        default:
          return Promise.resolve()
      }
    })

    const useAIChat = await loadHook()
    const first = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))
    await act(async () => { await first.result.current.send('first prompt') })
    act(() => {
      fire('ai-chat://done', {
        sessionId: 's1', messageId: 'm1', finishReason: 'stop', provider: 'openai', toolCallCount: 0,
      })
    })
    first.unmount()

    const second = renderHook(() => useAIChat())
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('ai_chat_get_history', { sessionId: 's1' }))
    await act(async () => { await second.result.current.send('live prompt') })

    await act(async () => {
      history.resolve({
        sessionId: 's1',
        messages: [{ id: 'old', role: 'assistant', text: 'stale answer', tools: [] }],
        busy: false,
      })
      await history.promise
    })

    expect(second.result.current.messages.map(message => message.text)).toEqual(['live prompt', ''])
    expect(second.result.current.messages.some(message => message.id === 'old')).toBe(false)
    expect(second.result.current.isStreaming).toBe(true)
  })

  it('hydrates the snapshot finish reason onto the final assistant message', async () => {
    let hydrate = false
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'ai_chat_new_session':
          return Promise.resolve('s1')
        case 'ai_chat_send':
          return Promise.resolve({ sessionId: 's1', messageId: 'm1', provider: 'openai' })
        case 'ai_chat_get_history':
          return Promise.resolve(hydrate ? {
            sessionId: 's1',
            messages: [
              { id: 'u1', role: 'user', text: 'hello', tools: [] },
              { id: 'a1', role: 'assistant', text: 'truncated answer', tools: [] },
            ],
            busy: false,
            finishReason: 'length',
          } : [])
        default:
          return Promise.resolve()
      }
    })

    const useAIChat = await loadHook()
    const first = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))
    await act(async () => { await first.result.current.send('hello') })
    first.unmount()
    hydrate = true

    const second = renderHook(() => useAIChat())
    await waitFor(() => expect(second.result.current.messages).toHaveLength(2))
    expect(second.result.current.messages[1]).toMatchObject({
      id: 'a1',
      finishReason: 'length',
    })
  })

  it('unregisters every listener on unmount', async () => {
    const useAIChat = await loadHook()
    const { unmount } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))
    unmount()
    expect(unlistenSpies).toHaveLength(7)
    for (const unlisten of unlistenSpies) {
      expect(unlisten).toHaveBeenCalled()
    }
  })

  it('rejected send surfaces the failure as an error bubble', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'ai_chat_new_session') return Promise.resolve('s1')
      if (cmd === 'ai_chat_send') return Promise.reject('No AI provider available')
      return Promise.resolve([])
    })
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))

    await act(async () => { await result.current.send('hello') })
    expect(result.current.isStreaming).toBe(false)
    const last = result.current.messages[result.current.messages.length - 1]
    expect(last?.error).toContain('No AI provider available')
  })

  it('does not send when AI insights are disabled', async () => {
    mockSettings = { openAiApiKey: 'sk-test', aiEnabled: false }
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))

    await act(async () => { await result.current.send('hello') })

    expect(invokeMock).not.toHaveBeenCalledWith('ai_chat_new_session')
    expect(invokeMock).not.toHaveBeenCalledWith('ai_chat_send', expect.anything())
    expect(result.current.messages).toHaveLength(0)
    expect(result.current.isStreaming).toBe(false)
  })

  it('shows concise handoff text while sending structured context to the backend', async () => {
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))

    await act(async () => {
      await result.current.send({
        displayText: 'Help me fix the disk warning.',
        query: 'Explain issue disk_space_low with the attached diagnostic context.',
        contextRefs: [
          { kind: 'issue', id: 'disk_space_low' },
          { kind: 'diagnostic', id: 'logical_disk' },
        ],
      })
    })

    expect(result.current.messages[0].text).toBe('Help me fix the disk warning.')
    expect(invokeMock).toHaveBeenCalledWith('ai_chat_send', expect.objectContaining({
      displayText: 'Help me fix the disk warning.',
      query: 'Explain issue disk_space_low with the attached diagnostic context.',
      contextRefs: [
        { kind: 'issue', id: 'disk_space_low' },
        { kind: 'diagnostic', id: 'logical_disk' },
      ],
    }))
  })

  it('pauses for cloud fallback consent and remembers the decision in memory', async () => {
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))
    await act(async () => { await result.current.send('check my PC') })

    act(() => {
      fire('ai-chat://fallback-required', {
        sessionId: 's1',
        messageId: 'm1',
        from: { providerId: 'ollama', executionClass: 'local_server' },
        to: { providerId: 'openai', executionClass: 'api_cloud' },
        reason: 'Local provider unavailable',
      })
    })
    expect(result.current.pendingFallback?.to.providerId).toBe('openai')
    expect(result.current.isStreaming).toBe(false)

    await act(async () => { await result.current.resolveFallback('allow') })
    expect(invokeMock).toHaveBeenCalledWith('ai_chat_resolve_fallback', {
      sessionId: 's1', messageId: 'm1', decision: 'allow',
    })
    expect(setSettingsMock).toHaveBeenCalledWith(expect.objectContaining({ cloudFallbackPolicy: 'allow' }))
    expect(result.current.pendingFallback).toBeNull()
    expect(result.current.isStreaming).toBe(true)
  })

  it('keeps one deduplicated Full Scan request after the assistant turn completes', async () => {
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(7))
    await act(async () => { await result.current.send('check my PC') })

    const request = {
      sessionId: 's1',
      messageId: 'm1',
      sourceScanId: 'scan-quick',
      kind: 'full',
      reason: 'Quick Scan does not include the required event logs.',
      question: 'check my PC',
    }
    act(() => {
      fire('ai-chat://done', {
        sessionId: 's1', messageId: 'm1', finishReason: 'stop', provider: 'openai', toolCallCount: 1,
      })
      fire('ai-chat://scan-request', request)
      fire('ai-chat://scan-request', request)
    })

    expect(result.current.isStreaming).toBe(false)
    expect(result.current.pendingFullScan).toEqual(request)
    act(() => result.current.dismissFullScanRequest())
    expect(result.current.pendingFullScan).toBeNull()
  })
})
