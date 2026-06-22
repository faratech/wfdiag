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
  const aiEnabled = settings.aiEnabled ?? true
  const [messages, setMessages] = useState<ChatMessageVM[]>([])
  const [isStreaming, setIsStreaming] = useState(false)
  const [sessionId, setSessionId] = useState<string | null>(persistentSessionId)

  const sessionIdRef = useRef<string | null>(persistentSessionId)
  // Accumulated delta text per messageId, applied on a throttled flush.
  const pendingTextRef = useRef<Map<string, string>>(new Map())
  const pendingDoneRef = useRef<Map<string, DonePayload>>(new Map())
  const pendingErrorRef = useRef<Map<string, string>>(new Map())
  const pendingToolsRef = useRef<Map<string, Map<string, ChatToolActivity>>>(new Map())
  const flushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const apiKeyRef = useRef<string | undefined>(settings.openAiApiKey)
  useEffect(() => { apiKeyRef.current = settings.openAiApiKey }, [settings.openAiApiKey])

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

  const applyTool = useCallback((tools: ChatToolActivity[], activity: ChatToolActivity) => {
    const index = tools.findIndex(t => t.callId === activity.callId)
    const next = [...tools]
    if (index >= 0) next[index] = activity
    else next.push(activity)
    return next
  }, [])

  const bufferTool = useCallback((messageId: string, activity: ChatToolActivity) => {
    const tools = pendingToolsRef.current.get(messageId) ?? new Map<string, ChatToolActivity>()
    tools.set(activity.callId, activity)
    pendingToolsRef.current.set(messageId, tools)
  }, [])

  const buildAssistantMessage = useCallback((messageId: string): ChatMessageVM => {
    const text = pendingTextRef.current.get(messageId) ?? ''
    pendingTextRef.current.delete(messageId)

    const tools = Array.from(pendingToolsRef.current.get(messageId)?.values() ?? [])
    pendingToolsRef.current.delete(messageId)

    const error = pendingErrorRef.current.get(messageId)
    pendingErrorRef.current.delete(messageId)

    const done = pendingDoneRef.current.get(messageId)
    pendingDoneRef.current.delete(messageId)

    return {
      id: messageId,
      role: 'assistant',
      text,
      streaming: !done && !error,
      tools,
      error,
    }
  }, [])

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
          setMessages(prev => {
            const activity: ChatToolActivity = {
              callId: p.callId,
              tool: p.tool,
              argsSummary: p.argsSummary,
              status: p.status === 'started' ? 'running' : p.status === 'completed' ? 'done' : 'failed',
              durationMs: p.durationMs,
              resultPreview: p.resultPreview,
            }
            let matched = false
            const next = prev.map(m => {
              if (m.id !== p.messageId) return m
              matched = true
              return { ...m, tools: applyTool(m.tools, activity) }
            })
            if (!matched) bufferTool(p.messageId, activity)
            return next
          })
        }),
        listen<DonePayload>('ai-chat://done', event => {
          const p = event.payload
          if (p.sessionId !== sessionIdRef.current) return
          flushDeltas()
          setMessages(prev => {
            let matched = false
            const next = prev.map(m => {
              if (m.id !== p.messageId) return m
              matched = true
              return { ...m, streaming: false }
            })
            if (!matched) pendingDoneRef.current.set(p.messageId, p)
            return next
          })
          setIsStreaming(false)
        }),
        listen<ErrorPayload>('ai-chat://error', event => {
          const p = event.payload
          if (p.sessionId !== sessionIdRef.current) return
          setMessages(prev => {
            let matched = false
            const next = prev.map(m => {
              if (m.id !== p.messageId) return m
              matched = true
              return { ...m, error: p.message, streaming: false }
            })
            if (!matched) pendingErrorRef.current.set(p.messageId, p.message)
            return next
          })
          setIsStreaming(false)
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
  }, [scheduleFlush, flushDeltas, applyTool, bufferTool])

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
    if (!trimmed || isStreaming || !aiEnabled) return
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
      const assistant = buildAssistantMessage(ack.messageId)
      setMessages(prev => [...prev, assistant])
      if (!assistant.streaming) {
        setIsStreaming(false)
      }
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
  }, [aiEnabled, isStreaming, adoptSession, buildAssistantMessage])

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
      pendingDoneRef.current.clear()
      pendingErrorRef.current.clear()
      pendingToolsRef.current.clear()
      setMessages([])
      setIsStreaming(false)
    } catch (err) {
      logger.error('useAIChat', 'Failed to start a new conversation', String(err))
    }
  }, [adoptSession])

  return { messages, send, stop, newConversation, isStreaming, sessionId, aiEnabled }
}
