import React from 'react'
import { makeStyles, shorthands } from '@fluentui/react-components'
import { EGUI_COLORS } from '../theme'

export type StatusBadgeVariant = 'success' | 'error' | 'warning' | 'info'

interface StatusBadgeProps {
  text: string
  variant: StatusBadgeVariant
  icon?: React.ReactNode
  size?: 'small' | 'medium'
}

const useStyles = makeStyles({
  badge: {
    display: 'inline-flex',
    alignItems: 'center',
    ...shorthands.gap('4px'),
    ...shorthands.borderRadius('4px'),
    fontWeight: 500,
  },
  small: {
    fontSize: '10px',
    ...shorthands.padding('2px', '6px'),
  },
  medium: {
    fontSize: '12px',
    ...shorthands.padding('3px', '8px'),
  },
  success: {
    backgroundColor: EGUI_COLORS.SUCCESS_BG,
    color: EGUI_COLORS.SUCCESS,
  },
  error: {
    backgroundColor: EGUI_COLORS.ERROR_BG,
    color: EGUI_COLORS.ERROR,
  },
  warning: {
    backgroundColor: EGUI_COLORS.WARNING_BG,
    color: EGUI_COLORS.WARNING,
  },
  info: {
    backgroundColor: EGUI_COLORS.INFO_BG,
    color: EGUI_COLORS.INFO,
  },
})

export const StatusBadge: React.FC<StatusBadgeProps> = ({
  text,
  variant,
  icon,
  size = 'small',
}) => {
  const styles = useStyles()

  return (
    <span
      className={`${styles.badge} ${styles[variant]} ${styles[size]}`}
    >
      {icon}
      {text}
    </span>
  )
}
