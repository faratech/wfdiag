/**
 * Shared status icon and color helpers
 * Provides consistent status rendering across components
 */

import React from 'react'
import {
  CheckmarkCircle20Regular,
  CheckmarkCircle12Filled,
  CheckmarkCircle24Regular,
  ErrorCircle20Regular,
  ErrorCircle12Filled,
  ErrorCircle24Regular,
  Warning20Regular,
  Warning12Filled,
  Warning24Regular,
  Info20Regular,
  Info12Regular,
  Info24Regular,
} from '@fluentui/react-icons'
import { StatusVariant, getStatusForegroundColor } from '../types/status'

type IconSize = 12 | 16 | 20 | 24

interface StatusIconOptions {
  size?: IconSize
  filled?: boolean
  color?: string
}

/**
 * Get status icon component with appropriate size and styling
 */
export function getStatusIcon(
  status: StatusVariant,
  options: StatusIconOptions = {}
): React.ReactNode {
  const { size = 20, filled = false, color } = options
  const iconColor = color || getStatusForegroundColor(status)
  const style = { color: iconColor }

  if (size === 12) {
    if (filled) {
      switch (status) {
        case 'success': return <CheckmarkCircle12Filled style={style} />
        case 'error': return <ErrorCircle12Filled style={style} />
        case 'warning': return <Warning12Filled style={style} />
        case 'info':
        default: return <Info12Regular style={style} />
      }
    } else {
      switch (status) {
        case 'success': return <CheckmarkCircle12Filled style={style} />
        case 'error': return <ErrorCircle12Filled style={style} />
        case 'warning': return <Warning12Filled style={style} />
        case 'info':
        default: return <Info12Regular style={style} />
      }
    }
  }

  if (size === 24) {
    switch (status) {
      case 'success': return <CheckmarkCircle24Regular style={style} />
      case 'error': return <ErrorCircle24Regular style={style} />
      case 'warning': return <Warning24Regular style={style} />
      case 'info':
      default: return <Info24Regular style={style} />
    }
  }

  // Default size 20
  switch (status) {
    case 'success': return <CheckmarkCircle20Regular style={style} />
    case 'error': return <ErrorCircle20Regular style={style} />
    case 'warning': return <Warning20Regular style={style} />
    case 'info':
    default: return <Info20Regular style={style} />
  }
}

/**
 * Get status dot element for compact status display
 */
export function getStatusDot(status: StatusVariant, size = 8): React.ReactNode {
  const color = getStatusForegroundColor(status)
  return (
    <span
      style={{
        display: 'inline-block',
        width: size,
        height: size,
        borderRadius: '50%',
        backgroundColor: color,
      }}
    />
  )
}
