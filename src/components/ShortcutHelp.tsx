import React from 'react'
import { Modal, Kbd } from './ui'

interface ShortcutHelpProps {
  open: boolean
  onClose: () => void
}

const SHORTCUTS: [string, string][] = [
  ['Ctrl+K', 'Open the command palette'],
  ['Ctrl+1 … Ctrl+6', 'Switch between screens'],
  ['Ctrl+Shift+Q', 'Run a Quick Scan'],
  ['Ctrl+Shift+F', 'Run a Full Scan'],
  ['Ctrl+/', 'Show this shortcut list'],
  ['Esc', 'Close dialogs and overlays'],
]

export const ShortcutHelp: React.FC<ShortcutHelpProps> = ({ open, onClose }) => (
  <Modal open={open} onClose={onClose} title="Keyboard Shortcuts" width={420}>
    {SHORTCUTS.map(([keys, desc]) => (
      <div key={keys} className="form-row">
        <span style={{ fontSize: 13 }}>{desc}</span>
        <span style={{ display: 'flex', gap: 3 }}>
          {keys.split(' … ').map((k, i, arr) => (
            <React.Fragment key={k}>
              <Kbd keys={k} />
              {i < arr.length - 1 && <span style={{ color: 'var(--wf-text-muted)', fontSize: 11 }}>…</span>}
            </React.Fragment>
          ))}
        </span>
      </div>
    ))}
  </Modal>
)
