import React, { useEffect, useState } from 'react'
import type { SettingsData } from './types'
import { Modal, Button } from './ui'

export type { SettingsData } from './types'

export interface SettingsDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  settings: SettingsData
  onSave: (settings: SettingsData) => void | Promise<void>
}

export const SettingsDialog: React.FC<SettingsDialogProps> = ({ open, onOpenChange, settings, onSave }) => {
  const [draft, setDraft] = useState<SettingsData>(settings)
  const [saving, setSaving] = useState(false)

  useEffect(() => { if (open) setDraft(settings) }, [open, settings])

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
      <div className="form-row">
        <div><strong>OpenAI API key</strong><div className="hint">Stored securely in the OS keyring</div></div>
        <input className="field-input" style={{ width: 260 }} type="password" value={draft.openAiApiKey || ''} placeholder="sk-…" onChange={e => set('openAiApiKey', e.target.value)} />
      </div>
      <div className="form-row">
        <div><strong>Preferred AI provider</strong></div>
        <select className="field-input" style={{ width: 180 }} value={draft.preferredAIProvider || 'auto'} onChange={e => set('preferredAIProvider', e.target.value as SettingsData['preferredAIProvider'])}>
          <option value="auto">Auto</option>
          <option value="phi_silica">Phi Silica (on-device)</option>
          <option value="foundry_local">Foundry Local (local server)</option>
          <option value="openai">OpenAI (cloud)</option>
        </select>
      </div>
      <div className="form-row">
        <div><strong>Enable AI insights</strong></div>
        <input type="checkbox" checked={draft.aiEnabled ?? true} onChange={e => set('aiEnabled', e.target.checked)} />
      </div>
      <div className="form-row">
        <div><strong>Local AI endpoint</strong><div className="hint">Optional. Leave empty to auto-discover Foundry Local</div></div>
        <input className="field-input" style={{ width: 260 }} type="text" value={draft.localAiEndpoint || ''} placeholder="http://127.0.0.1:55769" onChange={e => set('localAiEndpoint', e.target.value)} />
      </div>
      <div className="form-row">
        <div><strong>Phi Silica LAF token</strong><div className="hint">Optional. Microsoft-issued token unlocks the supported on-device path</div></div>
        <input className="field-input" style={{ width: 260 }} type="password" value={draft.phiSilicaLafToken || ''} placeholder="Leave empty for built-in" onChange={e => set('phiSilicaLafToken', e.target.value)} />
      </div>
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
