import React, { useState } from 'react'
import {
  makeStyles,
  tokens,
  shorthands,
  Text,
  Button,
  Tooltip
} from '@fluentui/react-components'
import {
  ChevronRight12Regular,
  ChevronDown12Regular,
  CheckmarkCircle12Filled,
  ErrorCircle12Filled,
  Warning12Filled,
  Copy16Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  row: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.padding('4px', '8px'),
    ...shorthands.gap('8px'),
    cursor: 'pointer',
    backgroundColor: 'transparent',
    ':hover': {
      backgroundColor: tokens.colorNeutralBackground1Hover,
    },
    ':focus': {
      backgroundColor: tokens.colorNeutralBackground1Selected,
      ...shorthands.outline('1px', 'solid', tokens.colorNeutralStroke1),
    },
    fontSize: tokens.fontSizeBase200,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
    minHeight: '28px',
  },
  rowError: {
    backgroundColor: tokens.colorPaletteRedBackground1,
    ':hover': {
      backgroundColor: tokens.colorPaletteRedBackground2,
    },
  },
  rowWarning: {
    backgroundColor: tokens.colorPaletteYellowBackground1,
    ':hover': {
      backgroundColor: tokens.colorPaletteYellowBackground2,
    },
  },
  expandIcon: {
    width: '12px',
    height: '12px',
    flexShrink: 0,
    color: tokens.colorNeutralForeground3,
  },
  statusIcon: {
    width: '12px',
    height: '12px',
    flexShrink: 0,
  },
  name: {
    flex: 1,
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    color: tokens.colorNeutralForeground1,
    fontSize: tokens.fontSizeBase200,
  },
  status: {
    width: '80px',
    textAlign: 'right',
    color: tokens.colorNeutralForeground2,
    fontSize: tokens.fontSizeBase100,
  },
  duration: {
    width: '50px',
    textAlign: 'right',
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase100,
  },
  details: {
    ...shorthands.padding('8px', '8px', '8px', '32px'),
    backgroundColor: tokens.colorNeutralBackground2,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
    fontSize: tokens.fontSizeBase200,
  },
  detailsHeader: {
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginBottom: '8px',
  },
  detailsLabel: {
    fontSize: tokens.fontSizeBase100,
    color: tokens.colorNeutralForeground3,
    textTransform: 'uppercase',
    fontWeight: tokens.fontWeightSemibold,
  },
  output: {
    fontFamily: 'Consolas, "Cascadia Code", monospace',
    fontSize: '11px',
    lineHeight: '1.4',
    color: tokens.colorNeutralForeground1,
    backgroundColor: tokens.colorNeutralBackground1,
    ...shorthands.padding('8px'),
    ...shorthands.borderRadius('2px'),
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke2),
    maxHeight: '200px',
    overflowY: 'auto',
    whiteSpace: 'pre-wrap',
    wordBreak: 'break-word',
  },
  error: {
    color: tokens.colorPaletteRedForeground1,
    backgroundColor: tokens.colorPaletteRedBackground1,
  },
  copyButton: {
    minWidth: 'auto',
    height: '20px',
    ...shorthands.padding('0', '6px'),
  },
})

interface DiagnosticRowProps {
  name: string
  status: 'passed' | 'failed' | 'warning'
  duration?: number
  output?: string
  error?: string
  onCopy?: (text: string) => void
}

export const DiagnosticRow: React.FC<DiagnosticRowProps> = ({
  name,
  status,
  duration,
  output,
  error,
  onCopy
}) => {
  const styles = useStyles()
  const [expanded, setExpanded] = useState(false)

  const getStatusIcon = () => {
    switch (status) {
      case 'passed':
        return <CheckmarkCircle12Filled className={styles.statusIcon} style={{ color: tokens.colorPaletteGreenForeground1 }} />
      case 'failed':
        return <ErrorCircle12Filled className={styles.statusIcon} style={{ color: tokens.colorPaletteRedForeground1 }} />
      case 'warning':
        return <Warning12Filled className={styles.statusIcon} style={{ color: tokens.colorPaletteYellowForeground1 }} />
    }
  }

  const getStatusText = () => {
    switch (status) {
      case 'passed': return 'OK'
      case 'failed': return 'Error'
      case 'warning': return 'Warning'
    }
  }

  const getRowClass = () => {
    let cls = styles.row
    if (status === 'failed') cls += ` ${styles.rowError}`
    if (status === 'warning') cls += ` ${styles.rowWarning}`
    return cls
  }

  const formatOutput = () => {
    if (error) return error
    if (!output) return 'No output'
    if (typeof output === 'string') {
      try {
        return JSON.stringify(JSON.parse(output), null, 2)
      } catch {
        return output
      }
    }
    return JSON.stringify(output, null, 2)
  }

  return (
    <>
      <div
        className={getRowClass()}
        onClick={() => setExpanded(!expanded)}
        tabIndex={0}
        role="row"
      >
        {expanded
          ? <ChevronDown12Regular className={styles.expandIcon} />
          : <ChevronRight12Regular className={styles.expandIcon} />
        }
        {getStatusIcon()}
        <Text className={styles.name}>{name}</Text>
        <Text className={styles.status}>{getStatusText()}</Text>
        <Text className={styles.duration}>{duration ? `${duration}ms` : '-'}</Text>
      </div>
      {expanded && (
        <div className={styles.details}>
          <div className={styles.detailsHeader}>
            <Text className={styles.detailsLabel}>Output</Text>
            {onCopy && (
              <Tooltip content="Copy output" relationship="label">
                <Button
                  appearance="subtle"
                  size="small"
                  icon={<Copy16Regular />}
                  className={styles.copyButton}
                  onClick={(e) => {
                    e.stopPropagation()
                    onCopy(formatOutput())
                  }}
                />
              </Tooltip>
            )}
          </div>
          <pre className={`${styles.output} ${error ? styles.error : ''}`}>
            {formatOutput()}
          </pre>
        </div>
      )}
    </>
  )
}

export default DiagnosticRow
