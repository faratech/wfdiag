import React from 'react'
import {
  makeStyles,
  shorthands,
  tokens,
  Text,
  Caption1,
  Caption2,
  Body1,
  Tooltip,
  ProgressBar
} from '@fluentui/react-components'
import {
  CheckmarkCircle12Regular,
  ErrorCircle12Regular,
  Warning12Regular,
  Info12Regular,
  ArrowUp12Regular,
  ArrowDown12Regular,
  ArrowRight12Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  dataRow: {
    display: 'grid',
    gridTemplateColumns: 'minmax(140px, 2fr) 3fr auto',
    alignItems: 'center',
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
    ...shorthands.borderRadius(tokens.borderRadiusSmall),
    transition: 'all 0.15s ease',
    position: 'relative',

    '&:hover': {
      backgroundColor: 'rgba(148, 163, 184, 0.03)',
    },

    '&::after': {
      content: '""',
      position: 'absolute',
      bottom: 0,
      left: tokens.spacingHorizontalM,
      right: tokens.spacingHorizontalM,
      height: '1px',
      background: 'linear-gradient(90deg, transparent, rgba(148, 163, 184, 0.05), transparent)',
    }
  },

  dataLabel: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase200,
    fontWeight: tokens.fontWeightRegular,
  },

  dataValue: {
    color: tokens.colorNeutralForeground1,
    fontSize: tokens.fontSizeBase300,
    fontWeight: tokens.fontWeightMedium,
    fontFamily: tokens.fontFamilyMonospace,
  },

  dataStatus: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
  },

  compactGrid: {
    display: 'grid',
    gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))',
    ...shorthands.gap(tokens.spacingVerticalS, tokens.spacingHorizontalM),
  },

  metricCard: {
    ...shorthands.padding(tokens.spacingVerticalM),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    background: 'linear-gradient(135deg, rgba(17, 24, 39, 0.5), rgba(31, 41, 55, 0.3))',
    ...shorthands.border('1px', 'solid', 'rgba(148, 163, 184, 0.06)'),
    transition: 'all 0.2s ease',

    '&:hover': {
      transform: 'translateY(-2px)',
      boxShadow: '0 4px 12px rgba(0, 0, 0, 0.1)',
    }
  },

  metricValue: {
    fontSize: tokens.fontSizeHero700,
    fontWeight: tokens.fontWeightBold,
    background: 'linear-gradient(135deg, #60A5FA, #3B82F6)',
    WebkitBackgroundClip: 'text',
    WebkitTextFillColor: 'transparent',
    marginBottom: tokens.spacingVerticalXS,
  },

  metricLabel: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
    textTransform: 'uppercase',
    letterSpacing: '0.05em',
  },

  metricTrend: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
    marginTop: tokens.spacingVerticalS,
  },

  trendUp: {
    color: '#10B981',
  },

  trendDown: {
    color: '#EF4444',
  },

  trendNeutral: {
    color: tokens.colorNeutralForeground3,
  },

  progressContainer: {
    marginTop: tokens.spacingVerticalS,
  },

  progressLabel: {
    display: 'flex',
    justifyContent: 'space-between',
    marginBottom: tokens.spacingVerticalXS,
  },

  listItem: {
    display: 'flex',
    alignItems: 'flex-start',
    ...shorthands.gap(tokens.spacingHorizontalS),
    ...shorthands.padding(tokens.spacingVerticalXS, 0),
  },

  listIcon: {
    marginTop: '2px',
    flexShrink: 0,
  },

  listContent: {
    flex: 1,
  },

  codeBlock: {
    fontFamily: 'Consolas, "Courier New", monospace',
    fontSize: tokens.fontSizeBase200,
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
    backgroundColor: 'rgba(0, 0, 0, 0.4)',
    ...shorthands.border('1px', 'solid', 'rgba(148, 163, 184, 0.1)'),
    ...shorthands.borderRadius(tokens.borderRadiusSmall),
    ...shorthands.overflow('auto'),
    maxHeight: '200px',
    position: 'relative',

    '&::before': {
      content: '""',
      position: 'absolute',
      top: 0,
      left: 0,
      right: 0,
      height: '2px',
      background: 'linear-gradient(90deg, #3B82F6, #8B5CF6, #3B82F6)',
      opacity: 0.3,
    }
  }
})

interface DataRowProps {
  label: string
  value: string | number | React.ReactNode
  status?: 'success' | 'error' | 'warning' | 'info'
  icon?: React.ReactNode
}

export const DataRow: React.FC<DataRowProps> = ({ label, value, status, icon }) => {
  const styles = useStyles()

  const getStatusIcon = () => {
    switch (status) {
      case 'success':
        return <CheckmarkCircle12Regular style={{ color: '#10B981' }} />
      case 'error':
        return <ErrorCircle12Regular style={{ color: '#EF4444' }} />
      case 'warning':
        return <Warning12Regular style={{ color: '#F59E0B' }} />
      case 'info':
        return <Info12Regular style={{ color: '#3B82F6' }} />
      default:
        return null
    }
  }

  return (
    <div className={styles.dataRow}>
      <div className={styles.dataLabel}>
        {icon}
        <Text>{label}</Text>
      </div>
      <div className={styles.dataValue}>{value}</div>
      {status && (
        <div className={styles.dataStatus}>
          {getStatusIcon()}
        </div>
      )}
    </div>
  )
}

interface MetricCardProps {
  value: string | number
  label: string
  trend?: 'up' | 'down' | 'neutral'
  trendValue?: string
  progress?: number
  color?: 'brand' | 'success' | 'warning' | 'error'
}

export const MetricCard: React.FC<MetricCardProps> = ({
  value,
  label,
  trend,
  trendValue,
  progress,
  color = 'brand'
}) => {
  const styles = useStyles()

  const getTrendIcon = () => {
    switch (trend) {
      case 'up':
        return <ArrowUp12Regular className={styles.trendUp} />
      case 'down':
        return <ArrowDown12Regular className={styles.trendDown} />
      default:
        return <ArrowRight12Regular className={styles.trendNeutral} />
    }
  }

  return (
    <div className={styles.metricCard}>
      <div className={styles.metricValue}>{value}</div>
      <Caption1 className={styles.metricLabel}>{label}</Caption1>

      {trend && trendValue && (
        <div className={styles.metricTrend}>
          {getTrendIcon()}
          <Caption2>{trendValue}</Caption2>
        </div>
      )}

      {progress !== undefined && (
        <div className={styles.progressContainer}>
          <div className={styles.progressLabel}>
            <Caption2>Progress</Caption2>
            <Caption2>{Math.round(progress)}%</Caption2>
          </div>
          <ProgressBar value={progress / 100} color={color} thickness="medium" />
        </div>
      )}
    </div>
  )
}

interface DataListProps {
  items: Array<{
    icon?: React.ReactNode
    title: string
    description?: string
    status?: 'success' | 'error' | 'warning' | 'info'
  }>
}

export const DataList: React.FC<DataListProps> = ({ items }) => {
  const styles = useStyles()

  const getStatusIcon = (status?: string) => {
    switch (status) {
      case 'success':
        return <CheckmarkCircle12Regular style={{ color: '#10B981' }} />
      case 'error':
        return <ErrorCircle12Regular style={{ color: '#EF4444' }} />
      case 'warning':
        return <Warning12Regular style={{ color: '#F59E0B' }} />
      case 'info':
        return <Info12Regular style={{ color: '#3B82F6' }} />
      default:
        return null
    }
  }

  return (
    <div>
      {items.map((item, index) => (
        <div key={index} className={styles.listItem}>
          <div className={styles.listIcon}>
            {item.icon || getStatusIcon(item.status)}
          </div>
          <div className={styles.listContent}>
            <Body1 style={{ marginBottom: tokens.spacingVerticalXXS }}>
              {item.title}
            </Body1>
            {item.description && (
              <Caption1 style={{ color: tokens.colorNeutralForeground3 }}>
                {item.description}
              </Caption1>
            )}
          </div>
        </div>
      ))}
    </div>
  )
}

interface CodeDisplayProps {
  code: string
  language?: string
}

export const CodeDisplay: React.FC<CodeDisplayProps> = ({ code, language }) => {
  const styles = useStyles()

  return (
    <Tooltip content={`${language || 'Code'} output`} relationship="label">
      <pre className={styles.codeBlock}>
        <code>{code}</code>
      </pre>
    </Tooltip>
  )
}

export const DataGrid: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const styles = useStyles()
  return <div className={styles.compactGrid}>{children}</div>
}