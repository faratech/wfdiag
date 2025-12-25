import React, { useCallback, useState } from 'react'
import {
  makeStyles,
  tokens,
  shorthands,
  Text,
  Button,
  Spinner,
  Tooltip,
  Badge
} from '@fluentui/react-components'
import {
  Sparkle20Regular,
  ArrowSync16Regular,
  ErrorCircle16Regular,
  Checkmark16Regular
} from '@fluentui/react-icons'
import { useAI } from '../hooks/useAI'

const useStyles = makeStyles({
  container: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalS),
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalM),
    backgroundColor: 'var(--theme-card-ai-bg, rgba(99, 102, 241, 0.08))',
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    ...shorthands.border('1px', 'solid', 'var(--theme-card-ai-border, rgba(99, 102, 241, 0.2))'),
  },

  header: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    ...shorthands.gap(tokens.spacingHorizontalS),
  },

  titleSection: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalS),
  },

  title: {
    fontSize: tokens.fontSizeBase200,
    fontWeight: tokens.fontWeightSemibold,
    color: 'var(--theme-ai-accent, rgb(99, 102, 241))',
    textTransform: 'uppercase' as const,
    letterSpacing: '0.5px',
  },

  content: {
    fontSize: tokens.fontSizeBase300,
    lineHeight: tokens.lineHeightBase300,
    color: tokens.colorNeutralForeground1,
  },

  fallback: {
    fontSize: tokens.fontSizeBase300,
    color: tokens.colorNeutralForeground3,
    fontStyle: 'italic',
  },

  error: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorPaletteRedForeground1,
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
  },

  loadingContainer: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalS),
  },

  analyzeButton: {
    minWidth: 'auto',
  },

  providerBadge: {
    fontSize: tokens.fontSizeBase100,
  }
})

export interface AIInterpretationPanelProps {
  /** Task ID for caching */
  taskId: string
  /** Task name for display and prompt context */
  taskName: string
  /** Diagnostic output to analyze */
  output: string
  /** Whether the diagnostic was successful */
  isSuccess: boolean
  /** Fallback message when AI is not available or disabled */
  fallbackMessage?: string
  /** Whether to show the analyze button (default: true) */
  showAnalyzeButton?: boolean
  /** Custom class name */
  className?: string
}

export const AIInterpretationPanel: React.FC<AIInterpretationPanelProps> = ({
  taskId,
  taskName,
  output,
  isSuccess,
  fallbackMessage,
  showAnalyzeButton = true,
  className
}) => {
  const styles = useStyles()
  const {
    aiEnabled,
    isAIAvailable,
    activeProvider,
    requestDiagnosticInterpretation,
    getCachedDiagnosticInterpretation,
    isDiagnosticLoading,
    getDiagnosticError,
    getProviderDisplayName
  } = useAI()

  const [hasRequested, setHasRequested] = useState(false)

  const cached = getCachedDiagnosticInterpretation(taskId)
  const isLoading = isDiagnosticLoading(taskId)
  const error = getDiagnosticError(taskId)

  const defaultFallback = isSuccess
    ? 'Operating within normal parameters.'
    : 'Review output for specific details.'

  const handleAnalyze = useCallback(async () => {
    setHasRequested(true)
    await requestDiagnosticInterpretation(taskId, taskName, output)
  }, [taskId, taskName, output, requestDiagnosticInterpretation])

  // If AI is disabled or unavailable, show fallback
  if (!aiEnabled || !isAIAvailable) {
    return (
      <div className={`${styles.container} ${className ?? ''}`}>
        <div className={styles.header}>
          <div className={styles.titleSection}>
            <Sparkle20Regular />
            <Text className={styles.title}>AI Interpretation</Text>
          </div>
          {!isAIAvailable && (
            <Badge appearance="outline" size="small" color="subtle">
              Unavailable
            </Badge>
          )}
        </div>
        <Text className={styles.fallback}>
          {fallbackMessage ?? defaultFallback}
        </Text>
      </div>
    )
  }

  // Show loading state
  if (isLoading) {
    return (
      <div className={`${styles.container} ${className ?? ''}`}>
        <div className={styles.header}>
          <div className={styles.titleSection}>
            <Sparkle20Regular />
            <Text className={styles.title}>AI Interpretation</Text>
          </div>
          <Badge appearance="tint" size="small" color="brand" className={styles.providerBadge}>
            {getProviderDisplayName(activeProvider)}
          </Badge>
        </div>
        <div className={styles.loadingContainer}>
          <Spinner size="tiny" />
          <Text size={200}>Analyzing with {getProviderDisplayName(activeProvider)}...</Text>
        </div>
      </div>
    )
  }

  // Show error state
  if (error && hasRequested) {
    return (
      <div className={`${styles.container} ${className ?? ''}`}>
        <div className={styles.header}>
          <div className={styles.titleSection}>
            <Sparkle20Regular />
            <Text className={styles.title}>AI Interpretation</Text>
          </div>
          {showAnalyzeButton && (
            <Tooltip content="Try again" relationship="label">
              <Button
                appearance="subtle"
                size="small"
                icon={<ArrowSync16Regular />}
                onClick={handleAnalyze}
                className={styles.analyzeButton}
              />
            </Tooltip>
          )}
        </div>
        <div className={styles.error}>
          <ErrorCircle16Regular />
          <Text>Analysis failed. Click to retry.</Text>
        </div>
      </div>
    )
  }

  // Show cached result
  if (cached) {
    return (
      <div className={`${styles.container} ${className ?? ''}`}>
        <div className={styles.header}>
          <div className={styles.titleSection}>
            <Sparkle20Regular />
            <Text className={styles.title}>AI Interpretation</Text>
            <Checkmark16Regular style={{ color: tokens.colorPaletteGreenForeground1 }} />
          </div>
          <Badge appearance="tint" size="small" color="brand" className={styles.providerBadge}>
            {getProviderDisplayName(activeProvider)}
          </Badge>
        </div>
        <Text className={styles.content}>{cached}</Text>
      </div>
    )
  }

  // Show analyze button (on-demand)
  return (
    <div className={`${styles.container} ${className ?? ''}`}>
      <div className={styles.header}>
        <div className={styles.titleSection}>
          <Sparkle20Regular />
          <Text className={styles.title}>AI Interpretation</Text>
        </div>
        {showAnalyzeButton && (
          <Tooltip content={`Analyze with ${getProviderDisplayName(activeProvider)}`} relationship="label">
            <Button
              appearance="subtle"
              size="small"
              icon={<Sparkle20Regular />}
              onClick={handleAnalyze}
              className={styles.analyzeButton}
            >
              Analyze
            </Button>
          </Tooltip>
        )}
      </div>
      <Text className={styles.fallback}>
        {fallbackMessage ?? defaultFallback}
      </Text>
    </div>
  )
}

export default AIInterpretationPanel
