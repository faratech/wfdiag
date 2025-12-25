import React from 'react'
import {
  makeStyles,
  tokens,
  Text
} from '@fluentui/react-components'
import { CheckmarkCircle24Regular, Warning24Regular, ErrorCircle24Regular } from '@fluentui/react-icons'

const useStyles = makeStyles({
  container: {
    marginBottom: tokens.spacingVerticalXL,
    display: 'flex',
    flexDirection: 'column',
    gap: tokens.spacingVerticalS,
  },
  headerRow: {
    display: 'flex',
    alignItems: 'center',
    gap: tokens.spacingHorizontalM,
  },
  statusIcon: {
    color: '#4ade80', // Default success green
  },
  summaryText: {
    fontSize: tokens.fontSizeBase500,
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground1,
    lineHeight: 1.4,
  },
  detailText: {
    fontSize: tokens.fontSizeBase300,
    color: tokens.colorNeutralForeground3,
    maxWidth: '800px',
    lineHeight: 1.5,
  }
})

interface SystemSummaryProps {
  taskCount: number
  durationMs: number
  faultCount: number
  warningCount: number
  completed: boolean
}

export const SystemSummary: React.FC<SystemSummaryProps> = ({
  taskCount,
  durationMs,
  faultCount,
  warningCount,
  completed
}) => {
  const styles = useStyles()

  if (!completed) {
    return null
  }

  const durationSeconds = (durationMs / 1000).toFixed(1)
  
  let statusIcon = <CheckmarkCircle24Regular className={styles.statusIcon} />
  let summaryMessage = `Diagnostics completed in ${durationSeconds} seconds.`
  let detailMessage = "System configuration aligns with recommended Windows standards. No user action required."

  if (faultCount > 0) {
    statusIcon = <ErrorCircle24Regular className={styles.statusIcon} style={{ color: tokens.colorPaletteRedForeground1 }} />
    summaryMessage = `${faultCount} critical issues detected.`
    detailMessage = "System stability may be compromised. Immediate attention recommended for the highlighted issues below."
  } else if (warningCount > 0) {
    statusIcon = <Warning24Regular className={styles.statusIcon} style={{ color: tokens.colorPaletteYellowForeground1 }} />
    summaryMessage = "No critical faults, but potential optimization areas detected."
    detailMessage = "System is stable, but some configurations deviate from optimal baselines. Review recommended adjustments."
  }

  return (
    <div className={styles.container}>
      <div className={styles.headerRow}>
        {statusIcon}
        <Text className={styles.summaryText}>
          {taskCount} diagnostics completed. {summaryMessage}
        </Text>
      </div>
      <Text className={styles.detailText} style={{ paddingLeft: '40px' }}>
        {detailMessage}
      </Text>
    </div>
  )
}