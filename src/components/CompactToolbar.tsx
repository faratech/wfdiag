import React from 'react'
import {
  makeStyles,
  tokens,
  shorthands,
  Button,
  Tooltip,
  Divider,
  ProgressBar,
  Text
} from '@fluentui/react-components'
import {
  Play16Regular,
  PlayCircle16Regular,
  Stop16Regular,
  Save16Regular,
  Copy16Regular,
  History16Regular,
  Delete16Regular
} from '@fluentui/react-icons'

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
  toolbarButton: {
    minWidth: 'auto',
    height: '24px',
    ...shorthands.padding('0', '8px'),
    fontSize: tokens.fontSizeBase200,
  },
  divider: {
    height: '16px',
    ...shorthands.margin('0', '4px'),
  },
  spacer: {
    flex: 1,
  },
  progressContainer: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('8px'),
    flex: 1,
    maxWidth: '300px',
  },
  progressBar: {
    flex: 1,
  },
  progressText: {
    fontSize: tokens.fontSizeBase100,
    color: tokens.colorNeutralForeground2,
    whiteSpace: 'nowrap',
  },
  statusText: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground2,
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

  return (
    <div className={styles.toolbar}>
      {/* Scan actions */}
      <Tooltip content="Quick Scan (essential checks)" relationship="label">
        <Button
          appearance="subtle"
          size="small"
          icon={<Play16Regular />}
          onClick={onQuickScan}
          disabled={isScanning}
          className={styles.toolbarButton}
        >
          Quick
        </Button>
      </Tooltip>

      <Tooltip content="Full Scan (all diagnostics)" relationship="label">
        <Button
          appearance="subtle"
          size="small"
          icon={<PlayCircle16Regular />}
          onClick={onFullScan}
          disabled={isScanning}
          className={styles.toolbarButton}
        >
          Full
        </Button>
      </Tooltip>

      {isScanning && onStop && (
        <Tooltip content="Stop scan" relationship="label">
          <Button
            appearance="subtle"
            size="small"
            icon={<Stop16Regular />}
            onClick={onStop}
            className={styles.toolbarButton}
          >
            Stop
          </Button>
        </Tooltip>
      )}

      <Divider vertical className={styles.divider} />

      {/* Progress or status */}
      {isScanning ? (
        <div className={styles.progressContainer}>
          <ProgressBar value={progress / 100} className={styles.progressBar} />
          <Text className={styles.progressText}>
            {Math.round(progress)}%
          </Text>
        </div>
      ) : (
        <Text className={styles.statusText}>
          {resultCount > 0 ? `${resultCount} checks completed` : 'Ready'}
        </Text>
      )}

      <div className={styles.spacer} />

      {/* Result actions */}
      {resultCount > 0 && !isScanning && (
        <>
          {onCompare && (
            <Tooltip content="Compare with previous scan" relationship="label">
              <Button
                appearance="subtle"
                size="small"
                icon={<History16Regular />}
                onClick={onCompare}
                className={styles.toolbarButton}
              />
            </Tooltip>
          )}

          {onCopy && (
            <Tooltip content="Copy results to clipboard" relationship="label">
              <Button
                appearance="subtle"
                size="small"
                icon={<Copy16Regular />}
                onClick={onCopy}
                className={styles.toolbarButton}
              />
            </Tooltip>
          )}

          {onExport && (
            <Tooltip content="Export results" relationship="label">
              <Button
                appearance="subtle"
                size="small"
                icon={<Save16Regular />}
                onClick={onExport}
                className={styles.toolbarButton}
              />
            </Tooltip>
          )}

          <Divider vertical className={styles.divider} />

          {onClear && (
            <Tooltip content="Clear results" relationship="label">
              <Button
                appearance="subtle"
                size="small"
                icon={<Delete16Regular />}
                onClick={onClear}
                className={styles.toolbarButton}
              />
            </Tooltip>
          )}
        </>
      )}
    </div>
  )
}

export default CompactToolbar
