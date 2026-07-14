import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { useAppContext } from '../contexts/AppContext'
import { useToast } from '../contexts/ToastContext'
import type { AIProviderId, AIProviderUse } from '../components/types'
import * as logger from '../utils/logger'

interface ReportAck { reportId: string; cached: boolean; provider: AIProviderId; providerUse?: AIProviderUse; report?: string }
interface ReportDelta { reportId: string; text: string }
interface ReportDone { reportId: string; finishReason: string; provider: AIProviderId; providerUse?: AIProviderUse }
interface ReportError { reportId: string; message: string }

export interface ScanReportState {
  report: string
  generating: boolean
  cancelling: boolean
  error: string | null
  hasResults: boolean
  aiEnabled: boolean
  lastProviderUse: AIProviderUse | null
  generate: (forceRefresh?: boolean) => Promise<void>
  cancel: () => Promise<void>
  copy: () => Promise<void>
}

/**
 * One-click AI health report for the current scan. Owns the generation state
 * machine and the `ai-report://*` streaming subscription (instant when served
 * from the backend cache). Presentation lives in ScanReportPanel.
 */
export function useScanReport(): ScanReportState {
  const { results, settings } = useAppContext()
  const { showSuccess, showError } = useToast()
  const [report, setReport] = useState('')
  const [generating, setGenerating] = useState(false)
  const [cancelling, setCancelling] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [lastProviderUse, setLastProviderUse] = useState<AIProviderUse | null>(null)
  const [reportSourceResults, setReportSourceResults] = useState<typeof results | null>(null)
  const aiEnabled = settings.aiEnabled ?? true
  const reportIdRef = useRef<string | null>(null)
  const generatingRef = useRef(false)
  const cancellingRef = useRef(false)
  const sourceResultsRef = useRef<typeof results | null>(null)
  const generationEpochRef = useRef(0)
  const pendingReportEventsRef = useRef<Map<string, {
    text: string
    done?: ReportDone
    error?: string
  }>>(new Map())
  const hasResults = Object.keys(results).length > 0

  // A report belongs to the exact result snapshot that started it. Hide and
  // cancel it as soon as a new scan (or targeted rerun) replaces that object;
  // otherwise the persistent AI workspace could label an old report as the
  // latest scan after the user navigated away and ran diagnostics.
  useEffect(() => {
    if (reportSourceResults === null || reportSourceResults === results) return
    generationEpochRef.current += 1
    sourceResultsRef.current = null
    generatingRef.current = false
    const reportId = reportIdRef.current
    reportIdRef.current = null
    pendingReportEventsRef.current.clear()
    if (reportId) {
      void invoke('ai_report_cancel', { reportId }).catch(err =>
        logger.warn('ScanReportPanel', 'Failed to cancel stale report', String(err))
      )
    }
  }, [reportSourceResults, results])

  const legacyProviderUse = useCallback((provider: AIProviderId): AIProviderUse => ({
    providerId: provider,
    executionClass: provider === 'phi_silica'
      ? 'on_device'
      : provider === 'foundry_local' || provider === 'ollama'
        ? 'local_server'
        : provider === 'codex_cli' || provider === 'claude_code'
          ? 'subscription_cloud'
          : 'api_cloud',
  }), [])

  const setGeneratingState = useCallback((value: boolean) => {
    generatingRef.current = value
    setGenerating(value)
  }, [])

  const setCancellingState = useCallback((value: boolean) => {
    cancellingRef.current = value
    setCancelling(value)
  }, [])

  const bufferReportDelta = useCallback((payload: ReportDelta) => {
    const pending = pendingReportEventsRef.current.get(payload.reportId) ?? { text: '' }
    pending.text += payload.text
    pendingReportEventsRef.current.set(payload.reportId, pending)
  }, [])

  const bufferReportDone = useCallback((payload: ReportDone) => {
    const pending = pendingReportEventsRef.current.get(payload.reportId) ?? { text: '' }
    pending.done = payload
    pendingReportEventsRef.current.set(payload.reportId, pending)
  }, [])

  const bufferReportError = useCallback((payload: ReportError) => {
    const pending = pendingReportEventsRef.current.get(payload.reportId) ?? { text: '' }
    pending.error = payload.message
    pendingReportEventsRef.current.set(payload.reportId, pending)
  }, [])

  const applyBufferedReportEvents = useCallback((reportId: string) => {
    const pending = pendingReportEventsRef.current.get(reportId)
    if (!pending) return
    pendingReportEventsRef.current.delete(reportId)
    if (pending.text) {
      setReport(prev => prev + pending.text)
    }
    if (pending.error) {
      setError(pending.error)
      setGeneratingState(false)
    } else if (pending.done) {
      setLastProviderUse(pending.done.providerUse || legacyProviderUse(pending.done.provider))
      setGeneratingState(false)
    }
  }, [legacyProviderUse, setGeneratingState])

  useEffect(() => {
    let disposed = false
    const unlistens: UnlistenFn[] = []
    const register = async () => {
      const resolved = await Promise.all([
        listen<ReportDelta>('ai-report://delta', event => {
          if (!reportIdRef.current) {
            bufferReportDelta(event.payload)
            return
          }
          if (event.payload.reportId !== reportIdRef.current) return
          setReport(prev => prev + event.payload.text)
        }),
        listen<ReportDone>('ai-report://done', event => {
          if (!reportIdRef.current) {
            bufferReportDone(event.payload)
            return
          }
          if (event.payload.reportId !== reportIdRef.current) return
          setLastProviderUse(event.payload.providerUse || legacyProviderUse(event.payload.provider))
          setGeneratingState(false)
        }),
        listen<ReportError>('ai-report://error', event => {
          if (!reportIdRef.current) {
            bufferReportError(event.payload)
            return
          }
          if (event.payload.reportId !== reportIdRef.current) return
          setError(event.payload.message)
          setGeneratingState(false)
        }),
      ])
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
    }
  }, [bufferReportDelta, bufferReportDone, bufferReportError, legacyProviderUse, setGeneratingState])

  const generate = useCallback(async (forceRefresh = false) => {
    if (cancellingRef.current || (generatingRef.current && sourceResultsRef.current === results)) return
    if (!aiEnabled) {
      setGeneratingState(false)
      return
    }
    const previousReportId = reportIdRef.current
    if (previousReportId && sourceResultsRef.current !== results) {
      void invoke('ai_report_cancel', { reportId: previousReportId }).catch(() => {})
    }
    const epoch = ++generationEpochRef.current
    sourceResultsRef.current = results
    setReportSourceResults(results)
    setError(null)
    setReport('')
    setLastProviderUse(null)
    setGeneratingState(true)
    reportIdRef.current = null
    pendingReportEventsRef.current.clear()
    try {
      const ack = await invoke<ReportAck>('ai_generate_report', {
        previousScanId: null,
        forceRefresh,
      })
      if (epoch !== generationEpochRef.current || sourceResultsRef.current !== results) {
        if (!ack.cached) {
          await invoke('ai_report_cancel', { reportId: ack.reportId }).catch(() => {})
        }
        if (cancellingRef.current) setCancellingState(false)
        return
      }
      reportIdRef.current = ack.reportId
      setLastProviderUse(ack.providerUse || legacyProviderUse(ack.provider))
      if (ack.report) {
        // Cache hit — full text, no events coming
        setReport(ack.report)
        setGeneratingState(false)
      } else {
        applyBufferedReportEvents(ack.reportId)
      }
    } catch (err) {
      if (epoch !== generationEpochRef.current || sourceResultsRef.current !== results) {
        if (cancellingRef.current) setCancellingState(false)
        return
      }
      setError(String(err))
      setGeneratingState(false)
    }
  }, [aiEnabled, applyBufferedReportEvents, legacyProviderUse, results, setCancellingState, setGeneratingState])

  const copy = useCallback(async () => {
    try {
      await writeText(report)
      showSuccess('Copied', 'Report copied to clipboard')
    } catch (err) {
      logger.error('ScanReportPanel', 'Failed to copy report', String(err))
      showError('Copy failed', 'Could not copy the report to the clipboard')
    }
  }, [report, showSuccess, showError])

  const cancel = useCallback(async () => {
    if (cancellingRef.current) return
    const reportId = reportIdRef.current
    const waitingForAck = generatingRef.current && reportId === null
    // Stop locally first. The backend IPC is best-effort and must not leave the
    // UI stuck in a generating state when a cancellation response is delayed
    // or fails. If setup is still awaiting its ack, generate() will cancel the
    // newly returned report id after noticing the invalidated epoch.
    generationEpochRef.current += 1
    sourceResultsRef.current = null
    setReportSourceResults(null)
    reportIdRef.current = null
    pendingReportEventsRef.current.clear()
    setGeneratingState(false)
    setCancellingState(true)
    if (!reportId) {
      // The stale generate() continuation owns the eventual backend cancel and
      // clears this flag after its ack/error arrives.
      if (!waitingForAck) setCancellingState(false)
      return
    }
    try {
      await invoke('ai_report_cancel', { reportId })
    } catch (err) {
      logger.error('ScanReportPanel', 'Failed to cancel report', String(err))
    } finally {
      setCancellingState(false)
    }
  }, [setCancellingState, setGeneratingState])

  const reportIsCurrent = reportSourceResults === results
  return {
    report: reportIsCurrent ? report : '',
    generating: reportIsCurrent ? generating : false,
    cancelling,
    error: reportIsCurrent ? error : null,
    hasResults,
    aiEnabled,
    lastProviderUse: reportIsCurrent ? lastProviderUse : null,
    generate,
    cancel,
    copy,
  }
}
