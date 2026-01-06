/**
 * BrandSection - Shared logo and branding display
 * Used by NavRail and NavigationHeader
 */

import React from 'react'
import {
  makeStyles,
  tokens,
  shorthands,
  Text,
  Caption1,
  Badge,
} from '@fluentui/react-components'

const useStyles = makeStyles({
  container: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
  },
  logo: {
    width: '40px',
    height: '40px',
    ...shorthands.borderRadius(tokens.borderRadiusCircular),
    background: 'var(--theme-accent-gradient)',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    boxShadow: '0 4px 12px rgba(51, 95, 227, 0.3)',
    flexShrink: 0,
  },
  logoSmall: {
    width: '36px',
    height: '36px',
    backgroundColor: tokens.colorBrandBackground,
    boxShadow: tokens.shadow4,
  },
  logoImg: {
    width: '28px',
    height: '28px',
    imageRendering: 'auto',
    objectFit: 'contain',
  },
  textSection: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalXXS),
  },
  title: {
    fontWeight: tokens.fontWeightBold,
  },
  titleLarge: {
    fontSize: tokens.fontSizeBase500,
    color: tokens.colorNeutralForeground1,
  },
  titleSmall: {
    fontSize: tokens.fontSizeBase400,
  },
  subtitle: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
  },
  hidden: {
    opacity: 0,
    width: 0,
    overflow: 'hidden',
  },
})

export interface BrandSectionProps {
  variant?: 'full' | 'compact' | 'minimal'
  showVersion?: boolean
  showSubtitle?: boolean
  version?: string
  subtitle?: string
  collapsed?: boolean
}

export const BrandSection: React.FC<BrandSectionProps> = ({
  variant = 'full',
  showVersion = true,
  showSubtitle = false,
  version = '2.1.9',
  subtitle = 'Professional System Analysis',
  collapsed = false,
}) => {
  const styles = useStyles()

  const logoClass = variant === 'minimal' ? `${styles.logo} ${styles.logoSmall}` : styles.logo
  const titleClass = variant === 'full' ? `${styles.title} ${styles.titleLarge}` : `${styles.title} ${styles.titleSmall}`
  const textClass = collapsed ? `${styles.textSection} ${styles.hidden}` : styles.textSection

  return (
    <div className={styles.container}>
      <div className={logoClass} aria-label="WF Diagnostics Logo">
        <img
          src="/icon.png"
          alt="WF Diagnostics"
          className={styles.logoImg}
        />
      </div>

      {variant !== 'minimal' && (
        <div className={textClass}>
          <Text className={titleClass}>
            WF Diagnostics
          </Text>
          {showSubtitle && (
            <Text className={styles.subtitle}>{subtitle}</Text>
          )}
          {variant === 'compact' && showVersion && (
            <Caption1 style={{ color: tokens.colorNeutralForeground3 }}>
              v{version}
            </Caption1>
          )}
        </div>
      )}

      {variant === 'full' && showVersion && (
        <Badge
          appearance="filled"
          color="informative"
          size="small"
        >
          v{version}
        </Badge>
      )}
    </div>
  )
}

export default BrandSection
