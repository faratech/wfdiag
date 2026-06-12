import React, { ReactNode } from 'react'
import { Kbd } from './Kbd'

export interface TooltipProps {
  content: ReactNode
  /** Optional shortcut hint rendered as key chips, e.g. "Ctrl+K" */
  shortcut?: string
  side?: 'top' | 'right'
  children: ReactNode
}

/** CSS-only tooltip; appears on hover and on keyboard focus of the child. */
export const Tooltip: React.FC<TooltipProps> = ({ content, shortcut, side = 'top', children }) => (
  <span className={`tooltip-wrap ${side === 'right' ? 'tip-right' : ''}`}>
    {children}
    <span className="tooltip" role="tooltip">
      {content}
      {shortcut && <Kbd keys={shortcut} />}
    </span>
  </span>
)
