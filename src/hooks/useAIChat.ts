import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useAppContext, type ActionProposal } from '../contexts/AppContext'
import type { AIProviderId, AIProviderUse, ChatPrompt } from '../components/types'
import * as logger from '../utils/logger'

/**
 * Event-driven AI chat state. Mount this hook once at app lifetime (through
 * AIWorkspaceProvider) so stream listeners stay alive while users change tabs.
 * The backend remains the durable owner of conversation history.
 */

export interface ChatToolActivity {
  callId: string
  tool: string
  argsSummary?: string
  status: 'queued' | 'running' | 'cancel_requested' | 'done' | 'failed' | 'cancelled' | 'timed_out'
  durationMs?: number
  resultPreview?: string
}

export interface ChatMessageVM {
  id: string
  role: 'user' | 'assistant'
  text: string
  streaming: boolean
  tools: ChatToolActivity[]
  stagedProposals?: ActionProposal[]
  providerUse?: AIProviderUse
  finishReason?: string
  error?: string
}

export interface ProviderFallbackRequest {
  sessionId: string
  messageId: string
  from: AIProviderUse
  to: AIProviderUse
  reason: string
}

export interface FullScanRequest {
  sessionId: string
  messageId: string
  sourceScanId: string
  kind: 'full'
  reason: string
  question: string
}

interface DeltaPayload { sessionId: string; messageId: string; text: string }
interface ToolPayload {
  sessionId: string
  messageId: string
  callId: string
  tool: string
  argsSummary: string
  status: 'started' | 'queued' | 'running' | 'cancel_requested' | 'completed' | 'failed' | 'cancelled' | 'timed_out'
  durationMs?: number
  resultPreview?: string
}
interface DonePayload {
  sessionId: string
  messageId: string
  finishReason: string
  provider: AIProviderId
  providerUse?: AIProviderUse
  toolCallCount: number
}
interface ErrorPayload { sessionId: string; messageId: string; message: string }
interface ProposalPayload { sessionId: string; messageId: string; proposal: ActionProposal }
interface ChatSendAck {
  sessionId: string
  messageId: string
  provider: AIProviderId
  providerUse?: AIProviderUse
}
interface HistoryToolView {
  callId: string
  tool: string
  argsSummary?: string
  status?: ToolPayload['status']
  durationMs?: number
  resultPreview?: string
}
interface HistoryMessageView {
  id: string
  role: 'user' | 'assistant'
  text: string
  tools: HistoryToolView[]
  providerUse?: AIProviderUse
  finishReason?: string
}
interface HistorySnapshot {
  sessionId: string
  messages: HistoryMessageView[]
  busy: boolean
  activeMessageId?: string
  finishReason?: string
  pendingFallback?: Omit<ProviderFallbackRequest, 'sessionId'>
}

const FLUSH_MS = 80

// Module scope is a compatibility backstop for direct hook remounts in tests
// and development. In the app the provider keeps this hook mounted for life.
let persistentSessionId: string | null = null

function providerUseFromLegacy(provider: AIProviderId): AIProviderUse {
  const executionClass = provider === 'phi_silica'
    ? 'on_device'
    : provider === 'foundry_local' || provider === 'ollama'
      ? 'local_server'
      : provider === 'codex_cli' || provider === 'claude_code'
        ? 'subscription_cloud'
        : 'api_cloud'
  return { providerId: provider, executionClass }
}

function normalizePrompt(input: string | ChatPrompt): Required<Pick<ChatPrompt, 'query'>> & Omit<ChatPrompt, 'query'> {
  if (typeof input === 'string') return { query: input, displayText: input, contextRefs: [] }
  return {
    query: input.query,
    displayText: input.displayText || input.query,
    contextRefs: input.contextRefs || [],
  }
}

function normalizeToolStatus(status: ToolPayload['status']): ChatToolActivity['status'] {
  if (status === 'completed') return 'done'
  if (status === 'started') return 'running'
  return status
}

export function useAIChat(providerReady = true) {
  const { settings, setSettings } = useAppContext()
  const aiEnabled = settings.aiEnabled ?? true
  const [messages, setMessages] = useState<ChatMessageVM[]>([])
  // Render-time mirror (same idiom as providerReadyRef below) so the flush
  // timer callback can tell which buffered message ids have live messages.
  const messagesRef = useRef<ChatMessageVM[]>([])
  messagesRef.current = messages
  const [isStreaming, setIsStreaming] = useState(false)
  const [isStopping, setIsStopping] = useState(false)
  const [sessionId, setSessionId] = useState<string | null>(persistentSessionId)
  const [pendingFallback, setPendingFallbackState] = useState<ProviderFallbackRequest | null>(null)
  const [pendingFullScan, setPendingFullScan] = useState<FullScanRequest | null>(null)
  const [lastProviderUse, setLastProviderUse] = useState<AIProviderUse | null>(null)

  const sessionIdRef = useRef<string | null>(persistentSessionId)
  const streamingRef = useRef(false)
  const pendingFallbackRef = useRef<ProviderFallbackRequest | null>(null)
  const pendingTextRef = useRef<Map<string, string>>(new Map())
  const pendingDoneRef = useRef<Map<string, DonePayload>>(new Map())
  const pendingErrorRef = useRef<Map<string, string>>(new Map())
  const pendingToolsRef = useRef<Map<string, Map<string, ChatToolActivity>>>(new Map())
  const pendingProposalsRef = useRef<Map<string, Map<string, ActionProposal>>>(new Map())
  const flushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const conversationEpochRef = useRef(0)
  const liveStateRevisionRef = useRef(0)
  const listenersReadyRef = useRef<Promise<void> | null>(null)
  const resolveListenersReadyRef = useRef<(() => void) | null>(null)
  const listenerRegistrationErrorRef = useRef<unknown>(null)
  const providerReadyRef = useRef(providerReady)
  // Update during render rather than in an effect: a provider transition and
  // a programmatic send can occur in the same commit.
  providerReadyRef.current = providerReady
  if (listenersReadyRef.current === null) {
    listenersReadyRef.current = new Promise(resolve => {
      resolveListenersReadyRef.current = resolve
    })
  }
  const markLiveState = useCallback(() => {
    liveStateRevisionRef.current += 1
  }, [])
  const setStreaming = useCallback((value: boolean) => {
    streamingRef.current = value
    setIsStreaming(value)
  }, [])

  const setPendingFallback = useCallback((value: ProviderFallbackRequest | null) => {
    pendingFallbackRef.current = value
    setPendingFallbackState(value)
  }, [])

  const adoptSession = useCallback((id: string) => {
    persistentSessionId = id
    sessionIdRef.current = id
    setSessionId(id)
  }, [])

  const flushDeltas = useCallback(() => {
    flushTimerRef.current = null
    const pending = pendingTextRef.current
    if (pending.size === 0) return
    // Drain only entries whose message exists yet — deltas that arrived
    // before their send ack stay buffered for buildAssistantMessage to pick
    // up. The updater reads an immutable snapshot and mutates nothing:
    // React may invoke it twice (StrictMode), and an updater that deleted
    // from the shared buffer would drop chunks its first pass consumed.
    const live = new Map<string, string>()
    pending.forEach((text, id) => {
      if (messagesRef.current.some(message => message.id === id)) live.set(id, text)
    })
    if (live.size === 0) return
    live.forEach((_, id) => pending.delete(id))
    setMessages(prev =>
      prev.map(message => {
        const extra = live.get(message.id)
        if (extra === undefined) return message
        return { ...message, text: message.text + extra }
      })
    )
  }, [])

  const scheduleFlush = useCallback(() => {
    if (flushTimerRef.current === null) {
      flushTimerRef.current = setTimeout(flushDeltas, FLUSH_MS)
    }
  }, [flushDeltas])

  const applyTool = useCallback((tools: ChatToolActivity[], activity: ChatToolActivity) => {
    const index = tools.findIndex(tool => tool.callId === activity.callId)
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

  const applyProposal = useCallback((proposals: ActionProposal[] | undefined, proposal: ActionProposal) => {
    const next = [...(proposals ?? [])]
    const index = next.findIndex(existing => existing.proposalId === proposal.proposalId)
    if (index >= 0) next[index] = proposal
    else next.push(proposal)
    return next
  }, [])

  const bufferProposal = useCallback((messageId: string, proposal: ActionProposal) => {
    const proposals = pendingProposalsRef.current.get(messageId) ?? new Map<string, ActionProposal>()
    proposals.set(proposal.proposalId, proposal)
    pendingProposalsRef.current.set(messageId, proposals)
  }, [])

  const buildAssistantMessage = useCallback((messageId: string, ackProviderUse?: AIProviderUse): ChatMessageVM => {
    const text = pendingTextRef.current.get(messageId) ?? ''
    pendingTextRef.current.delete(messageId)
    const tools = Array.from(pendingToolsRef.current.get(messageId)?.values() ?? [])
    pendingToolsRef.current.delete(messageId)
    const stagedProposals = Array.from(pendingProposalsRef.current.get(messageId)?.values() ?? [])
    pendingProposalsRef.current.delete(messageId)
    const error = pendingErrorRef.current.get(messageId)
    pendingErrorRef.current.delete(messageId)
    const done = pendingDoneRef.current.get(messageId)
    pendingDoneRef.current.delete(messageId)
    const providerUse = done?.providerUse || (done ? providerUseFromLegacy(done.provider) : ackProviderUse)
    return {
      id: messageId,
      role: 'assistant',
      text,
      streaming: !done && !error,
      tools,
      stagedProposals,
      providerUse,
      finishReason: done?.finishReason,
      error,
    }
  }, [])

  useEffect(() => {
    let disposed = false
    const unlistens: UnlistenFn[] = []
    const register = async () => {
      const settled = await Promise.allSettled([
        listen<DeltaPayload>('ai-chat://delta', event => {
          const payload = event.payload
          if (payload.sessionId !== sessionIdRef.current) return
          markLiveState()
          pendingTextRef.current.set(
            payload.messageId,
            (pendingTextRef.current.get(payload.messageId) ?? '') + payload.text,
          )
          scheduleFlush()
        }),
        listen<ToolPayload>('ai-chat://tool', event => {
          const payload = event.payload
          if (payload.sessionId !== sessionIdRef.current) return
          markLiveState()
          const activity: ChatToolActivity = {
            callId: payload.callId,
            tool: payload.tool,
            argsSummary: payload.argsSummary,
            status: normalizeToolStatus(payload.status),
            durationMs: payload.durationMs,
            resultPreview: payload.resultPreview,
          }
          setMessages(prev => {
            let matched = false
            const next = prev.map(message => {
              if (message.id !== payload.messageId) return message
              matched = true
              return { ...message, tools: applyTool(message.tools, activity) }
            })
            if (!matched) bufferTool(payload.messageId, activity)
            return next
          })
        }),
        listen<DonePayload>('ai-chat://done', event => {
          const payload = event.payload
          if (payload.sessionId !== sessionIdRef.current) return
          markLiveState()
          flushDeltas()
          const providerUse = payload.providerUse || providerUseFromLegacy(payload.provider)
          setLastProviderUse(providerUse)
          setPendingFallback(null)
          setMessages(prev => {
            let matched = false
            const next = prev.map(message => {
              if (message.id !== payload.messageId) return message
              matched = true
              return { ...message, streaming: false, providerUse, finishReason: payload.finishReason }
            })
            if (!matched) pendingDoneRef.current.set(payload.messageId, payload)
            return next
          })
          setStreaming(false)
          setIsStopping(false)
        }),
        listen<ErrorPayload>('ai-chat://error', event => {
          const payload = event.payload
          if (payload.sessionId !== sessionIdRef.current) return
          markLiveState()
          setPendingFallback(null)
          setMessages(prev => {
            let matched = false
            const next = prev.map(message => {
              if (message.id !== payload.messageId) return message
              matched = true
              return { ...message, error: payload.message, streaming: false }
            })
            if (!matched) pendingErrorRef.current.set(payload.messageId, payload.message)
            return next
          })
          setStreaming(false)
          setIsStopping(false)
        }),
        listen<ProviderFallbackRequest>('ai-chat://fallback-required', event => {
          const payload = event.payload
          if (payload.sessionId !== sessionIdRef.current) return
          markLiveState()
          setPendingFallback(payload)
          // The turn is paused rather than actively producing text.
          setStreaming(false)
          setIsStopping(false)
        }),
        listen<ProposalPayload>('ai-chat://proposal', event => {
          const payload = event.payload
          if (payload.sessionId !== sessionIdRef.current) return
          markLiveState()
          setMessages(prev => {
            let matched = false
            const next = prev.map(message => {
              if (message.id !== payload.messageId) return message
              matched = true
              return { ...message, stagedProposals: applyProposal(message.stagedProposals, payload.proposal) }
            })
            if (!matched) bufferProposal(payload.messageId, payload.proposal)
            return next
          })
        }),
        listen<FullScanRequest>('ai-chat://scan-request', event => {
          const payload = event.payload
          if (payload.sessionId !== sessionIdRef.current || payload.kind !== 'full') return
          markLiveState()
          setPendingFullScan(current => current?.messageId === payload.messageId ? current : payload)
        }),
      ])
      const resolved = settled.flatMap(result => result.status === 'fulfilled' ? [result.value] : [])
      if (disposed) {
        resolved.forEach(unlisten => unlisten())
        return
      }
      unlistens.push(...resolved)
      const rejected = settled.find(result => result.status === 'rejected')
      if (rejected?.status === 'rejected') {
        listenerRegistrationErrorRef.current = rejected.reason
        logger.error('useAIChat', 'Failed to register chat event listeners', String(rejected.reason))
      }
      resolveListenersReadyRef.current?.()
    }
    void register()
    return () => {
      disposed = true
      unlistens.forEach(unlisten => unlisten())
      if (flushTimerRef.current !== null) {
        clearTimeout(flushTimerRef.current)
        flushTimerRef.current = null
      }
    }
  }, [scheduleFlush, flushDeltas, applyTool, bufferTool, applyProposal, bufferProposal, markLiveState, setPendingFallback, setStreaming])

  useEffect(() => {
    const sid = sessionIdRef.current
    if (!sid) return
    let disposed = false
    const epoch = conversationEpochRef.current
    const revision = liveStateRevisionRef.current
    const isCurrent = () => !disposed
      && sessionIdRef.current === sid
      && conversationEpochRef.current === epoch
      && liveStateRevisionRef.current === revision

    invoke<HistoryMessageView[] | HistorySnapshot>('ai_chat_get_history', { sessionId: sid })
      .then(response => {
        const snapshot: HistorySnapshot = Array.isArray(response)
          ? { sessionId: sid, messages: response, busy: false }
          : response
        if (snapshot.sessionId !== sid || !isCurrent()) return
        let lastAssistantIndex = -1
        snapshot.messages.forEach((view, index) => {
          if (view.role === 'assistant') lastAssistantIndex = index
        })
        const next: ChatMessageVM[] = snapshot.messages.map((view, index) => ({
          id: view.id,
          role: view.role,
          text: view.text,
          streaming: snapshot.busy && view.id === snapshot.activeMessageId,
          tools: view.tools.map(tool => ({
            callId: tool.callId,
            tool: tool.tool,
            argsSummary: tool.argsSummary,
            status: normalizeToolStatus(tool.status ?? 'completed'),
            durationMs: tool.durationMs,
            resultPreview: tool.resultPreview,
          })),
          providerUse: view.providerUse,
          finishReason: view.finishReason
            ?? (index === lastAssistantIndex ? snapshot.finishReason : undefined),
        }))
        if (next.length > 0) {
          setMessages(current => current.length === 0 && isCurrent() ? next : current)
        }
        for (const view of snapshot.messages) {
          for (const tool of view.tools.filter(candidate => candidate.tool === 'stage_remediation')) {
            const match = tool.resultPreview?.match(/"proposalId"\s*:\s*"([^"]+)"/)
            if (!match) continue
            void invoke<ActionProposal>('action_get_proposal', { proposalId: match[1] })
              .then(proposal => {
                if (disposed || sessionIdRef.current !== sid || conversationEpochRef.current !== epoch) return
                setMessages(prev => prev.map(message =>
                  message.id === view.id
                    ? { ...message, stagedProposals: applyProposal(message.stagedProposals, proposal) }
                    : message))
              })
              .catch(() => { /* consumed or expired proposals are intentionally absent */ })
          }
        }
        const latestUse = [...next].reverse().find(message => message.providerUse)?.providerUse
        if (latestUse) setLastProviderUse(latestUse)
        setPendingFallback(snapshot.pendingFallback
          ? { ...snapshot.pendingFallback, sessionId: snapshot.sessionId || sid }
          : null)
        setStreaming(snapshot.busy && !snapshot.pendingFallback)
      })
      .catch(error => {
        if (!disposed) logger.error('useAIChat', 'Failed to load chat history', String(error))
      })
    return () => {
      disposed = true
    }
  }, [applyProposal, setPendingFallback, setStreaming])

  const send = useCallback(async (input: string | ChatPrompt): Promise<boolean> => {
    const request = normalizePrompt(input)
    const query = request.query.trim()
    const displayText = (request.displayText || query).trim()
    if (!query || streamingRef.current || pendingFallbackRef.current || !aiEnabled || !providerReadyRef.current) return false
    const requestedEpoch = conversationEpochRef.current
    await listenersReadyRef.current
    if (requestedEpoch !== conversationEpochRef.current) return false
    if (streamingRef.current || pendingFallbackRef.current || !aiEnabled || !providerReadyRef.current) return false
    if (listenerRegistrationErrorRef.current) {
      markLiveState()
      setMessages(prev => [...prev, {
        id: `listener_err_${Date.now()}`,
        role: 'assistant',
        text: '',
        streaming: false,
        tools: [],
        error: 'Chat could not start because its event connection is unavailable.',
      }])
      return false
    }
    const epoch = conversationEpochRef.current
    markLiveState()
    setMessages(prev => [...prev, {
      id: `local_${Date.now()}_${prev.length}`,
      role: 'user',
      text: displayText,
      streaming: false,
      tools: [],
    }])
    setStreaming(true)
    try {
      let sid = sessionIdRef.current
      if (!sid) {
        sid = await invoke<string>('ai_chat_new_session')
        // The provider can enter validation while session creation is in
        // flight. Re-check at this async boundary so a send that was valid at
        // click time cannot slip through with the newly selected provider's
        // stale status. A conversation reset likewise supersedes this send.
        if (epoch !== conversationEpochRef.current) return true
        if (!providerReadyRef.current) {
          throw new Error(
            'The selected AI provider is still being validated. Try again when it is ready.',
          )
        }
        adoptSession(sid)
      }
      const ack = await invoke<ChatSendAck>('ai_chat_send', {
        sessionId: sid,
        // message keeps this frontend compatible with pre-structured backends.
        message: query,
        query,
        displayText,
        contextRefs: request.contextRefs,
      })
      if (epoch !== conversationEpochRef.current) return true
      markLiveState()
      adoptSession(ack.sessionId)
      const providerUse = ack.providerUse || providerUseFromLegacy(ack.provider)
      setLastProviderUse(providerUse)
      const assistant = buildAssistantMessage(ack.messageId, providerUse)
      setMessages(prev => [...prev, assistant])
      if (!assistant.streaming) setStreaming(false)
    } catch (error) {
      if (epoch !== conversationEpochRef.current) return true
      markLiveState()
      setStreaming(false)
      setMessages(prev => [...prev, {
        id: `err_${Date.now()}`,
        role: 'assistant',
        text: '',
        streaming: false,
        tools: [],
        error: error instanceof Error ? error.message : String(error),
      }])
    }
    return true
  }, [aiEnabled, adoptSession, buildAssistantMessage, markLiveState, setStreaming])

  const stop = useCallback(async () => {
    const sid = sessionIdRef.current
    if (!sid) return
    const epoch = conversationEpochRef.current
    markLiveState()
    setIsStopping(true)
    try {
      await invoke('ai_chat_cancel', { sessionId: sid })
      if (sessionIdRef.current === sid && conversationEpochRef.current === epoch) {
        setPendingFallback(null)
      }
    } catch (error) {
      logger.error('useAIChat', 'Failed to cancel chat', String(error))
    } finally {
      if (sessionIdRef.current === sid && conversationEpochRef.current === epoch) {
        setIsStopping(false)
      }
    }
  }, [markLiveState, setPendingFallback])

  const resolveFallback = useCallback(async (decision: 'allow' | 'never') => {
    const fallback = pendingFallbackRef.current
    const sid = sessionIdRef.current
    if (!fallback || !sid) return
    markLiveState()
    if (decision === 'allow') setStreaming(true)
    try {
      await invoke('ai_chat_resolve_fallback', {
        sessionId: sid,
        messageId: fallback.messageId,
        decision,
      })
      setPendingFallback(null)
      setSettings({ ...settings, cloudFallbackPolicy: decision })
      if (decision === 'never') setStreaming(false)
    } catch (error) {
      setMessages(prev => [...prev, {
        id: `fallback_err_${Date.now()}`,
        role: 'assistant',
        text: '',
        streaming: false,
        tools: [],
        error: error instanceof Error ? error.message : String(error),
      }])
      setStreaming(false)
    }
  }, [markLiveState, setPendingFallback, setSettings, setStreaming, settings])

  const newConversation = useCallback(async () => {
    const oldSessionId = sessionIdRef.current
    conversationEpochRef.current++
    markLiveState()
    if (oldSessionId && (streamingRef.current || pendingFallbackRef.current)) {
      await invoke('ai_chat_cancel', { sessionId: oldSessionId }).catch(error =>
        logger.error('useAIChat', 'Failed to cancel previous chat turn', String(error))
      )
    }
    try {
      const sid = await invoke<string>('ai_chat_new_session')
      adoptSession(sid)
      pendingTextRef.current.clear()
      pendingDoneRef.current.clear()
      pendingErrorRef.current.clear()
      pendingToolsRef.current.clear()
      pendingProposalsRef.current.clear()
      setMessages([])
      setLastProviderUse(null)
      setPendingFallback(null)
      setPendingFullScan(null)
      setStreaming(false)
      setIsStopping(false)
    } catch (error) {
      logger.error('useAIChat', 'Failed to start a new conversation', String(error))
    }
  }, [adoptSession, markLiveState, setPendingFallback, setStreaming])

  return {
    messages,
    send,
    stop,
    resolveFallback,
    newConversation,
    isStreaming,
    isStopping,
    pendingFallback,
    pendingFullScan,
    dismissFullScanRequest: () => setPendingFullScan(null),
    lastProviderUse,
    sessionId,
    aiEnabled,
  }
}
