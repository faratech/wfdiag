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
    whiteSpace: 'pre-wrap',
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

export interface AIAnalysisPanelProps {
  /** Unique cache key for this analysis (e.g., "__monitoring_analysis__") */
  cacheKey: string
  /** Panel title */
  title: string
  /** Whether there is data available to analyze */
  hasData: boolean
  /** Cached analysis result */
  cachedResult?: string | null
  /** Whether analysis is loading */
  isLoading?: boolean
  /** Error message if analysis failed */
  error?: string | null
  /** Callback to trigger analysis */
  onAnalyze: () => void
  /** Custom analyze button text (default: "Analyze") */
  analyzeButtonText?: string
  /** Message when no data is available */
  noDataMessage?: string
  /** Message when ready to analyze */
  readyMessage?: string
  /** Custom class name */
  className?: string
}

export const AIAnalysisPanel: React.FC<AIAnalysisPanelProps> = ({
  cacheKey: _cacheKey,
  title,
  hasData,
  cachedResult,
  isLoading = false,
  error,
  onAnalyze,
  analyzeButtonText = 'Analyze',
  noDataMessage = 'No data available to analyze.',
  readyMessage = 'Click to get AI-powered analysis.',
  className
}) => {
  // cacheKey is passed for interface consistency but not used directly in this component
  void _cacheKey
  const styles = useStyles()
  const {
    aiEnabled,
    isAIAvailable,
    activeProvider,
    getProviderDisplayName
  } = useAI()

  const [hasRequested, setHasRequested] = useState(false)

  const handleAnalyze = useCallback(() => {
    setHasRequested(true)
    onAnalyze()
  }, [onAnalyze])

  // If AI is disabled or unavailable, show fallback with explanation
  if (!aiEnabled || !isAIAvailable) {
    const unavailableReason = !aiEnabled
      ? 'AI analysis is disabled in Settings.'
      : 'No AI provider available. Configure OpenAI in Settings or use a Copilot+ PC for local AI.'

    return (
      <div className={`${styles.container} ${className ?? ''}`}>
        <div className={styles.header}>
          <div className={styles.titleSection}>
            <Sparkle20Regular />
            <Text className={styles.title}>{title}</Text>
          </div>
          <Tooltip content={unavailableReason} relationship="description">
            <Badge appearance="outline" size="small" color="subtle">
              {!aiEnabled ? 'Disabled' : 'Unavailable'}
            </Badge>
          </Tooltip>
        </div>
        <Text className={styles.fallback}>
          {!aiEnabled ? 'Enable AI in Settings to use this feature.' : unavailableReason}
        </Text>
      </div>
    )
  }

  // No data to analyze
  if (!hasData) {
    return (
      <div className={`${styles.container} ${className ?? ''}`}>
        <div className={styles.header}>
          <div className={styles.titleSection}>
            <Sparkle20Regular />
            <Text className={styles.title}>{title}</Text>
          </div>
        </div>
        <Text className={styles.fallback}>{noDataMessage}</Text>
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
            <Text className={styles.title}>{title}</Text>
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
            <Text className={styles.title}>{title}</Text>
          </div>
          <Tooltip content="Try again" relationship="label">
            <Button
              appearance="subtle"
              size="small"
              icon={<ArrowSync16Regular />}
              onClick={handleAnalyze}
              className={styles.analyzeButton}
            />
          </Tooltip>
        </div>
        <div className={styles.error}>
          <ErrorCircle16Regular />
          <Text>{error || 'Analysis failed. Click to retry.'}</Text>
        </div>
      </div>
    )
  }

  // Show cached result
  if (cachedResult) {
    return (
      <div className={`${styles.container} ${className ?? ''}`}>
        <div className={styles.header}>
          <div className={styles.titleSection}>
            <Sparkle20Regular />
            <Text className={styles.title}>{title}</Text>
            <Checkmark16Regular style={{ color: tokens.colorPaletteGreenForeground1 }} />
          </div>
          <div style={{ display: 'flex', gap: tokens.spacingHorizontalS, alignItems: 'center' }}>
            <Badge appearance="tint" size="small" color="brand" className={styles.providerBadge}>
              {getProviderDisplayName(activeProvider)}
            </Badge>
            <Tooltip content="Refresh analysis" relationship="label">
              <Button
                appearance="subtle"
                size="small"
                icon={<ArrowSync16Regular />}
                onClick={handleAnalyze}
                className={styles.analyzeButton}
              />
            </Tooltip>
          </div>
        </div>
        <Text className={styles.content}>{cachedResult}</Text>
      </div>
    )
  }

  // Show analyze button (ready state)
  return (
    <div className={`${styles.container} ${className ?? ''}`}>
      <div className={styles.header}>
        <div className={styles.titleSection}>
          <Sparkle20Regular />
          <Text className={styles.title}>{title}</Text>
        </div>
        <Tooltip
          content={`Get AI-powered analysis using ${getProviderDisplayName(activeProvider)}`}
          relationship="description"
        >
          <Button
            appearance="subtle"
            size="small"
            icon={<Sparkle20Regular />}
            onClick={handleAnalyze}
            className={styles.analyzeButton}
          >
            {analyzeButtonText}
          </Button>
        </Tooltip>
      </div>
      <Text className={styles.fallback}>{readyMessage}</Text>
    </div>
  )
}

export default AIAnalysisPanel
