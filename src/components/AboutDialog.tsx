import React from 'react'

export interface AboutDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export const AboutDialog: React.FC<AboutDialogProps> = ({ open, onOpenChange }) => {
  if (!open) return null
  return (
    <div className="modal-overlay" onClick={() => onOpenChange(false)}>
      <div className="wf-block modal-card" style={{ maxWidth: 460, width: '90%' }} onClick={e => e.stopPropagation()}>
        <header className="wf-block-header">
          <span className="accent-bar" />
          <span>About</span>
          <button className="btn-icon" style={{ marginLeft: 'auto' }} onClick={() => onOpenChange(false)} title="Close"><i className="fa-solid fa-xmark" /></button>
        </header>
        <div style={{ padding: 24, textAlign: 'center' }}>
          <div className="rail-brand-mark" style={{ width: 56, height: 56, margin: '0 auto 14px', borderRadius: 12 }}>
            <img src="/wf-ds/icon-only.png" alt="" style={{ width: 36, height: 36 }} />
          </div>
          <h2 style={{ margin: '0 0 4px' }}>WindowsForum Diagnostics</h2>
          <p style={{ color: 'var(--wf-text-muted)', margin: '0 0 16px' }}>Version 2.2.0</p>
          <p style={{ fontSize: 13, color: 'var(--wf-text-muted)', lineHeight: 1.6 }}>
            A native Windows diagnostics tool by WindowsForum.com. Runs hardware, driver, storage,
            network, security and log diagnostics locally — with optional on-device or cloud AI analysis.
          </p>
          <div style={{ marginTop: 18 }}>
            <button className="btn primary" onClick={() => onOpenChange(false)}>Close</button>
          </div>
        </div>
      </div>
    </div>
  )
}
