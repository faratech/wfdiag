import React from 'react'
import {
  Text,
  makeStyles,
  shorthands,
  tokens
} from '@fluentui/react-components'

const useStyles = makeStyles({
  container: {
    display: 'grid',
    gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))',
    gap: tokens.spacingHorizontalL,
    marginBottom: tokens.spacingVerticalXL,
  },
  healthCard: {
    backgroundColor: tokens.colorNeutralBackground1,
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke2),
    ...shorthands.padding(tokens.spacingVerticalL),
    display: 'flex',
    flexDirection: 'column',
    transition: 'all 0.2s cubic-bezier(0.33, 1, 0.68, 1)',
    cursor: 'pointer',
    ':hover': {
      transform: 'translateY(-2px)',
      backgroundColor: tokens.colorNeutralBackground1Hover,
      ...shorthands.borderColor(tokens.colorBrandStroke1),
      boxShadow: tokens.shadow8,
    },
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
  },
  cardHeader: {
    display: 'flex',
    alignItems: 'center',
    gap: tokens.spacingHorizontalM,
    marginBottom: tokens.spacingVerticalM,
  },
  iconBox: {
    width: '32px',
    height: '32px',
    borderRadius: '8px',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: tokens.colorNeutralBackgroundAlpha,
    color: tokens.colorBrandForeground1,
  },
  scoreContainer: {
    display: 'flex',
    alignItems: 'baseline',
    gap: tokens.spacingHorizontalXS,
    marginBottom: tokens.spacingVerticalXS,
  },
  scoreValue: {
    fontSize: '28px',
    fontWeight: 700,
    color: tokens.colorNeutralForeground1,
    lineHeight: 1,
  },
  scoreLabel: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
    fontWeight: 600,
    textTransform: 'uppercase',
    letterSpacing: '0.05em',
  },
  riskBadge: {
    marginLeft: 'auto',
    padding: '2px 8px',
    borderRadius: '12px',
    fontSize: '11px',
    fontWeight: 700,
    textTransform: 'uppercase',
    letterSpacing: '0.05em',
  },
  riskLow: {
    backgroundColor: tokens.colorPaletteGreenBackground2,
    color: tokens.colorPaletteGreenForeground1,
    border: `1px solid ${tokens.colorPaletteGreenBorder2}`,
  },
  riskMonitor: {
    backgroundColor: tokens.colorPaletteYellowBackground2,
    color: tokens.colorPaletteYellowForeground1,
    border: `1px solid ${tokens.colorPaletteYellowBorder2}`,
  },
  riskElevated: {
    backgroundColor: tokens.colorPaletteRedBackground2,
    color: tokens.colorPaletteRedForeground1,
    border: `1px solid ${tokens.colorPaletteRedBorder2}`,
  },
  progressTrack: {
    height: '4px',
    backgroundColor: tokens.colorNeutralStroke2,
    borderRadius: '2px',
    overflow: 'hidden',
    marginTop: tokens.spacingVerticalS,
  },
  progressBar: {
    height: '100%',
    borderRadius: '2px',
    transition: 'width 1s cubic-bezier(0.33, 1, 0.68, 1)',
  }
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

  const getRiskStyle = (risk: string) => {
    switch (risk) {
      case 'Low': return styles.riskLow
      case 'Monitor': return styles.riskMonitor
      case 'Elevated': return styles.riskElevated
      default: return styles.riskLow
    }
  }

  const getProgressColor = (score: number) => {
    if (score >= 90) return tokens.colorPaletteGreenBackground1
    if (score >= 70) return tokens.colorPaletteYellowBackground1
    return tokens.colorPaletteRedBackground1
  }

  return (
    <div className={styles.container}>
      {metrics.map((metric) => (
        <div
          key={metric.id}
          className={styles.healthCard}
          onClick={() => onMetricClick(metric.id)}
          style={{
            borderColor: activeMetricId === metric.id ? tokens.colorBrandStroke1 : tokens.colorNeutralStroke2,
            backgroundColor: activeMetricId === metric.id ? tokens.colorNeutralBackground1Selected : undefined,
          }}
        >
          <div className={styles.cardHeader}>
            <div className={styles.iconBox}>
              {metric.icon}
            </div>
            <Text weight="semibold" style={{ color: tokens.colorNeutralForeground2 }}>
              {metric.label}
            </Text>
            <div className={`${styles.riskBadge} ${getRiskStyle(metric.risk)}`}>
              {metric.risk}
            </div>
          </div>

          <div className={styles.scoreContainer}>
            <span className={styles.scoreValue}>{metric.score}</span>
            <span className={styles.scoreLabel}>/ 100</span>
          </div>

          <div className={styles.progressTrack}>
            <div
              className={styles.progressBar}
              style={{
                width: `${metric.score}%`,
                backgroundColor: getProgressColor(metric.score)
              }}
            />
          </div>
        </div>
      ))}
    </div>
  )
}