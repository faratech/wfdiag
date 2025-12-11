import React from 'react'
import {
  Card,
  Caption1,
  Body1Strong,
  ProgressBar,
  tokens,
  makeStyles,
  shorthands,
  Badge
} from '@fluentui/react-components'

const useStyles = makeStyles({
  card: {
    ...shorthands.padding(tokens.spacingVerticalL, tokens.spacingHorizontalL),
    backgroundColor: 'rgba(30, 41, 59, 0.6)',
    backdropFilter: 'blur(10px)',
    ...shorthands.border('1px', 'solid', 'rgba(255, 255, 255, 0.1)'),
    transition: 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)',

    ':hover': {
      backgroundColor: 'rgba(30, 41, 59, 0.8)',
      transform: 'translateY(-2px)',
      boxShadow: '0 8px 24px rgba(59, 130, 246, 0.2)',
    }
  },
  iconContainer: {
    width: '40px',
    height: '40px',
    ...shorthands.borderRadius(tokens.borderRadiusCircular),
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    fontSize: '20px',
    color: tokens.colorNeutralBackground1,
    flexShrink: 0,
  },
  header: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    marginBottom: tokens.spacingVerticalM,
  },
  title: {
    color: tokens.colorNeutralForeground2,
  },
  value: {
    fontSize: tokens.fontSizeBase500,
    fontWeight: tokens.fontWeightBold,
    color: tokens.colorNeutralForeground1,
    marginBottom: tokens.spacingVerticalS,
  },
  subValue: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
    marginTop: tokens.spacingVerticalXS,
  },
  progressContainer: {
    marginTop: tokens.spacingVerticalS,
  },
  trend: {
    display: 'inline-flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
    fontSize: tokens.fontSizeBase200,
  },
  trendUp: {
    color: tokens.colorPaletteGreenForeground1,
  },
  trendDown: {
    color: tokens.colorPaletteRedForeground1,
  },
  badge: {
    marginLeft: tokens.spacingHorizontalS,
  }
})

export interface MetricCardProps {
  title: string
  value: string | number
  subValue?: string
  icon?: React.ReactNode
  color?: string
  utilization?: number
  trend?: 'up' | 'down' | 'stable'
  trendValue?: string
  badge?: string
  badgeColor?: 'brand' | 'danger' | 'important' | 'informative' | 'severe' | 'subtle' | 'success' | 'warning'
  onClick?: () => void
}

export const MetricCard: React.FC<MetricCardProps> = ({
  title,
  value,
  subValue,
  icon,
  color = '#3B82F6',
  utilization,
  trend,
  trendValue,
  badge,
  badgeColor = 'informative',
  onClick
}) => {
  const styles = useStyles()

  const getTrendIcon = () => {
    if (trend === 'up') return '↑'
    if (trend === 'down') return '↓'
    return '→'
  }

  const getTrendStyle = () => {
    if (trend === 'up') return styles.trendUp
    if (trend === 'down') return styles.trendDown
    return ''
  }

  return (
    <Card
      className={styles.card}
      onClick={onClick}
      style={{ cursor: onClick ? 'pointer' : 'default' }}
    >
      <div className={styles.header}>
        <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalM }}>
          {icon && (
            <div
              className={styles.iconContainer}
              style={{ background: `linear-gradient(135deg, ${color}, ${color}dd)` }}
            >
              {icon}
            </div>
          )}
          <div>
            <Caption1 className={styles.title}>
              {title}
              {badge && (
                <Badge
                  appearance="filled"
                  color={badgeColor}
                  className={styles.badge}
                  size="small"
                >
                  {badge}
                </Badge>
              )}
            </Caption1>
            <Body1Strong className={styles.value}>{value}</Body1Strong>
          </div>
        </div>
        {trend && trendValue && (
          <div className={`${styles.trend} ${getTrendStyle()}`}>
            <span>{getTrendIcon()}</span>
            <span>{trendValue}</span>
          </div>
        )}
      </div>

      {subValue && (
        <Caption1 className={styles.subValue}>{subValue}</Caption1>
      )}

      {utilization !== undefined && (
        <div className={styles.progressContainer}>
          <ProgressBar
            value={utilization / 100}
            color={
              utilization > 90 ? 'error' :
              utilization > 70 ? 'warning' :
              'brand'
            }
            thickness="large"
          />
          <Caption1
            style={{
              marginTop: tokens.spacingVerticalXS,
              color: tokens.colorNeutralForeground3
            }}
          >
            {utilization.toFixed(1)}% Utilization
          </Caption1>
        </div>
      )}
    </Card>
  )
}