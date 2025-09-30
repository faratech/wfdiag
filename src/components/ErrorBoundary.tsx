import React from 'react'
import {
  Card,
  Title1,
  Title3,
  Body1,
  Button,
  tokens,
  makeStyles,
  shorthands,
} from '@fluentui/react-components'
import { ErrorCircle24Regular, ArrowClockwise24Regular } from '@fluentui/react-icons'
import * as logger from '../utils/logger'

const useStyles = makeStyles({
  container: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    minHeight: '100vh',
    ...shorthands.padding(tokens.spacingVerticalXXL),
    background: 'linear-gradient(135deg, #0B0E19 0%, #12161F 100%)',
  },
  errorCard: {
    maxWidth: '600px',
    width: '100%',
    ...shorthands.padding(tokens.spacingVerticalXXXL, tokens.spacingHorizontalXXL),
    textAlign: 'center',
    background: 'linear-gradient(145deg, rgba(18, 22, 31, 0.95), rgba(26, 31, 46, 0.90))',
    backdropFilter: 'blur(32px) saturate(180%)',
    ...shorthands.border('1px', 'solid', 'rgba(239, 68, 68, 0.2)'),
    ...shorthands.borderRadius(tokens.borderRadiusLarge),
    boxShadow: `
      0 0 0 1px rgba(239, 68, 68, 0.1),
      0 8px 24px rgba(0, 0, 0, 0.4),
      inset 0 1px 0 rgba(255, 255, 255, 0.03)
    `,
  },
  iconContainer: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    marginBottom: tokens.spacingVerticalXL,
  },
  icon: {
    fontSize: '72px',
    color: '#EF4444',
    filter: 'drop-shadow(0 0 16px rgba(239, 68, 68, 0.4))',
  },
  title: {
    marginBottom: tokens.spacingVerticalM,
    color: tokens.colorNeutralForeground1,
  },
  message: {
    marginBottom: tokens.spacingVerticalL,
    color: tokens.colorNeutralForeground3,
    lineHeight: '1.6',
  },
  details: {
    marginTop: tokens.spacingVerticalL,
    marginBottom: tokens.spacingVerticalXL,
    ...shorthands.padding(tokens.spacingVerticalL, tokens.spacingHorizontalL),
    background: 'rgba(0, 0, 0, 0.3)',
    ...shorthands.border('1px', 'solid', 'rgba(239, 68, 68, 0.2)'),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    fontFamily: 'Consolas, "Courier New", monospace',
    fontSize: tokens.fontSizeBase200,
    color: '#EF4444',
    textAlign: 'left',
    whiteSpace: 'pre-wrap',
    wordBreak: 'break-word',
    maxHeight: '200px',
    overflowY: 'auto',
  },
  buttonGroup: {
    display: 'flex',
    ...shorthands.gap(tokens.spacingHorizontalM),
    justifyContent: 'center',
    flexWrap: 'wrap',
  },
})

interface ErrorBoundaryProps {
  children: React.ReactNode
}

interface ErrorBoundaryState {
  hasError: boolean
  error: Error | null
  errorInfo: React.ErrorInfo | null
}

export class ErrorBoundary extends React.Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props)
    this.state = {
      hasError: false,
      error: null,
      errorInfo: null,
    }
  }

  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    // Update state so the next render will show the fallback UI
    return {
      hasError: true,
      error,
    }
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo): void {
    // Log the error to our logging system
    logger.error('ErrorBoundary', 'Component error caught', {
      error: error.toString(),
      errorInfo: errorInfo.componentStack,
      stack: error.stack,
    })

    // Update state with error details
    this.setState({
      error,
      errorInfo,
    })
  }

  handleReload = (): void => {
    // Clear error state and reload the page
    window.location.reload()
  }

  handleReset = (): void => {
    // Try to recover by resetting the error boundary
    this.setState({
      hasError: false,
      error: null,
      errorInfo: null,
    })
  }

  render(): React.ReactNode {
    if (this.state.hasError) {
      return <ErrorFallback
        error={this.state.error}
        errorInfo={this.state.errorInfo}
        onReload={this.handleReload}
        onReset={this.handleReset}
      />
    }

    return this.props.children
  }
}

interface ErrorFallbackProps {
  error: Error | null
  errorInfo: React.ErrorInfo | null
  onReload: () => void
  onReset: () => void
}

const ErrorFallback: React.FC<ErrorFallbackProps> = ({
  error,
  errorInfo,
  onReload,
  onReset,
}) => {
  const styles = useStyles()

  return (
    <div className={styles.container}>
      <Card className={styles.errorCard}>
        <div className={styles.iconContainer}>
          <ErrorCircle24Regular className={styles.icon} />
        </div>

        <Title1 className={styles.title}>Something went wrong</Title1>

        <Body1 className={styles.message}>
          The application encountered an unexpected error. This has been logged and you can try to recover using the options below.
        </Body1>

        {error && (
          <div className={styles.details}>
            <Title3 style={{ marginBottom: tokens.spacingVerticalS, color: '#EF4444' }}>
              Error Details:
            </Title3>
            {error.message}
            {error.stack && (
              <>
                {'\n\n'}
                {error.stack.split('\n').slice(0, 5).join('\n')}
              </>
            )}
          </div>
        )}

        <div className={styles.buttonGroup}>
          <Button
            appearance="primary"
            icon={<ArrowClockwise24Regular />}
            onClick={onReset}
            size="large"
            style={{
              background: 'linear-gradient(135deg, #5B6EFF 0%, #6B7FFF 100%)',
              border: 'none',
            }}
          >
            Try Again
          </Button>

          <Button
            appearance="secondary"
            onClick={onReload}
            size="large"
            style={{
              background: 'rgba(30, 41, 59, 0.8)',
              border: '1px solid rgba(255, 255, 255, 0.1)',
              color: tokens.colorNeutralForeground1,
            }}
          >
            Reload Application
          </Button>
        </div>

        {import.meta.env.DEV && errorInfo && (
          <details style={{
            marginTop: tokens.spacingVerticalXL,
            textAlign: 'left',
            fontSize: tokens.fontSizeBase200,
            color: tokens.colorNeutralForeground3,
          }}>
            <summary style={{ cursor: 'pointer', marginBottom: tokens.spacingVerticalS }}>
              Component Stack (Development Only)
            </summary>
            <pre style={{
              padding: tokens.spacingVerticalM,
              background: 'rgba(0, 0, 0, 0.3)',
              borderRadius: tokens.borderRadiusSmall,
              overflowX: 'auto',
              fontSize: '11px',
            }}>
              {errorInfo.componentStack}
            </pre>
          </details>
        )}
      </Card>
    </div>
  )
}