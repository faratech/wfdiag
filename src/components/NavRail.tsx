import React, { useEffect, useState } from 'react'
import {
  makeStyles,
  shorthands,
  tokens,
  Text,
  Tooltip,
  Divider,
  Caption1,
  mergeClasses,
} from '@fluentui/react-components'
import {
  Stethoscope24Regular,
  ChartMultiple24Regular,
  Brain24Regular,
  Warning24Regular,
  History24Regular,
  Settings24Regular,
  Info24Regular,
  ChevronLeft24Regular,
  ChevronRight24Regular,
  Desktop20Regular,
} from '@fluentui/react-icons'
import {
  NAV_RAIL_WIDTH_COLLAPSED,
  NAV_RAIL_WIDTH_EXPANDED,
  NAV_RAIL_BREAKPOINT,
} from '../theme'
import type { TabValue } from './TabNavigation'

const useStyles = makeStyles({
  navRail: {
    display: 'flex',
    flexDirection: 'column',
    height: '100vh',
    background: 'var(--theme-sidebar-gradient)',
    borderRight: `1px solid ${tokens.colorNeutralStroke1}`,
    position: 'fixed',
    left: 0,
    top: 0,
    zIndex: 1000,
    transitionProperty: 'width',
    transitionDuration: tokens.durationNormal,
    transitionTimingFunction: tokens.curveEasyEase,
    boxShadow: tokens.shadow4,
  },

  collapsed: {
    width: `${NAV_RAIL_WIDTH_COLLAPSED}px`,
  },

  expanded: {
    width: `${NAV_RAIL_WIDTH_EXPANDED}px`,
  },

  header: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    ...shorthands.padding(tokens.spacingVerticalL),
    minHeight: '72px',
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

  logoImg: {
    width: '28px',
    height: '28px',
  },

  brandText: {
    display: 'flex',
    flexDirection: 'column',
    opacity: 1,
    transitionProperty: 'opacity, width',
    transitionDuration: tokens.durationNormal,
    overflow: 'hidden',
    whiteSpace: 'nowrap',
  },

  brandTextHidden: {
    opacity: 0,
    width: 0,
  },

  navSection: {
    flex: 1,
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalS),
    ...shorthands.gap(tokens.spacingVerticalXS),
    overflowY: 'auto',
    overflowX: 'hidden',
  },

  navItem: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalM),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    cursor: 'pointer',
    position: 'relative',
    color: tokens.colorNeutralForeground3,
    transitionProperty: 'background-color, color',
    transitionDuration: tokens.durationFaster,
    border: 'none',
    backgroundColor: 'transparent',
    width: '100%',
    textAlign: 'left',
    fontSize: tokens.fontSizeBase300,
    fontFamily: 'inherit',

    ':hover': {
      backgroundColor: tokens.colorNeutralBackground1Hover,
      color: tokens.colorNeutralForeground1,
    },

    ':focus-visible': {
      outlineColor: tokens.colorBrandStroke1,
      outlineStyle: 'solid',
      outlineWidth: '2px',
      outlineOffset: '-2px',
    },
  },

  navItemActive: {
    backgroundColor: tokens.colorNeutralBackground1Selected,
    color: tokens.colorBrandForeground1,
    fontWeight: tokens.fontWeightSemibold,

    '&::before': {
      content: '""',
      position: 'absolute',
      left: 0,
      top: '20%',
      bottom: '20%',
      width: '3px',
      backgroundColor: tokens.colorBrandBackground,
      ...shorthands.borderRadius(tokens.borderRadiusCircular),
    },
  },

  navItemIcon: {
    flexShrink: 0,
    width: '24px',
    height: '24px',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
  },

  navItemLabel: {
    flex: 1,
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    transitionProperty: 'opacity, width',
    transitionDuration: tokens.durationNormal,
  },

  navItemLabelHidden: {
    opacity: 0,
    width: 0,
  },

  navItemBadge: {
    marginLeft: 'auto',
  },

  footerSection: {
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalS),
    borderTop: `1px solid ${tokens.colorNeutralStroke2}`,
  },

  systemInfo: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalXS),
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    backgroundColor: tokens.colorNeutralBackground3,
    marginTop: tokens.spacingVerticalS,
    overflow: 'hidden',
  },

  systemInfoRow: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalS),
  },

  collapseButton: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    width: '100%',
    ...shorthands.padding(tokens.spacingVerticalS),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    cursor: 'pointer',
    border: 'none',
    backgroundColor: 'transparent',
    color: tokens.colorNeutralForeground3,
    transitionProperty: 'background-color, color',
    transitionDuration: tokens.durationFaster,
    marginTop: tokens.spacingVerticalS,

    ':hover': {
      backgroundColor: tokens.colorNeutralBackground1Hover,
      color: tokens.colorNeutralForeground1,
    },
  },
})

export interface NavRailProps {
  selectedItem: TabValue
  onItemSelect: (item: TabValue) => void
  collapsed?: boolean
  onToggleCollapse?: () => void
  issueCount?: number
  isMonitoringActive?: boolean
  hasAIKey?: boolean
  onOpenSettings?: () => void
  onOpenAbout?: () => void
  systemInfo?: {
    computer_name: string
    os_version: string
  } | null
}

interface NavItem {
  id: TabValue
  icon: React.ReactNode
  label: string
  badge?: React.ReactNode
}

export const NavRail: React.FC<NavRailProps> = ({
  selectedItem,
  onItemSelect,
  collapsed = false,
  onToggleCollapse,
  issueCount = 0,
  isMonitoringActive = false,
  onOpenSettings,
  onOpenAbout,
  systemInfo,
}) => {
  const styles = useStyles()
  const [isAutoCollapsed, setIsAutoCollapsed] = useState(false)

  // Auto-collapse on small screens
  useEffect(() => {
    const handleResize = () => {
      const shouldAutoCollapse = window.innerWidth < NAV_RAIL_BREAKPOINT
      setIsAutoCollapsed(shouldAutoCollapse)
    }

    handleResize()
    window.addEventListener('resize', handleResize)
    return () => window.removeEventListener('resize', handleResize)
  }, [])

  const isCollapsed = isAutoCollapsed || collapsed

  const navItems: NavItem[] = [
    {
      id: 'diagnostics',
      icon: <Stethoscope24Regular />,
      label: 'Diagnostics',
    },
    {
      id: 'monitoring',
      icon: <ChartMultiple24Regular />,
      label: 'Monitor',
      badge: isMonitoringActive ? (
        <span style={{
          width: 6,
          height: 6,
          borderRadius: '50%',
          backgroundColor: tokens.colorPaletteGreenForeground1,
          display: 'inline-block',
          animation: 'gentlePulse 2s ease-in-out infinite',
        }} />
      ) : undefined,
    },
    {
      id: 'ai',
      icon: <Brain24Regular />,
      label: 'AI Analysis',
    },
    {
      id: 'issues',
      icon: <Warning24Regular />,
      label: 'Issues',
      badge: issueCount > 0 ? (
        <span style={{
          minWidth: 18,
          height: 18,
          borderRadius: 9,
          backgroundColor: issueCount > 5 ? tokens.colorPaletteRedBackground2 : tokens.colorPaletteYellowBackground2,
          color: issueCount > 5 ? tokens.colorPaletteRedForeground1 : tokens.colorPaletteYellowForeground1,
          display: 'inline-flex',
          alignItems: 'center',
          justifyContent: 'center',
          fontSize: 11,
          fontWeight: 600,
          padding: '0 5px',
          animation: 'scaleIn 0.2s ease-out',
        }}>
          {issueCount}
        </span>
      ) : undefined,
    },
    {
      id: 'history',
      icon: <History24Regular />,
      label: 'History',
    },
  ]

  const renderNavItem = (item: NavItem) => {
    const isActive = selectedItem === item.id
    const button = (
      <button
        key={item.id}
        className={mergeClasses(styles.navItem, isActive && styles.navItemActive)}
        onClick={() => onItemSelect(item.id)}
        aria-current={isActive ? 'page' : undefined}
      >
        <span className={styles.navItemIcon}>{item.icon}</span>
        <span className={mergeClasses(styles.navItemLabel, isCollapsed && styles.navItemLabelHidden)}>
          {item.label}
        </span>
        {item.badge && !isCollapsed && <span className={styles.navItemBadge}>{item.badge}</span>}
        {item.badge && isCollapsed && (
          <span style={{ position: 'absolute', top: 4, right: 4 }}>{item.badge}</span>
        )}
      </button>
    )

    if (isCollapsed) {
      return (
        <Tooltip key={item.id} content={item.label} relationship="label" positioning="after">
          {button}
        </Tooltip>
      )
    }

    return button
  }

  return (
    <nav
      className={mergeClasses(styles.navRail, isCollapsed ? styles.collapsed : styles.expanded)}
      role="navigation"
      aria-label="Main navigation"
    >
      {/* Header with Logo */}
      <div className={styles.header}>
        <div className={styles.logo}>
          <img src="/icon.png" alt="WF Diagnostics" className={styles.logoImg} />
        </div>
        <div className={mergeClasses(styles.brandText, isCollapsed && styles.brandTextHidden)}>
          <Text weight="bold" size={400}>
            WF Diagnostics
          </Text>
          <Caption1 style={{ color: tokens.colorNeutralForeground3 }}>v2.1.8</Caption1>
        </div>
      </div>

      <Divider />

      {/* Navigation Items */}
      <div className={styles.navSection}>{navItems.map(renderNavItem)}</div>

      {/* Footer Section */}
      <div className={styles.footerSection}>
        {/* Settings */}
        {isCollapsed ? (
          <Tooltip content="Settings" relationship="label" positioning="after">
            <button className={styles.navItem} onClick={onOpenSettings}>
              <span className={styles.navItemIcon}>
                <Settings24Regular />
              </span>
            </button>
          </Tooltip>
        ) : (
          <button className={styles.navItem} onClick={onOpenSettings}>
            <span className={styles.navItemIcon}>
              <Settings24Regular />
            </span>
            <span className={styles.navItemLabel}>Settings</span>
          </button>
        )}

        {/* About */}
        {isCollapsed ? (
          <Tooltip content="About" relationship="label" positioning="after">
            <button className={styles.navItem} onClick={onOpenAbout}>
              <span className={styles.navItemIcon}>
                <Info24Regular />
              </span>
            </button>
          </Tooltip>
        ) : (
          <button className={styles.navItem} onClick={onOpenAbout}>
            <span className={styles.navItemIcon}>
              <Info24Regular />
            </span>
            <span className={styles.navItemLabel}>About</span>
          </button>
        )}

        {/* System Info (only when expanded) */}
        {!isCollapsed && systemInfo && (
          <div className={styles.systemInfo}>
            <div className={styles.systemInfoRow}>
              <Desktop20Regular style={{ color: tokens.colorNeutralForeground3, flexShrink: 0 }} />
              <Caption1
                style={{
                  color: tokens.colorNeutralForeground2,
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
              >
                {systemInfo.computer_name}
              </Caption1>
            </div>
            <Caption1
              style={{
                color: tokens.colorNeutralForeground3,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                marginLeft: '28px',
              }}
            >
              {systemInfo.os_version}
            </Caption1>
          </div>
        )}

        {/* Collapse Toggle (only on desktop) */}
        {!isAutoCollapsed && onToggleCollapse && (
          <Tooltip content={isCollapsed ? 'Expand' : 'Collapse'} relationship="label" positioning="after">
            <button className={styles.collapseButton} onClick={onToggleCollapse}>
              {isCollapsed ? <ChevronRight24Regular /> : <ChevronLeft24Regular />}
            </button>
          </Tooltip>
        )}
      </div>
    </nav>
  )
}
