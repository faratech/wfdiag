import React from 'react'
import {
  makeStyles,
  shorthands,
  tokens,
  Text,
  Caption1,
  Button,
} from '@fluentui/react-components'
import { AI_ACCENT_GRADIENT } from '../theme'

const useStyles = makeStyles({
  hero: {
    position: 'relative',
    ...shorthands.padding(tokens.spacingVerticalXXL, tokens.spacingHorizontalXXL),
    ...shorthands.borderRadius(tokens.borderRadiusXLarge),
    background: 'var(--theme-card-gradient)',
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke1),
    boxShadow: tokens.shadow8,
    overflow: 'hidden',
    marginBottom: tokens.spacingVerticalXL,

    // Hero gradient overlay
    '&::before': {
      content: '""',
      position: 'absolute',
      top: 0,
      left: 0,
      right: 0,
      bottom: 0,
      background: 'var(--theme-hero-gradient)',
      pointerEvents: 'none',
    },
  },

  heroCompact: {
    ...shorthands.padding(tokens.spacingVerticalL, tokens.spacingHorizontalXL),
  },

  heroMinimal: {
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalL),
    backgroundColor: 'transparent',
    boxShadow: 'none',
    ...shorthands.border('none'),

    '&::before': {
      display: 'none',
    },
  },

  content: {
    position: 'relative',
    zIndex: 1,
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    textAlign: 'center',
    ...shorthands.gap(tokens.spacingVerticalL),
  },

  iconContainer: {
    width: '80px',
    height: '80px',
    ...shorthands.borderRadius('50%'),
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    boxShadow: '0 8px 24px rgba(51, 95, 227, 0.35)',
    flexShrink: 0,
  },

  iconContainerCompact: {
    width: '56px',
    height: '56px',
  },

  iconContainerMinimal: {
    width: '48px',
    height: '48px',
    boxShadow: '0 4px 12px rgba(51, 95, 227, 0.25)',
  },

  title: {
    fontSize: tokens.fontSizeHero800,
    fontWeight: tokens.fontWeightBold,
    color: tokens.colorNeutralForeground1,
    lineHeight: tokens.lineHeightHero800,
    marginTop: tokens.spacingVerticalS,
  },

  titleCompact: {
    fontSize: tokens.fontSizeBase600,
  },

  titleMinimal: {
    fontSize: tokens.fontSizeBase500,
  },

  description: {
    fontSize: tokens.fontSizeBase400,
    color: tokens.colorNeutralForeground2,
    maxWidth: '600px',
    lineHeight: tokens.lineHeightBase400,
  },

  descriptionCompact: {
    fontSize: tokens.fontSizeBase300,
  },

  actions: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    flexWrap: 'wrap',
    ...shorthands.gap(tokens.spacingHorizontalM),
    marginTop: tokens.spacingVerticalM,
  },

  primaryButton: {
    minWidth: '140px',
    height: '44px',
    fontSize: tokens.fontSizeBase400,
  },

  secondaryButton: {
    minWidth: '140px',
    height: '44px',
    fontSize: tokens.fontSizeBase400,
  },

  stats: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    flexWrap: 'wrap',
    ...shorthands.gap(tokens.spacingHorizontalXL),
    marginTop: tokens.spacingVerticalL,
    ...shorthands.padding(tokens.spacingVerticalM),
    backgroundColor: tokens.colorNeutralBackground3,
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
  },

  statItem: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingVerticalXXS),
    minWidth: '80px',
  },

  statValue: {
    fontSize: tokens.fontSizeBase600,
    fontWeight: tokens.fontWeightBold,
    color: tokens.colorBrandForeground1,
  },

  statLabel: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
    textTransform: 'uppercase',
    letterSpacing: '0.05em',
  },
})

export interface HeroSectionProps {
  title: string
  description: string
  icon?: React.ReactNode
  iconGradient?: string
  primaryAction?: {
    label: string
    onClick: () => void
    icon?: React.ReactNode
    disabled?: boolean
  }
  secondaryAction?: {
    label: string
    onClick: () => void
    icon?: React.ReactNode
    disabled?: boolean
  }
  stats?: Array<{
    label: string
    value: string | number
  }>
  variant?: 'default' | 'compact' | 'minimal'
}

export const HeroSection: React.FC<HeroSectionProps> = ({
  title,
  description,
  icon,
  iconGradient = AI_ACCENT_GRADIENT,
  primaryAction,
  secondaryAction,
  stats,
  variant = 'default',
}) => {
  const styles = useStyles()

  const getHeroClass = () => {
    switch (variant) {
      case 'compact':
        return `${styles.hero} ${styles.heroCompact}`
      case 'minimal':
        return `${styles.hero} ${styles.heroMinimal}`
      default:
        return styles.hero
    }
  }

  const getIconContainerClass = () => {
    switch (variant) {
      case 'compact':
        return `${styles.iconContainer} ${styles.iconContainerCompact}`
      case 'minimal':
        return `${styles.iconContainer} ${styles.iconContainerMinimal}`
      default:
        return styles.iconContainer
    }
  }

  const getTitleClass = () => {
    switch (variant) {
      case 'compact':
        return `${styles.title} ${styles.titleCompact}`
      case 'minimal':
        return `${styles.title} ${styles.titleMinimal}`
      default:
        return styles.title
    }
  }

  const getDescriptionClass = () => {
    switch (variant) {
      case 'compact':
        return `${styles.description} ${styles.descriptionCompact}`
      default:
        return styles.description
    }
  }

  return (
    <div className={getHeroClass()}>
      <div className={styles.content}>
        {icon && (
          <div className={getIconContainerClass()} style={{ background: iconGradient }}>
            {icon}
          </div>
        )}

        <Text className={getTitleClass()}>{title}</Text>

        <Text className={getDescriptionClass()}>{description}</Text>

        {(primaryAction || secondaryAction) && (
          <div className={styles.actions}>
            {primaryAction && (
              <Button
                appearance="primary"
                onClick={primaryAction.onClick}
                disabled={primaryAction.disabled}
                className={styles.primaryButton}
              >
                {primaryAction.icon}
                {primaryAction.label}
              </Button>
            )}
            {secondaryAction && (
              <Button
                appearance="secondary"
                onClick={secondaryAction.onClick}
                disabled={secondaryAction.disabled}
                className={styles.secondaryButton}
              >
                {secondaryAction.icon}
                {secondaryAction.label}
              </Button>
            )}
          </div>
        )}

        {stats && stats.length > 0 && (
          <div className={styles.stats}>
            {stats.map((stat, index) => (
              <div key={index} className={styles.statItem}>
                <Text className={styles.statValue}>{stat.value}</Text>
                <Caption1 className={styles.statLabel}>{stat.label}</Caption1>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
