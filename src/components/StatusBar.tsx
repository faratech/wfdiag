import React from 'react'
import {
  makeStyles,
  tokens,
  shorthands,
  Text
} from '@fluentui/react-components'
import {
  CheckmarkCircle12Filled,
  ErrorCircle12Filled,
  Warning12Filled,
  Clock12Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  statusBar: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    ...shorthands.padding('4px', '12px'),
    backgroundColor: tokens.colorNeutralBackground3,
    borderTop: `1px solid ${tokens.colorNeutralStroke1}`,
    minHeight: '22px',
    fontSize: tokens.fontSizeBase100,
    color: tokens.colorNeutralForeground2,
  },
  left: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('16px'),
  },
  right: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('16px'),
  },
  stat: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('4px'),
  },
  icon: {
    width: '12px',
    height: '12px',
  },
})

interface StatusBarProps {
  passed?: number
  failed?: number
  warnings?: number
  duration?: number
  isAdmin?: boolean
  currentTask?: string
}

export const StatusBar: React.FC<StatusBarProps> = ({
  passed = 0,
  failed = 0,
  warnings = 0,
  duration,
  isAdmin,
  currentTask
}) => {
  const styles = useStyles()

  const formatDuration = (ms: number) => {
    const seconds = Math.round(ms / 1000)
    if (seconds < 60) return `${seconds}s`
    const minutes = Math.floor(seconds / 60)
    const secs = seconds % 60
    return `${minutes}m ${secs}s`
  }

  return (
    <div className={styles.statusBar}>
      <div className={styles.left}>
        {currentTask ? (
          <Text>{currentTask}</Text>
        ) : (
          <>
            <div className={styles.stat}>
              <CheckmarkCircle12Filled className={styles.icon} style={{ color: tokens.colorPaletteGreenForeground1 }} />
              <Text>{passed} passed</Text>
            </div>
            {failed > 0 && (
              <div className={styles.stat}>
                <ErrorCircle12Filled className={styles.icon} style={{ color: tokens.colorPaletteRedForeground1 }} />
                <Text>{failed} failed</Text>
              </div>
            )}
            {warnings > 0 && (
              <div className={styles.stat}>
                <Warning12Filled className={styles.icon} style={{ color: tokens.colorPaletteYellowForeground1 }} />
                <Text>{warnings} warnings</Text>
              </div>
            )}
          </>
        )}
      </div>
      <div className={styles.right}>
        {duration !== undefined && duration > 0 && (
          <div className={styles.stat}>
            <Clock12Regular className={styles.icon} />
            <Text>{formatDuration(duration)}</Text>
          </div>
        )}
        <Text>{isAdmin ? 'Administrator' : 'Standard User'}</Text>
      </div>
    </div>
  )
}

export default StatusBar
