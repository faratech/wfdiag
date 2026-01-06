/**
 * CommandBar - Full-featured toolbar with scan controls, export actions, and status
 * Uses shared toolbar components for consistency with CompactToolbar
 */

import React from 'react'
import {
  Toolbar,
  ToolbarButton,
  ToolbarDivider,
  ToolbarGroup,
  makeStyles,
  tokens,
  shorthands,
  Menu,
  MenuTrigger,
  MenuPopover,
  MenuList,
  MenuItem,
  MenuDivider,
  Tooltip
} from '@fluentui/react-components'
import {
  ArrowClockwise20Regular,
  Filter20Regular,
  Share20Regular,
} from '@fluentui/react-icons'
import { ScanActionButtons, ResultActionButtons, ScanStatusIndicator, ScanStatus } from './toolbar'

const useStyles = makeStyles({
  toolbar: {
    minHeight: '32px',
    backgroundColor: tokens.colorNeutralBackground2,
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke2),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    ...shorthands.padding('4px', tokens.spacingHorizontalS),
    marginBottom: tokens.spacingVerticalS,
    flexWrap: 'wrap',
    ...shorthands.gap('4px'),
  },
  toolbarGroup: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('4px'),
    flexWrap: 'wrap',
  },
  secondaryActions: {
    '@media (max-width: 1024px)': {
      display: 'none',
    },
  },
})

export interface CommandBarProps {
  // Scan controls
  onQuickScan?: () => void
  onFullScan?: () => void
  onStopScan?: () => void
  isScanning?: boolean

  // Export actions
  onExport?: () => void
  onCopyToClipboard?: () => void
  onShareToForum?: () => void
  onEmailReport?: () => void
  onGenerateSupportPackage?: () => void

  // View controls
  onToggleFilter?: () => void
  onClearResults?: () => void
  onCompareScans?: () => void

  // Status
  scanStatus?: ScanStatus
  resultCount?: number
}

export const CommandBar: React.FC<CommandBarProps> = ({
  onQuickScan,
  onFullScan,
  onStopScan,
  isScanning = false,
  onExport,
  onCopyToClipboard,
  onShareToForum,
  onEmailReport,
  onGenerateSupportPackage,
  onToggleFilter,
  onClearResults,
  onCompareScans,
  scanStatus = 'idle',
  resultCount = 0,
}) => {
  const styles = useStyles()
  const hasResults = resultCount > 0

  return (
    <Toolbar
      className={styles.toolbar}
      aria-label="Command bar"
      size="small"
    >
      <ToolbarGroup className={styles.toolbarGroup}>
        {/* Scan Actions */}
        <ScanActionButtons
          onQuickScan={onQuickScan || (() => {})}
          onFullScan={onFullScan || (() => {})}
          onStop={onStopScan}
          isScanning={isScanning}
          variant="full"
        />

        <Tooltip content="Refresh results" relationship="description">
          <ToolbarButton
            appearance="subtle"
            icon={<ArrowClockwise20Regular />}
            onClick={onQuickScan}
            disabled={isScanning}
            aria-label="Refresh"
          />
        </Tooltip>

        <ToolbarDivider />

        {/* View Controls */}
        <span className={styles.secondaryActions}>
          <Tooltip content="Filter results" relationship="description">
            <ToolbarButton
              appearance="subtle"
              icon={<Filter20Regular />}
              onClick={onToggleFilter}
              aria-label="Toggle filter"
            />
          </Tooltip>
        </span>

        {/* Result Actions */}
        <ResultActionButtons
          onCompare={onCompareScans}
          disabled={!hasResults}
          variant="full"
        />

        <ToolbarDivider />

        <ResultActionButtons
          onExport={onExport}
          disabled={!hasResults}
          variant="full"
        />

        <span className={styles.secondaryActions}>
          <ResultActionButtons
            onCopy={onCopyToClipboard}
            disabled={!hasResults}
            variant="full"
          />
        </span>

        {/* Share Menu */}
        <Menu>
          <MenuTrigger disableButtonEnhancement>
            <ToolbarButton
              appearance="subtle"
              icon={<Share20Regular />}
              disabled={!hasResults}
              aria-label="Share options"
            />
          </MenuTrigger>
          <MenuPopover>
            <MenuList>
              <MenuItem onClick={onShareToForum} disabled={!onShareToForum}>
                Share to WindowsForum
              </MenuItem>
              <MenuItem onClick={onEmailReport} disabled={!onEmailReport}>
                Email Report
              </MenuItem>
              <MenuDivider />
              <MenuItem onClick={onGenerateSupportPackage} disabled={!onGenerateSupportPackage}>
                Generate Support Package
              </MenuItem>
            </MenuList>
          </MenuPopover>
        </Menu>

        <ToolbarDivider />

        {/* Clear */}
        <ResultActionButtons
          onClear={onClearResults}
          disabled={!hasResults}
          variant="full"
        />
      </ToolbarGroup>

      {/* Status Indicator */}
      <div style={{ marginLeft: 'auto' }}>
        <ScanStatusIndicator
          status={scanStatus}
          resultCount={resultCount}
          variant="full"
        />
      </div>
    </Toolbar>
  )
}
