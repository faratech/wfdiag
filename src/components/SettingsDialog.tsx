import React, { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import type { AIProviderId, SettingsData } from './types'
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

const PROVIDER_OPTIONS: { id: AIProviderId; label: string }[] = [
  { id: 'phi_silica', label: 'Phi Silica (on-device)' },
  { id: 'foundry_local', label: 'Foundry Local (local server)' },
  { id: 'ollama', label: 'Ollama (local server)' },
  { id: 'custom_openai', label: 'Custom endpoint' },
  { id: 'openai', label: 'OpenAI (cloud)' },
  { id: 'anthropic', label: 'Anthropic Claude (cloud)' },
  { id: 'gemini', label: 'Google Gemini (cloud)' },
  { id: 'deepseek', label: 'DeepSeek (cloud)' },
]

/** Model picker: curated options + "default" + whatever custom value is set */
const ModelSelect: React.FC<{
  value: string | undefined
  defaultModel: string
  options: string[]
  ariaLabel: string
  onChange: (value: string) => void
}> = ({ value, defaultModel, options, ariaLabel, onChange }) => {
  const items = value && value.trim() !== '' && !options.includes(value)
    ? [value, ...options]
    : options
  return (
    <select className="field-input" aria-label={ariaLabel} value={value || ''} onChange={e => onChange(e.target.value)}>
      <option value="">Default ({defaultModel})</option>
      {items.map(model => <option key={model} value={model}>{model}</option>)}
    </select>
  )
}

export const SettingsDialog: React.FC<SettingsDialogProps> = (props) =>
  props.open ? <SettingsDialogInner {...props} /> : null

// Mounted only while open (SettingsDialog gates on it), so the draft and the
// configured-provider selection seed straight from the live settings on mount
// — no in-render reseeding of a long-lived dialog.
const SettingsDialogInner: React.FC<SettingsDialogProps> = ({ open, onOpenChange, settings, onSave }) => {
  const [draft, setDraft] = useState<SettingsData>(settings)
  const [saving, setSaving] = useState(false)
  const [ollamaModels, setOllamaModels] = useState<string[]>([])
  // Which provider's fields are shown in "Provider setup" — presentation only,
  // not persisted. Seeds from the active provider when it's concrete.
  const [configProvider, setConfigProvider] = useState<AIProviderId>(() =>
    settings.preferredAIProvider && settings.preferredAIProvider !== 'auto'
      ? settings.preferredAIProvider
      : 'openai'
  )

  // Populate the Ollama model dropdown on open; a missing or stopped Ollama
  // simply leaves the free-text input in place.
  useEffect(() => {
    let cancelled = false
    invoke<string[]>('ai_list_ollama_models')
      .then(models => { if (!cancelled) setOllamaModels(models) })
      .catch(() => { if (!cancelled) setOllamaModels([]) })
    return () => { cancelled = true }
  }, [])

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
        <div><strong>Active AI</strong><div className="hint">One provider answers at a time; Auto picks local first, then cloud</div></div>
        <select
          className="field-input"
          aria-label="Active AI provider"
          value={draft.preferredAIProvider || 'auto'}
          onChange={e => {
            const value = e.target.value as SettingsData['preferredAIProvider']
            set('preferredAIProvider', value)
            // Editing follows the chosen provider so its fields appear below
            if (value && value !== 'auto') setConfigProvider(value)
          }}
        >
          <option value="auto">Auto</option>
          {PROVIDER_OPTIONS.map(p => <option key={p.id} value={p.id}>{p.label}</option>)}
        </select>
      </div>

      <SectionTitle>Provider setup</SectionTitle>
      <div className="form-row">
        <div><strong>Configure</strong><div className="hint">Keys are stored in the OS secret store, never in the settings file</div></div>
        <select
          className="field-input"
          aria-label="Provider to configure"
          value={configProvider}
          onChange={e => setConfigProvider(e.target.value as AIProviderId)}
        >
          {PROVIDER_OPTIONS.map(p => <option key={p.id} value={p.id}>{p.label}</option>)}
        </select>
      </div>

      {configProvider === 'openai' && (
        <div className="form-row">
          <div><strong>OpenAI API key</strong></div>
          <input className="field-input" type="password" value={draft.openAiApiKey || ''} placeholder="sk-…" onChange={e => set('openAiApiKey', e.target.value)} />
        </div>
      )}

      {configProvider === 'anthropic' && (
        <>
          <div className="form-row">
            <div><strong>Anthropic API key</strong></div>
            <input className="field-input" type="password" value={draft.anthropicApiKey || ''} placeholder="sk-ant-…" onChange={e => set('anthropicApiKey', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong></div>
            <ModelSelect
              ariaLabel="Anthropic model"
              value={draft.anthropicModel}
              defaultModel="claude-sonnet-4-6"
              options={['claude-sonnet-4-6', 'claude-opus-4-8', 'claude-haiku-4-5']}
              onChange={v => set('anthropicModel', v)}
            />
          </div>
        </>
      )}

      {configProvider === 'gemini' && (
        <>
          <div className="form-row">
            <div><strong>Gemini API key</strong></div>
            <input className="field-input" type="password" value={draft.geminiApiKey || ''} placeholder="AIza…" onChange={e => set('geminiApiKey', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong></div>
            <ModelSelect
              ariaLabel="Gemini model"
              value={draft.geminiModel}
              defaultModel="gemini-2.5-flash"
              options={['gemini-2.5-flash', 'gemini-2.5-pro', 'gemini-2.5-flash-lite']}
              onChange={v => set('geminiModel', v)}
            />
          </div>
        </>
      )}

      {configProvider === 'deepseek' && (
        <>
          <div className="form-row">
            <div><strong>DeepSeek API key</strong></div>
            <input className="field-input" type="password" value={draft.deepseekApiKey || ''} placeholder="sk-…" onChange={e => set('deepseekApiKey', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong></div>
            <ModelSelect
              ariaLabel="DeepSeek model"
              value={draft.deepseekModel}
              defaultModel="deepseek-chat"
              options={['deepseek-chat', 'deepseek-reasoner']}
              onChange={v => set('deepseekModel', v)}
            />
          </div>
        </>
      )}

      {configProvider === 'foundry_local' && (
        <div className="form-row">
          <div><strong>Endpoint</strong><div className="hint">Optional. Leave empty to auto-discover Foundry Local</div></div>
          <input className="field-input" type="text" value={draft.localAiEndpoint || ''} placeholder="http://127.0.0.1:55769" onChange={e => set('localAiEndpoint', e.target.value)} />
        </div>
      )}

      {configProvider === 'ollama' && (
        <>
          <div className="form-row">
            <div><strong>Endpoint</strong><div className="hint">Optional. Leave empty for the default port</div></div>
            <input className="field-input" type="text" value={draft.ollamaEndpoint || ''} placeholder="http://127.0.0.1:11434" onChange={e => set('ollamaEndpoint', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong><div className="hint">Empty uses the first installed model</div></div>
            {ollamaModels.length > 0 ? (
              <select className="field-input" aria-label="Ollama model" value={draft.ollamaModel || ''} onChange={e => set('ollamaModel', e.target.value)}>
                <option value="">Auto (first installed)</option>
                {ollamaModels.map(model => <option key={model} value={model}>{model}</option>)}
              </select>
            ) : (
              <input className="field-input" type="text" value={draft.ollamaModel || ''} placeholder="llama3.2" onChange={e => set('ollamaModel', e.target.value)} />
            )}
          </div>
        </>
      )}

      {configProvider === 'phi_silica' && (
        <div className="form-row">
          <div><strong>LAF token</strong><div className="hint">Optional. Microsoft-issued token; requires the Store version on a Copilot+ PC</div></div>
          <input className="field-input" type="password" value={draft.phiSilicaLafToken || ''} placeholder="Leave empty for built-in" onChange={e => set('phiSilicaLafToken', e.target.value)} />
        </div>
      )}

      {configProvider === 'custom_openai' && (
        <>
          <div className="form-row">
            <div><strong>Endpoint URL</strong><div className="hint">OpenRouter, Groq, or any /v1/chat/completions server</div></div>
            <input className="field-input" type="text" value={draft.customEndpoint || ''} placeholder="https://openrouter.ai/api" onChange={e => set('customEndpoint', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong><div className="hint">Required. The model id your provider documents</div></div>
            <input className="field-input" type="text" value={draft.customModel || ''} placeholder="anthropic/claude-haiku-4-5" onChange={e => set('customModel', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>API key</strong><div className="hint">Optional for local proxies</div></div>
            <input className="field-input" type="password" value={draft.customApiKey || ''} placeholder="" onChange={e => set('customApiKey', e.target.value)} />
          </div>
        </>
      )}

      <SectionTitle>General</SectionTitle>
      <div className="form-row">
        <div><strong>Theme</strong></div>
        <select className="field-input" value={draft.theme || 'dark'} onChange={e => set('theme', e.target.value as SettingsData['theme'])}>
          <option value="dark">Dark</option>
          <option value="light">Light</option>
          <option value="auto">Auto (system)</option>
        </select>
      </div>
      <div className="form-row">
        <div><strong>Export format</strong></div>
        <select className="field-input" value={draft.exportFormat || 'text'} onChange={e => set('exportFormat', e.target.value as SettingsData['exportFormat'])}>
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
        <input className="field-input" type="number" min={1} max={16} value={draft.maxConcurrentTasks ?? 5} onChange={e => set('maxConcurrentTasks', Number(e.target.value))} />
      </div>
    </Modal>
  )
}
