import React from 'react'
import { Modal, Button } from './ui'

export interface AboutDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export const AboutDialog: React.FC<AboutDialogProps> = ({ open, onOpenChange }) => (
  <Modal open={open} onClose={() => onOpenChange(false)} title="About" width={460}>
    <div style={{ padding: 8, textAlign: 'center' }}>
      <div className="rail-brand-mark" style={{ width: 56, height: 56, margin: '0 auto 14px', borderRadius: 12 }}>
        <img src="/wf-ds/icon-only.png" alt="" style={{ width: 36, height: 36 }} />
      </div>
      <h2 style={{ margin: '0 0 4px' }}>WindowsForum Diagnostics</h2>
      <p style={{ color: 'var(--wf-text-muted)', margin: '0 0 16px' }}>Version 2.3.0</p>
      <p style={{ fontSize: 13, color: 'var(--wf-text-muted)', lineHeight: 1.6 }}>
        A native Windows diagnostics tool by WindowsForum.com. Runs hardware, driver, storage,
        network, security and log diagnostics locally — with optional on-device or cloud AI analysis.
      </p>
      <div style={{ marginTop: 18 }}>
        <Button variant="primary" onClick={() => onOpenChange(false)}>Close</Button>
      </div>
    </div>
  </Modal>
)
