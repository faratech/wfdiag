import React, { useState } from 'react'
import {
  Card,
  Badge,
  Button,
  tokens,
  makeStyles,
  shorthands,
  Caption1,
  Caption2,
  Subtitle2
} from '@fluentui/react-components'
import {
  CheckmarkCircle16Regular,
  ErrorCircle16Regular,
  Clock16Regular,
  ChevronRight16Regular,
  Copy16Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  card: {
    marginBottom: tokens.spacingVerticalM,
    backgroundColor: 'rgba(30, 41, 59, 0.3)',
    ...shorthands.border('1px', 'solid', 'rgba(255, 255, 255, 0.05)'),
  },
  header: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalL),
    cursor: 'pointer',
    ':hover': {
      backgroundColor: 'rgba(30, 41, 59, 0.5)',
    }
  },
  titleSection: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalL),
    flex: 1,
  },
  metaSection: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
  },
  content: {
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalL),
    backgroundColor: 'rgba(15, 23, 42, 0.5)',
  },
  dataGrid: {
    display: 'grid',
    gridTemplateColumns: 'minmax(150px, 1fr) 2fr',
    ...shorthands.gap(tokens.spacingHorizontalM, tokens.spacingVerticalS),
    marginBottom: tokens.spacingVerticalM,
  },
  dataLabel: {
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase200,
  },
  dataValue: {
    color: tokens.colorNeutralForeground1,
    fontSize: tokens.fontSizeBase300,
    wordBreak: 'break-word',
  },
  errorMessage: {
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalL),
    backgroundColor: 'rgba(239, 68, 68, 0.1)',
    ...shorthands.border('1px', 'solid', 'rgba(239, 68, 68, 0.3)'),
    ...shorthands.borderRadius(tokens.borderRadiusSmall),
    marginTop: tokens.spacingVerticalM,
  },
  codeBlock: {
    fontFamily: 'Consolas, "Courier New", monospace',
    fontSize: tokens.fontSizeBase200,
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalM),
    backgroundColor: 'rgba(0, 0, 0, 0.3)',
    ...shorthands.borderRadius(tokens.borderRadiusSmall),
    whiteSpace: 'pre-wrap',
    wordBreak: 'break-word',
    marginTop: tokens.spacingVerticalM,
  },
  expandIcon: {
    transition: 'transform 0.2s ease',
  },
  expandIconRotated: {
    transform: 'rotate(90deg)',
  }
})

export interface ScanResultCardProps {
  taskName: string
  taskDescription?: string
  success: boolean
  duration: number
  output?: any
  error?: string
  category?: string
  onCopyOutput?: (output: string) => void
}

export const ScanResultCard: React.FC<ScanResultCardProps> = ({
  taskName,
  taskDescription,
  success,
  duration,
  output,
  error,
  category,
  onCopyOutput
}) => {
  const styles = useStyles()
  const [isExpanded, setIsExpanded] = useState(false)

  const formatDuration = (ms: number) => {
    if (ms < 1000) return `${ms}ms`
    return `${(ms / 1000).toFixed(2)}s`
  }

  const renderOutput = (data: any, depth: number = 0): React.ReactNode => {
    if (data === null || data === undefined) return <Caption2>No data</Caption2>

    if (typeof data === 'string') {
      return <span className={styles.dataValue}>{data}</span>
    }

    if (typeof data === 'boolean') {
      return (
        <Badge
          appearance="filled"
          color={data ? 'success' : 'subtle'}
          size="small"
        >
          {data ? 'Yes' : 'No'}
        </Badge>
      )
    }

    if (typeof data === 'number') {
      return <span className={styles.dataValue}>{data.toLocaleString()}</span>
    }

    if (Array.isArray(data)) {
      if (data.length === 0) return <Caption2>Empty array</Caption2>

      return (
        <div style={{ marginLeft: depth * 16 }}>
          {data.slice(0, 10).map((item, index) => (
            <div key={index} style={{ marginBottom: tokens.spacingVerticalXS }}>
              {typeof item === 'object' ?
                renderOutput(item, depth + 1) :
                <Caption1>{String(item)}</Caption1>
              }
            </div>
          ))}
          {data.length > 10 && (
            <Caption2 style={{ fontStyle: 'italic' }}>
              ... and {data.length - 10} more items
            </Caption2>
          )}
        </div>
      )
    }

    if (typeof data === 'object') {
      const entries = Object.entries(data).filter(([, v]) => v !== undefined)
      if (entries.length === 0) return <Caption2>Empty object</Caption2>

      return (
        <div className={styles.dataGrid} style={{ marginLeft: depth * 16 }}>
          {entries.slice(0, 20).map(([key, value]) => (
            <React.Fragment key={key}>
              <Caption1 className={styles.dataLabel}>
                {key.replace(/_/g, ' ').replace(/\b\w/g, l => l.toUpperCase())}:
              </Caption1>
              <div>{renderOutput(value, depth + 1)}</div>
            </React.Fragment>
          ))}
          {entries.length > 20 && (
            <Caption2 style={{ gridColumn: '1 / -1', fontStyle: 'italic' }}>
              ... and {entries.length - 20} more properties
            </Caption2>
          )}
        </div>
      )
    }

    return <Caption1>{String(data)}</Caption1>
  }

  const parseOutput = () => {
    if (!output) return null

    try {
      return typeof output === 'string' ? JSON.parse(output) : output
    } catch {
      return output
    }
  }

  return (
    <Card className={styles.card}>
      <div
        className={styles.header}
        onClick={() => setIsExpanded(!isExpanded)}
      >
        <div className={styles.titleSection}>
          {success ?
            <CheckmarkCircle16Regular primaryFill="#10B981" /> :
            <ErrorCircle16Regular primaryFill="#EF4444" />
          }
          <div>
            <Subtitle2>{taskName}</Subtitle2>
            {taskDescription && (
              <Caption2 style={{
                color: tokens.colorNeutralForeground3,
                marginTop: tokens.spacingVerticalXXS,
                display: 'block'
              }}>
                {taskDescription}
              </Caption2>
            )}
          </div>
        </div>

        <div className={styles.metaSection}>
          {category && (
            <Badge appearance="tint" size="small">
              {category}
            </Badge>
          )}
          <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalXS }}>
            <Clock16Regular />
            <Caption1>{formatDuration(duration)}</Caption1>
          </div>
          <div className={`${styles.expandIcon} ${isExpanded ? styles.expandIconRotated : ''}`}>
            <ChevronRight16Regular />
          </div>
        </div>
      </div>

      {isExpanded && (
        <div className={styles.content}>
          {error ? (
            <div className={styles.errorMessage}>
              <Caption1 style={{ color: '#EF4444' }}>
                <ErrorCircle16Regular style={{ marginRight: tokens.spacingHorizontalXS }} />
                Error: {error}
              </Caption1>
            </div>
          ) : (
            <div>
              {renderOutput(parseOutput())}
              {onCopyOutput && output && (
                <Button
                  appearance="subtle"
                  icon={<Copy16Regular />}
                  size="small"
                  onClick={() => onCopyOutput(typeof output === 'string' ? output : JSON.stringify(output, null, 2))}
                  style={{ marginTop: tokens.spacingVerticalS }}
                >
                  Copy Output
                </Button>
              )}
            </div>
          )}
        </div>
      )}
    </Card>
  )
}