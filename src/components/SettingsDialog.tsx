import React, { useEffect, useState } from 'react'
import type { SettingsData } from './types'

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

  if (!open) return null

  const set = <K extends keyof SettingsData>(k: K, v: SettingsData[K]) =>
    setDraft(d => ({ ...d, [k]: v }))

  const save = async () => {
    setSaving(true)
    try { await onSave(draft) } finally { setSaving(false) }
  }

  const field: React.CSSProperties = {
    width: '100%', padding: '7px 10px', borderRadius: 4,
    border: '1px solid var(--hairline-strong)', background: 'var(--wf-input-bg)',
    color: 'var(--wf-text)', fontSize: 13, fontFamily: 'inherit',
  }
  const row: React.CSSProperties = { display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '8px 0', borderBottom: '1px solid var(--hairline)', gap: 16 }

  return (
    <div className="modal-overlay" onClick={() => onOpenChange(false)}>
      <div className="wf-block modal-card" style={{ maxWidth: 560, width: '92%' }} onClick={e => e.stopPropagation()}>
        <header className="wf-block-header">
          <span className="accent-bar" />
          <span>Settings</span>
          <button className="btn-icon" style={{ marginLeft: 'auto' }} onClick={() => onOpenChange(false)} title="Close"><i className="fa-solid fa-xmark" /></button>
        </header>
        <div style={{ padding: 16, maxHeight: '70vh', overflow: 'auto' }}>
          <div style={row}>
            <div><strong>OpenAI API key</strong><div style={{ fontSize: 11, color: 'var(--wf-text-muted)' }}>Stored securely in the OS keyring</div></div>
            <input style={{ ...field, width: 260 }} type="password" value={draft.openAiApiKey || ''} placeholder="sk-…" onChange={e => set('openAiApiKey', e.target.value)} />
          </div>
          <div style={row}>
            <div><strong>Preferred AI provider</strong></div>
            <select style={{ ...field, width: 180 }} value={draft.preferredAIProvider || 'auto'} onChange={e => set('preferredAIProvider', e.target.value as SettingsData['preferredAIProvider'])}>
              <option value="auto">Auto</option>
              <option value="phi_silica">Phi Silica (on-device)</option>
              <option value="openai">OpenAI (cloud)</option>
            </select>
          </div>
          <div style={row}>
            <div><strong>Enable AI insights</strong></div>
            <input type="checkbox" checked={draft.aiEnabled ?? true} onChange={e => set('aiEnabled', e.target.checked)} />
          </div>
          <div style={row}>
            <div><strong>Theme</strong></div>
            <select style={{ ...field, width: 180 }} value={draft.theme || 'dark'} onChange={e => set('theme', e.target.value as SettingsData['theme'])}>
              <option value="dark">Dark</option>
              <option value="light">Light</option>
              <option value="auto">Auto (system)</option>
            </select>
          </div>
          <div style={row}>
            <div><strong>Export format</strong></div>
            <select style={{ ...field, width: 180 }} value={draft.exportFormat || 'text'} onChange={e => set('exportFormat', e.target.value as SettingsData['exportFormat'])}>
              <option value="text">Text</option>
              <option value="json">JSON</option>
              <option value="html">HTML</option>
            </select>
          </div>
          <div style={row}>
            <div><strong>Auto-save scans</strong></div>
            <input type="checkbox" checked={draft.autoSave ?? true} onChange={e => set('autoSave', e.target.checked)} />
          </div>
          <div style={row}>
            <div><strong>Scan on startup</strong></div>
            <input type="checkbox" checked={draft.scanOnStartup ?? false} onChange={e => set('scanOnStartup', e.target.checked)} />
          </div>
          <div style={{ ...row, borderBottom: 'none' }}>
            <div><strong>Max concurrent tasks</strong></div>
            <input style={{ ...field, width: 90 }} type="number" min={1} max={16} value={draft.maxConcurrentTasks ?? 5} onChange={e => set('maxConcurrentTasks', Number(e.target.value))} />
          </div>
        </div>
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, padding: 14, borderTop: '1px solid var(--hairline)' }}>
          <button className="btn" onClick={() => onOpenChange(false)}>Cancel</button>
          <button className="btn primary" onClick={save} disabled={saving}>
            {saving ? <><i className="fa-solid fa-spinner fa-spin" /> Saving…</> : 'Save'}
          </button>
        </div>
      </div>
    </div>
  )
}
