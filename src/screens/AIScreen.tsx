import React, { useState, useRef, useEffect } from 'react'
import { useAIContext } from '../contexts/AIContext'
import { useAppContext } from '../contexts/AppContext'
import { renderMarkdownLite } from '../utils/markdownLite'

interface Msg { role: 'user' | 'bot'; text: string }

export const AIScreen: React.FC = () => {
  const { analyzeGeneric, aiStatus, isAIAvailable, activeProvider, sessionId, interpretations } = useAIContext()
  const { results, setShowSettings } = useAppContext()
  const [transcript, setTranscript] = useState<Msg[]>([])
  const [input, setInput] = useState('')
  const [thinking, setThinking] = useState(false)
  const endRef = useRef<HTMLDivElement>(null)

  useEffect(() => { endRef.current?.scrollIntoView({ behavior: 'smooth', block: 'end' }) }, [transcript, thinking])

  const send = async (text: string) => {
    if (!text.trim() || thinking) return
    setTranscript(t => [...t, { role: 'user', text }])
    setInput('')
    if (!isAIAvailable) {
      setTranscript(t => [...t, { role: 'bot', text: 'No AI provider is available. Configure an OpenAI API key in Settings, install Foundry Local (winget install Microsoft.FoundryLocal), or run on a Copilot+ PC for on-device Phi Silica.' }])
      return
    }
    setThinking(true)
    try {
      const resultSummary = Object.entries(results)
        .map(([id, r]) => `${id}: ${r.success ? 'OK' : 'FAIL'}${r.error ? ` (${r.error})` : ''}`)
        .join('\n')
      const reply = await analyzeGeneric(
        `chat:${Date.now()}`,
        `User question: ${text}\n\nCurrent diagnostic results:\n${resultSummary || '(no scan run yet)'}`
      )
      setTranscript(t => [...t, { role: 'bot', text: reply || '(no response)' }])
    } catch (e) {
      setTranscript(t => [...t, { role: 'bot', text: `Error: ${String(e)}` }])
    } finally {
      setThinking(false)
    }
  }

  const suggestions = ['Summarize my latest scan', 'What failed and why?', 'Any security concerns?', 'How do I free up disk space?']

  return (
    <div className="scrollable" style={{ padding: '0 24px 24px', display: 'flex', gap: 14, minHeight: 0 }}>
      <div className="wf-block" style={{ flex: 2, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
        <header className="wf-block-header">
          <img src="/wf-ds/chatgpt-bot-avatar.webp" alt="" style={{ width: 24, height: 24, borderRadius: '50%' }} />
          <span>WindowsForum AI Assistant</span>
          <span className="count">
            <span className="tag"><span className="status-dot success" style={{ boxShadow: 'none' }} /> {activeProvider === 'phi_silica' ? 'on-device · Phi Silica' : activeProvider === 'foundry_local' ? 'local · Foundry Local' : activeProvider === 'openai' ? 'cloud · OpenAI' : 'no provider'}</span>
          </span>
        </header>
        <div className="chat-shell" style={{ minHeight: 360 }}>
          <div className="chat-msgs" role="log" aria-live="polite" aria-label="AI conversation">
            {transcript.length === 0 && (
              <div className="chat-msg bot">
                <div className="av"><img src="/wf-ds/chatgpt-bot-avatar.webp" alt="" /></div>
                <div className="msg-col">
                  <span className="msg-sender">WF Assistant</span>
                  <div className="bubble">Ask me about your diagnostics. I read the current scan results and explain failures, risks and fixes.</div>
                </div>
              </div>
            )}
            {transcript.map((m, i) => (
              <div key={i} className={`chat-msg ${m.role}`}>
                <div className="av">{m.role === 'user' ? 'ME' : <img src="/wf-ds/chatgpt-bot-avatar.webp" alt="" />}</div>
                <div className="msg-col">
                  <span className="msg-sender">{m.role === 'user' ? 'You' : 'WF Assistant'}</span>
                  <div className="bubble">
                    {m.role === 'bot'
                      ? renderMarkdownLite(m.text)
                      : <div style={{ whiteSpace: 'pre-wrap' }}>{m.text}</div>}
                  </div>
                </div>
              </div>
            ))}
            {thinking && (
              <div className="chat-msg bot" aria-busy="true">
                <div className="av"><img src="/wf-ds/chatgpt-bot-avatar.webp" alt="" /></div>
                <div className="msg-col">
                  <span className="msg-sender">WF Assistant</span>
                  <div className="bubble"><i className="fa-solid fa-circle-notch fa-spin" aria-hidden="true" /> Analyzing latest scan…</div>
                </div>
              </div>
            )}
            <div ref={endRef} />
          </div>
          <div className="chat-suggestions">
            {suggestions.map(s => <button key={s} className="suggest" onClick={() => send(s)}>{s}</button>)}
          </div>
          <form className="chat-input" onSubmit={e => { e.preventDefault(); send(input) }}>
            <input type="text" placeholder="Ask about any diagnostic, error or trend…" value={input} onChange={e => setInput(e.target.value)} />
            <button type="submit" className="btn primary"><i className="fa-solid fa-paper-plane" /> Send</button>
          </form>
        </div>
      </div>

      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 12, minWidth: 0 }}>
        <div className="wf-block">
          <header className="wf-block-header"><span className="accent-bar" /><span>Models</span></header>
          <div style={{ padding: 12, fontSize: 13 }}>
            {([
              ['Phi Silica (on-device NPU)', aiStatus?.phi_silica_available ? (aiStatus.phi_silica_ready ? 'Ready' : (aiStatus.phi_silica_message || 'Available')) : 'Unavailable', 'var(--wf-success-feature)', activeProvider === 'phi_silica'],
              ['Foundry Local (local server)', aiStatus?.foundry_local_available ? `Running at ${aiStatus.foundry_local_endpoint || 'local endpoint'}` : 'Not detected', '#c98a00', activeProvider === 'foundry_local'],
              ['OpenAI (cloud)', aiStatus?.openai_api_key_set ? 'API key configured' : 'Not configured', 'var(--wf-paletteColor1)', activeProvider === 'openai'],
            ] as const).map((m, i, arr) => (
              <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '8px 0', borderBottom: i < arr.length - 1 ? '1px solid var(--hairline)' : 'none' }}>
                <span className="status-dot" style={{ background: m[2], boxShadow: 'none' }} />
                <div style={{ flex: 1 }}>
                  <div style={{ fontWeight: 600 }}>{m[0]}</div>
                  <div style={{ fontSize: 11, color: 'var(--wf-text-muted)' }}>{m[1]}</div>
                </div>
                {m[3] && <span className="tag">Active</span>}
              </div>
            ))}
            <button className="btn ghost" style={{ marginTop: 8 }} onClick={() => setShowSettings(true)}><i className="fa-solid fa-gear" /> AI settings</button>
          </div>
        </div>

        <div className="wf-block">
          <header className="wf-block-header"><span className="accent-bar" /><span>Session Context</span></header>
          <div style={{ padding: 12, fontSize: 12, color: 'var(--wf-text-muted)' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', padding: '4px 0' }}><span>Session ID</span><span className="wf-mono">{sessionId}</span></div>
            <div style={{ display: 'flex', justifyContent: 'space-between', padding: '4px 0' }}><span>Diagnostics in context</span><span>{Object.keys(results).length}</span></div>
            <div style={{ display: 'flex', justifyContent: 'space-between', padding: '4px 0' }}><span>Cached interpretations</span><span>{Object.keys(interpretations).length}</span></div>
          </div>
        </div>

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
