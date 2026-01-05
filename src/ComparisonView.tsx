import React, { useState, useCallback } from 'react'
import {
  Text,
  Badge,
  Button,
  Dropdown,
  Option,
  Divider,
  Dialog,
  DialogTrigger,
  DialogSurface,
  DialogTitle,
  DialogBody,
  DialogActions,
  DialogContent,
  makeStyles,
  shorthands,
  tokens,
} from '@fluentui/react-components'
import {
  ArrowLeftRegular,
  ArrowSyncRegular,
  CheckmarkCircleRegular,
  ErrorCircleRegular,
  WarningRegular,
  History20Regular,
  ChevronRight12Regular,
} from '@fluentui/react-icons'
import { PageHeader, EmptyState, AIAnalysisPanel } from './components'
import { useScanHistory } from './hooks/useScanHistory'
import { useAI } from './hooks/useAI'
import { useComparison, ComparisonFilter } from './hooks/useComparison'
import { useJsonDiff } from './hooks/useJsonDiff'
import { useToast } from './contexts/ToastContext'
import * as logger from './utils/logger'
import './styles.css'

const useStyles = makeStyles({
  container: {
    maxWidth: '1000px',
    margin: '0 auto',
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalL),
  },
  summaryGrid: {
    display: 'grid',
    gridTemplateColumns: 'repeat(4, 1fr)',
    ...shorthands.gap(tokens.spacingHorizontalM),
  },
  summaryCard: {
    ...shorthands.padding(tokens.spacingVerticalM),
    textAlign: 'center',
  },
  filterButtons: {
    display: 'flex',
    ...shorthands.gap(tokens.spacingHorizontalM),
  },
  changesList: {
    maxHeight: '600px',
    overflowY: 'auto',
  },
  breadcrumbs: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
    marginBottom: tokens.spacingVerticalS,
  },
  breadcrumbItem: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
    cursor: 'pointer',
    transitionProperty: 'color',
    transitionDuration: tokens.durationNormal,
    ':hover': {
      color: tokens.colorBrandForeground1,
    },
  },
  breadcrumbCurrent: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground1,
    fontWeight: tokens.fontWeightSemibold,
  },
  breadcrumbSeparator: {
    color: tokens.colorNeutralForeground4,
  },
})

interface ComparisonViewProps {
  onClose: () => void
}

export const ComparisonView: React.FC<ComparisonViewProps> = ({ onClose }) => {
  const styles = useStyles()
  const { scans, loading: scansLoading, error: scansError, refreshScans } = useScanHistory()
  const { comparison, loading: compareLoading, error: compareError, compareScans, clearComparison, getFilteredChanges } = useComparison()
  const { findJsonDifferences, formatDifference } = useJsonDiff()
  const { showError, showSuccess } = useToast()

  // AI analysis hooks
  const {
    requestComparisonAnalysis,
    getCachedComparisonAnalysis,
    isComparisonAnalysisLoading,
    getComparisonAnalysisError,
  } = useAI()

  const [selectedCurrent, setSelectedCurrent] = useState<string>('')
  const [selectedPrevious, setSelectedPrevious] = useState<string>('')
  const [filter, setFilter] = useState<ComparisonFilter>('all')
  const [showClearDialog, setShowClearDialog] = useState(false)

  // AI comparison analysis handler
  const handleComparisonAnalysis = useCallback(() => {
    if (!comparison) return
    const comparisonJson = JSON.stringify({
      total_changes: comparison.total_changes,
      new_failures: comparison.new_failures.map(f => ({
        task_name: f.task_name,
        category: f.category,
      })),
      new_successes: comparison.new_successes.map(s => ({
        task_name: s.task_name,
        category: s.category,
      })),
      output_changes: comparison.status_unchanged
        .filter(c => c.output_changed)
        .map(c => ({
          task_name: c.task_name,
          category: c.category,
        })),
    }, null, 2)
    requestComparisonAnalysis(comparisonJson)
  }, [comparison, requestComparisonAnalysis])

  const handleCompare = async () => {
    if (!selectedCurrent || !selectedPrevious) {
      return
    }
    await compareScans(selectedCurrent, selectedPrevious)
  }

  const handleStartOver = () => {
    clearComparison()
    setSelectedCurrent('')
    setSelectedPrevious('')
    setFilter('all')
  }

  const handleClearHistory = async () => {
    try {
      const { invoke } = await import('@tauri-apps/api/core')
      await invoke('clear_scan_history')
      await refreshScans()
      setSelectedCurrent('')
      setSelectedPrevious('')
      setShowClearDialog(false)
      showSuccess('History Cleared', 'All scan history has been successfully deleted.')
    } catch (error) {
      logger.error('ComparisonView', 'Failed to clear scan history', error)
      showError('Failed to Clear History', String(error))
    }
  }

  const formatTimestamp = (timestamp: string) => {
    return new Date(timestamp).toLocaleString()
  }

  const formatDuration = (durationMs: number) => {
    const seconds = Math.round(durationMs / 1000)
    if (seconds < 60) return `${seconds}s`
    const minutes = Math.floor(seconds / 60)
    const remainingSeconds = seconds % 60
    return `${minutes}m ${remainingSeconds}s`
  }

  const getChangeIcon = (change: any) => {
    if (!change.current_success && change.previous_success) {
      return <ErrorCircleRegular style={{ color: '#ef4444' }} />
    } else if (change.current_success && !change.previous_success) {
      return <CheckmarkCircleRegular style={{ color: '#10b981' }} />
    } else if (change.output_changed) {
      return <WarningRegular style={{ color: '#f59e0b' }} />
    }
    return null
  }

  const getChangeDescription = (change: any) => {
    if (!change.current_success && change.previous_success) {
      return 'Now failing'
    } else if (change.current_success && !change.previous_success) {
      return 'Now passing'
    } else if (change.output_changed) {
      return 'Output changed'
    }
    return 'No change'
  }

  const backAction = (
    <Button
      appearance="subtle"
      onClick={onClose}
      icon={<ArrowLeftRegular />}
    >
      Back
    </Button>
  )

  // Loading state
  if (scansLoading) {
    return (
      <div className={styles.container}>
        <PageHeader
          title="Scan Comparison"
          description="Loading scan history..."
          icon={<History20Regular />}
          actions={backAction}
        />
        <div className="glass-card" style={{ padding: 48, textAlign: 'center' }}>
          <i className="fas fa-spinner fa-spin" style={{ fontSize: 32, color: '#3b82f6', marginBottom: 16 }}></i>
          <Text size={400} style={{ color: 'var(--theme-foreground)' }}>Loading scan history...</Text>
        </div>
      </div>
    )
  }

  // Error state
  if (scansError) {
    return (
      <div className={styles.container}>
        <PageHeader
          title="Scan Comparison"
          description="Compare diagnostic results between scans"
          icon={<History20Regular />}
          actions={backAction}
        />
        <div className="glass-card">
          <EmptyState
            title="Error Loading Scans"
            description={scansError}
            variant="error"
            primaryAction={{
              label: 'Retry',
              onClick: refreshScans,
            }}
          />
        </div>
      </div>
    )
  }

  // Empty state
  if (scans.length === 0) {
    return (
      <div className={styles.container}>
        <PageHeader
          title="Scan Comparison"
          description="Compare diagnostic results between scans"
          icon={<History20Regular />}
          actions={backAction}
        />
        <div className="glass-card">
          <EmptyState
            title="No Scans Available"
            description="Run some diagnostic scans first to compare results"
            icon={<ArrowSyncRegular style={{ width: 40, height: 40 }} />}
            primaryAction={{
              label: 'Back to Diagnostics',
              onClick: onClose,
            }}
          />
        </div>
      </div>
    )
  }

  const clearHistoryDialog = !comparison && scans.length > 0 ? (
    <Dialog open={showClearDialog} onOpenChange={(_, data) => setShowClearDialog(data.open)}>
      <DialogTrigger disableButtonEnhancement>
        <Button
          appearance="subtle"
          style={{
            color: tokens.colorPaletteRedForeground1,
          }}
        >
          Clear History
        </Button>
      </DialogTrigger>
      <DialogSurface>
        <DialogBody>
          <DialogTitle>Clear Scan History?</DialogTitle>
          <DialogContent>
            <Text>
              Are you sure you want to clear all scan history? This action cannot be undone.
            </Text>
          </DialogContent>
          <DialogActions>
            <DialogTrigger disableButtonEnhancement>
              <Button appearance="secondary">Cancel</Button>
            </DialogTrigger>
            <Button appearance="primary" onClick={handleClearHistory}>
              Clear History
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  ) : null

  return (
    <div className={styles.container}>
      {/* Breadcrumb Navigation */}
      <nav className={styles.breadcrumbs} aria-label="Breadcrumb">
        <span className={styles.breadcrumbItem} onClick={onClose}>
          Diagnostics
        </span>
        <ChevronRight12Regular className={styles.breadcrumbSeparator} />
        <span className={styles.breadcrumbCurrent}>
          {comparison ? 'Scan Comparison' : 'Scan History'}
        </span>
      </nav>

      {/* Page Header */}
      <PageHeader
        title="Scan Comparison"
        description={comparison
          ? 'Review changes between selected scans'
          : 'Compare diagnostic results between scans'}
        icon={<History20Regular />}
        actions={
          <div style={{ display: 'flex', gap: tokens.spacingHorizontalM }}>
            {backAction}
            {clearHistoryDialog}
          </div>
        }
      />

      {/* Scan Selection */}
      {!comparison && (
        <div className="glass-card" style={{ padding: 24 }}>
          <Text size={300} style={{ color: tokens.colorNeutralForeground3, marginBottom: 24, display: 'block' }}>
            Select two scans to compare their diagnostic results and identify changes
          </Text>

          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16, marginBottom: 24 }}>
            <div>
              <Text size={300} weight="semibold" style={{ color: tokens.colorNeutralForeground1, display: 'block', marginBottom: 8 }}>
                Current Scan
              </Text>
              <Dropdown
                placeholder="Select current scan"
                value={selectedCurrent}
                selectedOptions={selectedCurrent ? [selectedCurrent] : []}
                onOptionSelect={(_, data) => {
                  if (data.optionValue) {
                    setSelectedCurrent(data.optionValue)
                  }
                }}
                style={{ width: '100%' }}
              >
                {scans.map(scan => (
                  <Option key={scan.id} value={scan.id} text={formatTimestamp(scan.timestamp)}>
                    <div>
                      <Text size={300} weight="semibold" style={{ display: 'block' }}>
                        {formatTimestamp(scan.timestamp)}
                      </Text>
                      <Text size={200} style={{ color: tokens.colorNeutralForeground3 }}>
                        {scan.task_count} tasks • {formatDuration(scan.duration_ms)} • {scan.success_count} passed
                      </Text>
                    </div>
                  </Option>
                ))}
              </Dropdown>
            </div>

            <div>
              <Text size={300} weight="semibold" style={{ color: tokens.colorNeutralForeground1, display: 'block', marginBottom: 8 }}>
                Previous Scan
              </Text>
              <Dropdown
                placeholder="Select previous scan"
                value={selectedPrevious}
                selectedOptions={selectedPrevious ? [selectedPrevious] : []}
                onOptionSelect={(_, data) => {
                  if (data.optionValue) {
                    setSelectedPrevious(data.optionValue)
                  }
                }}
                style={{ width: '100%' }}
              >
                {scans
                  .filter(scan => scan.id !== selectedCurrent)
                  .map(scan => (
                    <Option key={scan.id} value={scan.id} text={formatTimestamp(scan.timestamp)}>
                      <div>
                        <Text size={300} weight="semibold" style={{ display: 'block' }}>
                          {formatTimestamp(scan.timestamp)}
                        </Text>
                        <Text size={200} style={{ color: tokens.colorNeutralForeground3 }}>
                          {scan.task_count} tasks • {formatDuration(scan.duration_ms)} • {scan.success_count} passed
                        </Text>
                      </div>
                    </Option>
                  ))}
              </Dropdown>
            </div>
          </div>

          <div style={{ textAlign: 'center' }}>
            <Button
              appearance="primary"
              onClick={handleCompare}
              disabled={!selectedCurrent || !selectedPrevious || compareLoading}
              icon={<ArrowSyncRegular />}
            >
              {compareLoading ? 'Comparing...' : 'Compare Scans'}
            </Button>
          </div>

          {compareError && (
            <div style={{
              marginTop: 16,
              background: 'rgba(239, 68, 68, 0.1)',
              border: '1px solid rgba(239, 68, 68, 0.3)',
              borderRadius: 8,
              padding: 16,
              color: '#ef4444'
            }}>
              <ErrorCircleRegular style={{ marginRight: 8 }} />
              {compareError}
            </div>
          )}
        </div>
      )}

      {/* Comparison Results */}
      {comparison && (
        <>
          {/* Summary Cards */}
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 16, marginBottom: 24 }}>
            <div className="glass-card" style={{ padding: 16, textAlign: 'center' }}>
              <Text size={600} weight="bold" style={{ color: '#3b82f6', display: 'block' }}>
                {comparison.total_changes}
              </Text>
              <Text size={200} style={{ color: '#94a3b8' }}>Total Changes</Text>
            </div>
            <div className="glass-card" style={{ padding: 16, textAlign: 'center' }}>
              <Text size={600} weight="bold" style={{ color: '#ef4444', display: 'block' }}>
                {comparison.new_failures.length}
              </Text>
              <Text size={200} style={{ color: '#94a3b8' }}>New Failures</Text>
            </div>
            <div className="glass-card" style={{ padding: 16, textAlign: 'center' }}>
              <Text size={600} weight="bold" style={{ color: '#10b981', display: 'block' }}>
                {comparison.new_successes.length}
              </Text>
              <Text size={200} style={{ color: '#94a3b8' }}>New Successes</Text>
            </div>
            <div className="glass-card" style={{ padding: 16, textAlign: 'center' }}>
              <Text size={600} weight="bold" style={{ color: '#f59e0b', display: 'block' }}>
                {comparison.status_unchanged.filter(c => c.output_changed).length}
              </Text>
              <Text size={200} style={{ color: '#94a3b8' }}>Output Changes</Text>
            </div>
          </div>

          {/* AI Comparison Analysis */}
          <AIAnalysisPanel
            cacheKey="__comparison_analysis__"
            title="AI Comparison Analysis"
            hasData={comparison.total_changes > 0}
            cachedResult={getCachedComparisonAnalysis()}
            isLoading={isComparisonAnalysisLoading()}
            error={getComparisonAnalysisError()}
            onAnalyze={handleComparisonAnalysis}
            analyzeButtonText="Analyze Changes"
            noDataMessage="No changes detected between scans."
            readyMessage="Click to get AI-powered analysis of what has improved or degraded between scans."
          />

          {/* Filter Tabs and Results */}
          <div className="glass-card" style={{ padding: 24 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 20 }}>
              <div style={{ display: 'flex', gap: 12 }}>
                <Button
                  appearance={filter === 'all' ? 'primary' : 'secondary'}
                  onClick={() => setFilter('all')}
                  size="small"
                >
                  All Changes ({getFilteredChanges('all').length})
                </Button>
                <Button
                  appearance={filter === 'failures' ? 'primary' : 'secondary'}
                  onClick={() => setFilter('failures')}
                  size="small"
                >
                  New Failures ({comparison.new_failures.length})
                </Button>
                <Button
                  appearance={filter === 'successes' ? 'primary' : 'secondary'}
                  onClick={() => setFilter('successes')}
                  size="small"
                >
                  New Successes ({comparison.new_successes.length})
                </Button>
                <Button
                  appearance={filter === 'changes' ? 'primary' : 'secondary'}
                  onClick={() => setFilter('changes')}
                  size="small"
                >
                  Output Changes ({comparison.status_unchanged.filter(c => c.output_changed).length})
                </Button>
              </div>
              
              <Button
                appearance="subtle"
                onClick={handleStartOver}
                size="small"
              >
                Start Over
              </Button>
            </div>

            <Divider style={{ margin: '16px 0' }} />

            {/* Changes List */}
            <div style={{ maxHeight: 600, overflowY: 'auto' }}>
              {getFilteredChanges(filter).length === 0 ? (
                <div style={{ textAlign: 'center', padding: 48, color: '#94a3b8' }}>
                  <Text size={300}>No {filter === 'all' ? '' : filter} changes found</Text>
                </div>
              ) : (
                getFilteredChanges(filter).map((change, index) => (
                  <div
                    key={`${change.task_id}-${index}`}
                    style={{
                      padding: 16,
                      marginBottom: 8,
                      background: 'rgba(30, 41, 59, 0.3)',
                      borderRadius: 8,
                      border: '1px solid rgba(71, 85, 105, 0.3)'
                    }}
                  >
                    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                        {getChangeIcon(change)}
                        <Text size={400} weight="semibold" style={{ color: '#f1f5f9' }}>
                          {change.task_name}
                        </Text>
                      </div>
                      <div style={{ display: 'flex', gap: 8 }}>
                        <Badge appearance="tint" style={{ background: 'rgba(100, 116, 139, 0.2)', color: '#94a3b8' }}>
                          {change.category}
                        </Badge>
                        <Badge 
                          appearance="filled"
                          style={{ 
                            background: !change.current_success && change.previous_success ? '#ef4444' :
                                       change.current_success && !change.previous_success ? '#10b981' :
                                       change.output_changed ? '#f59e0b' : '#6b7280'
                          }}
                        >
                          {getChangeDescription(change)}
                        </Badge>
                      </div>
                    </div>
                    
                    <Text size={200} style={{ color: '#cbd5e1' }}>
                      {change.task_id}
                    </Text>

                    {change.output_changed && (
                      <details style={{ marginTop: 8 }}>
                        <summary style={{ color: '#94a3b8', cursor: 'pointer', fontSize: 12 }}>
                          View Exact Changes
                        </summary>
                        <div style={{ marginTop: 8 }}>
                          {(() => {
                            // Try to find JSON differences first
                            const differences = findJsonDifferences(change.previous_output, change.current_output)
                            
                            if (differences && differences.length > 0) {
                              // Show only the exact differences
                              return (
                                <div style={{
                                  background: 'rgba(0, 0, 0, 0.3)',
                                  border: '1px solid rgba(71, 85, 105, 0.3)',
                                  borderRadius: 4,
                                  padding: 12,
                                  fontSize: 12,
                                  color: '#cbd5e1',
                                  fontFamily: 'monospace'
                                }}>
                                  <Text size={200} weight="semibold" style={{ color: '#f59e0b', display: 'block', marginBottom: 8 }}>
                                    Changes Detected ({differences.length}):
                                  </Text>
                                  {differences.map((diff, i) => (
                                    <div key={i} style={{ 
                                      marginBottom: 6,
                                      paddingLeft: 8,
                                      borderLeft: `3px solid ${
                                        diff.type === 'added' ? '#10b981' :
                                        diff.type === 'removed' ? '#ef4444' :
                                        diff.type === 'modified' ? '#f59e0b' :
                                        '#6b7280'
                                      }`
                                    }}>
                                      <div style={{ 
                                        color: diff.type === 'added' ? '#10b981' :
                                               diff.type === 'removed' ? '#ef4444' :
                                               diff.type === 'modified' ? '#f59e0b' :
                                               '#6b7280',
                                        fontSize: 11
                                      }}>
                                        {formatDifference(diff)}
                                      </div>
                                    </div>
                                  ))}
                                </div>
                              )
                            } else {
                              // Fallback to text diff for non-JSON content
                              const lines1 = change.previous_output.split('\n')
                              const lines2 = change.current_output.split('\n')
                              const maxLines = Math.max(lines1.length, lines2.length)
                              const changes = []
                              
                              for (let i = 0; i < maxLines; i++) {
                                const line1 = lines1[i] || ''
                                const line2 = lines2[i] || ''
                                if (line1 !== line2) {
                                  if (line1 && !line2) {
                                    changes.push({ type: 'removed', line: i + 1, content: line1 })
                                  } else if (!line1 && line2) {
                                    changes.push({ type: 'added', line: i + 1, content: line2 })
                                  } else if (line1 !== line2) {
                                    changes.push({ type: 'modified', line: i + 1, from: line1, to: line2 })
                                  }
                                }
                              }
                              
                              return (
                                <div style={{
                                  background: 'rgba(0, 0, 0, 0.3)',
                                  border: '1px solid rgba(71, 85, 105, 0.3)',
                                  borderRadius: 4,
                                  padding: 12,
                                  fontSize: 12,
                                  color: '#cbd5e1',
                                  fontFamily: 'monospace'
                                }}>
                                  <Text size={200} weight="semibold" style={{ color: '#f59e0b', display: 'block', marginBottom: 8 }}>
                                    Line Changes ({changes.length}):
                                  </Text>
                                  {changes.slice(0, 10).map((change, i) => (
                                    <div key={i} style={{ 
                                      marginBottom: 6,
                                      paddingLeft: 8,
                                      borderLeft: `3px solid ${
                                        change.type === 'added' ? '#10b981' :
                                        change.type === 'removed' ? '#ef4444' :
                                        '#f59e0b'
                                      }`
                                    }}>
                                      <div style={{ 
                                        color: change.type === 'added' ? '#10b981' :
                                               change.type === 'removed' ? '#ef4444' :
                                               '#f59e0b',
                                        fontSize: 11
                                      }}>
                                        {change.type === 'removed' && `Line ${change.line}: Removed "${change.content}"`}
                                        {change.type === 'added' && `Line ${change.line}: Added "${change.content}"`}
                                        {change.type === 'modified' && `Line ${change.line}: Changed from "${change.from}" to "${change.to}"`}
                                      </div>
                                    </div>
                                  ))}
                                  {changes.length > 10 && (
                                    <div style={{ color: '#94a3b8', fontSize: 10, marginTop: 8 }}>
                                      ... and {changes.length - 10} more changes
                                    </div>
                                  )}
                                </div>
                              )
                            }
                          })()}
                        </div>
                      </details>
                    )}
                  </div>
                ))
              )}
            </div>
          </div>
        </>
      )}
    </div>
  )
}