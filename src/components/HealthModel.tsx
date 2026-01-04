import React from 'react'
import {
  Text,
  makeStyles,
  shorthands,
  tokens
} from '@fluentui/react-components'
import {
  CheckmarkCircle16Filled,
  Warning16Filled,
  ErrorCircle16Filled,
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  container: {
    display: 'grid',
    gridTemplateColumns: 'repeat(auto-fit, minmax(180px, 1fr))',
    gap: tokens.spacingHorizontalM,
    marginBottom: tokens.spacingVerticalL,
  },
  healthCard: {
    backgroundColor: tokens.colorNeutralBackground1,
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke2),
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalM),
    display: 'flex',
    flexDirection: 'column',
    transition: 'all 0.15s ease-out',
    cursor: 'pointer',
    position: 'relative',
    overflow: 'hidden',
    ':hover': {
      backgroundColor: tokens.colorNeutralBackground1Hover,
      ...shorthands.borderColor(tokens.colorNeutralStroke1),
      boxShadow: tokens.shadow4,
    },
    ':active': {
      transform: 'scale(0.98)',
    },
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
  },
  cardActive: {
    ...shorthands.borderColor(tokens.colorBrandStroke1),
    backgroundColor: tokens.colorNeutralBackground1Selected,
    boxShadow: `0 0 0 1px ${tokens.colorBrandStroke1}`,
  },
  cardHeader: {
    display: 'flex',
    alignItems: 'center',
    gap: tokens.spacingHorizontalS,
    marginBottom: tokens.spacingVerticalS,
  },
  iconContainer: {
    width: '28px',
    height: '28px',
    borderRadius: '6px',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: tokens.colorNeutralBackground3,
    color: tokens.colorNeutralForeground2,
    flexShrink: 0,
  },
  labelText: {
    fontSize: tokens.fontSizeBase200,
    fontWeight: 500,
    color: tokens.colorNeutralForeground2,
    letterSpacing: '0.01em',
    lineHeight: 1.2,
    flex: 1,
  },
  meterContainer: {
    display: 'flex',
    alignItems: 'center',
    gap: tokens.spacingHorizontalS,
    marginTop: 'auto',
  },
  meterTrack: {
    flex: 1,
    height: '6px',
    backgroundColor: tokens.colorNeutralBackground3,
    borderRadius: '3px',
    overflow: 'hidden',
    position: 'relative',
  },
  meterFill: {
    height: '100%',
    borderRadius: '3px',
    transition: 'width 0.6s cubic-bezier(0.16, 1, 0.3, 1)',
  },
  statusIndicator: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    width: '18px',
    height: '18px',
    flexShrink: 0,
  },
  // Status-specific fills
  fillLow: {
    background: `linear-gradient(90deg, ${tokens.colorPaletteGreenBackground1} 0%, ${tokens.colorPaletteGreenForeground1} 100%)`,
  },
  fillMonitor: {
    background: `linear-gradient(90deg, ${tokens.colorPaletteYellowBackground1} 0%, ${tokens.colorPaletteYellowForeground1} 100%)`,
  },
  fillElevated: {
    background: `linear-gradient(90deg, ${tokens.colorPaletteRedBackground1} 0%, ${tokens.colorPaletteRedForeground1} 100%)`,
  },
})

interface HealthMetric {
  id: string
  label: string
  score: number
  risk: 'Low' | 'Monitor' | 'Elevated'
  icon: React.ReactNode
}

interface HealthModelProps {
  metrics: HealthMetric[]
  onMetricClick: (id: string) => void
  activeMetricId?: string | null
}

export const HealthModel: React.FC<HealthModelProps> = ({
  metrics,
  onMetricClick,
  activeMetricId
}) => {
  const styles = useStyles()

  const getFillClass = (risk: string) => {
    switch (risk) {
      case 'Low': return styles.fillLow
      case 'Monitor': return styles.fillMonitor
      case 'Elevated': return styles.fillElevated
      default: return styles.fillLow
    }
  }

  const getStatusIcon = (risk: string) => {
    switch (risk) {
      case 'Low':
        return <CheckmarkCircle16Filled primaryFill={tokens.colorPaletteGreenForeground1} />
      case 'Monitor':
        return <Warning16Filled primaryFill={tokens.colorPaletteYellowForeground1} />
      case 'Elevated':
        return <ErrorCircle16Filled primaryFill={tokens.colorPaletteRedForeground1} />
      default:
        return <CheckmarkCircle16Filled primaryFill={tokens.colorPaletteGreenForeground1} />
    }
  }

  return (
    <div className={styles.container}>
      {metrics.map((metric) => {
        const isActive = activeMetricId === metric.id
        return (
          <div
            key={metric.id}
            className={`${styles.healthCard} ${isActive ? styles.cardActive : ''}`}
            onClick={() => onMetricClick(metric.id)}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault()
                onMetricClick(metric.id)
              }
            }}
            aria-pressed={isActive}
          >
            <div className={styles.cardHeader}>
              <div className={styles.iconContainer}>
                {metric.icon}
              </div>
              <Text className={styles.labelText}>
                {metric.label}
              </Text>
            </div>

            <div className={styles.meterContainer}>
              <div className={styles.meterTrack}>
                <div
                  className={`${styles.meterFill} ${getFillClass(metric.risk)}`}
                  style={{ width: `${metric.score}%` }}
                />
              </div>
              <div className={styles.statusIndicator}>
                {getStatusIcon(metric.risk)}
              </div>
            </div>
          </div>
        )
      })}
    </div>
  )
}
