import React, { ReactNode } from 'react'

export interface EmptyStateProps {
  /** FontAwesome icon name without the fa-solid prefix */
  icon: string
  title: string
  sub?: string
  actions?: ReactNode
}

export const EmptyState: React.FC<EmptyStateProps> = ({ icon, title, sub, actions }) => (
  <div className="empty-state">
    <div className="es-icon"><i className={`fa-solid ${icon}`} aria-hidden="true" /></div>
    <p className="es-title">{title}</p>
    {sub && <p className="es-sub">{sub}</p>}
    {actions && <div className="es-actions">{actions}</div>}
  </div>
)
