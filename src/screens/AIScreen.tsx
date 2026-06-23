import React, { useState, useRef, useEffect } from 'react'
import { useAIContext, type AIProvider, type ProviderInfo } from '../contexts/AIContext'
import { useAppContext } from '../contexts/AppContext'
import { useAIChat } from '../hooks/useAIChat'
import { ChatMessageBubble } from '../components/chat/ChatMessageBubble'
import { ScanReportPanel } from '../components/chat/ScanReportPanel'

export const PROVIDER_LABELS: Record<Exclude<AIProvider, 'none'>, string> = {
  phi_silica: 'Phi Silica (on-device NPU)',
  foundry_local: 'Foundry Local (local server)',
  ollama: 'Ollama (local server)',
  custom_openai: 'Custom endpoint',
  openai: 'OpenAI (cloud)',
  anthropic: 'Anthropic Claude (cloud)',
  gemini: 'Google Gemini (cloud)',
  deepseek: 'DeepSeek (cloud)',
}

const PROVIDER_TAGS: Record<AIProvider, string> = {
  none: 'no provider',
  phi_silica: 'on-device · Phi Silica',
  foundry_local: 'local · Foundry Local',
  ollama: 'local · Ollama',
  custom_openai: 'custom endpoint',
  openai: 'cloud · OpenAI',
  anthropic: 'cloud · Claude',
  gemini: 'cloud · Gemini',
  deepseek: 'cloud · DeepSeek',
}

/** Status line for one provider row in the Models panel */
function providerDetail(p: ProviderInfo, phiMessage?: string): string {
  if (p.available) {
    const parts: string[] = []
    if (p.model) parts.push(p.model)
    if (p.endpoint) parts.push(p.endpoint)
    return parts.length > 0 ? parts.join(' · ') : 'Ready'
  }
  switch (p.id) {
    case 'phi_silica':
      return phiMessage || 'Unavailable'
    case 'foundry_local':
      return 'Not detected — install with: winget install Microsoft.FoundryLocal'
    case 'ollama':
      return 'Not detected — install from ollama.com'
    case 'custom_openai':
      return p.configured ? 'Configured but unreachable' : 'Set endpoint and model in Settings'
    default:
      return 'No API key — add one in Settings'
  }
}

export const AIScreen: React.FC = () => {
  const { aiStatus, activeProvider } = useAIContext()
  const { setShowSettings, pendingChatPrompt, setPendingChatPrompt } = useAppContext()
  const { messages, send, stop, newConversation, isStreaming, aiEnabled } = useAIChat()
  const [input, setInput] = useState('')
  const endRef = useRef<HTMLDivElement>(null)

  useEffect(() => { endRef.current?.scrollIntoView({ behavior: 'smooth', block: 'end' }) }, [messages, isStreaming])

  // "Ask AI" deep-link from the Issues screen: consume once (clear BEFORE
  // sending so a re-render can never double-fire)
  useEffect(() => {
    if (pendingChatPrompt && aiEnabled && !isStreaming) {
      const prompt = pendingChatPrompt
      setPendingChatPrompt(null)
      void send(prompt)
    } else if (pendingChatPrompt && !aiEnabled) {
      setPendingChatPrompt(null)
    }
  }, [pendingChatPrompt, aiEnabled, isStreaming, send, setPendingChatPrompt])

  const submit = (text: string) => {
    if (!text.trim() || isStreaming || !aiEnabled) return
    setInput('')
    void send(text)
  }

  const suggestions = ['Summarize my latest scan', 'What failed and why?', 'Any security concerns?', 'How do I free up disk space?']
  const supportsTools = aiStatus?.providers?.find(p => p.id === activeProvider)?.supports_tools ?? false

  return (
    <div className="scrollable screen-pad ai-grid">
      <div className="wf-block ai-chat-col">
        <header className="wf-block-header">
          <img src="/wf-ds/chatgpt-bot-avatar.webp" alt="" style={{ width: 24, height: 24, borderRadius: '50%' }} />
          <span>WindowsForum AI Assistant</span>
          <span className="count" style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span className="tag"><span className="status-dot success" style={{ boxShadow: 'none' }} /> {PROVIDER_TAGS[activeProvider]}</span>
            {messages.length > 0 && (
              <button className="btn ghost" title="New conversation" disabled={!aiEnabled || isStreaming} onClick={() => { void newConversation() }}>
                <i className="fa-solid fa-plus" aria-hidden="true" /> New
              </button>
            )}
          </span>
        </header>
        <div className="chat-shell">
          <div className="chat-msgs" role="log" aria-live="polite" aria-label="AI conversation">
            {messages.length === 0 && (
              <div className="chat-msg bot">
                <div className="av"><img src="/wf-ds/chatgpt-bot-avatar.webp" alt="" /></div>
                <div className="msg-col">
                  <span className="msg-sender">WF Assistant</span>
                  <div className="bubble">
                    {supportsTools
                      ? 'Ask me about this PC. I can run diagnostics myself — disk, memory, network, drivers, logs — and answer from the real data.'
                      : 'Ask me about your diagnostics. I read the current scan results and explain failures, risks and fixes.'}
                  </div>
                </div>
              </div>
            )}
            {messages.map(m => <ChatMessageBubble key={m.id} message={m} />)}
            <div ref={endRef} />
          </div>
          <div className="chat-suggestions">
            {suggestions.map(s => <button key={s} className="suggest" disabled={isStreaming || !aiEnabled} onClick={() => submit(s)}>{s}</button>)}
          </div>
          <form className="chat-input" onSubmit={e => { e.preventDefault(); submit(input) }}>
            <input type="text" placeholder="Ask about any diagnostic, error or trend…" value={input} onChange={e => setInput(e.target.value)} disabled={!aiEnabled} />
            {isStreaming ? (
              <button type="button" className="btn" onClick={() => { void stop() }}>
                <i className="fa-solid fa-stop" aria-hidden="true" /> Stop
              </button>
            ) : (
              <button type="submit" className="btn primary" disabled={!aiEnabled}><i className="fa-solid fa-paper-plane" /> Send</button>
            )}
          </form>
        </div>
      </div>

      <div className="ai-side-col">
        <div className="wf-block">
          <header className="wf-block-header"><span className="accent-bar" /><span>Models</span></header>
          <div style={{ padding: 12, fontSize: 13 }}>
            {(aiStatus?.providers ?? []).map((p, i, arr) => (
              <div key={p.id} style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '8px 0', borderBottom: i < arr.length - 1 ? '1px solid var(--hairline)' : 'none' }}>
                <span className="status-dot" style={{ background: p.available ? 'var(--wf-success-feature)' : 'var(--wf-text-muted)', boxShadow: 'none' }} />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontWeight: 600 }}>{PROVIDER_LABELS[p.id]}</div>
                  <div style={{ fontSize: 11, color: 'var(--wf-text-muted)', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                    {providerDetail(p, aiStatus?.phi_silica_message)}
                  </div>
                </div>
                {activeProvider === p.id && <span className="tag">Active</span>}
              </div>
            ))}
            {!aiStatus?.providers?.length && (
              <div style={{ color: 'var(--wf-text-muted)', fontSize: 12, padding: '4px 0' }}>Checking providers…</div>
            )}
            <button className="btn ghost" style={{ marginTop: 8 }} onClick={() => setShowSettings(true)}><i className="fa-solid fa-gear" /> AI settings</button>
          </div>
        </div>

        <ScanReportPanel />

        <div className="wf-block">
          <header className="wf-block-header"><span className="accent-bar" /><span>Privacy</span></header>
          <div style={{ padding: 12, fontSize: 12, lineHeight: 1.55, color: 'var(--wf-text-muted)' }}>
            On-device analysis runs through <strong style={{ color: 'var(--wf-text)' }}>Windows AI Foundation</strong> (Phi Silica). No diagnostics leave this PC unless you send them to a configured cloud model.
          </div>
        </div>
      </div>
    </div>
  )
}
