import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useAIChat } from '../hooks/useAIChat'
import { useScanReport } from '../hooks/useScanReport'
import { useScanner, type ScanRunOutcome } from '../hooks/useScanner'
import { useAIContext } from './AIContext'
import { useAppContext } from './AppContext'
import type { ChatPrompt } from '../components/types'

const EXPLICIT_GENERAL_KNOWLEDGE = /\b(?:in general|generally)\b|^\s*(?:please\s+)?(?:define\b|what\s+does\b.+\bmean\b|what\s+is\s+windows\??\s*$)/i
const CASUAL_NON_DEVICE_CHAT = /^\s*(?:(?:hi|hello|hey|thanks|thank\s+you|good\s+(?:morning|afternoon|evening))[\s.!?,]*|(?:who\s+are\s+you|what\s+can\s+you\s+do|tell\s+me\s+a\s+joke|write\s+(?:me\s+)?(?:a\s+)?(?:poem|story|email))[\s.!?,]*)$/i

/**
 * The assistant is a device-diagnostics surface, so substantive requests are
 * evidence-dependent by default. Explicit definitions and casual chat are the
 * narrow exceptions. This avoids a brittle noun list that misses symptoms
 * such as freezing, flickering, USB disconnects, or an unknown hostname.
 */
export function requiresScanData(prompt: ChatPrompt | string): boolean {
  if (typeof prompt !== 'string' && (prompt.contextRefs?.length ?? 0) > 0) return true
  const text = (typeof prompt === 'string' ? prompt : `${prompt.displayText ?? ''} ${prompt.query}`).trim()
  if (!text || EXPLICIT_GENERAL_KNOWLEDGE.test(text) || CASUAL_NON_DEVICE_CHAT.test(text)) return false
  return true
}

function promptLabel(prompt: ChatPrompt | string): string {
  return typeof prompt === 'string' ? prompt : (prompt.displayText || prompt.query)
}

function asChatPrompt(prompt: ChatPrompt | string): ChatPrompt {
  return typeof prompt === 'string'
    ? { query: prompt, displayText: prompt, contextRefs: [] }
    : prompt
}

function withoutScanGate(prompt: ChatPrompt | string): ChatPrompt | string {
  if (typeof prompt === 'string' || !prompt.scanGate) return prompt
  const { scanGate: _scanGate, ...request } = prompt
  return request
}

function scanRequestId(): string {
  return `scan_${Date.now()}_${Math.random().toString(36).slice(2)}`
}

type AIWorkspaceContextValue = {
  chat: ReturnType<typeof useAIChat>
  scanReport: ReturnType<typeof useScanReport>
  queuePrompt: (prompt: ChatPrompt | string) => void
  retryPendingScan: () => void
  cancelPendingPrompt: () => void
  acceptFullScanRequest: () => void
  dismissFullScanRequest: () => void
  reportPreparation: { status: 'idle' | 'waiting' | 'running' | 'failed'; observedRunning?: boolean; error?: string }
  retryReportPreparation: () => void
  cancelReportPreparation: () => void
}

const AIWorkspaceContext = createContext<AIWorkspaceContextValue | null>(null)

/** Keeps chat/report subscriptions alive even when the AI tab is not visible. */
export const AIWorkspaceProvider: React.FC<{ children: ReactNode }> = ({ children }) => {
  const { isAIAvailable, isLoading } = useAIContext()
  const {
    availableTasks,
    sessionId: scanSessionId,
    results,
    isRunning,
    diagnosticsError,
    pendingChatPrompt,
    setPendingChatPrompt,
    pendingScanReport,
    setPendingScanReport,
  } = useAppContext()
  // The backend is authoritative too, but keeping this gate beside the
  // long-lived chat hook closes the provider-switch window for every caller —
  // including deep links that bypass the visible composer.
  const chat = useAIChat(!isLoading && isAIAvailable)
  const scanReport = useScanReport()
  const {
    aiEnabled,
    isStreaming,
    pendingFallback,
    pendingFullScan,
    send: sendChat,
    dismissFullScanRequest,
  } = chat
  const { runQuickScanTracked, runFullScanTracked, stopScan } = useScanner()
  const activePreflightsRef = useRef(new Set<string>())
  const dispatchingPromptRef = useRef<ChatPrompt | string | null>(null)
  const [reportPreparation, setReportPreparation] = useState<AIWorkspaceContextValue['reportPreparation']>({ status: 'idle' })
  const publishReportPreparation = useCallback((next: AIWorkspaceContextValue['reportPreparation']) => {
    queueMicrotask(() => setReportPreparation(next))
  }, [])

  const queuePrompt = useCallback((prompt: ChatPrompt | string) => {
    if (!promptLabel(prompt).trim()) return
    // The composer is disabled while a request is pending; keep this guard at
    // the lifetime boundary too so a late deep-link cannot replace it.
    setPendingChatPrompt(current => current ?? prompt)
  }, [setPendingChatPrompt])

  const updatePendingGate = useCallback((
    requestId: string,
    update: (gate: NonNullable<ChatPrompt['scanGate']>) => NonNullable<ChatPrompt['scanGate']>,
  ) => {
    setPendingChatPrompt(current => {
      if (!current || typeof current === 'string' || current.scanGate?.requestId !== requestId) return current
      return { ...current, scanGate: update(current.scanGate) }
    })
  }, [setPendingChatPrompt])

  const runPreflight = useCallback(async (
    requestId: string,
    kind: 'quick' | 'full',
  ) => {
    if (activePreflightsRef.current.has(requestId)) return
    activePreflightsRef.current.add(requestId)
    const run = kind === 'quick' ? runQuickScanTracked : runFullScanTracked
    let outcome: ScanRunOutcome
    try {
      outcome = await run()
    } catch {
      outcome = 'failed'
    } finally {
      activePreflightsRef.current.delete(requestId)
    }
    updatePendingGate(requestId, gate => {
      if (outcome === 'completed') return { ...gate, status: 'ready', error: undefined }
      if (outcome === 'busy') {
        // The module lock is set just before AppContext publishes isRunning.
        // Give that render a chance to arrive; if it never does, retry rather
        // than falsely treating lock contention as a failed scan.
        setTimeout(() => {
          updatePendingGate(requestId, current => current.status === 'waiting' && !current.observedRunning
            ? { ...current, status: 'queued' }
            : current)
        }, 50)
        return { ...gate, status: 'waiting', observedRunning: false, error: undefined }
      }
      return {
        ...gate,
        status: 'failed',
        error: outcome === 'cancelled'
          ? 'The scan was stopped before the question could be sent.'
          : 'The scan could not be completed. Your question is still waiting.',
      }
    })
  }, [runFullScanTracked, runQuickScanTracked, updatePendingGate])

  // A consent request is tied to the exact Quick/targeted scan the model
  // inspected. If a manual scan replaces it before the user decides, discard
  // the stale request and let the next question reassess the new evidence.
  useEffect(() => {
    if (pendingFullScan && pendingFullScan.sourceScanId !== scanSessionId) {
      dismissFullScanRequest()
    }
  }, [dismissFullScanRequest, pendingFullScan, scanSessionId])

  // Prepare and dispatch every queued prompt at app lifetime. This remains
  // mounted while tabs change, so navigation cannot lose the question or
  // start a second scan.
  useEffect(() => {
    if (!pendingChatPrompt) return
    const hasResults = Object.keys(results).length > 0
    const structured = asChatPrompt(pendingChatPrompt)
    const gate = structured.scanGate

    if (!gate && !hasResults && requiresScanData(pendingChatPrompt)) {
      setPendingChatPrompt({
        ...structured,
        scanGate: {
          requestId: scanRequestId(),
          kind: 'quick',
          status: isRunning ? 'waiting' : 'queued',
          observedRunning: isRunning,
        },
      })
      return
    }

    if (gate) {
      // A scan replaces the current diagnostic session. Never do that while
      // the model is still finishing a turn against the previous evidence.
      if (isStreaming) return
      if (gate.status === 'queued') {
        if (isRunning) {
          updatePendingGate(gate.requestId, current => ({ ...current, status: 'waiting', observedRunning: true }))
          return
        }
        if (gate.kind === 'quick' && hasResults) {
          updatePendingGate(gate.requestId, current => ({ ...current, status: 'ready' }))
          return
        }
        // Do not fail a prompt just because the task catalog is still loading.
        if (availableTasks.length === 0) return
        updatePendingGate(gate.requestId, current => ({ ...current, status: 'running', error: undefined }))
        void runPreflight(gate.requestId, gate.kind)
        return
      }
      if (gate.status === 'waiting') {
        if (isRunning) {
          if (!gate.observedRunning) {
            updatePendingGate(gate.requestId, current => ({ ...current, observedRunning: true }))
          }
          return
        }
        if (!gate.observedRunning) return
        if (gate.kind === 'quick' && hasResults) {
          updatePendingGate(gate.requestId, current => ({ ...current, status: 'ready', error: undefined }))
        } else if (gate.kind === 'full') {
          updatePendingGate(gate.requestId, current => ({ ...current, status: 'queued', error: undefined }))
        } else {
          updatePendingGate(gate.requestId, current => ({
            ...current,
            status: 'failed',
            error: diagnosticsError || 'The existing scan ended without usable results.',
          }))
        }
        return
      }
      if (gate.status === 'running' || gate.status === 'failed') return
      if (gate.status === 'ready' && !hasResults) {
        updatePendingGate(gate.requestId, current => ({
          ...current,
          status: 'failed',
          error: diagnosticsError || 'The scan completed without usable results.',
        }))
        return
      }
    }

    if (isLoading || !aiEnabled || !isAIAvailable || isStreaming || pendingFallback || pendingFullScan) return
    if (dispatchingPromptRef.current === pendingChatPrompt) return
    const request = withoutScanGate(pendingChatPrompt)
    const queuedPrompt = pendingChatPrompt
    dispatchingPromptRef.current = queuedPrompt
    void sendChat(request)
      .then(accepted => {
        if (!accepted) return
        setPendingChatPrompt(current => current === queuedPrompt ? null : current)
      })
      .finally(() => {
        if (dispatchingPromptRef.current === queuedPrompt) dispatchingPromptRef.current = null
      })
  }, [
    availableTasks.length,
    aiEnabled,
    diagnosticsError,
    isAIAvailable,
    isLoading,
    isRunning,
    isStreaming,
    pendingChatPrompt,
    pendingFallback,
    pendingFullScan,
    results,
    runPreflight,
    sendChat,
    setPendingChatPrompt,
    updatePendingGate,
  ])

  // The report entry point follows the same evidence-first rule. A deep-link
  // or the explicit report action starts one Quick Scan and generates only
  // after results exist; it never asks the model to explain an empty scan.
  useEffect(() => {
    if (!pendingScanReport) {
      if (reportPreparation.status !== 'idle') publishReportPreparation({ status: 'idle' })
      return
    }
    const hasResults = Object.keys(results).length > 0
    if (hasResults) return
    if (isRunning) {
      if (reportPreparation.status === 'idle' || !reportPreparation.observedRunning) {
        publishReportPreparation({ status: 'waiting', observedRunning: true })
      }
      return
    }
    if (reportPreparation.status === 'waiting') {
      if (!reportPreparation.observedRunning) return
      publishReportPreparation({
        status: 'failed',
        error: diagnosticsError || 'The scan ended without usable results.',
      })
      return
    }
    if (reportPreparation.status !== 'idle' || availableTasks.length === 0) return
    publishReportPreparation({ status: 'running' })
    void runQuickScanTracked().then(outcome => {
      if (outcome === 'completed') {
        setReportPreparation({ status: 'idle' })
      } else if (outcome === 'busy') {
        setReportPreparation({ status: 'waiting', observedRunning: false })
        setTimeout(() => {
          setReportPreparation(current => current.status === 'waiting' && !current.observedRunning
            ? { status: 'idle' }
            : current)
        }, 50)
      } else {
        setReportPreparation({
          status: 'failed',
          error: outcome === 'cancelled'
            ? 'The Quick Scan was stopped before the report could be generated.'
            : 'The Quick Scan could not be completed.',
        })
      }
    })
  }, [
    availableTasks.length,
    diagnosticsError,
    isRunning,
    pendingScanReport,
    publishReportPreparation,
    reportPreparation.observedRunning,
    reportPreparation.status,
    results,
    runQuickScanTracked,
  ])

  const retryPendingScan = useCallback(() => {
    setPendingChatPrompt(current => {
      if (!current || typeof current === 'string' || !current.scanGate) return current
      return { ...current, scanGate: { ...current.scanGate, status: 'queued', error: undefined } }
    })
  }, [setPendingChatPrompt])

  const cancelPendingPrompt = useCallback(() => {
    const gate = typeof pendingChatPrompt === 'string' ? undefined : pendingChatPrompt?.scanGate
    if (gate?.status === 'running') stopScan()
    setPendingChatPrompt(null)
  }, [pendingChatPrompt, setPendingChatPrompt, stopScan])

  const acceptFullScanRequest = useCallback(() => {
    const request = pendingFullScan
    if (!request || pendingChatPrompt || isStreaming || isRunning || request.sourceScanId !== scanSessionId) return
    dismissFullScanRequest()
    setPendingChatPrompt({
      displayText: 'Run the Full Scan and continue',
      // Use the exact question that already fit the original provider budget.
      // A verbose synthetic continuation can crowd the 2,500-character Phi
      // Silica window and cause its history trimmer to lose the real request.
      query: request.question,
      scanGate: {
        requestId: scanRequestId(),
        kind: 'full',
        status: isRunning ? 'waiting' : 'queued',
        observedRunning: isRunning,
        reason: request.reason,
      },
    })
  }, [dismissFullScanRequest, isRunning, isStreaming, pendingChatPrompt, pendingFullScan, scanSessionId, setPendingChatPrompt])

  const retryReportPreparation = useCallback(() => {
    setReportPreparation({ status: 'idle' })
  }, [])

  const cancelReportPreparation = useCallback(() => {
    if (reportPreparation.status === 'running') stopScan()
    setPendingScanReport(false)
    setReportPreparation({ status: 'idle' })
  }, [reportPreparation.status, setPendingScanReport, stopScan])

  const value = useMemo<AIWorkspaceContextValue>(() => ({
    chat,
    scanReport,
    queuePrompt,
    retryPendingScan,
    cancelPendingPrompt,
    acceptFullScanRequest,
    dismissFullScanRequest,
    reportPreparation,
    retryReportPreparation,
    cancelReportPreparation,
  }), [
    acceptFullScanRequest,
    cancelPendingPrompt,
    cancelReportPreparation,
    chat,
    dismissFullScanRequest,
    queuePrompt,
    reportPreparation,
    retryPendingScan,
    retryReportPreparation,
    scanReport,
  ])
  return (
    <AIWorkspaceContext.Provider value={value}>
      {children}
    </AIWorkspaceContext.Provider>
  )
}

export function useAIWorkspace(): AIWorkspaceContextValue {
  const value = useContext(AIWorkspaceContext)
  if (!value) throw new Error('useAIWorkspace must be used within AIWorkspaceProvider')
  return value
}
