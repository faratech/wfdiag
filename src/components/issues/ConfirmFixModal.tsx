import React from 'react'
import type { RemediationSummary } from '../../contexts/AppContext'
import { Modal, Button } from '../ui'

export interface ConfirmFixModalProps {
  remediation: RemediationSummary | null
  isAdmin: boolean
  onConfirm: (remediation: RemediationSummary) => void
  onCancel: () => void
  onRestartAsAdmin: () => void
}

/**
 * Confirmation dialog for Repair-tier remediations. The backend refuses to
 * run these without confirmed=true — this modal is where that consent is
 * collected, stating exactly what will run.
 */
export const ConfirmFixModal: React.FC<ConfirmFixModalProps> = ({
  remediation, isAdmin, onConfirm, onCancel, onRestartAsAdmin,
}) => {
  if (!remediation) return null
  const blocked = remediation.admin_required && !isAdmin

  return (
    <Modal
      open
      onClose={onCancel}
      title={remediation.label}
      width={460}
      footer={
        <>
          <Button onClick={onCancel}>Cancel</Button>
          {blocked ? (
            <Button variant="primary" icon="fa-shield-halved" onClick={onRestartAsAdmin}>
              Restart as Administrator
            </Button>
          ) : (
            <Button variant="primary" icon="fa-screwdriver-wrench" onClick={() => onConfirm(remediation)}>
              Run repair
            </Button>
          )}
        </>
      }
    >
      <p style={{ marginTop: 0 }}>This will run: <strong>{remediation.description}</strong></p>
      {blocked && (
        <p className="chat-error" role="alert">
          <i className="fa-solid fa-shield-halved" aria-hidden="true" /> This repair needs
          administrator rights. Restart the app as administrator first.
        </p>
      )}
      {!blocked && remediation.admin_required && (
        <p style={{ color: 'var(--wf-text-muted)', fontSize: 13 }}>
          <i className="fa-solid fa-shield-halved" aria-hidden="true" /> Runs with administrator rights.
        </p>
      )}
      {remediation.requires_restart && (
        <p style={{ color: 'var(--warn-fg, #c98a00)', fontSize: 13 }}>
          <i className="fa-solid fa-rotate-right" aria-hidden="true" /> A restart is required for
          this change to take effect.
        </p>
      )}
      {remediation.long_running && (
        <p style={{ color: 'var(--wf-text-muted)', fontSize: 13 }}>
          <i className="fa-solid fa-clock" aria-hidden="true" /> This can take 10–30 minutes.
          Keep the app open until it finishes.
        </p>
      )}
    </Modal>
  )
}
