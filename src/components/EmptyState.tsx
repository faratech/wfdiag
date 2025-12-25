import React from 'react'
import {
  makeStyles,
  shorthands,
  tokens,
  Text,
  Caption1,
  Button,
} from '@fluentui/react-components'
import {
  DocumentSearch24Regular,
  ErrorCircle24Regular,
  Search24Regular,
  CheckmarkCircle24Regular,
  Info24Regular,
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  container: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    textAlign: 'center',
    ...shorthands.padding(tokens.spacingVerticalXXXL, tokens.spacingHorizontalXXL),
    minHeight: '300px',
  },

  iconContainer: {
    width: '80px',
    height: '80px',
    ...shorthands.borderRadius('50%'),
    backgroundColor: tokens.colorNeutralBackground3,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    marginBottom: tokens.spacingVerticalL,
    color: tokens.colorNeutralForeground3,
  },

  iconContainerSuccess: {
    backgroundColor: tokens.colorPaletteGreenBackground2,
    color: tokens.colorPaletteGreenForeground1,
  },

  iconContainerError: {
    backgroundColor: tokens.colorPaletteRedBackground2,
    color: tokens.colorPaletteRedForeground1,
  },

  iconContainerWarning: {
    backgroundColor: tokens.colorPaletteYellowBackground2,
    color: tokens.colorPaletteYellowForeground1,
  },

  title: {
    fontSize: tokens.fontSizeBase500,
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground1,
    marginBottom: tokens.spacingVerticalS,
  },

  description: {
    fontSize: tokens.fontSizeBase300,
    color: tokens.colorNeutralForeground3,
    maxWidth: '400px',
    lineHeight: tokens.lineHeightBase300,
    marginBottom: tokens.spacingVerticalL,
  },

  actions: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
    flexWrap: 'wrap',
  },
})

export interface EmptyStateProps {
  title: string
  description: string
  icon?: React.ReactNode
  variant?: 'default' | 'search' | 'error' | 'success' | 'info'
  primaryAction?: {
    label: string
    onClick: () => void
    icon?: React.ReactNode
  }
  secondaryAction?: {
    label: string
    onClick: () => void
    icon?: React.ReactNode
  }
}

export const EmptyState: React.FC<EmptyStateProps> = ({
  title,
  description,
  icon,
  variant = 'default',
  primaryAction,
  secondaryAction,
}) => {
  const styles = useStyles()

  const getDefaultIcon = () => {
    switch (variant) {
      case 'search':
        return <Search24Regular style={{ width: 40, height: 40 }} />
      case 'error':
        return <ErrorCircle24Regular style={{ width: 40, height: 40 }} />
      case 'success':
        return <CheckmarkCircle24Regular style={{ width: 40, height: 40 }} />
      case 'info':
        return <Info24Regular style={{ width: 40, height: 40 }} />
      default:
        return <DocumentSearch24Regular style={{ width: 40, height: 40 }} />
    }
  }

  const getIconContainerClass = () => {
    switch (variant) {
      case 'success':
        return `${styles.iconContainer} ${styles.iconContainerSuccess}`
      case 'error':
        return `${styles.iconContainer} ${styles.iconContainerError}`
      case 'search':
        return `${styles.iconContainer} ${styles.iconContainerWarning}`
      default:
        return styles.iconContainer
    }
  }

  return (
    <div className={styles.container}>
      <div className={getIconContainerClass()}>{icon || getDefaultIcon()}</div>

      <Text className={styles.title}>{title}</Text>

      <Caption1 className={styles.description}>{description}</Caption1>

      {(primaryAction || secondaryAction) && (
        <div className={styles.actions}>
          {primaryAction && (
            <Button appearance="primary" onClick={primaryAction.onClick}>
              {primaryAction.label}
            </Button>
          )}
          {secondaryAction && (
            <Button appearance="secondary" onClick={secondaryAction.onClick}>
              {secondaryAction.label}
            </Button>
          )}
        </div>
      )}
    </div>
  )
}
