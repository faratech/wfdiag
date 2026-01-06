/**
 * CompactToolbar - Minimal toolbar with scan controls and result actions
 * Uses shared toolbar components for consistency with CommandBar
 */

import React from 'react'
import {
  makeStyles,
  tokens,
  shorthands,
  Divider,
} from '@fluentui/react-components'
import { ScanActionButtons, ResultActionButtons, ScanStatusIndicator } from './toolbar'

const useStyles = makeStyles({
  toolbar: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('2px'),
    ...shorthands.padding('4px', '8px'),
    backgroundColor: tokens.colorNeutralBackground3,
    borderBottom: `1px solid ${tokens.colorNeutralStroke1}`,
    minHeight: '32px',
  },
  divider: {
    height: '16px',
    ...shorthands.margin('0', '4px'),
  },
  spacer: {
    flex: 1,
  },
  statusContainer: {
    flex: 1,
    display: 'flex',
    alignItems: 'center',
    marginLeft: '8px',
  },
})

interface CompactToolbarProps {
  onQuickScan: () => void
  onFullScan: () => void
  onStop?: () => void
  onExport?: () => void
  onCopy?: () => void
  onCompare?: () => void
  onClear?: () => void
  isScanning: boolean
  progress?: number
  resultCount?: number
}

export const CompactToolbar: React.FC<CompactToolbarProps> = ({
  onQuickScan,
  onFullScan,
  onStop,
  onExport,
  onCopy,
  onCompare,
  onClear,
  isScanning,
  progress = 0,
  resultCount = 0
}) => {
  const styles = useStyles()
  const hasResults = resultCount > 0 && !isScanning

  return (
    <div className={styles.toolbar}>
      {/* Scan actions */}
      <ScanActionButtons
        onQuickScan={onQuickScan}
        onFullScan={onFullScan}
        onStop={onStop}
        isScanning={isScanning}
        variant="compact"
      />

      <Divider vertical className={styles.divider} />

      {/* Status indicator */}
      <div className={styles.statusContainer}>
        <ScanStatusIndicator
          status={isScanning ? 'scanning' : (resultCount > 0 ? 'complete' : 'idle')}
          resultCount={resultCount}
          progress={progress}
          variant="compact"
        />
      </div>

      <div className={styles.spacer} />

      {/* Result actions */}
      {hasResults && (
        <>
          <ResultActionButtons
            onExport={onExport}
            onCopy={onCopy}
            onCompare={onCompare}
            variant="compact"
          />

          <Divider vertical className={styles.divider} />

          <ResultActionButtons
            onClear={onClear}
            variant="compact"
          />
        </>
      )}
    </div>
  )
}

export default CompactToolbar
