import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useAppContext } from '../contexts/AppContext'
import * as logger from '../utils/logger'

/**
 * Event-driven AI chat state. The backend owns the conversation history and
 * the tool loop (`ai_chat_send` returns immediately); this hook turns the
 * `ai-chat://*` event stream into renderable message view-models.
 *
 * Delta handling is double-throttled by design: the backend coalesces to
 * protect IPC, and this hook batches into ~80 ms flushes to protect React.
 */

export interface ChatToolActivity {
  callId: string
  tool: string
  argsSummary?: string
  status: 'running' | 'done' | 'failed'
  durationMs?: number
  resultPreview?: string
}

export interface ChatMessageVM {
  id: string
  role: 'user' | 'assistant'
  text: string
  streaming: boolean
  tools: ChatToolActivity[]
  error?: string
}

interface DeltaPayload { sessionId: string; messageId: string; text: string }
interface ToolPayload {
  sessionId: string
  messageId: string
  callId: string
  tool: string
  argsSummary: string
  status: 'started' | 'completed' | 'failed'
  durationMs?: number
  resultPreview?: string
}
interface DonePayload { sessionId: string; messageId: string; finishReason: string; provider: string; toolCallCount: number }
interface ErrorPayload { sessionId: string; messageId: string; message: string }
interface ChatSendAck { sessionId: string; messageId: string; provider: string }
interface HistoryToolView { callId: string; tool: string; resultPreview?: string }
interface HistoryMessageView { id: string; role: 'user' | 'assistant'; text: string; tools: HistoryToolView[] }

const FLUSH_MS = 80

// Survives screen remounts so the conversation continues when the user
// navigates away and back (module scope, not persisted to disk).
let persistentSessionId: string | null = null

export function useAIChat() {
  const { settings } = useAppContext()
  const [messages, setMessages] = useState<ChatMessageVM[]>([])
  const [isStreaming, setIsStreaming] = useState(false)
  const [sessionId, setSessionId] = useState<string | null>(persistentSessionId)

  const sessionIdRef = useRef<string | null>(persistentSessionId)
  // Accumulated delta text per messageId, applied on a throttled flush.
  const pendingTextRef = useRef<Map<string, string>>(new Map())
  const flushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const apiKeyRef = useRef<string | undefined>(settings.openAiApiKey)
  apiKeyRef.current = settings.openAiApiKey

  const adoptSession = useCallback((id: string) => {
    persistentSessionId = id
    sessionIdRef.current = id
    setSessionId(id)
  }, [])

  const flushDeltas = useCallback(() => {
    flushTimerRef.current = null
    const pending = pendingTextRef.current
    if (pending.size === 0) return
    setMessages(prev => {
      const applied = new Set<string>()
      const next = prev.map(m => {
        const extra = pending.get(m.id)
        if (extra === undefined) return m
        applied.add(m.id)
        return { ...m, text: m.text + extra }
      })
      // Deltas for a message we haven't rendered yet (ack still in flight)
      // stay buffered for the next flush instead of being dropped.
      applied.forEach(id => pending.delete(id))
      return next
    })
  }, [])

  const scheduleFlush = useCallback(() => {
    if (flushTimerRef.current === null) {
      flushTimerRef.current = setTimeout(flushDeltas, FLUSH_MS)
    }
  }, [flushDeltas])

  // Register the four event listeners once.
  useEffect(() => {
    let disposed = false
    const unlistens: UnlistenFn[] = []

    const register = async () => {
      const handlers: Array<Promise<UnlistenFn>> = [
        listen<DeltaPayload>('ai-chat://delta', event => {
          const p = event.payload
          if (p.sessionId !== sessionIdRef.current) return
          const current = pendingTextRef.current.get(p.messageId) ?? ''
          pendingTextRef.current.set(p.messageId, current + p.text)
          scheduleFlush()
        }),
        listen<ToolPayload>('ai-chat://tool', event => {
          const p = event.payload
          if (p.sessionId !== sessionIdRef.current) return
          setMessages(prev => prev.map(m => {
            if (m.id !== p.messageId) return m
            const activity: ChatToolActivity = {
              callId: p.callId,
              tool: p.tool,
              argsSummary: p.argsSummary,
              status: p.status === 'started' ? 'running' : p.status === 'completed' ? 'done' : 'failed',
              durationMs: p.durationMs,
              resultPreview: p.resultPreview,
            }
            const index = m.tools.findIndex(t => t.callId === p.callId)
            const tools = [...m.tools]
            if (index >= 0) tools[index] = activity
            else tools.push(activity)
            return { ...m, tools }
          }))
        }),
        listen<DonePayload>('ai-chat://done', event => {
          const p = event.payload
          if (p.sessionId !== sessionIdRef.current) return
          flushDeltas()
          setMessages(prev => prev.map(m => (m.id === p.messageId ? { ...m, streaming: false } : m)))
          setIsStreaming(false)
        }),
        listen<ErrorPayload>('ai-chat://error', event => {
          const p = event.payload
          if (p.sessionId !== sessionIdRef.current) return
          setMessages(prev => prev.map(m => (
            m.id === p.messageId ? { ...m, error: p.message, streaming: false } : m
          )))
        }),
      ]
      const resolved = await Promise.all(handlers)
      if (disposed) {
        resolved.forEach(u => u())
        return
      }
      unlistens.push(...resolved)
    }
    void register()

    return () => {
      disposed = true
      unlistens.forEach(u => u())
      if (flushTimerRef.current !== null) {
        clearTimeout(flushTimerRef.current)
        flushTimerRef.current = null
      }
    }
  }, [scheduleFlush, flushDeltas])

  // Rehydrate an existing conversation after a screen remount.
  useEffect(() => {
    const sid = sessionIdRef.current
    if (!sid) return
    invoke<HistoryMessageView[]>('ai_chat_get_history', { sessionId: sid })
      .then(views => {
        if (views.length === 0) return
        setMessages(views.map(v => ({
          id: v.id,
          role: v.role,
          text: v.text,
          streaming: false,
          tools: v.tools.map(t => ({
            callId: t.callId,
            tool: t.tool,
            status: 'done' as const,
            resultPreview: t.resultPreview,
          })),
        })))
      })
      .catch(err => logger.error('useAIChat', 'Failed to load chat history', String(err)))
  }, [])

  const send = useCallback(async (text: string) => {
    const trimmed = text.trim()
    if (!trimmed || isStreaming) return
    setMessages(prev => [...prev, {
      id: `local_${Date.now()}_${prev.length}`,
      role: 'user',
      text: trimmed,
      streaming: false,
      tools: [],
    }])
    setIsStreaming(true)
    try {
      // Establish the session id BEFORE sending so no early event is
      // filtered out by the sessionId guard.
      let sid = sessionIdRef.current
      if (!sid) {
        sid = await invoke<string>('ai_chat_new_session')
        adoptSession(sid)
      }
      const ack = await invoke<ChatSendAck>('ai_chat_send', {
        sessionId: sid,
        message: trimmed,
        apiKey: apiKeyRef.current || null,
      })
      adoptSession(ack.sessionId)
      setMessages(prev => [...prev, {
        id: ack.messageId,
        role: 'assistant',
        text: '',
        streaming: true,
        tools: [],
      }])
    } catch (err) {
      setIsStreaming(false)
      setMessages(prev => [...prev, {
        id: `err_${Date.now()}`,
        role: 'assistant',
        text: '',
        streaming: false,
        tools: [],
        error: String(err),
      }])
    }
  }, [isStreaming, adoptSession])

  const stop = useCallback(async () => {
    const sid = sessionIdRef.current
    if (!sid) return
    try {
      await invoke('ai_chat_cancel', { sessionId: sid })
    } catch (err) {
      logger.error('useAIChat', 'Failed to cancel chat', String(err))
    }
  }, [])

  const newConversation = useCallback(async () => {
    try {
      const sid = await invoke<string>('ai_chat_new_session')
      adoptSession(sid)
      pendingTextRef.current.clear()
      setMessages([])
      setIsStreaming(false)
    } catch (err) {
      logger.error('useAIChat', 'Failed to start a new conversation', String(err))
    }
  }, [adoptSession])

  return { messages, send, stop, newConversation, isStreaming, sessionId }
}
