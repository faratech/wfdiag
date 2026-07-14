import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useAIContext, type AIProvider } from '../contexts/AIContext'
import { useAIWorkspace } from '../contexts/AIWorkspaceContext'
import { useAppContext } from '../contexts/AppContext'
import type { AIExecutionClass, AIProviderUse, ChatPrompt } from '../components/types'
import { ChatMessageBubble } from '../components/chat/ChatMessageBubble'
import { ScanReportPanelView } from '../components/chat/ScanReportPanel'

export const PROVIDER_LABELS: Record<Exclude<AIProvider, 'none'>, string> = {
  phi_silica: 'Phi Silica',
  foundry_local: 'Foundry Local',
  ollama: 'Ollama',
  custom_openai: 'Custom endpoint',
  codex_cli: 'ChatGPT via Codex',
  claude_code: 'Claude Code',
  openai: 'OpenAI',
  anthropic: 'Anthropic Claude',
  gemini: 'Google Gemini',
  deepseek: 'DeepSeek',
}

const EXECUTION_LABELS: Record<AIExecutionClass, string> = {
  on_device: 'On device',
  local_server: 'Local server',
  subscription_cloud: 'Subscription cloud',
  api_cloud: 'API cloud',
}

function providerUseFor(provider: AIProvider): AIProviderUse | null {
  if (provider === 'none') return null
  return {
    providerId: provider,
    executionClass: provider === 'phi_silica'
      ? 'on_device'
      : provider === 'foundry_local' || provider === 'ollama'
        ? 'local_server'
        : provider === 'codex_cli' || provider === 'claude_code'
          ? 'subscription_cloud'
          : 'api_cloud',
  }
}

function pendingPromptLabel(prompt: ChatPrompt | string): string {
  return typeof prompt === 'string' ? prompt : (prompt.displayText || prompt.query)
}

export const AIScreen: React.FC = () => {
  const { aiStatus, activeProvider, isAIAvailable, isLoading } = useAIContext()
  const {
    setShowSettings,
    pendingChatPrompt,
    pendingScanReport,
    setPendingScanReport,
    aiMode: mode,
    setAIMode: setMode,
    isRunning: scanRunning,
    currentProgress,
    currentTaskName,
  } = useAppContext()
  const {
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
  } = useAIWorkspace()
  const {
    messages,
    stop,
    resolveFallback,
    newConversation,
    isStreaming,
    isStopping,
    pendingFallback,
    pendingFullScan,
    lastProviderUse,
    aiEnabled,
  } = chat
  const [input, setInput] = useState('')
  const transcriptRef = useRef<HTMLDivElement>(null)
  const nearBottomRef = useRef(true)
  const [showJump, setShowJump] = useState(false)

  const updateScrollPosition = useCallback(() => {
    const transcript = transcriptRef.current
    if (!transcript) return
    const distance = transcript.scrollHeight - transcript.scrollTop - transcript.clientHeight
    nearBottomRef.current = distance < 72
    setShowJump(!nearBottomRef.current)
  }, [])

  const jumpToLatest = useCallback(() => {
    const transcript = transcriptRef.current
    if (!transcript) return
    transcript.scrollTop = transcript.scrollHeight
    nearBottomRef.current = true
    setShowJump(false)
  }, [])

  // Follow new content only while the user is already near the end. Reading
  // older messages is never interrupted by streaming deltas.
  useEffect(() => {
    if (!nearBottomRef.current) {
      setShowJump(true)
      return
    }
    const frame = requestAnimationFrame(jumpToLatest)
    return () => cancelAnimationFrame(frame)
  }, [messages, pendingFallback, pendingChatPrompt, jumpToLatest])

  // Diagnostics deep-link: open the dedicated report mode and generate once.
  useEffect(() => {
    if (!pendingScanReport) return
    // Provider discovery is asynchronous. Keep the intent pending until a
    // provider is actually available instead of consuming the click while the
    // status request still reports `none`.
    if (isLoading || !scanReport.hasResults || !scanReport.aiEnabled || !isAIAvailable) return
    setPendingScanReport(false)
    if (!scanReport.generating) void scanReport.generate()
  }, [pendingScanReport, setPendingScanReport, scanReport, isAIAvailable, isLoading])

  const submit = (text: string) => {
    if (!text.trim() || pendingChatPrompt || pendingFullScan || isLoading || isStreaming || pendingFallback || !aiEnabled || !isAIAvailable) return
    setInput('')
    nearBottomRef.current = true
    queuePrompt(text)
  }

  const operationRunning = mode === 'report'
    ? scanReport.generating || scanReport.cancelling
    : isStreaming || isStopping
  const operationProviderUse = mode === 'report' ? scanReport.lastProviderUse : lastProviderUse
  // Completed messages/reports retain their own provider attribution. The
  // workspace badge describes what will answer next, except while an active
  // operation has already acknowledged the provider it is actually using.
  const providerUse = (operationRunning ? operationProviderUse : null) || providerUseFor(activeProvider)
  const providerLabel = isLoading && !operationRunning
    ? 'Checking AI provider'
    : providerUse ? PROVIDER_LABELS[providerUse.providerId] : 'No provider'
  const executionLabel = isLoading && !operationRunning
    ? 'Please wait'
    : providerUse ? EXECUTION_LABELS[providerUse.executionClass] : 'Not connected'
  const cloudExecution = providerUse?.executionClass === 'api_cloud' || providerUse?.executionClass === 'subscription_cloud'
  const supportsTools = aiStatus?.providers?.find(provider => provider.id === activeProvider)?.supports_tools ?? false
  const suggestions = useMemo(() => [
    'Summarize my latest scan',
    'What failed and why?',
    'Any security concerns?',
    'How do I free up disk space?',
  ], [])

  const unavailable = !aiEnabled || (!isLoading && !isAIAvailable)
  const waitingPrompt = pendingChatPrompt ? pendingPromptLabel(pendingChatPrompt) : null
  const scanGate = typeof pendingChatPrompt === 'string' ? undefined : pendingChatPrompt?.scanGate
  const preparingPrompt = !!scanGate && scanGate.status !== 'ready'
  const composerBlocked = !!pendingChatPrompt || !!pendingFullScan

  return (
    <div className="ai-workspace">
      <div className="ai-modebar">
        <div className="ai-mode-tabs" role="tablist" aria-label="AI analysis mode">
          <button
            id="ai-mode-assistant"
            type="button"
            role="tab"
            aria-selected={mode === 'assistant'}
            aria-controls="ai-assistant-panel"
            className={mode === 'assistant' ? 'active' : ''}
            onClick={() => setMode('assistant')}
          >
            <i className="fa-solid fa-comments" aria-hidden="true" /> Assistant
          </button>
          <button
            id="ai-mode-report"
            type="button"
            role="tab"
            aria-selected={mode === 'report'}
            aria-controls="ai-report-panel"
            className={mode === 'report' ? 'active' : ''}
            onClick={() => setMode('report')}
          >
            <i className="fa-solid fa-file-lines" aria-hidden="true" /> Scan Report
          </button>
        </div>
        <div className="ai-runtime-summary" role="status" aria-label={`${providerLabel}, ${executionLabel}`}>
          <span className={`privacy-dot ${cloudExecution ? 'cloud' : providerUse ? 'private' : 'off'}`} aria-hidden="true" />
          <span className="ai-runtime-provider">{providerLabel}</span>
          <span className="ai-runtime-class">{executionLabel}</span>
          <button type="button" className="btn-icon" onClick={() => setShowSettings(true)} aria-label="Open AI settings" title="AI settings">
            <i className="fa-solid fa-gear" aria-hidden="true" />
          </button>
        </div>
      </div>

      <section
        id="ai-assistant-panel"
        role="tabpanel"
        aria-labelledby="ai-mode-assistant"
        className="ai-mode-panel"
        hidden={mode !== 'assistant'}
      >
        <div className="wf-block ai-chat-card">
          <header className="wf-block-header ai-chat-header">
            <img src="/wf-ds/chatgpt-bot-avatar.webp" alt="" />
            <div>
              <span>WindowsForum Assistant</span>
              <small>{supportsTools ? 'Can inspect this PC with read-only tools' : 'Explains the current diagnostic results'}</small>
            </div>
            {messages.length > 0 && (
              <button
                className="btn ghost count"
                type="button"
                disabled={isStreaming || !!pendingFallback || !!pendingFullScan || !!pendingChatPrompt}
                onClick={() => { void newConversation() }}
              >
                <i className="fa-solid fa-plus" aria-hidden="true" /> New conversation
              </button>
            )}
          </header>

          <div className="chat-shell">
            <div className="chat-transcript-wrap">
              <div
                ref={transcriptRef}
                className="chat-msgs"
                role="log"
                aria-live="polite"
                aria-relevant="additions text"
                aria-label="AI conversation"
                aria-busy={isStreaming}
                onScroll={updateScrollPosition}
              >
                {messages.length === 0 && !unavailable && !isLoading && (
                  <div className="chat-welcome">
                    <img src="/wf-ds/chatgpt-bot-avatar.webp" alt="" />
                    <h2>What would you like to understand?</h2>
                    <p>
                      {supportsTools
                        ? 'Ask about this PC. The assistant can run read-only checks and answer from the results.'
                        : 'Ask about the latest diagnostics, failures, risks, or next steps.'}
                    </p>
                  </div>
                )}

                {isLoading && messages.length === 0 && (
                  <div className="ai-empty-state" role="status">
                    <i className="fa-solid fa-circle-notch fa-spin" aria-hidden="true" />
                    <h2>Checking AI availability…</h2>
                  </div>
                )}

                {!aiEnabled && messages.length === 0 && (
                  <div className="ai-empty-state">
                    <i className="fa-solid fa-power-off" aria-hidden="true" />
                    <h2>AI insights are turned off</h2>
                    <p>Enable them in Settings to use the assistant or create scan reports.</p>
                    <button className="btn primary" type="button" onClick={() => setShowSettings(true)}>Open Settings</button>
                  </div>
                )}

                {aiEnabled && !isLoading && !isAIAvailable && messages.length === 0 && (
                  <div className="ai-empty-state">
                    <i className="fa-solid fa-plug-circle-xmark" aria-hidden="true" />
                    <h2>Connect an AI provider</h2>
                    <p>Choose a local, subscription, or API provider in Settings. Diagnostics remain on this PC until a cloud provider is used.</p>
                    {waitingPrompt && <p className="pending-handoff">Waiting to ask: “{waitingPrompt}”</p>}
                    <button className="btn primary" type="button" onClick={() => setShowSettings(true)}>Configure AI</button>
                  </div>
                )}

                {messages.map(message => <ChatMessageBubble key={message.id} message={message} />)}

                {scanGate && (
                  <div
                    className="fallback-card scan-preflight-card"
                    role={scanGate.status === 'failed' ? 'alert' : 'status'}
                    aria-live="polite"
                  >
                    <i
                      className={scanGate.status === 'failed'
                        ? 'fa-solid fa-triangle-exclamation'
                        : 'fa-solid fa-stethoscope'}
                      aria-hidden="true"
                    />
                    <div>
                      <strong>
                        {scanGate.status === 'failed'
                          ? `${scanGate.kind === 'full' ? 'Full' : 'Quick'} Scan did not finish`
                          : scanGate.status === 'waiting'
                            ? 'Waiting for the current scan'
                            : `Running ${scanGate.kind === 'full' ? 'Full' : 'Quick'} Scan before asking`}
                      </strong>
                      <p>“{waitingPrompt}”</p>
                      {scanGate.reason && <small>{scanGate.reason}</small>}
                      {scanGate.status === 'failed' ? (
                        <p>{scanGate.error}</p>
                      ) : scanGate.status === 'running' && scanRunning ? (
                        <small>{Math.round(currentProgress)}% · {currentTaskName || 'Collecting diagnostics…'}</small>
                      ) : (
                        <small>The question will be sent automatically when usable results are ready.</small>
                      )}
                      <div className="fallback-actions">
                        {scanGate.status === 'failed' && (
                          <button className="btn primary" type="button" onClick={retryPendingScan}>Retry scan</button>
                        )}
                        <button
                          className="btn"
                          type="button"
                          onClick={() => {
                            if (waitingPrompt) setInput(waitingPrompt)
                            cancelPendingPrompt()
                          }}
                        >
                          {scanGate.status === 'running' ? 'Stop scan' : 'Cancel question'}
                        </button>
                      </div>
                    </div>
                  </div>
                )}

                {pendingFullScan && (
                  <div className="fallback-card full-scan-request" role="alert">
                    <i className="fa-solid fa-magnifying-glass-plus" aria-hidden="true" />
                    <div>
                      <strong>Run a Full Scan for more evidence?</strong>
                      <p>{pendingFullScan.reason}</p>
                      <small>The current Quick Scan stays available if the Full Scan is stopped or fails.</small>
                      <div className="fallback-actions">
                        <button className="btn primary" type="button" disabled={!!pendingChatPrompt || isStreaming || scanRunning} onClick={acceptFullScanRequest}>Run Full Scan</button>
                        <button className="btn" type="button" onClick={dismissFullScanRequest}>Not now</button>
                      </div>
                    </div>
                  </div>
                )}

                {pendingFallback && (
                  <div className="fallback-card" role="alert" aria-labelledby="fallback-title">
                    <i className="fa-solid fa-cloud-arrow-up" aria-hidden="true" />
                    <div>
                      <strong id="fallback-title">Continue with {PROVIDER_LABELS[pendingFallback.to.providerId]}?</strong>
                      <p>The private provider could not finish. Continuing sends this question and its selected diagnostic context to an {EXECUTION_LABELS[pendingFallback.to.executionClass].toLowerCase()} provider. This choice is remembered and can be changed in Settings.</p>
                      {pendingFallback.reason && <small>{pendingFallback.reason}</small>}
                      <div className="fallback-actions">
                        <button className="btn primary" type="button" onClick={() => { void resolveFallback('allow') }}>Allow cloud fallback</button>
                        <button className="btn" type="button" onClick={() => { void resolveFallback('never') }}>Keep data local</button>
                      </div>
                    </div>
                  </div>
                )}
              </div>

              {showJump && (
                <button className="jump-latest" type="button" onClick={jumpToLatest}>
                  <i className="fa-solid fa-arrow-down" aria-hidden="true" /> Jump to latest
                </button>
              )}
            </div>

            {messages.length === 0 && !unavailable && !isLoading && (
              <div className="chat-suggestions" aria-label="Suggested questions">
                {suggestions.map(suggestion => (
                  <button key={suggestion} className="suggest" type="button" disabled={isLoading || isStreaming || composerBlocked} onClick={() => submit(suggestion)}>
                    {suggestion}
                  </button>
                ))}
              </div>
            )}

            {messages.length > 0 && unavailable && (
              <div className="chat-unavailable" role="status">
                <i className="fa-solid fa-circle-info" aria-hidden="true" />
                <span>{aiEnabled ? 'No AI provider is currently available.' : 'AI insights are turned off.'}</span>
                <button type="button" className="btn ghost" onClick={() => setShowSettings(true)}>Open Settings</button>
              </div>
            )}

            <form className="chat-input" onSubmit={event => { event.preventDefault(); submit(input) }}>
              <label className="sr-only" htmlFor="ai-chat-input">Message the AI assistant</label>
              <textarea
                id="ai-chat-input"
                rows={1}
                placeholder={preparingPrompt ? 'Preparing scan evidence…' : isLoading ? 'Checking AI provider…' : unavailable ? 'Configure an AI provider to start…' : 'Ask about a diagnostic, error, or trend…'}
                value={input}
                onChange={event => setInput(event.target.value)}
                onKeyDown={event => {
                  if (event.key === 'Enter' && !event.shiftKey) {
                    event.preventDefault()
                    submit(input)
                  }
                }}
                disabled={isLoading || unavailable || !!pendingFallback || composerBlocked}
              />
              {isStreaming ? (
                <button type="button" className="btn" disabled={isStopping} onClick={() => { void stop() }}>
                  <i className={isStopping ? 'fa-solid fa-circle-notch fa-spin' : 'fa-solid fa-stop'} aria-hidden="true" /> {isStopping ? 'Stopping…' : 'Stop'}
                </button>
              ) : (
                <button type="submit" className="btn primary" disabled={isLoading || unavailable || !!pendingFallback || composerBlocked || !input.trim()}>
                  <i className="fa-solid fa-paper-plane" aria-hidden="true" /> Send
                </button>
              )}
            </form>
          </div>
        </div>
      </section>

      <section
        id="ai-report-panel"
        role="tabpanel"
        aria-labelledby="ai-mode-report"
        className="ai-mode-panel"
        hidden={mode !== 'report'}
      >
        <ScanReportPanelView
          state={scanReport}
          available={isAIAvailable}
          loading={isLoading}
          onConfigure={() => setShowSettings(true)}
          onPrepare={() => setPendingScanReport(true)}
          preparation={reportPreparation}
          onRetryPrepare={retryReportPreparation}
          onCancelPrepare={cancelReportPreparation}
        />
      </section>
    </div>
  )
}
