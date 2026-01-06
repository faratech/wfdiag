import React from 'react'
import {
  makeStyles,
  shorthands,
  tokens,
  Text,
  Caption1,
  Body1,
  Tooltip
} from '@fluentui/react-components'
import {
  CheckmarkCircle12Regular,
  ErrorCircle12Regular,
  Warning12Regular,
  Info12Regular
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
      backgroundColor: tokens.colorSubtleBackgroundHover,
    },

    '&::after': {
      content: '""',
      position: 'absolute',
      bottom: 0,
      left: tokens.spacingHorizontalM,
      right: tokens.spacingHorizontalM,
      height: '1px',
      backgroundColor: tokens.colorNeutralStroke2,
      opacity: 0.5,
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
    background: 'var(--theme-card-gradient)',
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke2),
    transition: 'all 0.2s ease',

    '&:hover': {
      transform: 'translateY(-2px)',
      boxShadow: tokens.shadow8,
    }
  },

  metricValue: {
    fontSize: tokens.fontSizeHero700,
    fontWeight: tokens.fontWeightBold,
    color: tokens.colorBrandForeground1,
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
    color: tokens.colorPaletteGreenForeground1,
  },

  trendDown: {
    color: tokens.colorPaletteRedForeground1,
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
    color: tokens.colorNeutralForeground1,
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
    backgroundColor: tokens.colorNeutralBackground4,
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke2),
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
      background: 'var(--theme-accent-gradient)',
      opacity: 0.5,
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
        return <CheckmarkCircle12Regular primaryFill={tokens.colorPaletteGreenForeground1} />
      case 'error':
        return <ErrorCircle12Regular primaryFill={tokens.colorPaletteRedForeground1} />
      case 'warning':
        return <Warning12Regular primaryFill={tokens.colorPaletteYellowForeground1} />
      case 'info':
        return <Info12Regular primaryFill={tokens.colorPaletteBlueForeground2} />
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
        return <CheckmarkCircle12Regular primaryFill={tokens.colorPaletteGreenForeground1} />
      case 'error':
        return <ErrorCircle12Regular primaryFill={tokens.colorPaletteRedForeground1} />
      case 'warning':
        return <Warning12Regular primaryFill={tokens.colorPaletteYellowForeground1} />
      case 'info':
        return <Info12Regular primaryFill={tokens.colorPaletteBlueForeground2} />
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