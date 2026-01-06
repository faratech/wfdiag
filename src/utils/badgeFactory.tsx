/**
 * Badge factory for navigation and status badges
 * Provides consistent badge rendering across NavRail and TabNavigation
 */

import React from 'react'
import {
  Badge,
  CounterBadge,
} from '@fluentui/react-components'

export type BadgeVariant = 'count' | 'dot' | 'new' | 'status'

// CounterBadge only accepts these colors
type CounterBadgeColor = 'brand' | 'danger' | 'important' | 'informative'

// Badge accepts more colors
type BadgeColor = CounterBadgeColor | 'severe' | 'subtle' | 'success' | 'warning'

export interface NavigationBadgeProps {
  count?: number
  variant?: BadgeVariant
  color?: BadgeColor
  showZero?: boolean
  max?: number
  pulse?: boolean
}

/**
 * Map badge color to counter badge color (fallback for unsupported colors)
 */
function toCounterBadgeColor(color: BadgeColor): CounterBadgeColor {
  switch (color) {
    case 'brand':
    case 'danger':
    case 'important':
    case 'informative':
      return color
    case 'warning':
    case 'severe':
      return 'danger'
    case 'success':
      return 'brand'
    case 'subtle':
    default:
      return 'informative'
  }
}

/**
 * Create a navigation badge for nav items
 */
export function createNavigationBadge({
  count = 0,
  variant = 'count',
  color = 'brand',
  showZero = false,
  max = 99,
  pulse = false,
}: NavigationBadgeProps): React.ReactNode | null {
  // Don't show if count is 0 and showZero is false
  if (count === 0 && !showZero && variant === 'count') {
    return null
  }

  const pulseStyle = pulse ? {
    animation: 'pulse 2s infinite',
  } : {}

  switch (variant) {
    case 'count':
      return (
        <CounterBadge
          count={count}
          color={toCounterBadgeColor(color)}
          size="small"
          overflowCount={max}
          style={pulseStyle}
        />
      )

    case 'dot':
      return (
        <Badge
          appearance="filled"
          color={color}
          size="tiny"
          style={{
            width: 8,
            height: 8,
            minWidth: 8,
            ...pulseStyle,
          }}
        />
      )

    case 'new':
      return (
        <Badge
          appearance="filled"
          color={color}
          size="small"
          style={pulseStyle}
        >
          New
        </Badge>
      )

    case 'status':
      return (
        <Badge
          appearance="tint"
          color={color}
          size="small"
          style={pulseStyle}
        >
          {count}
        </Badge>
      )

    default:
      return null
  }
}

/**
 * Badge for showing issue/warning counts
 */
export function createIssueBadge(
  issues: number,
  warnings: number = 0
): React.ReactNode | null {
  const total = issues + warnings
  if (total === 0) return null

  return (
    <CounterBadge
      count={total}
      color={issues > 0 ? 'danger' : 'important'}
      size="small"
      overflowCount={99}
    />
  )
}

/**
 * Badge for showing scan progress
 */
export function createProgressBadge(
  isScanning: boolean,
  progress?: number
): React.ReactNode | null {
  if (!isScanning) return null

  return (
    <Badge
      appearance="tint"
      color="brand"
      size="small"
      style={{
        animation: 'pulse 2s infinite',
      }}
    >
      {progress !== undefined ? `${Math.round(progress)}%` : '...'}
    </Badge>
  )
}

/**
 * Badge for showing history count
 */
export function createHistoryBadge(count: number): React.ReactNode | null {
  if (count === 0) return null

  return (
    <CounterBadge
      count={count}
      color="informative"
      size="small"
      overflowCount={99}
    />
  )
}
