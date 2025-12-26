import React from 'react'
import {
  makeStyles,
  shorthands,
  tokens,
  Text,
  Button,
  Badge,
  Tooltip,
  Menu,
  MenuTrigger,
  MenuPopover,
  MenuList,
  MenuItem,
  MenuDivider
} from '@fluentui/react-components'
import {
  Settings20Regular,
  Info20Regular,
  ArrowExportUp20Regular,
  Shield20Regular,
  Person20Regular,
  Desktop20Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  header: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalXXL),
    background: 'var(--theme-sidebar-gradient)',
    borderBottom: `1px solid ${tokens.colorNeutralStroke1}`,
    position: 'sticky',
    top: 0,
    zIndex: 1000,
    boxShadow: tokens.shadow4,
  },

  brandSection: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
  },

  logo: {
    width: '36px',
    height: '36px',
    ...shorthands.borderRadius(tokens.borderRadiusCircular),
    backgroundColor: tokens.colorBrandBackground,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    boxShadow: tokens.shadow4,
    position: 'relative',
  },

  titleSection: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalXXS),
  },

  title: {
    fontSize: tokens.fontSizeBase500,
    fontWeight: tokens.fontWeightBold,
    color: tokens.colorNeutralForeground1,
  },

  subtitle: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
  },

  infoSection: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalL),
  },

  systemInfo: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalL),
    backgroundColor: tokens.colorNeutralBackground3,
    ...shorthands.borderRadius(tokens.borderRadiusLarge),
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke2),
    transitionProperty: 'background-color',
    transitionDuration: tokens.durationNormal,

    ':hover': {
      backgroundColor: tokens.colorNeutralBackground3Hover,
    }
  },

  divider: {
    width: '1px',
    height: '24px',
    backgroundColor: tokens.colorNeutralStroke2,
  },

  actionButtons: {
    display: 'flex',
    ...shorthands.gap(tokens.spacingHorizontalS),
  }
})

export interface NavigationHeaderProps {
  computerName?: string
  osVersion?: string
  isAdmin?: boolean
  onRestartAsAdmin?: () => void
  onOpenSettings?: () => void
  onOpenAbout?: () => void
  onExportDiagnostics?: () => void
  version?: string
}

export const NavigationHeader: React.FC<NavigationHeaderProps> = ({
  computerName,
  osVersion,
  isAdmin,
  onRestartAsAdmin,
  onOpenSettings,
  onOpenAbout,
  onExportDiagnostics,
  version = '2.1.7'
}) => {
  const styles = useStyles()

  return (
    <header className={styles.header} role="banner">
      <div className={styles.brandSection}>
        <div className={styles.logo} aria-label="WF Diagnostics Logo">
          <img
            src="/icon.png"
            alt="WF Diagnostics"
            style={{ width: '28px', height: '28px' }}
          />
        </div>

        <div className={styles.titleSection}>
          <Text className={styles.title}>WF Diagnostics</Text>
          <Text className={styles.subtitle}>Professional System Analysis</Text>
        </div>

        <Badge
          appearance="filled"
          color="informative"
          size="small"
        >
          v{version}
        </Badge>
      </div>

      <div className={styles.infoSection}>
        {computerName && (
          <>
            <div className={styles.systemInfo}>
              <Desktop20Regular primaryFill={tokens.colorNeutralForeground2} />
              <div style={{ display: 'flex', flexDirection: 'column', gap: tokens.spacingVerticalXXS }}>
                <Text size={200} weight="semibold">
                  {computerName}
                </Text>
                <Text size={100} style={{ color: tokens.colorNeutralForeground3 }}>
                  {osVersion}
                </Text>
              </div>
            </div>

            <div className={styles.divider} />
          </>
        )}

        {isAdmin !== undefined && (
          <Tooltip
            content={isAdmin ? 'Running with administrator privileges' : 'Limited access - some features unavailable'}
            relationship="description"
          >
            <Badge
              appearance="filled"
              color={isAdmin ? 'success' : 'warning'}
              icon={isAdmin ? <Shield20Regular /> : <Person20Regular />}
            >
              {isAdmin ? 'Admin' : 'Standard'}
            </Badge>
          </Tooltip>
        )}

        <div className={styles.actionButtons}>
          {!isAdmin && onRestartAsAdmin && (
            <Tooltip content="Restart with administrator privileges for full access" relationship="description">
              <Button
                appearance="secondary"
                icon={<Shield20Regular />}
                onClick={onRestartAsAdmin}
                size="small"
              >
                Elevate
              </Button>
            </Tooltip>
          )}

          {onExportDiagnostics && (
            <Tooltip content="Export diagnostic results" relationship="description">
              <Button
                appearance="subtle"
                icon={<ArrowExportUp20Regular />}
                onClick={onExportDiagnostics}
                size="small"
                aria-label="Export diagnostics"
              />
            </Tooltip>
          )}

          <Menu>
            <MenuTrigger disableButtonEnhancement>
              <Button
                appearance="subtle"
                icon={<Settings20Regular />}
                size="small"
                aria-label="Settings menu"
              />
            </MenuTrigger>
            <MenuPopover>
              <MenuList>
                <MenuItem onClick={onOpenSettings} icon={<Settings20Regular />}>
                  Settings
                </MenuItem>
                <MenuDivider />
                <MenuItem onClick={onOpenAbout} icon={<Info20Regular />}>
                  About
                </MenuItem>
              </MenuList>
            </MenuPopover>
          </Menu>
        </div>
      </div>
    </header>
  )
}