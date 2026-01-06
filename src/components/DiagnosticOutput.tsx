/**
 * DiagnosticOutput - Shared diagnostic output and error display
 * Used by DiagnosticDetailPanel and DiagnosticRow
 */

import React, { useMemo } from 'react'
import {
  makeStyles,
  tokens,
  shorthands,
  Button,
  Tooltip,
} from '@fluentui/react-components'
import { Copy16Regular } from '@fluentui/react-icons'
import { EGUI_COLORS } from '../theme'

const useStyles = makeStyles({
  container: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalM),
  },
  header: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
  },
  label: {
    fontSize: tokens.fontSizeBase200,
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground2,
    textTransform: 'uppercase',
    letterSpacing: '0.5px',
  },
  copyButton: {
    minWidth: 'auto',
  },
  output: {
    fontFamily: 'Consolas, Monaco, "Courier New", monospace',
    fontSize: '12px',
    lineHeight: '1.5',
    ...shorthands.padding(tokens.spacingVerticalM),
    backgroundColor: tokens.colorNeutralBackground3,
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    ...shorthands.overflow('auto'),
    maxHeight: '400px',
    whiteSpace: 'pre-wrap',
    wordBreak: 'break-word',
    color: tokens.colorNeutralForeground1,
  },
  outputCompact: {
    maxHeight: '200px',
    fontSize: '11px',
    ...shorthands.padding(tokens.spacingVerticalS),
  },
  errorFrame: {
    backgroundColor: EGUI_COLORS.ERROR_BG,
    ...shorthands.border('1px', 'solid', EGUI_COLORS.ERROR),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    ...shorthands.overflow('hidden'),
  },
  errorHeader: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalS),
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
    backgroundColor: 'rgba(239, 68, 68, 0.15)',
    borderBottom: `1px solid ${EGUI_COLORS.ERROR}`,
    color: EGUI_COLORS.ERROR,
    fontWeight: tokens.fontWeightSemibold,
    fontSize: tokens.fontSizeBase200,
  },
  errorContent: {
    ...shorthands.padding(tokens.spacingVerticalM),
    fontFamily: 'Consolas, Monaco, "Courier New", monospace',
    fontSize: '12px',
    lineHeight: '1.5',
    color: tokens.colorNeutralForeground1,
    whiteSpace: 'pre-wrap',
    wordBreak: 'break-word',
  },
})

export interface DiagnosticOutputProps {
  output?: string | object | null
  error?: string | null
  variant?: 'full' | 'compact'
  showHeader?: boolean
  onCopy?: (text: string) => void
}

/**
 * Format diagnostic output for display
 */
export function formatDiagnosticOutput(output: string | object | null | undefined): string {
  if (!output) return 'No output'

  if (typeof output === 'string') {
    // Try to parse and pretty-print JSON
    try {
      const parsed = JSON.parse(output)
      return JSON.stringify(parsed, null, 2)
    } catch {
      return output
    }
  }

  return JSON.stringify(output, null, 2)
}

/**
 * Parse diagnostic output into structured data if possible
 */
export function parseDiagnosticOutput(output: string | object | null | undefined): unknown {
  if (!output) return null

  if (typeof output === 'string') {
    try {
      return JSON.parse(output)
    } catch {
      return output
    }
  }

  return output
}

export const DiagnosticOutput: React.FC<DiagnosticOutputProps> = ({
  output,
  error,
  variant = 'full',
  showHeader = true,
  onCopy,
}) => {
  const styles = useStyles()

  const formattedOutput = useMemo(() => formatDiagnosticOutput(output), [output])

  const handleCopy = () => {
    if (onCopy) {
      onCopy(error || formattedOutput)
    }
  }

  const outputClass = variant === 'compact'
    ? `${styles.output} ${styles.outputCompact}`
    : styles.output

  return (
    <div className={styles.container}>
      {/* Error display */}
      {error && (
        <div className={styles.errorFrame}>
          <div className={styles.errorHeader}>
            <span>⚠</span>
            <span>Error</span>
          </div>
          <div className={styles.errorContent}>{error}</div>
        </div>
      )}

      {/* Output display */}
      {showHeader && (
        <div className={styles.header}>
          <span className={styles.label}>Output</span>
          {onCopy && (
            <Tooltip content="Copy to clipboard" relationship="label">
              <Button
                appearance="subtle"
                size="small"
                icon={<Copy16Regular />}
                className={styles.copyButton}
                onClick={handleCopy}
              />
            </Tooltip>
          )}
        </div>
      )}

      <pre className={outputClass}>
        {formattedOutput}
      </pre>
    </div>
  )
}

export default DiagnosticOutput
