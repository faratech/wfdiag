/**
 * AppMenu - Shared settings/about menu buttons
 * Used by NavRail and NavigationHeader
 */

import React from 'react'
import {
  makeStyles,
  tokens,
  shorthands,
  Button,
  Tooltip,
  Menu,
  MenuTrigger,
  MenuPopover,
  MenuList,
  MenuItem,
  MenuDivider,
} from '@fluentui/react-components'
import {
  Settings20Regular,
  Settings24Regular,
  Info20Regular,
  Info24Regular,
  ArrowExportUp20Regular,
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  container: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalXS),
  },
  navItem: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    background: 'transparent',
    ...shorthands.border('none'),
    color: tokens.colorNeutralForeground2,
    cursor: 'pointer',
    width: '100%',
    textAlign: 'left',
    transitionProperty: 'background, color',
    transitionDuration: tokens.durationNormal,
    ':hover': {
      backgroundColor: tokens.colorNeutralBackground3,
      color: tokens.colorNeutralForeground1,
    },
  },
  navItemIcon: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    width: '24px',
    height: '24px',
    flexShrink: 0,
  },
  navItemLabel: {
    fontSize: tokens.fontSizeBase300,
    fontWeight: tokens.fontWeightRegular,
  },
})

export interface AppMenuProps {
  onOpenSettings?: () => void
  onOpenAbout?: () => void
  onExportDiagnostics?: () => void
  variant?: 'buttons' | 'menu' | 'nav'
  collapsed?: boolean
  size?: 'small' | 'medium'
}

/**
 * Navigation button variant - for NavRail sidebar
 */
const NavButtons: React.FC<AppMenuProps> = ({
  onOpenSettings,
  onOpenAbout,
  collapsed = false,
}) => {
  const styles = useStyles()

  const SettingsButton = (
    <button className={styles.navItem} onClick={onOpenSettings}>
      <span className={styles.navItemIcon}>
        <Settings24Regular />
      </span>
      {!collapsed && <span className={styles.navItemLabel}>Settings</span>}
    </button>
  )

  const AboutButton = (
    <button className={styles.navItem} onClick={onOpenAbout}>
      <span className={styles.navItemIcon}>
        <Info24Regular />
      </span>
      {!collapsed && <span className={styles.navItemLabel}>About</span>}
    </button>
  )

  return (
    <div className={styles.container}>
      {collapsed ? (
        <>
          <Tooltip content="Settings" relationship="label" positioning="after">
            {SettingsButton}
          </Tooltip>
          <Tooltip content="About" relationship="label" positioning="after">
            {AboutButton}
          </Tooltip>
        </>
      ) : (
        <>
          {SettingsButton}
          {AboutButton}
        </>
      )}
    </div>
  )
}

/**
 * Dropdown menu variant - for NavigationHeader
 */
const MenuDropdown: React.FC<AppMenuProps> = ({
  onOpenSettings,
  onOpenAbout,
  onExportDiagnostics,
}) => (
  <Menu>
    <MenuTrigger disableButtonEnhancement>
      <Button
        appearance="subtle"
        icon={<Settings20Regular />}
        size="small"
      />
    </MenuTrigger>
    <MenuPopover>
      <MenuList>
        <MenuItem onClick={onOpenSettings} icon={<Settings20Regular />}>
          Settings
        </MenuItem>
        <MenuItem onClick={onOpenAbout} icon={<Info20Regular />}>
          About
        </MenuItem>
        {onExportDiagnostics && (
          <>
            <MenuDivider />
            <MenuItem onClick={onExportDiagnostics} icon={<ArrowExportUp20Regular />}>
              Export Diagnostics
            </MenuItem>
          </>
        )}
      </MenuList>
    </MenuPopover>
  </Menu>
)

/**
 * Simple button variant - inline buttons
 */
const SimpleButtons: React.FC<AppMenuProps> = ({
  onOpenSettings,
  onOpenAbout,
  size = 'small',
}) => (
  <>
    <Tooltip content="Settings" relationship="description">
      <Button
        appearance="subtle"
        icon={<Settings20Regular />}
        onClick={onOpenSettings}
        size={size}
      />
    </Tooltip>
    <Tooltip content="About" relationship="description">
      <Button
        appearance="subtle"
        icon={<Info20Regular />}
        onClick={onOpenAbout}
        size={size}
      />
    </Tooltip>
  </>
)

export const AppMenu: React.FC<AppMenuProps> = (props) => {
  const { variant = 'buttons' } = props

  switch (variant) {
    case 'nav':
      return <NavButtons {...props} />
    case 'menu':
      return <MenuDropdown {...props} />
    case 'buttons':
    default:
      return <SimpleButtons {...props} />
  }
}

export default AppMenu
