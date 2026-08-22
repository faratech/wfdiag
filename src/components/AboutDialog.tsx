import React from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Modal, Button } from './ui'
import type { UpdateInfo } from '../hooks/useUpdateCheck'

export interface AboutDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Set when a newer GitHub release was found by the startup update check */
  updateInfo?: UpdateInfo | null
}

export const AboutDialog: React.FC<AboutDialogProps> = ({ open, onOpenChange, updateInfo }) => (
  <Modal open={open} onClose={() => onOpenChange(false)} title="About" width={460}>
    <div style={{ padding: 8, textAlign: 'center' }}>
      <div className="rail-brand-mark" style={{ width: 56, height: 56, margin: '0 auto 14px', borderRadius: 12 }}>
        <img src="/wf-ds/app-badge.png" alt="" style={{ width: 36, height: 36 }} />
      </div>
      <h2 style={{ margin: '0 0 4px' }}>WindowsForum Diagnostics</h2>
      <p style={{ color: 'var(--wf-text-muted)', margin: '0 0 16px' }}>Version 2.5.7</p>
      <p style={{ fontSize: 13, color: 'var(--wf-text-muted)', lineHeight: 1.6 }}>
        A native Windows diagnostics tool by WindowsForum.com. Runs hardware, driver, storage,
        network, security and log diagnostics locally — with optional on-device or cloud AI analysis.
      </p>
      {updateInfo && (
        <p style={{ fontSize: 13, margin: '14px 0 0', color: 'var(--wf-text-muted)' }}>
          <i className="fa-solid fa-circle-up" style={{ marginRight: 6, color: 'var(--info-fg)' }} />
          Version {updateInfo.version} is available.
        </p>
      )}
      <div style={{ marginTop: 18, display: 'flex', gap: 8, justifyContent: 'center' }}>
        {updateInfo && (
          <Button variant="primary" onClick={() => { void invoke('open_url', { url: updateInfo.html_url }) }}>
            Download v{updateInfo.version}
          </Button>
        )}
        <Button onClick={() => { void invoke('open_url', { url: 'https://windowsforum.com/' }) }}>
          <i className="fa-solid fa-globe" /> WindowsForum
        </Button>
        <Button onClick={() => { void invoke('open_url', { url: 'https://github.com/faratech/wfdiag' }) }}>
          <i className="fa-brands fa-github" /> GitHub
        </Button>
        <Button variant={updateInfo ? 'default' : 'primary'} onClick={() => onOpenChange(false)}>Close</Button>
      </div>
    </div>
  </Modal>
)
