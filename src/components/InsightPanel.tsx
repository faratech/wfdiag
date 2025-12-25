import React from 'react'
import {
  makeStyles,
  shorthands,
  tokens,
  Text,
  Button,
  Spinner,
  Badge
} from '@fluentui/react-components'
import { Sparkle24Regular, ArrowSync16Regular } from '@fluentui/react-icons'

const useStyles = makeStyles({
  container: {
    backgroundColor: tokens.colorBrandBackground2,
    borderLeft: `3px solid ${tokens.colorBrandStroke1}`,
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalL),
    marginBottom: tokens.spacingVerticalL,
    display: 'flex',
    gap: tokens.spacingHorizontalM,
    alignItems: 'flex-start',
    borderRadius: `0 ${tokens.borderRadiusMedium} ${tokens.borderRadiusMedium} 0`,
  },
  icon: {
    color: tokens.colorPalettePurpleForeground2,
    marginTop: '2px',
  },
  content: {
    display: 'flex',
    flexDirection: 'column',
    gap: tokens.spacingVerticalXS,
    flex: 1,
  },
  header: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: tokens.spacingHorizontalS,
  },
  titleSection: {
    display: 'flex',
    alignItems: 'center',
    gap: tokens.spacingHorizontalS,
  },
  title: {
    fontWeight: 600,
    color: tokens.colorPalettePurpleForeground2,
    fontSize: tokens.fontSizeBase200,
    textTransform: 'uppercase',
    letterSpacing: '0.05em',
  },
  text: {
    color: tokens.colorNeutralForeground2,
    fontSize: tokens.fontSizeBase300,
    lineHeight: 1.5,
  },
  loadingContainer: {
    display: 'flex',
    alignItems: 'center',
    gap: tokens.spacingHorizontalS,
  },
  analyzeButton: {
    minWidth: 'auto',
  },
  providerBadge: {
    fontSize: tokens.fontSizeBase100,
  }
})

interface InsightPanelProps {
  title?: string
  content: string
  /** Whether AI is available and enabled */
  aiEnabled?: boolean
  /** Current AI interpretation (if available) */
  aiContent?: string | null
  /** Callback to request AI analysis */
  onRequestAI?: () => void
  /** Whether AI analysis is in progress */
  isLoadingAI?: boolean
  /** AI provider name for display */
  aiProviderName?: string
  /** Whether AI has been analyzed */
  hasAIResult?: boolean
}

export const InsightPanel: React.FC<InsightPanelProps> = ({
  title = "System Analysis",
  content,
  aiEnabled = false,
  aiContent,
  onRequestAI,
  isLoadingAI = false,
  aiProviderName,
  hasAIResult = false
}) => {
  const styles = useStyles()

  // Use AI content if available, otherwise use default content
  const displayContent = aiContent || content

  return (
    <div className={styles.container}>
      <Sparkle24Regular className={styles.icon} />
      <div className={styles.content}>
        <div className={styles.header}>
          <div className={styles.titleSection}>
            <Text className={styles.title}>{title}</Text>
            {hasAIResult && aiProviderName && (
              <Badge appearance="tint" size="small" color="brand" className={styles.providerBadge}>
                {aiProviderName}
              </Badge>
            )}
          </div>
          {aiEnabled && onRequestAI && !isLoadingAI && !hasAIResult && (
            <Button
              appearance="subtle"
              size="small"
              icon={<Sparkle24Regular />}
              onClick={onRequestAI}
              className={styles.analyzeButton}
            >
              Analyze with AI
            </Button>
          )}
          {aiEnabled && onRequestAI && !isLoadingAI && hasAIResult && (
            <Button
              appearance="subtle"
              size="small"
              icon={<ArrowSync16Regular />}
              onClick={onRequestAI}
              className={styles.analyzeButton}
              title="Re-analyze"
            />
          )}
        </div>
        {isLoadingAI ? (
          <div className={styles.loadingContainer}>
            <Spinner size="tiny" />
            <Text size={200}>Analyzing with {aiProviderName || 'AI'}...</Text>
          </div>
        ) : (
          <Text className={styles.text}>{displayContent}</Text>
        )}
      </div>
    </div>
  )
}