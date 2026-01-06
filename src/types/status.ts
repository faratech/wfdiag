/**
 * Shared status types and utilities
 * Used by StatusBadge, StatusBar, StatusCard, and other status-related components
 */

import { tokens } from '@fluentui/react-components'

/**
 * Standard status variants used across the application
 */
export type StatusVariant = 'success' | 'error' | 'warning' | 'info'

/**
 * Extended status for diagnostic results
 */
export type DiagnosticStatus = 'success' | 'error' | 'warning' | 'running' | 'pending'

/**
 * Color configuration for status variants
 */
export interface StatusColors {
  foreground: string
  background: string
}

/**
 * Get Fluent UI badge color for status variant
 */
export function getStatusBadgeColor(status: StatusVariant): 'success' | 'danger' | 'warning' | 'informative' {
  switch (status) {
    case 'success': return 'success'
    case 'error': return 'danger'
    case 'warning': return 'warning'
    case 'info':
    default: return 'informative'
  }
}

/**
 * Get foreground color token for status variant
 */
export function getStatusForegroundColor(status: StatusVariant): string {
  switch (status) {
    case 'success': return tokens.colorPaletteGreenForeground1
    case 'error': return tokens.colorPaletteRedForeground1
    case 'warning': return tokens.colorPaletteYellowForeground1
    case 'info':
    default: return tokens.colorPaletteBlueForeground2
  }
}

/**
 * Get background color token for status variant (subtle)
 */
export function getStatusBackgroundColor(status: StatusVariant): string {
  switch (status) {
    case 'success': return tokens.colorPaletteGreenBackground1
    case 'error': return tokens.colorPaletteRedBackground1
    case 'warning': return tokens.colorPaletteYellowBackground1
    case 'info':
    default: return tokens.colorPaletteBlueBackground2
  }
}

/**
 * Convert diagnostic status to display status variant
 */
export function diagnosticStatusToVariant(status: DiagnosticStatus): StatusVariant {
  switch (status) {
    case 'success': return 'success'
    case 'error': return 'error'
    case 'warning': return 'warning'
    case 'running':
    case 'pending':
    default: return 'info'
  }
}
