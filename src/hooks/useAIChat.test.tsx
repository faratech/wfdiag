import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { renderHook, act, waitFor } from '@testing-library/react'

// useAIChat keeps the session id at module scope so conversations survive
// remounts; tests reload the module to start clean.

const invokeMock = vi.fn()
type Handler = (event: { payload: unknown }) => void
let eventHandlers: Map<string, Handler>
let unlistenSpies: Array<ReturnType<typeof vi.fn>>

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, handler: Handler) => {
    eventHandlers.set(name, handler)
    const unlisten = vi.fn()
    unlistenSpies.push(unlisten)
    return Promise.resolve(unlisten)
  },
}))
vi.mock('../contexts/AppContext', () => ({
  useAppContext: () => ({ settings: { openAiApiKey: 'sk-test' } }),
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
      default:
        return Promise.reject(new Error(`unexpected command ${cmd}`))
    }
  })
}

beforeEach(() => {
  eventHandlers = new Map()
  unlistenSpies = []
  invokeMock.mockReset()
  mockBackend()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('useAIChat', () => {
  it('sends a message: optimistic user bubble, ack creates the streaming assistant', async () => {
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(4))

    await act(async () => { await result.current.send('check my disk') })

    expect(invokeMock).toHaveBeenCalledWith('ai_chat_send', {
      sessionId: 's1',
      message: 'check my disk',
      apiKey: 'sk-test',
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
    expect(eventHandlers.size).toBe(4)

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

  it('tracks tool activity per call id from started to completed', async () => {
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(4))
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
    await waitFor(() => expect(eventHandlers.size).toBe(4))
    await act(async () => { await result.current.send('hello') })

    act(() => {
      fire('ai-chat://done', { sessionId: 'someone-else', messageId: 'm1', finishReason: 'stop', provider: 'openai', toolCallCount: 0 })
    })
    expect(result.current.isStreaming).toBe(true)
  })

  it('renders backend errors as an error bubble and stops streaming on error event', async () => {
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(4))
    await act(async () => { await result.current.send('hello') })

    act(() => {
      fire('ai-chat://error', { sessionId: 's1', messageId: 'm1', message: 'rate limited' })
      fire('ai-chat://done', { sessionId: 's1', messageId: 'm1', finishReason: 'error', provider: 'openai', toolCallCount: 0 })
    })
    expect(result.current.messages[1].error).toBe('rate limited')
    expect(result.current.isStreaming).toBe(false)
  })

  it('stop cancels via the backend; new conversation resets state', async () => {
    const useAIChat = await loadHook()
    const { result } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(4))
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

  it('unregisters every listener on unmount', async () => {
    const useAIChat = await loadHook()
    const { unmount } = renderHook(() => useAIChat())
    await waitFor(() => expect(eventHandlers.size).toBe(4))
    unmount()
    expect(unlistenSpies).toHaveLength(4)
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
    await waitFor(() => expect(eventHandlers.size).toBe(4))

    await act(async () => { await result.current.send('hello') })
    expect(result.current.isStreaming).toBe(false)
    const last = result.current.messages[result.current.messages.length - 1]
    expect(last?.error).toContain('No AI provider available')
  })
})
