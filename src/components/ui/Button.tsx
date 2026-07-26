import React from 'react'
import { Tooltip } from './Tooltip'

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'default' | 'primary' | 'ghost' | 'danger'
  /** FontAwesome icon name without the fa-solid prefix, e.g. "fa-bolt" */
  icon?: string
  /** Swaps the icon for a spinner and disables the button */
  loading?: boolean
  /** When disabled, wraps the button in a tooltip explaining why */
  disabledReason?: string
}

export const Button: React.FC<ButtonProps> = ({
  variant = 'default',
  icon,
  loading,
  disabledReason,
  className,
  children,
  disabled,
  ...rest
}) => {
  const cls = ['btn', variant !== 'default' ? variant : '', className].filter(Boolean).join(' ')
  const btn = (
    <button className={cls} disabled={disabled || loading} aria-busy={loading || undefined} {...rest}>
      {loading
        ? <i className="fa-solid fa-circle-notch fa-spin" aria-hidden="true" />
        : icon && <i className={`fa-solid ${icon}`} aria-hidden="true" />}
      {children}
    </button>
  )
  if (disabled && !loading && disabledReason) {
    return <Tooltip content={disabledReason}>{btn}</Tooltip>
  }
  return btn
}
