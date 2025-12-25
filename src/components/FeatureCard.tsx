import React from 'react'
import {
  makeStyles,
  shorthands,
  tokens,
  Text,
  Caption1,
  Badge,
} from '@fluentui/react-components'

const useStyles = makeStyles({
  card: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.padding(tokens.spacingVerticalXL),
    background: 'var(--theme-card-gradient)',
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke1),
    ...shorthands.borderRadius(tokens.borderRadiusLarge),
    boxShadow: tokens.shadow4,
    cursor: 'pointer',
    position: 'relative',
    transitionProperty: 'transform, box-shadow, background',
    transitionDuration: tokens.durationNormal,
    transitionTimingFunction: tokens.curveEasyEase,

    ':hover': {
      transform: 'translateY(-4px)',
      boxShadow: tokens.shadow16,
      background: 'var(--theme-card-gradient-hover)',
    },

    ':focus-visible': {
      outlineColor: tokens.colorBrandStroke1,
      outlineStyle: 'solid',
      outlineWidth: '2px',
      outlineOffset: '2px',
    },
  },

  cardDisabled: {
    opacity: 0.6,
    cursor: 'not-allowed',

    ':hover': {
      transform: 'none',
      boxShadow: tokens.shadow4,
    },
  },

  iconContainer: {
    width: '48px',
    height: '48px',
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    marginBottom: tokens.spacingVerticalM,
    flexShrink: 0,
  },

  title: {
    fontSize: tokens.fontSizeBase400,
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground1,
    marginBottom: tokens.spacingVerticalS,
  },

  description: {
    fontSize: tokens.fontSizeBase300,
    color: tokens.colorNeutralForeground3,
    lineHeight: tokens.lineHeightBase300,
    flex: 1,
  },

  badge: {
    position: 'absolute',
    top: tokens.spacingVerticalM,
    right: tokens.spacingHorizontalM,
  },
})

export interface FeatureCardProps {
  title: string
  description: string
  icon: React.ReactNode
  iconColor?: string
  iconBackground?: string
  onClick?: () => void
  badge?: string
  badgeColor?: 'brand' | 'danger' | 'important' | 'informative' | 'severe' | 'subtle' | 'success' | 'warning'
  disabled?: boolean
}

export const FeatureCard: React.FC<FeatureCardProps> = ({
  title,
  description,
  icon,
  iconColor = tokens.colorBrandForeground1,
  iconBackground = tokens.colorBrandBackground2,
  onClick,
  badge,
  badgeColor = 'informative',
  disabled = false,
}) => {
  const styles = useStyles()

  const handleClick = () => {
    if (!disabled && onClick) {
      onClick()
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      handleClick()
    }
  }

  return (
    <div
      className={`${styles.card} ${disabled ? styles.cardDisabled : ''}`}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      role="button"
      tabIndex={disabled ? -1 : 0}
      aria-disabled={disabled}
    >
      {badge && (
        <Badge className={styles.badge} appearance="tint" color={badgeColor} size="small">
          {badge}
        </Badge>
      )}

      <div className={styles.iconContainer} style={{ backgroundColor: iconBackground, color: iconColor }}>
        {icon}
      </div>

      <Text className={styles.title}>{title}</Text>

      <Caption1 className={styles.description}>{description}</Caption1>
    </div>
  )
}
