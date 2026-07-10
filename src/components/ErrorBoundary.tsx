import React from 'react'
import * as logger from '../utils/logger'

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
    this.state = { hasError: false, error: null, errorInfo: null }
  }

  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    return { hasError: true, error }
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo): void {
    logger.error('ErrorBoundary', 'Component error caught', {
      error: error.toString(),
      errorInfo: errorInfo.componentStack,
      stack: error.stack,
    })
    this.setState({ error, errorInfo })
  }

  handleReload = (): void => { window.location.reload() }
  handleReset = (): void => { this.setState({ hasError: false, error: null, errorInfo: null }) }

  render(): React.ReactNode {
    if (!this.state.hasError) return this.props.children
    const { error, errorInfo } = this.state
    return (
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', minHeight: '100vh', padding: 32 }}>
        <div className="wf-block" style={{ maxWidth: 600, width: '100%', padding: 32, textAlign: 'center' }}>
          <i className="fa-solid fa-circle-exclamation" style={{ fontSize: 56, color: 'var(--err-fg)' }} />
          <h1 style={{ margin: '16px 0 8px' }}>Something went wrong</h1>
          <p style={{ color: 'var(--wf-text-muted)', lineHeight: 1.6 }}>
            The application encountered an unexpected error. It has been logged. You can try to recover below.
          </p>
          {error && (
            <pre className="code-block" style={{ textAlign: 'left', marginTop: 16, color: 'var(--err-fg)', maxHeight: 200 }}>
              {error.message}
              {error.stack ? '\n\n' + error.stack.split('\n').slice(0, 5).join('\n') : ''}
            </pre>
          )}
          <div style={{ display: 'flex', gap: 12, justifyContent: 'center', flexWrap: 'wrap', marginTop: 20 }}>
            <button className="btn primary" onClick={this.handleReset}><i className="fa-solid fa-arrows-rotate" /> Try Again</button>
            <button className="btn" onClick={this.handleReload}>Reload Application</button>
          </div>
          {import.meta.env.DEV && errorInfo && (
            <details style={{ marginTop: 20, textAlign: 'left', fontSize: 12, color: 'var(--wf-text-muted)' }}>
              <summary style={{ cursor: 'pointer' }}>Component Stack (dev only)</summary>
              <pre className="code-block" style={{ marginTop: 8 }}>{errorInfo.componentStack}</pre>
            </details>
          )}
        </div>
      </div>
    )
  }
}
