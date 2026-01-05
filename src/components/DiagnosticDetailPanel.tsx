import React from 'react'
import { makeStyles, shorthands, tokens, Button, Spinner } from '@fluentui/react-components'
import { Copy16Regular, Sparkle20Regular } from '@fluentui/react-icons'
import { DiagnosticItem } from './DiagnosticListPanel'
import { StatusBadge } from './StatusBadge'
import { JsonRenderer, parseJsonSafe } from './JsonRenderer'
import { EGUI_COLORS, TASK_ICONS } from '../theme'

interface DiagnosticDetailPanelProps {
  item: DiagnosticItem | null
  onCopy?: () => void
  // AI integration
  aiEnabled?: boolean
  aiContent?: string
  onRequestAI?: () => void
  isLoadingAI?: boolean
}

const useStyles = makeStyles({
  panel: {
    flex: 1,
    display: 'flex',
    flexDirection: 'column',
    height: '100%',
    overflow: 'hidden',
    backgroundColor: tokens.colorNeutralBackground1,
  },
  emptyState: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    height: '100%',
    color: tokens.colorNeutralForeground3,
  },
  emptyTitle: {
    fontSize: '16px',
    fontWeight: 500,
    marginBottom: '8px',
  },
  emptyHint: {
    fontSize: '13px',
  },
  header: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('12px'),
    ...shorthands.padding('16px', '20px'),
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
    flexShrink: 0,
  },
  headerIcon: {
    fontSize: '24px',
  },
  headerTitle: {
    fontSize: '18px',
    fontWeight: 600,
    color: tokens.colorNeutralForeground1,
    flex: 1,
  },
  headerMeta: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('12px'),
  },
  duration: {
    fontSize: '12px',
    color: tokens.colorNeutralForeground3,
  },
  content: {
    flex: 1,
    overflow: 'auto',
    ...shorthands.padding('20px'),
  },
  aiPanel: {
    backgroundColor: EGUI_COLORS.AI_PANEL_BG,
    ...shorthands.border('1px', 'solid', EGUI_COLORS.AI_PANEL_STROKE),
    ...shorthands.borderRadius('8px'),
    ...shorthands.padding('16px'),
    marginBottom: '20px',
  },
  aiHeader: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('8px'),
    marginBottom: '12px',
  },
  aiTitle: {
    fontSize: '13px',
    fontWeight: 600,
    color: EGUI_COLORS.PRIMARY,
  },
  aiContent: {
    fontSize: '13px',
    lineHeight: '1.6',
    color: tokens.colorNeutralForeground1,
    whiteSpace: 'pre-wrap',
  },
  aiLoading: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('8px'),
    fontSize: '13px',
    color: tokens.colorNeutralForeground3,
  },
  aiButton: {
    marginTop: '8px',
  },
  errorFrame: {
    backgroundColor: 'rgba(220, 80, 80, 0.1)',
    ...shorthands.border('1px', 'solid', EGUI_COLORS.ERROR),
    ...shorthands.borderRadius('6px'),
    ...shorthands.padding('12px'),
    marginBottom: '16px',
  },
  errorTitle: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('6px'),
    fontSize: '13px',
    fontWeight: 600,
    color: EGUI_COLORS.ERROR,
    marginBottom: '8px',
  },
  errorContent: {
    fontSize: '12px',
    color: tokens.colorNeutralForeground1,
    whiteSpace: 'pre-wrap',
    fontFamily: 'monospace',
  },
  outputSection: {
    marginTop: '8px',
  },
  sectionTitle: {
    fontSize: '12px',
    fontWeight: 600,
    color: tokens.colorNeutralForeground3,
    marginBottom: '12px',
    textTransform: 'uppercase',
    letterSpacing: '0.5px',
  },
})

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  const seconds = ms / 1000
  if (seconds < 60) return `${seconds.toFixed(1)}s`
  const minutes = Math.floor(seconds / 60)
  const remainingSeconds = seconds % 60
  return `${minutes}m ${remainingSeconds.toFixed(0)}s`
}

export const DiagnosticDetailPanel: React.FC<DiagnosticDetailPanelProps> = ({
  item,
  onCopy,
  aiEnabled,
  aiContent,
  onRequestAI,
  isLoadingAI,
}) => {
  const styles = useStyles()

  // Empty state when no item selected
  if (!item) {
    return (
      <div className={styles.panel}>
        <div className={styles.emptyState}>
          <div className={styles.emptyTitle}>Select a diagnostic</div>
          <div className={styles.emptyHint}>
            Click an item on the left to view details
          </div>
        </div>
      </div>
    )
  }

  const taskIcon = TASK_ICONS[item.id] || '\u{1F4C4}'
  const parsedOutput = parseJsonSafe(item.output)

  return (
    <div className={styles.panel}>
      {/* Header */}
      <div className={styles.header}>
        <span className={styles.headerIcon}>{taskIcon}</span>
        <span className={styles.headerTitle}>{item.name}</span>
        <div className={styles.headerMeta}>
          <StatusBadge
            text={item.success ? '\u2713 Passed' : '\u2717 Failed'}
            variant={item.success ? 'success' : 'error'}
            size="medium"
          />
          <span className={styles.duration}>{formatDuration(item.durationMs)}</span>
          {onCopy && (
            <Button
              icon={<Copy16Regular />}
              appearance="subtle"
              size="small"
              onClick={onCopy}
              title="Copy to clipboard"
            />
          )}
        </div>
      </div>

      {/* Content */}
      <div className={styles.content}>
        {/* AI Interpretation Panel */}
        {aiEnabled && (
          <div className={styles.aiPanel}>
            <div className={styles.aiHeader}>
              <Sparkle20Regular style={{ color: EGUI_COLORS.PRIMARY }} />
              <span className={styles.aiTitle}>AI Interpretation</span>
            </div>
            {isLoadingAI ? (
              <div className={styles.aiLoading}>
                <Spinner size="tiny" />
                <span>Analyzing...</span>
              </div>
            ) : aiContent ? (
              <div className={styles.aiContent}>{aiContent}</div>
            ) : (
              <Button
                className={styles.aiButton}
                appearance="subtle"
                size="small"
                onClick={onRequestAI}
              >
                Analyze with AI
              </Button>
            )}
          </div>
        )}

        {/* Error display */}
        {item.error && (
          <div className={styles.errorFrame}>
            <div className={styles.errorTitle}>
              <span>\u26A0</span>
              <span>Error</span>
            </div>
            <div className={styles.errorContent}>{item.error}</div>
          </div>
        )}

        {/* Output section */}
        <div className={styles.outputSection}>
          <div className={styles.sectionTitle}>Output</div>
          <JsonRenderer value={parsedOutput} />
        </div>
      </div>
    </div>
  )
}
