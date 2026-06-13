import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { useAppContext } from '../contexts/AppContext'
import { useToast } from '../contexts/ToastContext'
import * as logger from '../utils/logger'

interface ReportAck { reportId: string; cached: boolean; provider: string; report?: string }
interface ReportDelta { reportId: string; text: string }
interface ReportDone { reportId: string; finishReason: string; provider: string }
interface ReportError { reportId: string; message: string }

export interface ScanReportState {
  report: string
  generating: boolean
  error: string | null
  hasResults: boolean
  generate: () => Promise<void>
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
  const [error, setError] = useState<string | null>(null)
  const reportIdRef = useRef<string | null>(null)
  const apiKeyRef = useRef<string | undefined>(settings.openAiApiKey)
  useEffect(() => { apiKeyRef.current = settings.openAiApiKey }, [settings.openAiApiKey])

  const hasResults = Object.keys(results).length > 0

  useEffect(() => {
    let disposed = false
    const unlistens: UnlistenFn[] = []
    const register = async () => {
      const resolved = await Promise.all([
        listen<ReportDelta>('ai-report://delta', event => {
          if (event.payload.reportId !== reportIdRef.current) return
          setReport(prev => prev + event.payload.text)
        }),
        listen<ReportDone>('ai-report://done', event => {
          if (event.payload.reportId !== reportIdRef.current) return
          setGenerating(false)
        }),
        listen<ReportError>('ai-report://error', event => {
          if (event.payload.reportId !== reportIdRef.current) return
          setError(event.payload.message)
          setGenerating(false)
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
  }, [])

  const generate = useCallback(async () => {
    setError(null)
    setReport('')
    setGenerating(true)
    try {
      const ack = await invoke<ReportAck>('ai_generate_report', {
        previousScanId: null,
        apiKey: apiKeyRef.current || null,
      })
      reportIdRef.current = ack.reportId
      if (ack.report) {
        // Cache hit — full text, no events coming
        setReport(ack.report)
        setGenerating(false)
      }
    } catch (err) {
      setError(String(err))
      setGenerating(false)
    }
  }, [])

  const copy = useCallback(async () => {
    try {
      await writeText(report)
      showSuccess('Copied', 'Report copied to clipboard')
    } catch (err) {
      logger.error('ScanReportPanel', 'Failed to copy report', String(err))
      showError('Copy failed', 'Could not copy the report to the clipboard')
    }
  }, [report, showSuccess, showError])

  return { report, generating, error, hasResults, generate, copy }
}
