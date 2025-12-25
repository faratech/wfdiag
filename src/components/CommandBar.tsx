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
  Play20Regular,
  Stop20Regular,
  ArrowClockwise20Regular,
  Filter20Regular,
  Save20Regular,
  Copy20Regular,
  Share20Regular,
  Delete20Regular,
  History20Regular,
  ChevronDown20Regular,
  Search20Regular,
  Flash20Regular,
  CheckmarkCircle20Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  toolbar: {
    backgroundColor: 'rgba(30, 41, 59, 0.6)',
    backdropFilter: 'blur(10px)',
    ...shorthands.border('1px', 'solid', 'rgba(255, 255, 255, 0.1)'),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
    marginBottom: tokens.spacingVerticalM,
  },

  toolbarGroup: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
  },

  statusIndicator: {
    display: 'inline-flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
    ...shorthands.padding(tokens.spacingVerticalXS, tokens.spacingHorizontalS),
    ...shorthands.borderRadius(tokens.borderRadiusSmall),
    fontSize: tokens.fontSizeBase200,
  },

  statusActive: {
    backgroundColor: 'rgba(16, 185, 129, 0.2)',
    color: '#10B981',
  },

  statusInactive: {
    backgroundColor: 'rgba(100, 116, 139, 0.2)',
    color: tokens.colorNeutralForeground3,
  }
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
  scanStatus?: 'idle' | 'scanning' | 'complete' | 'error'
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

  const getScanStatusIndicator = () => {
    switch (scanStatus) {
      case 'scanning':
        return (
          <div className={`${styles.statusIndicator} ${styles.statusActive}`}>
            <div style={{ animation: 'pulse 2s infinite' }}>●</div>
            Scanning...
          </div>
        )
      case 'complete':
        return (
          <div className={`${styles.statusIndicator} ${styles.statusActive}`}>
            <CheckmarkCircle20Regular />
            {resultCount} Results
          </div>
        )
      case 'error':
        return (
          <div className={styles.statusIndicator} style={{
            backgroundColor: 'rgba(239, 68, 68, 0.2)',
            color: '#EF4444'
          }}>
            Error
          </div>
        )
      default:
        return (
          <div className={`${styles.statusIndicator} ${styles.statusInactive}`}>
            Ready
          </div>
        )
    }
  }

  return (
    <Toolbar
      className={styles.toolbar}
      aria-label="Command bar"
      size="small"
    >
      <ToolbarGroup className={styles.toolbarGroup}>
        {/* Scan Actions */}
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

        {isScanning && (
          <Tooltip content="Stop current scan" relationship="description">
            <ToolbarButton
              appearance="subtle"
              icon={<Stop20Regular />}
              onClick={onStopScan}
              aria-label="Stop scan"
            />
          </Tooltip>
        )}

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
        <Tooltip content="Filter results" relationship="description">
          <ToolbarButton
            appearance="subtle"
            icon={<Filter20Regular />}
            onClick={onToggleFilter}
            aria-label="Toggle filter"
          />
        </Tooltip>

        <Tooltip content="Compare with previous scan" relationship="description">
          <ToolbarButton
            appearance="subtle"
            icon={<History20Regular />}
            onClick={onCompareScans}
            disabled={resultCount === 0}
            aria-label="Compare scans"
          />
        </Tooltip>

        <ToolbarDivider />

        {/* Export Actions */}
        <Tooltip content="Export results" relationship="description">
          <ToolbarButton
            appearance="subtle"
            icon={<Save20Regular />}
            onClick={onExport}
            disabled={resultCount === 0}
            aria-label="Export"
          />
        </Tooltip>

        <Tooltip content="Copy to clipboard" relationship="description">
          <ToolbarButton
            appearance="subtle"
            icon={<Copy20Regular />}
            onClick={onCopyToClipboard}
            disabled={resultCount === 0}
            aria-label="Copy"
          />
        </Tooltip>

        <Menu>
          <MenuTrigger disableButtonEnhancement>
            <ToolbarButton
              appearance="subtle"
              icon={<Share20Regular />}
              disabled={resultCount === 0}
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
        <Tooltip content="Clear results" relationship="description">
          <ToolbarButton
            appearance="subtle"
            icon={<Delete20Regular />}
            onClick={onClearResults}
            disabled={resultCount === 0}
            aria-label="Clear results"
          />
        </Tooltip>
      </ToolbarGroup>

      {/* Status Indicator */}
      <div style={{ marginLeft: 'auto' }}>
        {getScanStatusIndicator()}
      </div>
    </Toolbar>
  )
}