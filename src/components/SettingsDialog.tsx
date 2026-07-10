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
  { id: 'codex_cli', label: 'ChatGPT via Codex CLI (subscription)' },
  { id: 'claude_code', label: 'Claude via Claude Code CLI (subscription)' },
  { id: 'openai', label: 'OpenAI (cloud)' },
  { id: 'anthropic', label: 'Anthropic Claude (cloud)' },
  { id: 'gemini', label: 'Google Gemini (cloud)' },
  { id: 'deepseek', label: 'DeepSeek (cloud)' },
]

function configuredProviderFromSettings(settings: SettingsData): AIProviderId {
  if (settings.preferredAIProvider && settings.preferredAIProvider !== 'auto') {
    return settings.preferredAIProvider
  }
  if (settings.phiSilicaLafToken) return 'phi_silica'
  if (settings.localAiEndpoint) return 'foundry_local'
  if (settings.ollamaEndpoint || settings.ollamaModel) return 'ollama'
  if (settings.customEndpoint || settings.customModel || settings.customApiKey) return 'custom_openai'
  if (settings.codexCliPath || settings.codexModel) return 'codex_cli'
  if (settings.claudeCliPath || settings.claudeModel) return 'claude_code'
  if (settings.openAiApiKey) return 'openai'
  if (settings.anthropicApiKey || settings.anthropicModel) return 'anthropic'
  if (settings.geminiApiKey || settings.geminiModel) return 'gemini'
  if (settings.deepseekApiKey || settings.deepseekModel) return 'deepseek'
  return 'openai'
}

/** Sign-in state of a CLI bridge provider (auth lives entirely in the CLI) */
interface BridgeStatus {
  installed: boolean
  signedIn: boolean
  path?: string
}

/**
 * Sign-in row for a subscription CLI bridge (Codex, Claude Code). The
 * buttons only drive the CLI's own login/logout commands — the CLI opens
 * the browser and stores its credentials itself; this app never sees a
 * token.
 */
const BridgeAuthRow: React.FC<{
  provider: 'codex_cli' | 'claude_code'
  accountLabel: string
  hint: string
  signInText: string
  notDetectedText: string
}> = ({ provider, accountLabel, hint, signInText, notDetectedText }) => {
  const [status, setStatus] = useState<BridgeStatus | null>(null)
  const [busy, setBusy] = useState<'sign-in' | 'sign-out' | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    invoke<BridgeStatus>('ai_bridge_status', { provider })
      .then(s => { if (!cancelled) setStatus(s) })
      .catch(() => { if (!cancelled) setStatus({ installed: false, signedIn: false }) })
    return () => { cancelled = true }
  }, [provider])

  const signIn = async () => {
    setBusy('sign-in')
    setError(null)
    try {
      setStatus(await invoke<BridgeStatus>('ai_bridge_sign_in', { provider }))
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      invoke<BridgeStatus>('ai_bridge_status', { provider, refresh: true })
        .then(setStatus)
        .catch(() => {})
    } finally {
      setBusy(null)
    }
  }

  const cancelSignIn = () => {
    void invoke('ai_bridge_sign_in_cancel', { provider }).catch(() => {})
  }

  const signOut = async () => {
    setBusy('sign-out')
    setError(null)
    try {
      setStatus(await invoke<BridgeStatus>('ai_bridge_sign_out', { provider }))
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(null)
    }
  }

  return (
    <>
      <div className="form-row">
        <div>
          <strong>{accountLabel}</strong>
          <div className="hint">{hint}</div>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          {status === null && <span className="hint">Checking…</span>}
          {status !== null && !status.installed && <span className="hint">{notDetectedText}</span>}
          {status?.installed && status.signedIn && (
            <>
              <span className="tag">Signed in</span>
              <Button onClick={signOut} loading={busy === 'sign-out'} disabled={busy !== null}>Sign out</Button>
            </>
          )}
          {status?.installed && !status.signedIn && (
            <>
              <Button variant="primary" onClick={signIn} loading={busy === 'sign-in'} disabled={busy !== null}>
                {busy === 'sign-in' ? 'Waiting for browser…' : signInText}
              </Button>
              {busy === 'sign-in' && <Button onClick={cancelSignIn}>Cancel</Button>}
            </>
          )}
        </div>
      </div>
      {error && (
        <div role="alert" className="tag error" style={{ display: 'block', marginBottom: 8, whiteSpace: 'normal' }}>
          {error}
        </div>
      )}
    </>
  )
}

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
  const [saveError, setSaveError] = useState<string | null>(null)
  // Which provider's fields are shown in "Provider setup" — presentation only,
  // not persisted. With Active AI = Auto, seed from configured provider data
  // so saved non-OpenAI settings are visible when the dialog reopens.
  const [configProvider, setConfigProvider] = useState<AIProviderId>(() =>
    configuredProviderFromSettings(settings)
  )

  // Live model list per provider, fetched from the provider's own list API
  // when its pane opens. Draft credentials are passed so an unsaved key or
  // endpoint works immediately; a failed fetch leaves the pane on its
  // hardcoded fallback options (Codex has no list API — the backend returns
  // its hardcoded catalog).
  const [providerModels, setProviderModels] = useState<Record<string, string[]>>({})
  useEffect(() => {
    const provider = configProvider
    if (provider === 'phi_silica') return
    const apiKey =
      provider === 'openai' ? draft.openAiApiKey
      : provider === 'anthropic' ? draft.anthropicApiKey
      : provider === 'gemini' ? draft.geminiApiKey
      : provider === 'deepseek' ? draft.deepseekApiKey
      : provider === 'custom_openai' ? draft.customApiKey
      : undefined
    const endpoint =
      provider === 'custom_openai' ? draft.customEndpoint
      : provider === 'foundry_local' ? draft.localAiEndpoint
      : provider === 'ollama' ? draft.ollamaEndpoint
      : undefined
    // Cloud listings need a key; don't fire requests while the field is empty
    if (['openai', 'anthropic', 'gemini', 'deepseek'].includes(provider) && !apiKey) return
    if (provider === 'custom_openai' && !endpoint) return
    let cancelled = false
    // Debounced: the key/endpoint fields retrigger this as the user types
    const timer = setTimeout(() => {
      invoke<string[]>('ai_list_models', { provider, apiKey, endpoint })
        .then(models => { if (!cancelled) setProviderModels(m => ({ ...m, [provider]: models })) })
        .catch(() => { if (!cancelled) setProviderModels(m => ({ ...m, [provider]: [] })) })
    }, 400)
    return () => { cancelled = true; clearTimeout(timer) }
  }, [
    configProvider,
    draft.openAiApiKey,
    draft.anthropicApiKey,
    draft.geminiApiKey,
    draft.deepseekApiKey,
    draft.customApiKey,
    draft.customEndpoint,
    draft.localAiEndpoint,
    draft.ollamaEndpoint,
  ])

  /** Fetched list when available, otherwise the pane's hardcoded fallback */
  const modelsFor = (provider: string, fallback: string[]): string[] => {
    const fetched = providerModels[provider]
    return fetched && fetched.length > 0 ? fetched : fallback
  }
  const ollamaModels = providerModels['ollama'] ?? []
  const customModels = providerModels['custom_openai'] ?? []

  const set = <K extends keyof SettingsData>(k: K, v: SettingsData[K]) => {
    setSaveError(null)
    setDraft(d => ({ ...d, [k]: v }))
  }

  const save = async () => {
    setSaving(true)
    setSaveError(null)
    try {
      await onSave(draft)
    } catch (error) {
      setSaveError(error instanceof Error ? error.message : String(error))
    } finally {
      setSaving(false)
    }
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
      {saveError && (
        <div
          role="alert"
          className="tag error"
          style={{ display: 'block', marginBottom: 12, whiteSpace: 'normal' }}
        >
          Settings were not saved: {saveError}
        </div>
      )}
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
        <>
          <div className="form-row">
            <div><strong>OpenAI API key</strong></div>
            <input className="field-input" type="password" value={draft.openAiApiKey || ''} placeholder="sk-…" onChange={e => set('openAiApiKey', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong></div>
            <ModelSelect
              ariaLabel="OpenAI model"
              value={draft.openAiModel}
              defaultModel="gpt-5-nano"
              options={modelsFor('openai', ['gpt-5-nano', 'gpt-5-mini', 'gpt-5'])}
              onChange={v => set('openAiModel', v)}
            />
          </div>
        </>
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
              options={modelsFor('anthropic', ['claude-sonnet-4-6', 'claude-opus-4-8', 'claude-haiku-4-5'])}
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
              options={modelsFor('gemini', ['gemini-2.5-flash', 'gemini-2.5-pro', 'gemini-2.5-flash-lite'])}
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
              options={modelsFor('deepseek', ['deepseek-chat', 'deepseek-reasoner'])}
              onChange={v => set('deepseekModel', v)}
            />
          </div>
        </>
      )}

      {configProvider === 'foundry_local' && (
        <>
          <div className="form-row">
            <div><strong>Endpoint</strong><div className="hint">Optional. Leave empty to auto-discover Foundry Local</div></div>
            <input className="field-input" type="text" value={draft.localAiEndpoint || ''} placeholder="http://127.0.0.1:55769" onChange={e => set('localAiEndpoint', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong><div className="hint">Models the running service reports; empty uses the default</div></div>
            <ModelSelect
              ariaLabel="Foundry Local model"
              value={draft.localAiModel}
              defaultModel="phi-4-mini"
              options={modelsFor('foundry_local', [])}
              onChange={v => set('localAiModel', v)}
            />
          </div>
        </>
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
            {customModels.length > 0 ? (
              <select
                className="field-input"
                aria-label="Custom endpoint model"
                value={draft.customModel || ''}
                onChange={e => set('customModel', e.target.value)}
              >
                <option value="">Choose a model…</option>
                {(draft.customModel && !customModels.includes(draft.customModel)
                  ? [draft.customModel, ...customModels]
                  : customModels
                ).map(model => <option key={model} value={model}>{model}</option>)}
              </select>
            ) : (
              <input className="field-input" type="text" value={draft.customModel || ''} placeholder="anthropic/claude-haiku-4-5" onChange={e => set('customModel', e.target.value)} />
            )}
          </div>
          <div className="form-row">
            <div><strong>API key</strong><div className="hint">Optional for local proxies</div></div>
            <input className="field-input" type="password" value={draft.customApiKey || ''} placeholder="" onChange={e => set('customApiKey', e.target.value)} />
          </div>
        </>
      )}

      {configProvider === 'codex_cli' && (
        <>
          <BridgeAuthRow
            provider="codex_cli"
            accountLabel="ChatGPT account"
            hint="OpenAI's own login opens in your browser; usage bills to your ChatGPT plan"
            signInText="Sign in with ChatGPT"
            notDetectedText="Codex CLI not detected"
          />
          <div className="form-row">
            <div><strong>CLI path</strong><div className="hint">Optional. Empty auto-detects codex on PATH (npm install -g @openai/codex)</div></div>
            <input className="field-input" type="text" value={draft.codexCliPath || ''} placeholder="Auto-detected" onChange={e => set('codexCliPath', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong><div className="hint">Optional. Empty uses the CLI&apos;s default model</div></div>
            <ModelSelect
              ariaLabel="Codex model"
              value={draft.codexModel}
              defaultModel="chosen by Codex"
              options={modelsFor('codex_cli', ['gpt-5.6-sol', 'gpt-5.6-terra', 'gpt-5.6-luna', 'gpt-5.5', 'gpt-5.3-codex-spark'])}
              onChange={v => set('codexModel', v)}
            />
          </div>
          <div className="hint" style={{ marginBottom: 12 }}>
            Runs through OpenAI&apos;s Codex CLI with your ChatGPT plan — no API key, and this app never stores an OpenAI token.
          </div>
        </>
      )}

      {configProvider === 'claude_code' && (
        <>
          <BridgeAuthRow
            provider="claude_code"
            accountLabel="Claude account"
            hint="Anthropic's own login opens in your browser; usage bills to your Claude plan"
            signInText="Sign in with Claude"
            notDetectedText="Claude Code not detected"
          />
          <div className="form-row">
            <div><strong>CLI path</strong><div className="hint">Optional. Empty auto-detects claude on PATH (npm install -g @anthropic-ai/claude-code)</div></div>
            <input className="field-input" type="text" value={draft.claudeCliPath || ''} placeholder="Auto-detected" onChange={e => set('claudeCliPath', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong><div className="hint">Optional. Empty uses the CLI&apos;s default model</div></div>
            <ModelSelect
              ariaLabel="Claude Code model"
              value={draft.claudeModel}
              defaultModel="chosen by Claude Code"
              options={modelsFor('claude_code', ['sonnet', 'opus', 'haiku'])}
              onChange={v => set('claudeModel', v)}
            />
          </div>
          <div className="hint" style={{ marginBottom: 12 }}>
            Runs through Anthropic&apos;s Claude Code CLI with your Claude plan — no API key, and this app never stores a token. If sign-in doesn&apos;t complete here, run claude in a terminal and log in once.
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
