import React, { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import type { SettingsData } from './types'
import { Modal, Button } from './ui'

export type { SettingsData } from './types'

export interface SettingsDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  settings: SettingsData
  onSave: (settings: SettingsData) => void | Promise<void>
}

const SectionTitle: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div className="settings-section-title">{children}</div>
)

export const SettingsDialog: React.FC<SettingsDialogProps> = ({ open, onOpenChange, settings, onSave }) => {
  const [draft, setDraft] = useState<SettingsData>(settings)
  const [saving, setSaving] = useState(false)
  const [ollamaModels, setOllamaModels] = useState<string[]>([])

  useEffect(() => { if (open) setDraft(settings) }, [open, settings])

  // Populate the Ollama model dropdown when the dialog opens; a missing or
  // stopped Ollama simply leaves the free-text input in place.
  useEffect(() => {
    if (!open) return
    let cancelled = false
    invoke<string[]>('ai_list_ollama_models')
      .then(models => { if (!cancelled) setOllamaModels(models) })
      .catch(() => { if (!cancelled) setOllamaModels([]) })
    return () => { cancelled = true }
  }, [open])

  const set = <K extends keyof SettingsData>(k: K, v: SettingsData[K]) =>
    setDraft(d => ({ ...d, [k]: v }))

  const save = async () => {
    setSaving(true)
    try { await onSave(draft) } finally { setSaving(false) }
  }

  return (
    <Modal
      open={open}
      onClose={() => onOpenChange(false)}
      title="Settings"
      width={560}
      footer={
        <>
          <Button onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button variant="primary" onClick={save} loading={saving}>
            {saving ? 'Saving…' : 'Save'}
          </Button>
        </>
      }
    >
      <SectionTitle>AI</SectionTitle>
      <div className="form-row">
        <div><strong>Enable AI insights</strong></div>
        <input type="checkbox" checked={draft.aiEnabled ?? true} onChange={e => set('aiEnabled', e.target.checked)} />
      </div>
      <div className="form-row">
        <div><strong>Preferred AI provider</strong><div className="hint">Auto picks local providers first, then cloud</div></div>
        <select className="field-input" style={{ width: 220 }} value={draft.preferredAIProvider || 'auto'} onChange={e => set('preferredAIProvider', e.target.value as SettingsData['preferredAIProvider'])}>
          <option value="auto">Auto</option>
          <option value="phi_silica">Phi Silica (on-device)</option>
          <option value="foundry_local">Foundry Local (local server)</option>
          <option value="ollama">Ollama (local server)</option>
          <option value="custom_openai">Custom endpoint</option>
          <option value="openai">OpenAI (cloud)</option>
          <option value="anthropic">Anthropic Claude (cloud)</option>
          <option value="gemini">Google Gemini (cloud)</option>
        </select>
      </div>

      <SectionTitle>Cloud AI</SectionTitle>
      <div className="form-row">
        <div><strong>OpenAI API key</strong><div className="hint">Keys are stored in the OS secret store, never in the settings file</div></div>
        <input className="field-input" style={{ width: 260 }} type="password" value={draft.openAiApiKey || ''} placeholder="sk-…" onChange={e => set('openAiApiKey', e.target.value)} />
      </div>
      <div className="form-row">
        <div><strong>Anthropic API key</strong></div>
        <input className="field-input" style={{ width: 260 }} type="password" value={draft.anthropicApiKey || ''} placeholder="sk-ant-…" onChange={e => set('anthropicApiKey', e.target.value)} />
      </div>
      <div className="form-row">
        <div><strong>Anthropic model</strong><div className="hint">Optional. Also: claude-haiku-4-5, claude-opus-4-8</div></div>
        <input className="field-input" style={{ width: 260 }} type="text" value={draft.anthropicModel || ''} placeholder="claude-sonnet-4-6" onChange={e => set('anthropicModel', e.target.value)} />
      </div>
      <div className="form-row">
        <div><strong>Gemini API key</strong></div>
        <input className="field-input" style={{ width: 260 }} type="password" value={draft.geminiApiKey || ''} placeholder="AIza…" onChange={e => set('geminiApiKey', e.target.value)} />
      </div>
      <div className="form-row">
        <div><strong>Gemini model</strong><div className="hint">Optional</div></div>
        <input className="field-input" style={{ width: 260 }} type="text" value={draft.geminiModel || ''} placeholder="gemini-2.5-flash" onChange={e => set('geminiModel', e.target.value)} />
      </div>

      <SectionTitle>Local AI</SectionTitle>
      <div className="form-row">
        <div><strong>Foundry Local endpoint</strong><div className="hint">Optional. Leave empty to auto-discover Foundry Local</div></div>
        <input className="field-input" style={{ width: 260 }} type="text" value={draft.localAiEndpoint || ''} placeholder="http://127.0.0.1:55769" onChange={e => set('localAiEndpoint', e.target.value)} />
      </div>
      <div className="form-row">
        <div><strong>Ollama endpoint</strong><div className="hint">Optional. Leave empty for the default port</div></div>
        <input className="field-input" style={{ width: 260 }} type="text" value={draft.ollamaEndpoint || ''} placeholder="http://127.0.0.1:11434" onChange={e => set('ollamaEndpoint', e.target.value)} />
      </div>
      <div className="form-row">
        <div><strong>Ollama model</strong><div className="hint">Empty uses the first installed model</div></div>
        {ollamaModels.length > 0 ? (
          <select className="field-input" style={{ width: 260 }} value={draft.ollamaModel || ''} onChange={e => set('ollamaModel', e.target.value)}>
            <option value="">Auto (first installed)</option>
            {ollamaModels.map(model => <option key={model} value={model}>{model}</option>)}
          </select>
        ) : (
          <input className="field-input" style={{ width: 260 }} type="text" value={draft.ollamaModel || ''} placeholder="llama3.2" onChange={e => set('ollamaModel', e.target.value)} />
        )}
      </div>
      <div className="form-row">
        <div><strong>Phi Silica LAF token</strong><div className="hint">Optional. Microsoft-issued token unlocks the supported on-device path</div></div>
        <input className="field-input" style={{ width: 260 }} type="password" value={draft.phiSilicaLafToken || ''} placeholder="Leave empty for built-in" onChange={e => set('phiSilicaLafToken', e.target.value)} />
      </div>

      <SectionTitle>Custom OpenAI-compatible endpoint</SectionTitle>
      <div className="form-row">
        <div><strong>Endpoint URL</strong><div className="hint">OpenRouter, Groq, or any /v1/chat/completions server</div></div>
        <input className="field-input" style={{ width: 260 }} type="text" value={draft.customEndpoint || ''} placeholder="https://openrouter.ai/api" onChange={e => set('customEndpoint', e.target.value)} />
      </div>
      <div className="form-row">
        <div><strong>Model</strong><div className="hint">Required. The model id your provider documents</div></div>
        <input className="field-input" style={{ width: 260 }} type="text" value={draft.customModel || ''} placeholder="anthropic/claude-haiku-4-5" onChange={e => set('customModel', e.target.value)} />
      </div>
      <div className="form-row">
        <div><strong>API key</strong><div className="hint">Optional for local proxies</div></div>
        <input className="field-input" style={{ width: 260 }} type="password" value={draft.customApiKey || ''} placeholder="" onChange={e => set('customApiKey', e.target.value)} />
      </div>

      <SectionTitle>General</SectionTitle>
      <div className="form-row">
        <div><strong>Theme</strong></div>
        <select className="field-input" style={{ width: 180 }} value={draft.theme || 'dark'} onChange={e => set('theme', e.target.value as SettingsData['theme'])}>
          <option value="dark">Dark</option>
          <option value="light">Light</option>
          <option value="auto">Auto (system)</option>
        </select>
      </div>
      <div className="form-row">
        <div><strong>Export format</strong></div>
        <select className="field-input" style={{ width: 180 }} value={draft.exportFormat || 'text'} onChange={e => set('exportFormat', e.target.value as SettingsData['exportFormat'])}>
          <option value="text">Text</option>
          <option value="json">JSON</option>
          <option value="html">HTML</option>
        </select>
      </div>
      <div className="form-row">
        <div><strong>Auto-save scans</strong></div>
        <input type="checkbox" checked={draft.autoSave ?? true} onChange={e => set('autoSave', e.target.checked)} />
      </div>
      <div className="form-row">
        <div><strong>Desktop notifications</strong><div className="hint">Notify when a scan finishes in the background</div></div>
        <input type="checkbox" checked={draft.showNotifications ?? true} onChange={e => set('showNotifications', e.target.checked)} />
      </div>
      <div className="form-row">
        <div><strong>Scan on startup</strong></div>
        <input type="checkbox" checked={draft.scanOnStartup ?? false} onChange={e => set('scanOnStartup', e.target.checked)} />
      </div>
      <div className="form-row">
        <div><strong>Close to tray</strong><div className="hint">Closing the window keeps the app running in the system tray</div></div>
        <input type="checkbox" checked={draft.closeToTray ?? false} onChange={e => set('closeToTray', e.target.checked)} />
      </div>
      <div className="form-row">
        <div><strong>Max concurrent tasks</strong></div>
        <input className="field-input" style={{ width: 90 }} type="number" min={1} max={16} value={draft.maxConcurrentTasks ?? 5} onChange={e => set('maxConcurrentTasks', Number(e.target.value))} />
      </div>
    </Modal>
  )
}
