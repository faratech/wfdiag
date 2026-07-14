import React from 'react'
import type { ActionProposal } from '../../contexts/AppContext'
import { Modal, Button } from '../ui'

export interface ConfirmFixModalProps {
  proposal: ActionProposal | null
  isAdmin: boolean
  onConfirm: (proposalId: string) => void
  onCancel: () => void
  onRestartAsAdmin: () => void
}

/**
 * Reviews the immutable backend proposal. Approval sends only its opaque id;
 * programs and arguments never cross from the webview into execution.
 */
export const ConfirmFixModal: React.FC<ConfirmFixModalProps> = ({
  proposal, isAdmin, onConfirm, onCancel, onRestartAsAdmin,
}) => {
  if (!proposal) return null
  const blocked = proposal.actions.some(action => action.remediation.admin_required) && !isAdmin
  const repair = proposal.actions.some(action => action.remediation.tier === 'repair')
  const requiresRestart = proposal.actions.some(action => action.remediation.requires_restart)
  const schedulesRestart = proposal.actions.some(action => action.remediation.id === 'restart_system')
  const longRunning = proposal.actions.some(action => action.remediation.long_running)
  const canStop = proposal.actions.every(action => action.remediation.cancellable)
  const batch = proposal.approvalScope === 'batch'

  return (
    <Modal
      open
      onClose={onCancel}
      title={batch ? `Review ${proposal.actions.length} actions` : proposal.actions[0].remediation.label}
      width={520}
      footer={
        <>
          <Button onClick={onCancel}>Cancel</Button>
          {blocked ? (
            <Button variant="primary" icon="fa-shield-halved" onClick={onRestartAsAdmin}>
              Restart as Administrator
            </Button>
          ) : (
            <Button
              variant="primary"
              icon={schedulesRestart ? 'fa-rotate-right' : repair ? 'fa-screwdriver-wrench' : 'fa-wand-magic-sparkles'}
              onClick={() => onConfirm(proposal.proposalId)}
            >
              {batch ? `Run these ${proposal.actions.length} actions` : schedulesRestart ? 'Schedule restart' : repair ? 'Run repair once' : 'Run once'}
            </Button>
          )}
        </>
      }
    >
      <p style={{ marginTop: 0 }}>
        Review the exact, catalog-backed action{batch ? 's' : ''}. This approval expires after
        10 minutes and can be used only once.
      </p>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        {proposal.actions.map(action => (
          <section key={action.remediation.id} className="wf-block" style={{ padding: 10 }}>
            <strong>{action.remediation.label}</strong>
            <p style={{ margin: '4px 0', fontSize: 13 }}>{action.remediation.description}</p>
            <ul style={{ margin: '6px 0 0', paddingLeft: 20, fontSize: 12.5 }}>
              {action.steps.map((step, index) => <li key={`${action.remediation.id}-${index}`}>{step}</li>)}
            </ul>
          </section>
        ))}
      </div>
      {blocked && (
        <p className="chat-error" role="alert">
          <i className="fa-solid fa-shield-halved" aria-hidden="true" /> This action needs
          administrator rights. Restart the app as administrator first.
        </p>
      )}
      {!blocked && proposal.actions.some(action => action.remediation.admin_required) && (
        <p style={{ color: 'var(--wf-text-muted)', fontSize: 13 }}>
          <i className="fa-solid fa-shield-halved" aria-hidden="true" /> Runs with administrator rights.
        </p>
      )}
      {schedulesRestart && (
        <p style={{ color: 'var(--warn-fg, #c98a00)', fontSize: 13 }}>
          <i className="fa-solid fa-rotate-right" aria-hidden="true" /> Save your work first.
          Windows will restart 60 seconds after approval; run <code>shutdown /a</code> to cancel.
        </p>
      )}
      {requiresRestart && !schedulesRestart && (
        <p style={{ color: 'var(--warn-fg, #c98a00)', fontSize: 13 }}>
          <i className="fa-solid fa-rotate-right" aria-hidden="true" /> A restart is required for
          this change to take effect.
        </p>
      )}
      {longRunning && (
        <p style={{ color: 'var(--wf-text-muted)', fontSize: 13 }}>
          <i className="fa-solid fa-clock" aria-hidden="true" /> This can take 10–30 minutes.
          Keep the app open until it finishes.{canStop ? ' You can stop it safely.' : ' It cannot be stopped safely once it starts.'}
        </p>
      )}
    </Modal>
  )
}
