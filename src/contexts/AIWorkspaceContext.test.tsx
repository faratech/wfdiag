import { cloneElement, useState, type Dispatch, type ReactElement, type SetStateAction } from 'react'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ChatPrompt } from '../components/types'
import type { ScanRunOutcome } from '../hooks/useScanner'
import { AIWorkspaceProvider, requiresScanData, useAIWorkspace } from './AIWorkspaceContext'

type ResultMap = Record<string, { success: boolean; output: string; duration_ms: number }>

interface MockAppValue {
  availableTasks: Array<{ id: string; name: string; admin_required: boolean }>
  sessionId: string | null
  results: ResultMap
  setResults: Dispatch<SetStateAction<ResultMap>>
  isRunning: boolean
  diagnosticsError: string | null
  pendingChatPrompt: ChatPrompt | string | null
  setPendingChatPrompt: Dispatch<SetStateAction<ChatPrompt | string | null>>
  pendingScanReport: boolean
  setPendingScanReport: Dispatch<SetStateAction<boolean>>
}

let appValue: MockAppValue
let initialResults: ResultMap
let initialPendingPrompt: ChatPrompt | string | null
let initialPendingReport: boolean
let aiAvailable: boolean
let quickOutcome: ScanRunOutcome
let fullOutcome: ScanRunOutcome
let chatState: Record<string, unknown>

const send = vi.fn()
const runQuickScanTracked = vi.fn()
const runFullScanTracked = vi.fn()
const stopScan = vi.fn()
const dismissFullScanRequest = vi.fn()

vi.mock('./AppContext', () => ({
  useAppContext: () => appValue,
}))

vi.mock('./AIContext', () => ({
  useAIContext: () => ({ isAIAvailable: aiAvailable, isLoading: false }),
}))

vi.mock('../hooks/useScanReport', () => ({
  useScanReport: () => ({
    report: '', generating: false, cancelling: false, error: null,
    hasResults: Object.keys(appValue.results).length > 0,
    aiEnabled: true, lastProviderUse: null,
    generate: vi.fn(), cancel: vi.fn(), copy: vi.fn(),
  }),
}))

vi.mock('../hooks/useAIChat', () => ({
  useAIChat: () => chatState,
}))

vi.mock('../hooks/useScanner', () => ({
  useScanner: () => ({
    runQuickScanTracked,
    runFullScanTracked,
    stopScan,
  }),
}))

function MockAppProvider({ children }: { children: ReactElement }) {
  const [results, setResults] = useState<ResultMap>(initialResults)
  const [pendingChatPrompt, setPendingChatPrompt] = useState<ChatPrompt | string | null>(initialPendingPrompt)
  const [pendingScanReport, setPendingScanReport] = useState(initialPendingReport)
  // The mocked hook reads this snapshot; cloning the child below forces it to
  // consume the new snapshot on every harness render.
  // eslint-disable-next-line react-hooks/globals
  appValue = {
    availableTasks: [{ id: 'os_info', name: 'OS information', admin_required: false }],
    sessionId: Object.keys(results).length > 0 ? 'quick-session' : null,
    results,
    setResults,
    isRunning: false,
    diagnosticsError: null,
    pendingChatPrompt,
    setPendingChatPrompt,
    pendingScanReport,
    setPendingScanReport,
  }
  return cloneElement(children)
}

function Probe() {
  const workspace = useAIWorkspace()
  return (
    <>
      <button type="button" onClick={() => workspace.queuePrompt('Summarize my latest scan')}>Queue summary</button>
      <button type="button" onClick={workspace.acceptFullScanRequest}>Accept full scan</button>
      <output data-testid="pending">
        {typeof appValue.pendingChatPrompt === 'string'
          ? appValue.pendingChatPrompt
          : appValue.pendingChatPrompt?.scanGate?.status ?? ''}
      </output>
    </>
  )
}

function renderWorkspace() {
  return render(
    <MockAppProvider>
      <AIWorkspaceProvider><Probe /></AIWorkspaceProvider>
    </MockAppProvider>,
  )
}

beforeEach(() => {
  initialResults = {}
  initialPendingPrompt = null
  initialPendingReport = false
  aiAvailable = true
  quickOutcome = 'completed'
  fullOutcome = 'completed'
  send.mockReset().mockResolvedValue(true)
  stopScan.mockReset()
  dismissFullScanRequest.mockReset().mockImplementation(() => {
    chatState = { ...chatState, pendingFullScan: null }
  })
  runQuickScanTracked.mockReset().mockImplementation(async () => {
    if (quickOutcome === 'completed') {
      appValue.setResults({ os_info: { success: true, output: '{}', duration_ms: 1 } })
    }
    return quickOutcome
  })
  runFullScanTracked.mockReset().mockImplementation(async () => {
    if (fullOutcome === 'completed') {
      appValue.setResults({
        os_info: { success: true, output: '{}', duration_ms: 1 },
        event_logs: { success: true, output: '{}', duration_ms: 1 },
      })
    }
    return fullOutcome
  })
  chatState = {
    aiEnabled: true,
    isStreaming: false,
    pendingFallback: null,
    pendingFullScan: null,
    send,
    dismissFullScanRequest,
  }
})

describe('requiresScanData', () => {
  it('recognizes every suggested PC-data question without classifying general chat', () => {
    for (const prompt of [
      'Summarize my latest scan',
      'What failed and why?',
      'Any security concerns?',
      'How do I free up disk space?',
      'Why does my Wi-Fi keep disconnecting?',
      'Which GPU is installed?',
      'What Windows version is this?',
      'Is TPM available?',
      'How much free space do I have?',
      'Why does it keep freezing?',
      'Why is the screen flickering?',
      'Why are my USB devices disconnecting?',
      'What is the hostname?',
      'Hi, summarize my latest scan',
      'Thanks—what failed?',
      'Hey, why is my PC slow?',
    ]) {
      expect(requiresScanData(prompt)).toBe(true)
    }
    expect(requiresScanData('Explain what UEFI means in general')).toBe(false)
    expect(requiresScanData('What is Windows?')).toBe(false)
    expect(requiresScanData('Tell me a joke')).toBe(false)
    expect(requiresScanData({ query: 'Explain this', contextRefs: [{ kind: 'diagnostic', id: 'os_info' }] })).toBe(true)
  })
})

describe('AIWorkspace scan preparation', () => {
  it('runs one Quick Scan for the exact suggestion, then sends the preserved prompt once', async () => {
    renderWorkspace()
    fireEvent.click(screen.getByRole('button', { name: 'Queue summary' }))

    await waitFor(() => expect(runQuickScanTracked).toHaveBeenCalledOnce())
    await waitFor(() => expect(send).toHaveBeenCalledWith(expect.objectContaining({
      query: 'Summarize my latest scan',
      displayText: 'Summarize my latest scan',
    })))
    expect(send).toHaveBeenCalledOnce()
    expect(runFullScanTracked).not.toHaveBeenCalled()
  })

  it('uses existing results without starting another scan', async () => {
    initialResults = { os_info: { success: true, output: '{}', duration_ms: 1 } }
    renderWorkspace()
    fireEvent.click(screen.getByRole('button', { name: 'Queue summary' }))

    await waitFor(() => expect(send).toHaveBeenCalledWith('Summarize my latest scan'))
    expect(runQuickScanTracked).not.toHaveBeenCalled()
    expect(runFullScanTracked).not.toHaveBeenCalled()
  })

  it.each(['failed', 'cancelled'] as const)('retains the prompt and never sends when Quick Scan is %s', async outcome => {
    quickOutcome = outcome
    renderWorkspace()
    fireEvent.click(screen.getByRole('button', { name: 'Queue summary' }))

    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('failed'))
    expect(send).not.toHaveBeenCalled()
    expect(runQuickScanTracked).toHaveBeenCalledOnce()
  })

  it('keeps the prepared prompt when the provider is unavailable after scanning', async () => {
    aiAvailable = false
    renderWorkspace()
    fireEvent.click(screen.getByRole('button', { name: 'Queue summary' }))

    await waitFor(() => expect(runQuickScanTracked).toHaveBeenCalledOnce())
    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('ready'))
    expect(send).not.toHaveBeenCalled()
  })

  it('retains the prompt when chat cannot atomically accept it', async () => {
    initialResults = { os_info: { success: true, output: '{}', duration_ms: 1 } }
    send.mockResolvedValue(false)
    renderWorkspace()
    fireEvent.click(screen.getByRole('button', { name: 'Queue summary' }))

    await waitFor(() => expect(send).toHaveBeenCalledOnce())
    expect(screen.getByTestId('pending')).toHaveTextContent('Summarize my latest scan')
  })

  it('runs an approved Full Scan and sends a scan-grounded continuation', async () => {
    initialResults = { os_info: { success: true, output: '{}', duration_ms: 1 } }
    chatState = {
      ...chatState,
      pendingFullScan: {
        sessionId: 'chat-1', messageId: 'answer-1', kind: 'full',
        sourceScanId: 'quick-session',
        reason: 'Event-log evidence is outside Quick Scan coverage.',
        question: 'Why did the PC crash?',
      },
    }
    renderWorkspace()
    fireEvent.click(screen.getByRole('button', { name: 'Accept full scan' }))

    await waitFor(() => expect(runFullScanTracked).toHaveBeenCalledOnce())
    await waitFor(() => expect(send).toHaveBeenCalledOnce())
    expect(runQuickScanTracked).not.toHaveBeenCalled()
    expect(send).toHaveBeenCalledWith(expect.objectContaining({
      displayText: 'Run the Full Scan and continue',
      query: 'Why did the PC crash?',
    }))
    expect(send.mock.calls[0]?.[0]).not.toHaveProperty('contextRefs')
  })

  it('automatically prepares a pending report with a Quick Scan', async () => {
    initialPendingReport = true
    renderWorkspace()

    await waitFor(() => expect(runQuickScanTracked).toHaveBeenCalledOnce())
    expect(send).not.toHaveBeenCalled()
    expect(runFullScanTracked).not.toHaveBeenCalled()
  })
})
