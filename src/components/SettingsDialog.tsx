import React, { useCallback, useEffect, useId, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import type { AIProviderId, SettingsData } from './types'
import type { AIProviderStatus } from '../contexts/AIContext'
import { Modal, Button } from './ui'

export type { SettingsData } from './types'

export interface SettingsDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  settings: SettingsData
  aiStatus?: AIProviderStatus | null
  aiStatusLoading?: boolean
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
  if (settings.customEndpoint || settings.customModel || settings.customApiKey || settings.customApiKeySet) return 'custom_openai'
  if (settings.codexCliPath || settings.codexModel) return 'codex_cli'
  if (settings.claudeCliPath || settings.claudeModel) return 'claude_code'
  if (settings.openAiApiKey || settings.openAiApiKeySet) return 'openai'
  if (settings.anthropicApiKey || settings.anthropicApiKeySet || settings.anthropicModel) return 'anthropic'
  if (settings.geminiApiKey || settings.geminiApiKeySet || settings.geminiModel) return 'gemini'
  if (settings.deepseekApiKey || settings.deepseekApiKeySet || settings.deepseekModel) return 'deepseek'
  return 'openai'
}

const SecretInput: React.FC<{
  label: string
  value: string | undefined
  configured?: boolean
  placeholder: string
  onChange: (value: string) => void
}> = ({ label, value, configured, placeholder, onChange }) => (
  <div className="credential-control">
    <input
      className="field-input"
      aria-label={label}
      type="password"
      value={value || ''}
      placeholder={configured && !value ? 'Stored securely — enter a replacement' : placeholder}
      onChange={event => onChange(event.target.value)}
    />
    {configured && !value && <span className="credential-state"><i className="fa-solid fa-lock" aria-hidden="true" /> Configured</span>}
    {configured && (
      <button type="button" className="btn ghost credential-remove" aria-label={`Remove ${label}`} onClick={() => onChange('')}>
        Remove
      </button>
    )}
  </div>
)

/** Sign-in state of a CLI bridge provider (auth lives entirely in the CLI) */
interface BridgeStatus {
  installed: boolean
  signedIn: boolean
  path?: string
}

type BridgeInstallResponse =
  | { kind: 'installed'; status: BridgeStatus }
  | {
      kind: 'vendorFallbackConfirmationRequired'
      reason: 'explicit_approval_missing' | 'winget_unavailable' | 'winget_failed'
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
  /** Receives the freshly installed binary's path so the caller can pin it in settings (the running app's PATH is stale until relaunch). */
  onInstalledPath?: (path: string) => void
}> = ({ provider, accountLabel, hint, signInText, notDetectedText, onInstalledPath }) => {
  const [status, setStatus] = useState<BridgeStatus | null>(null)
  const [busy, setBusy] = useState<'sign-in' | 'sign-out' | 'install' | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [installConfirmation, setInstallConfirmation] = useState<'winget' | 'vendor' | null>(null)
  const [installMethod, setInstallMethod] = useState<'winget' | 'vendor' | null>(null)
  const installActiveRef = useRef(false)

  useEffect(() => {
    let cancelled = false
    invoke<BridgeStatus>('ai_bridge_status', { provider })
      .then(s => { if (!cancelled) setStatus(s) })
      .catch(() => { if (!cancelled) setStatus({ installed: false, signedIn: false }) })
    return () => { cancelled = true }
  }, [provider])

  useEffect(() => () => {
    // Changing provider panes or closing Settings must not leave a detached
    // installer running. The native cancellation closes its Job Object.
    if (installActiveRef.current) {
      void invoke('ai_bridge_install_cancel', { provider }).catch(() => {})
    }
  }, [provider])

  // Every process-producing request carries explicit approval. Winget is the
  // only first attempt; the mutable vendor bootstrap is a separate request
  // after a separate confirmation and is never an automatic fallback.
  const install = async (method: 'winget' | 'vendor') => {
    setInstallConfirmation(null)
    setInstallMethod(method)
    installActiveRef.current = true
    setBusy('install')
    setError(null)
    try {
      const response = await invoke<BridgeInstallResponse>('ai_bridge_install', {
        provider,
        method: method === 'winget' ? 'winget' : 'vendor_power_shell',
        confirmed: true,
        fallbackConfirmed: method === 'vendor',
      })
      if (response.kind === 'vendorFallbackConfirmationRequired') {
        setInstallConfirmation('vendor')
        return
      }
      const installed = response.status
      setStatus(installed)
      // The backend only returns an absolute path after it exists and its
      // allowlisted status command has run. Persist no speculative path.
      if (installed.path && onInstalledPath) onInstalledPath(installed.path)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      installActiveRef.current = false
      setBusy(null)
      setInstallMethod(null)
    }
  }

  const cancelInstall = () => {
    void invoke('ai_bridge_install_cancel', { provider }).catch(() => {})
  }

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
          {status !== null && !status.installed && (
            <>
              <span className="hint">{notDetectedText}</span>
              <Button
                variant="primary"
                onClick={() => setInstallConfirmation('winget')}
                loading={busy === 'install'}
                disabled={busy !== null}
              >
                {busy === 'install'
                  ? installMethod === 'vendor' ? 'Running vendor installer…' : 'Installing with winget…'
                  : 'Install CLI'}
              </Button>
              {busy === 'install' && <Button onClick={cancelInstall}>Cancel</Button>}
            </>
          )}
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
      {status?.installed && !status.signedIn && (
        <div className="hint" style={{ marginBottom: 8 }}>
          Signing in opens the vendor&apos;s own browser login — any plan tier works, and this
          app never sees your credentials. No subscription? Skip this CLI entirely: put an API
          key under the OpenAI or Anthropic sections above, or run fully local with Foundry
          Local / Ollama.
        </div>
      )}
      <Modal
        open={installConfirmation === 'winget'}
        onClose={() => setInstallConfirmation(null)}
        title={`Install ${provider === 'codex_cli' ? 'Codex CLI' : 'Claude Code'}`}
        width={500}
        footer={(
          <>
            <Button onClick={() => setInstallConfirmation(null)}>Cancel</Button>
            <Button variant="primary" onClick={() => void install('winget')}>Install with winget</Button>
          </>
        )}
      >
        <p>
          Windows Package Manager will download and install the official{' '}
          <strong>{provider === 'codex_cli' ? 'OpenAI.Codex' : 'Anthropic.ClaudeCode'}</strong>{' '}
          package from the winget source. The operation can take several minutes.
        </p>
        <p className="hint">
          This installs the CLI only. It will not open a login flow or access subscription credentials.
        </p>
      </Modal>
      <Modal
        open={installConfirmation === 'vendor'}
        onClose={() => setInstallConfirmation(null)}
        title="Confirm vendor installer fallback"
        width={540}
        footer={(
          <>
            <Button onClick={() => setInstallConfirmation(null)}>Cancel</Button>
            <Button variant="primary" onClick={() => void install('vendor')}>Run vendor installer</Button>
          </>
        )}
      >
        <p>
          Winget could not complete this installation. The fallback downloads and executes the
          vendor&apos;s current PowerShell bootstrap from{' '}
          <strong>{provider === 'codex_cli' ? 'chatgpt.com' : 'claude.ai'}</strong>.
        </p>
        <p className="hint">
          The remotely hosted script can change after this app is released. It runs only after this
          separate approval, is time-bounded and cancellable, and does not sign you in.
        </p>
      </Modal>
    </>
  )
}

interface ModelCatalogEntry {
  id: string
  label?: string
  description?: string
}

interface ModelCatalogResponse {
  models: ModelCatalogEntry[]
  defaultModel?: string
}

interface ModelCatalogState {
  catalog?: ModelCatalogResponse
  loading: boolean
  error?: string
  blocked?: string
  stale: boolean
}

interface ModelCatalogRequest extends Record<string, unknown> {
  provider: AIProviderId
  apiKey?: string
  endpoint?: string
  cliPath?: string
}

/** Editable model picker backed by the provider's live catalog. */
const ModelPicker: React.FC<{
  value: string | undefined
  state?: ModelCatalogState
  ariaLabel: string
  emptyPlaceholder: string
  onChange: (value: string) => void
  onRefresh: () => void
}> = ({ value, state, ariaLabel, emptyPlaceholder, onChange, onRefresh }) => {
  const generatedId = useId().replace(/:/g, '')
  const listId = `model-catalog-list-${generatedId}`
  const statusId = `model-catalog-status-${generatedId}`
  const selectionId = `model-catalog-selection-${generatedId}`
  const inputRef = useRef<HTMLInputElement>(null)
  const [open, setOpen] = useState(false)
  const [queryDirty, setQueryDirty] = useState(false)
  const [activeIndex, setActiveIndex] = useState(-1)
  const catalog = state?.catalog
  const models: ModelCatalogEntry[] = []
  const modelIndexes = new Map<string, number>()
  for (const candidate of catalog?.models ?? []) {
    const id = typeof candidate.id === 'string' ? candidate.id.trim() : ''
    if (!id) continue
    const existingIndex = modelIndexes.get(id)
    if (existingIndex === undefined) {
      modelIndexes.set(id, models.length)
      models.push({ ...candidate, id })
    } else {
      const existing = models[existingIndex]
      models[existingIndex] = {
        id,
        label: existing.label || candidate.label,
        description: existing.description || candidate.description,
      }
    }
  }
  const normalizedQuery = (value || '').trim().toLocaleLowerCase()
  const visibleModels = queryDirty && normalizedQuery
    ? models.filter(model =>
        [model.id, model.label, model.description]
          .filter(Boolean)
          .some(text => text!.toLocaleLowerCase().includes(normalizedQuery))
      )
    : models
  const selectedModel = value
    ? models.find(model => model.id === value.trim())
    : undefined
  const placeholder = catalog?.defaultModel
    ? `Default (${catalog.defaultModel})`
    : emptyPlaceholder
  const status = state?.error
    ? `${state.stale ? 'Could not refresh' : 'Could not load'} models: ${state.error}. ${state.stale ? 'Showing previously loaded results.' : 'Enter a model ID manually.'}`
    : state?.blocked
      ? `${state.blocked} You can still enter a model ID manually.`
      : state?.loading
        ? `${catalog ? 'Refreshing' : 'Loading'} models…`
        : state?.stale
          ? 'Showing previously loaded model results.'
          : catalog && catalog.models.length === 0
            ? 'No models were reported. Enter a model ID manually.'
            : catalog?.defaultModel
              ? `Provider default: ${catalog.defaultModel}`
              : ''
  const selectedDescription = value?.trim()
    ? selectedModel
      ? {
          label: selectedModel.label && selectedModel.label !== selectedModel.id
            ? selectedModel.label
            : 'Selected model',
          id: selectedModel.id,
          description: selectedModel.description,
          custom: false,
        }
      : {
          label: 'Custom or unavailable model',
          id: value.trim(),
          description: undefined,
          custom: true,
        }
    : undefined
  const describedBy = [
    status ? statusId : undefined,
    selectedDescription ? selectionId : undefined,
  ].filter(Boolean).join(' ') || undefined
  const activeModel = visibleModels[activeIndex]
  const activeOptionId = activeModel
    ? `${listId}-option-${activeIndex}`
    : undefined

  const openList = () => {
    setQueryDirty(false)
    setOpen(true)
    const selectedIndex = models.findIndex(model => model.id === value?.trim())
    setActiveIndex(selectedIndex >= 0 ? selectedIndex : models.length > 0 ? 0 : -1)
  }

  const selectModel = (model: ModelCatalogEntry) => {
    onChange(model.id)
    setQueryDirty(false)
    setOpen(false)
    setActiveIndex(-1)
    inputRef.current?.focus()
  }

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Escape' && open) {
      event.preventDefault()
      setOpen(false)
      setActiveIndex(-1)
      return
    }
    if (event.key === 'Enter' && open && activeModel) {
      event.preventDefault()
      selectModel(activeModel)
      return
    }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      if (!open) {
        openList()
        return
      }
      if (visibleModels.length === 0) return
      setActiveIndex(current => {
        if (event.key === 'ArrowDown') return Math.min(current + 1, visibleModels.length - 1)
        return current <= 0 ? 0 : current - 1
      })
      return
    }
    if (event.key === 'Home' && open && visibleModels.length > 0) {
      event.preventDefault()
      setActiveIndex(0)
      return
    }
    if (event.key === 'End' && open && visibleModels.length > 0) {
      event.preventDefault()
      setActiveIndex(visibleModels.length - 1)
    }
  }

  useEffect(() => {
    if (!open || !activeOptionId) return
    const option = document.getElementById(activeOptionId)
    if (option && 'scrollIntoView' in option) {
      option.scrollIntoView({ block: 'nearest' })
    }
  }, [activeOptionId, open])

  return (
    <div
      className="model-picker"
      onBlur={event => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
          setOpen(false)
          setActiveIndex(-1)
        }
      }}
    >
      <div className="model-picker-controls">
        <input
          ref={inputRef}
          className="field-input"
          aria-label={ariaLabel}
          aria-describedby={describedBy}
          aria-autocomplete="list"
          aria-controls={listId}
          aria-expanded={open}
          aria-activedescendant={open ? activeOptionId : undefined}
          role="combobox"
          type="text"
          value={value || ''}
          placeholder={placeholder}
          autoComplete="off"
          onFocus={() => {
            if (!open) openList()
          }}
          onChange={event => {
            onChange(event.target.value)
            setQueryDirty(true)
            setOpen(true)
            setActiveIndex(0)
          }}
          onKeyDown={handleKeyDown}
        />
        <Button
          type="button"
          variant="ghost"
          className="model-picker-toggle"
          icon={open ? 'fa-chevron-up' : 'fa-chevron-down'}
          aria-label={`${open ? 'Hide' : 'Show'} ${ariaLabel} options`}
          aria-expanded={open}
          aria-controls={listId}
          onClick={() => {
            if (open) {
              setOpen(false)
              setActiveIndex(-1)
            } else {
              openList()
              inputRef.current?.focus()
            }
          }}
        />
        <Button
          type="button"
          variant="ghost"
          className="model-picker-refresh"
          icon="fa-rotate"
          aria-label={`Refresh ${ariaLabel} list`}
          title={`Refresh ${ariaLabel} list`}
          loading={state?.loading}
          onClick={onRefresh}
        >
          Refresh
        </Button>
      </div>
      {open && (
        <div className="model-picker-listbox" id={listId} role="listbox" aria-label={`${ariaLabel} options`}>
          {visibleModels.map((model, index) => {
            const label = model.label && model.label !== model.id ? model.label : model.id
            return (
              <div
                key={model.id}
                id={`${listId}-option-${index}`}
                className={`model-picker-option${activeIndex === index ? ' active' : ''}${value?.trim() === model.id ? ' selected' : ''}`}
                role="option"
                aria-selected={value?.trim() === model.id}
                data-model-id={model.id}
                onMouseDown={event => event.preventDefault()}
                onMouseEnter={() => setActiveIndex(index)}
                onClick={() => selectModel(model)}
              >
                <div className="model-picker-option-heading">
                  <span>{label}</span>
                  <code>{model.id}</code>
                </div>
                {model.description && <div className="model-picker-option-description">{model.description}</div>}
              </div>
            )
          })}
          {visibleModels.length === 0 && (
            <div className="model-picker-empty" role="status">
              No catalog matches. The model ID you type will still be saved.
            </div>
          )}
        </div>
      )}
      {selectedDescription && (
        <div
          id={selectionId}
          className={`model-picker-selection${selectedDescription.custom ? ' custom' : ''}`}
        >
          <span>{selectedDescription.label}</span>
          <code>{selectedDescription.id}</code>
          {selectedDescription.description && <small>{selectedDescription.description}</small>}
        </div>
      )}
      {status && (
        <div
          id={statusId}
          className={`model-picker-status${state?.error ? ' error' : ''}${state?.stale ? ' stale' : ''}`}
          role={state?.error ? 'alert' : 'status'}
        >
          {status}
        </div>
      )}
    </div>
  )
}

function modelCatalogRequest(provider: AIProviderId, draft: SettingsData): {
  args: ModelCatalogRequest
  blocked?: string
} {
  const apiKey =
    provider === 'openai' ? draft.openAiApiKey
    : provider === 'anthropic' ? draft.anthropicApiKey
    : provider === 'gemini' ? draft.geminiApiKey
    : provider === 'deepseek' ? draft.deepseekApiKey
    : provider === 'custom_openai' ? draft.customApiKey
    : undefined
  const apiKeyConfigured =
    provider === 'openai' ? draft.openAiApiKeySet
    : provider === 'anthropic' ? draft.anthropicApiKeySet
    : provider === 'gemini' ? draft.geminiApiKeySet
    : provider === 'deepseek' ? draft.deepseekApiKeySet
    : provider === 'custom_openai' ? draft.customApiKeySet
    : false
  const endpoint =
    provider === 'custom_openai' ? draft.customEndpoint
    : provider === 'foundry_local' ? draft.localAiEndpoint
    : provider === 'ollama' ? draft.ollamaEndpoint
    : undefined
  const cliPath =
    provider === 'codex_cli' ? draft.codexCliPath
    : provider === 'claude_code' ? draft.claudeCliPath
    : undefined
  const needsApiKey = ['openai', 'anthropic', 'gemini', 'deepseek'].includes(provider)
  const blocked =
    needsApiKey && !apiKey && !apiKeyConfigured
      ? 'Enter an API key to load the available models.'
      : provider === 'custom_openai' && !endpoint
        ? 'Enter an endpoint URL to load the available models.'
        : undefined

  return {
    args: { provider, apiKey, endpoint, cliPath },
    blocked,
  }
}

export const SettingsDialog: React.FC<SettingsDialogProps> = (props) =>
  props.open ? <SettingsDialogInner {...props} /> : null

// Mounted only while open (SettingsDialog gates on it), so the draft and the
// configured-provider selection seed straight from the live settings on mount
// — no in-render reseeding of a long-lived dialog.
const SettingsDialogInner: React.FC<SettingsDialogProps> = ({ open, onOpenChange, settings, aiStatus, aiStatusLoading, onSave }) => {
  const [draft, setDraft] = useState<SettingsData>(settings)
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  // Which provider's fields are shown in "Provider setup" — presentation only,
  // not persisted. With AI provider = Auto, seed from configured provider data
  // so saved non-OpenAI settings are visible when the dialog reopens.
  const [configProvider, setConfigProvider] = useState<AIProviderId>(() =>
    configuredProviderFromSettings(settings)
  )
  const phiUnavailable = !!aiStatus
    && (!aiStatus.phi_silica_available || !aiStatus.phi_silica_ready)
  const phiStatusPending = aiStatusLoading === true || aiStatus === null
  const phiBlocked = phiStatusPending || phiUnavailable
  const phiUnavailableReason = aiStatus?.phi_silica_message
    || 'Phi Silica is unavailable or not ready on this PC.'
  const phiBlockedReason = phiStatusPending
    ? 'Checking whether Phi Silica is available on this PC. Wait for the check to finish before selecting it.'
    : phiUnavailableReason

  // Catalogs are session-only. A failed refresh retains the most recent
  // successful list, marks it stale, and always leaves manual entry available.
  const [modelCatalogs, setModelCatalogs] = useState<Partial<Record<AIProviderId, ModelCatalogState>>>({})
  const requestVersions = useRef<Partial<Record<AIProviderId, number>>>({})
  const refreshTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const mounted = useRef(true)
  const draftForCatalog = useRef(draft)

  useEffect(() => {
    draftForCatalog.current = draft
  }, [draft])

  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      if (refreshTimer.current) clearTimeout(refreshTimer.current)
    }
  }, [])

  const loadModels = useCallback(async (provider: AIProviderId) => {
    if (provider === 'phi_silica') return
    const { args, blocked } = modelCatalogRequest(provider, draftForCatalog.current)
    const version = (requestVersions.current[provider] ?? 0) + 1
    requestVersions.current[provider] = version

    if (blocked) {
      setModelCatalogs(current => {
        const previous = current[provider]
        return {
          ...current,
          [provider]: {
            catalog: previous?.catalog,
            loading: false,
            blocked,
            stale: !!previous?.catalog,
          },
        }
      })
      return
    }

    setModelCatalogs(current => {
      const previous = current[provider]
      return {
        ...current,
        [provider]: {
          catalog: previous?.catalog,
          loading: true,
          stale: previous?.stale ?? false,
        },
      }
    })

    try {
      const catalog = await invoke<ModelCatalogResponse>('ai_list_models', args)
      if (!mounted.current || requestVersions.current[provider] !== version) return
      setModelCatalogs(current => ({
        ...current,
        [provider]: {
          catalog: {
            models: Array.isArray(catalog.models) ? catalog.models : [],
            defaultModel: catalog.defaultModel,
          },
          loading: false,
          stale: false,
        },
      }))
    } catch (error) {
      if (!mounted.current || requestVersions.current[provider] !== version) return
      const message = error instanceof Error ? error.message : String(error)
      setModelCatalogs(current => {
        const previous = current[provider]
        return {
          ...current,
          [provider]: {
            catalog: previous?.catalog,
            loading: false,
            error: message,
            stale: !!previous?.catalog,
          },
        }
      })
    }
  }, [])

  // Refresh once when a pane opens and when its unsaved connection details
  // change. There is intentionally no polling.
  useEffect(() => {
    if (refreshTimer.current) clearTimeout(refreshTimer.current)
    if (configProvider === 'phi_silica') return
    refreshTimer.current = setTimeout(() => {
      refreshTimer.current = null
      void loadModels(configProvider)
    }, 400)
    return () => {
      if (refreshTimer.current) {
        clearTimeout(refreshTimer.current)
        refreshTimer.current = null
      }
    }
  }, [
    configProvider,
    loadModels,
    draft.openAiApiKey,
    draft.openAiApiKeySet,
    draft.anthropicApiKey,
    draft.anthropicApiKeySet,
    draft.geminiApiKey,
    draft.geminiApiKeySet,
    draft.deepseekApiKey,
    draft.deepseekApiKeySet,
    draft.customApiKey,
    draft.customApiKeySet,
    draft.customEndpoint,
    draft.localAiEndpoint,
    draft.ollamaEndpoint,
    draft.codexCliPath,
    draft.claudeCliPath,
  ])

  const refreshModels = () => {
    if (refreshTimer.current) {
      clearTimeout(refreshTimer.current)
      refreshTimer.current = null
    }
    void loadModels(configProvider)
  }

  const set = <K extends keyof SettingsData>(k: K, v: SettingsData[K]) => {
    setSaveError(null)
    setDraft(d => ({ ...d, [k]: v }))
  }

  const setSecret = (
    key: 'openAiApiKey' | 'anthropicApiKey' | 'geminiApiKey' | 'deepseekApiKey' | 'customApiKey',
    flag: 'openAiApiKeySet' | 'anthropicApiKeySet' | 'geminiApiKeySet' | 'deepseekApiKeySet' | 'customApiKeySet',
    value: string,
  ) => {
    setSaveError(null)
    setDraft(current => ({
      ...current,
      [key]: value,
      [flag]: value ? true : false,
    }))
  }

  const save = async () => {
    setSaving(true)
    setSaveError(null)
    try {
      if (draft.preferredAIProvider === 'phi_silica' && phiBlocked) {
        setSaveError(phiBlockedReason)
        return
      }
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
      width={640}
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
      <SectionTitle>AI assistant</SectionTitle>
      <p className="settings-section-intro">
        Choose how AI is used across Assistant, Scan Report, and issue explanations. Provider credentials are managed below.
      </p>
      <div className="form-row">
        <div><strong>Enable AI insights</strong></div>
        <input aria-label="Enable AI insights" type="checkbox" checked={draft.aiEnabled ?? true} onChange={e => set('aiEnabled', e.target.checked)} />
      </div>
      <div className="form-row">
        <div><strong>AI provider</strong><div className="hint">Auto picks local first, then configured cloud providers</div></div>
        <select
          className="field-input"
          aria-label="AI provider"
          value={draft.preferredAIProvider || 'auto'}
          onChange={e => {
            const value = e.target.value as SettingsData['preferredAIProvider']
            set('preferredAIProvider', value)
            // Editing follows the chosen provider so its fields appear below
            if (value && value !== 'auto') setConfigProvider(value)
          }}
        >
          <option value="auto">Auto</option>
          {PROVIDER_OPTIONS.map(p => (
            <option
              key={p.id}
              value={p.id}
              // A disabled option that is also the <select>'s current value
              // traps the control (browsers render/behave as if the whole
              // select were disabled). Once Phi is confirmed unavailable
              // (not just still checking) and it's already selected, leave
              // it selectable so the user can always pick a different
              // provider — Save still blocks while it stays selected.
              disabled={
                p.id === 'phi_silica' &&
                phiBlocked &&
                !(phiUnavailable && draft.preferredAIProvider === 'phi_silica')
              }
            >
              {p.label}
            </option>
          ))}
        </select>
      </div>
      {phiBlocked && (
        <div className="settings-provider-status" role="status">
          <i className={`fa-solid ${phiStatusPending ? 'fa-circle-notch fa-spin' : 'fa-circle-info'}`} aria-hidden="true" /> {phiBlockedReason}
        </div>
      )}

      <div className="form-row">
        <div>
          <strong>Cloud fallback</strong>
          <div className="hint">When Auto cannot finish with an on-device or local provider</div>
        </div>
        <select
          className="field-input"
          aria-label="Cloud fallback policy"
          value={draft.cloudFallbackPolicy || 'ask'}
          onChange={e => set('cloudFallbackPolicy', e.target.value as SettingsData['cloudFallbackPolicy'])}
        >
          <option value="ask">Ask every time</option>
          <option value="allow">Allow automatically</option>
          <option value="never">Never use cloud fallback</option>
        </select>
      </div>
      <div className="form-row">
        <div>
          <strong>Web grounding</strong>
          <div className="hint">Allow supported providers to look up current public information</div>
        </div>
        <input
          aria-label="Enable web grounding"
          type="checkbox"
          checked={draft.networkGroundingEnabled ?? false}
          onChange={e => set('networkGroundingEnabled', e.target.checked)}
        />
      </div>

      <SectionTitle>Provider setup</SectionTitle>
      <p className="settings-section-intro">
        Configure credentials for any provider here — independent of which one is active above. Local providers keep prompts on this PC; subscription and API providers receive only the question and selected diagnostic context.
      </p>
      <div className="form-row settings-provider-navigator">
        <div><strong>Set up provider</strong><div className="hint">Browse and edit any provider's settings, whether or not it's currently active</div></div>
        <select
          className="field-input"
          aria-label="Provider to set up"
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
            <SecretInput label="OpenAI API key" value={draft.openAiApiKey} configured={draft.openAiApiKeySet} placeholder="sk-…" onChange={value => setSecret('openAiApiKey', 'openAiApiKeySet', value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong></div>
            <ModelPicker
              ariaLabel="OpenAI model"
              value={draft.openAiModel}
              state={modelCatalogs.openai}
              emptyPlaceholder="Use app default"
              onChange={v => set('openAiModel', v)}
              onRefresh={refreshModels}
            />
          </div>
        </>
      )}

      {configProvider === 'anthropic' && (
        <>
          <div className="form-row">
            <div><strong>Anthropic API key</strong></div>
            <SecretInput label="Anthropic API key" value={draft.anthropicApiKey} configured={draft.anthropicApiKeySet} placeholder="sk-ant-…" onChange={value => setSecret('anthropicApiKey', 'anthropicApiKeySet', value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong></div>
            <ModelPicker
              ariaLabel="Anthropic model"
              value={draft.anthropicModel}
              state={modelCatalogs.anthropic}
              emptyPlaceholder="Use app default"
              onChange={v => set('anthropicModel', v)}
              onRefresh={refreshModels}
            />
          </div>
        </>
      )}

      {configProvider === 'gemini' && (
        <>
          <div className="form-row">
            <div><strong>Gemini API key</strong></div>
            <SecretInput label="Gemini API key" value={draft.geminiApiKey} configured={draft.geminiApiKeySet} placeholder="AIza…" onChange={value => setSecret('geminiApiKey', 'geminiApiKeySet', value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong></div>
            <ModelPicker
              ariaLabel="Gemini model"
              value={draft.geminiModel}
              state={modelCatalogs.gemini}
              emptyPlaceholder="Use app default"
              onChange={v => set('geminiModel', v)}
              onRefresh={refreshModels}
            />
          </div>
        </>
      )}

      {configProvider === 'deepseek' && (
        <>
          <div className="form-row">
            <div><strong>DeepSeek API key</strong></div>
            <SecretInput label="DeepSeek API key" value={draft.deepseekApiKey} configured={draft.deepseekApiKeySet} placeholder="sk-…" onChange={value => setSecret('deepseekApiKey', 'deepseekApiKeySet', value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong></div>
            <ModelPicker
              ariaLabel="DeepSeek model"
              value={draft.deepseekModel}
              state={modelCatalogs.deepseek}
              emptyPlaceholder="Use app default"
              onChange={v => set('deepseekModel', v)}
              onRefresh={refreshModels}
            />
          </div>
        </>
      )}

      {configProvider === 'foundry_local' && (
        <>
          <div className="form-row">
            <div><strong>Endpoint</strong><div className="hint">Optional. Leave empty to auto-discover Foundry Local</div></div>
            <input className="field-input" aria-label="Foundry Local endpoint" type="text" value={draft.localAiEndpoint || ''} placeholder="http://127.0.0.1:55769" onChange={e => set('localAiEndpoint', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong><div className="hint">Models the running service reports; empty uses the default</div></div>
            <ModelPicker
              ariaLabel="Foundry Local model"
              value={draft.localAiModel}
              state={modelCatalogs.foundry_local}
              emptyPlaceholder="Use service default"
              onChange={v => set('localAiModel', v)}
              onRefresh={refreshModels}
            />
          </div>
        </>
      )}

      {configProvider === 'ollama' && (
        <>
          <div className="form-row">
            <div><strong>Endpoint</strong><div className="hint">Optional. Leave empty for the default port</div></div>
            <input className="field-input" aria-label="Ollama endpoint" type="text" value={draft.ollamaEndpoint || ''} placeholder="http://127.0.0.1:11434" onChange={e => set('ollamaEndpoint', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong><div className="hint">Empty uses the first installed model</div></div>
            <ModelPicker
              ariaLabel="Ollama model"
              value={draft.ollamaModel}
              state={modelCatalogs.ollama}
              emptyPlaceholder="Auto (first installed)"
              onChange={v => set('ollamaModel', v)}
              onRefresh={refreshModels}
            />
          </div>
        </>
      )}

      {configProvider === 'phi_silica' && (
        <div className="form-row">
          <div><strong>LAF token</strong><div className="hint">Optional. Microsoft-issued token; requires the Store version on a Copilot+ PC</div></div>
          <input className="field-input" aria-label="Phi Silica LAF token" type="password" value={draft.phiSilicaLafToken || ''} placeholder="Leave empty for built-in" onChange={e => set('phiSilicaLafToken', e.target.value)} />
        </div>
      )}

      {configProvider === 'custom_openai' && (
        <>
          <div className="form-row">
            <div><strong>Endpoint URL</strong><div className="hint">OpenRouter, Groq, or any /v1/chat/completions server</div></div>
            <input className="field-input" aria-label="Custom endpoint URL" type="text" value={draft.customEndpoint || ''} placeholder="https://openrouter.ai/api" onChange={e => set('customEndpoint', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong><div className="hint">Required. The model id your provider documents</div></div>
            <ModelPicker
              ariaLabel="Custom endpoint model"
              value={draft.customModel}
              state={modelCatalogs.custom_openai}
              emptyPlaceholder="Enter model ID"
              onChange={v => set('customModel', v)}
              onRefresh={refreshModels}
            />
          </div>
          <div className="form-row">
            <div><strong>API key</strong><div className="hint">Optional for local proxies</div></div>
            <SecretInput label="Custom endpoint API key" value={draft.customApiKey} configured={draft.customApiKeySet} placeholder="Optional" onChange={value => setSecret('customApiKey', 'customApiKeySet', value)} />
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
            onInstalledPath={path => set('codexCliPath', path)}
          />
          <div className="form-row">
            <div><strong>CLI path</strong><div className="hint">Optional. Empty auto-detects codex — use Install CLI above if it is missing</div></div>
            <input className="field-input" aria-label="Codex CLI path" type="text" value={draft.codexCliPath || ''} placeholder="Auto-detected" onChange={e => set('codexCliPath', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong><div className="hint">Optional. Empty uses the CLI&apos;s default model</div></div>
            <ModelPicker
              ariaLabel="Codex model"
              value={draft.codexModel}
              state={modelCatalogs.codex_cli}
              emptyPlaceholder="Use Codex CLI default"
              onChange={v => set('codexModel', v)}
              onRefresh={refreshModels}
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
            onInstalledPath={path => set('claudeCliPath', path)}
          />
          <div className="form-row">
            <div><strong>CLI path</strong><div className="hint">Optional. Empty auto-detects claude — use Install CLI above if it is missing</div></div>
            <input className="field-input" aria-label="Claude Code CLI path" type="text" value={draft.claudeCliPath || ''} placeholder="Auto-detected" onChange={e => set('claudeCliPath', e.target.value)} />
          </div>
          <div className="form-row">
            <div><strong>Model</strong><div className="hint">Optional. Empty uses the CLI&apos;s default model</div></div>
            <ModelPicker
              ariaLabel="Claude Code model"
              value={draft.claudeModel}
              state={modelCatalogs.claude_code}
              emptyPlaceholder="Use Claude Code default"
              onChange={v => set('claudeModel', v)}
              onRefresh={refreshModels}
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
        <select className="field-input" aria-label="Theme" value={draft.theme || 'dark'} onChange={e => set('theme', e.target.value as SettingsData['theme'])}>
          <option value="dark">Dark</option>
          <option value="light">Light</option>
          <option value="auto">Auto (system)</option>
        </select>
      </div>
      <div className="form-row">
        <div><strong>Export format</strong></div>
        <select className="field-input" aria-label="Export format" value={draft.exportFormat || 'text'} onChange={e => set('exportFormat', e.target.value as SettingsData['exportFormat'])}>
          <option value="text">Text</option>
          <option value="json">JSON</option>
          <option value="html">HTML</option>
        </select>
      </div>
      <div className="form-row">
        <div><strong>Auto-save scans</strong></div>
        <input aria-label="Auto-save scans" type="checkbox" checked={draft.autoSave ?? true} onChange={e => set('autoSave', e.target.checked)} />
      </div>
      <div className="form-row">
        <div><strong>Desktop notifications</strong><div className="hint">Notify when a scan finishes in the background</div></div>
        <input aria-label="Desktop notifications" type="checkbox" checked={draft.showNotifications ?? true} onChange={e => set('showNotifications', e.target.checked)} />
      </div>
      <div className="form-row">
        <div><strong>Scan on startup</strong></div>
        <input aria-label="Scan on startup" type="checkbox" checked={draft.scanOnStartup ?? false} onChange={e => set('scanOnStartup', e.target.checked)} />
      </div>
      <div className="form-row">
        <div><strong>Close to tray</strong><div className="hint">Closing the window keeps the app running in the system tray</div></div>
        <input aria-label="Close to tray" type="checkbox" checked={draft.closeToTray ?? false} onChange={e => set('closeToTray', e.target.checked)} />
      </div>
      <div className="form-row">
        <div><strong>Max concurrent tasks</strong></div>
        <input className="field-input" aria-label="Max concurrent tasks" type="number" min={1} max={16} value={draft.maxConcurrentTasks ?? 5} onChange={e => set('maxConcurrentTasks', Number(e.target.value))} />
      </div>
    </Modal>
  )
}
