/**
 * Shared scan status indicator used by CommandBar and CompactToolbar
 */

import React from 'react'
import {
  makeStyles,
  tokens,
  shorthands,
  ProgressBar,
  Text,
} from '@fluentui/react-components'
import { CheckmarkCircle20Regular } from '@fluentui/react-icons'

const useStyles = makeStyles({
  // Full variant styles
  statusIndicator: {
    display: 'inline-flex',
    alignItems: 'center',
    ...shorthands.gap('4px'),
    ...shorthands.padding('2px', '8px'),
    ...shorthands.borderRadius('4px'),
    fontSize: '11px',
  },
  statusActive: {
    backgroundColor: 'rgba(80, 180, 80, 0.2)',
    color: 'rgb(80, 180, 80)',
  },
  statusInactive: {
    backgroundColor: tokens.colorNeutralBackground3,
    color: tokens.colorNeutralForeground3,
  },
  statusError: {
    backgroundColor: 'rgba(239, 68, 68, 0.2)',
    color: '#EF4444',
  },

  // Compact variant styles
  progressContainer: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('8px'),
    flex: 1,
    maxWidth: '300px',
  },
  progressBar: {
    flex: 1,
  },
  progressText: {
    fontSize: tokens.fontSizeBase100,
    color: tokens.colorNeutralForeground2,
    whiteSpace: 'nowrap',
  },
  statusText: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground2,
  },
})

export type ScanStatus = 'idle' | 'scanning' | 'complete' | 'error'

export interface ScanStatusIndicatorProps {
  status: ScanStatus
  resultCount?: number
  progress?: number
  variant?: 'compact' | 'full'
}

/**
 * Compact variant - shows progress bar when scanning, text otherwise
 */
const CompactStatusIndicator: React.FC<ScanStatusIndicatorProps> = ({
  status,
  resultCount = 0,
  progress = 0,
}) => {
  const styles = useStyles()

  if (status === 'scanning') {
    return (
      <div className={styles.progressContainer}>
        <ProgressBar value={progress / 100} className={styles.progressBar} />
        <Text className={styles.progressText}>
          {Math.round(progress)}%
        </Text>
      </div>
    )
  }

  return (
    <Text className={styles.statusText}>
      {resultCount > 0 ? `${resultCount} checks completed` : 'Ready'}
    </Text>
  )
}

/**
 * Full variant - badge-style status indicator
 */
const FullStatusIndicator: React.FC<ScanStatusIndicatorProps> = ({
  status,
  resultCount = 0,
}) => {
  const styles = useStyles()

  switch (status) {
    case 'scanning':
      return (
        <div className={`${styles.statusIndicator} ${styles.statusActive}`}>
          <div style={{ animation: 'pulse 2s infinite' }}>●</div>
          Scanning...
        </div>
      )
    case 'complete':
      return (
        <div className={`${styles.statusIndicator} ${styles.statusActive}`}>
          <CheckmarkCircle20Regular />
          {resultCount} Results
        </div>
      )
    case 'error':
      return (
        <div className={`${styles.statusIndicator} ${styles.statusError}`}>
          Error
        </div>
      )
    default:
      return (
        <div className={`${styles.statusIndicator} ${styles.statusInactive}`}>
          Ready
        </div>
      )
  }
}

export const ScanStatusIndicator: React.FC<ScanStatusIndicatorProps> = (props) => {
  const { variant = 'compact' } = props

  if (variant === 'full') {
    return <FullStatusIndicator {...props} />
  }

  return <CompactStatusIndicator {...props} />
}

export default ScanStatusIndicator
