/**
 * Shared scan action buttons used by CommandBar and CompactToolbar
 */

import React from 'react'
import {
  Button,
  Tooltip,
  tokens,
  Menu,
  MenuTrigger,
  MenuPopover,
  MenuList,
  MenuItem,
  ToolbarButton,
} from '@fluentui/react-components'
import {
  Play16Regular,
  Play20Regular,
  PlayCircle16Regular,
  Stop16Regular,
  Stop20Regular,
  Flash20Regular,
  Search20Regular,
  ChevronDown20Regular,
} from '@fluentui/react-icons'

export interface ScanActionButtonsProps {
  onQuickScan: () => void
  onFullScan: () => void
  onStop?: () => void
  isScanning: boolean
  variant?: 'compact' | 'full'
  showLabels?: boolean
}

/**
 * Compact variant - separate Quick/Full/Stop buttons
 */
const CompactScanButtons: React.FC<ScanActionButtonsProps> = ({
  onQuickScan,
  onFullScan,
  onStop,
  isScanning,
  showLabels = true,
}) => (
  <>
    <Tooltip content="Quick Scan (essential checks)" relationship="label">
      <Button
        appearance="subtle"
        size="small"
        icon={<Play16Regular />}
        onClick={onQuickScan}
        disabled={isScanning}
      >
        {showLabels && 'Quick'}
      </Button>
    </Tooltip>

    <Tooltip content="Full Scan (all diagnostics)" relationship="label">
      <Button
        appearance="subtle"
        size="small"
        icon={<PlayCircle16Regular />}
        onClick={onFullScan}
        disabled={isScanning}
      >
        {showLabels && 'Full'}
      </Button>
    </Tooltip>

    {isScanning && onStop && (
      <Tooltip content="Stop scan" relationship="label">
        <Button
          appearance="subtle"
          size="small"
          icon={<Stop16Regular />}
          onClick={onStop}
        >
          {showLabels && 'Stop'}
        </Button>
      </Tooltip>
    )}
  </>
)

/**
 * Full variant - dropdown menu with scan options
 */
const FullScanButtons: React.FC<ScanActionButtonsProps> = ({
  onQuickScan,
  onFullScan,
  onStop,
  isScanning,
}) => (
  <>
    <Menu>
      <MenuTrigger disableButtonEnhancement>
        <ToolbarButton
          appearance="primary"
          icon={<Play20Regular />}
          disabled={isScanning}
        >
          Start Scan
          <ChevronDown20Regular style={{ marginLeft: tokens.spacingHorizontalXS }} />
        </ToolbarButton>
      </MenuTrigger>
      <MenuPopover>
        <MenuList>
          <MenuItem
            onClick={onQuickScan}
            icon={<Flash20Regular />}
            disabled={isScanning}
          >
            Quick Scan
            <span style={{
              marginLeft: 'auto',
              fontSize: tokens.fontSizeBase100,
              color: tokens.colorNeutralForeground3
            }}>
              ~30 sec
            </span>
          </MenuItem>
          <MenuItem
            onClick={onFullScan}
            icon={<Search20Regular />}
            disabled={isScanning}
          >
            Full Scan
            <span style={{
              marginLeft: 'auto',
              fontSize: tokens.fontSizeBase100,
              color: tokens.colorNeutralForeground3
            }}>
              3-5 min
            </span>
          </MenuItem>
        </MenuList>
      </MenuPopover>
    </Menu>

    {isScanning && onStop && (
      <Tooltip content="Stop current scan" relationship="description">
        <ToolbarButton
          appearance="subtle"
          icon={<Stop20Regular />}
          onClick={onStop}
          aria-label="Stop scan"
        />
      </Tooltip>
    )}
  </>
)

export const ScanActionButtons: React.FC<ScanActionButtonsProps> = (props) => {
  const { variant = 'compact' } = props

  if (variant === 'full') {
    return <FullScanButtons {...props} />
  }

  return <CompactScanButtons {...props} />
}

export default ScanActionButtons
