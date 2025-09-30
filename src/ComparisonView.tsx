import React, { useState } from 'react'
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
} from '@fluentui/react-components'
import {
  ArrowLeftRegular,
  ArrowSyncRegular,
  CheckmarkCircleRegular,
  ErrorCircleRegular,
  WarningRegular,
} from '@fluentui/react-icons'
import { useScanHistory } from './hooks/useScanHistory'
import { useComparison, ComparisonFilter } from './hooks/useComparison'
import { useJsonDiff } from './hooks/useJsonDiff'
import { useToast } from './contexts/ToastContext'
import './styles.css'

interface ComparisonViewProps {
  onClose: () => void
}

export const ComparisonView: React.FC<ComparisonViewProps> = ({ onClose }) => {
  const { scans, loading: scansLoading, error: scansError, refreshScans } = useScanHistory()
  const { comparison, loading: compareLoading, error: compareError, compareScans, clearComparison, getFilteredChanges } = useComparison()
  const { findJsonDifferences, formatDifference } = useJsonDiff()
  const { showError, showSuccess } = useToast()

  const [selectedCurrent, setSelectedCurrent] = useState<string>('')
  const [selectedPrevious, setSelectedPrevious] = useState<string>('')
  const [filter, setFilter] = useState<ComparisonFilter>('all')
  const [showClearDialog, setShowClearDialog] = useState(false)

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
      console.error('Failed to clear scan history:', error)
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

  // Loading state
  if (scansLoading) {
    return (
      <div style={{ 
        maxWidth: 1000, 
        margin: '0 auto', 
        padding: 24,
        textAlign: 'center' 
      }}>
        <div className="glass-card" style={{ padding: 48 }}>
          <i className="fas fa-spinner fa-spin" style={{ fontSize: 32, color: '#3b82f6', marginBottom: 16 }}></i>
          <Text size={400} style={{ color: '#f1f5f9' }}>Loading scan history...</Text>
        </div>
      </div>
    )
  }

  // Error state
  if (scansError) {
    return (
      <div style={{ maxWidth: 1000, margin: '0 auto', padding: 24 }}>
        <div className="glass-card" style={{ padding: 24 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16 }}>
            <Button
              appearance="subtle"
              onClick={onClose}
              icon={<ArrowLeftRegular />}
            >
              Back
            </Button>
            <Text size={500} weight="semibold" style={{ color: '#f1f5f9' }}>
              Scan Comparison
            </Text>
          </div>

          <div style={{
            background: 'rgba(239, 68, 68, 0.1)',
            border: '1px solid rgba(239, 68, 68, 0.3)',
            borderRadius: 8,
            padding: 16,
            color: '#ef4444'
          }}>
            <ErrorCircleRegular style={{ marginRight: 8 }} />
            {scansError}
          </div>

          <div style={{ marginTop: 16, textAlign: 'center' }}>
            <Button onClick={refreshScans}>
              Retry
            </Button>
          </div>
        </div>
      </div>
    )
  }

  // Empty state
  if (scans.length === 0) {
    return (
      <div style={{ maxWidth: 1000, margin: '0 auto', padding: 24 }}>
        <div className="glass-card" style={{ padding: 24 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 24 }}>
            <Button
              appearance="subtle"
              onClick={onClose}
              icon={<ArrowLeftRegular />}
            >
              Back
            </Button>
            <Text size={500} weight="semibold" style={{ color: '#f1f5f9' }}>
              Scan Comparison
            </Text>
          </div>

          <div style={{ textAlign: 'center', padding: 48 }}>
            <ArrowSyncRegular style={{ fontSize: 48, color: '#94a3b8', marginBottom: 16 }} />
            <Text size={600} weight="bold" style={{ color: '#f1f5f9', display: 'block', marginBottom: 12 }}>
              No Scans Available
            </Text>
            <Text size={300} style={{ color: '#94a3b8', marginBottom: 24 }}>
              Run some diagnostic scans first to compare results
            </Text>
            <Button onClick={onClose}>
              Back to Diagnostics
            </Button>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div style={{ maxWidth: 1000, margin: '0 auto', padding: 24 }}>
      {/* Header */}
      <div className="glass-card" style={{ padding: 24, marginBottom: 24 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 20 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Button
              appearance="subtle"
              onClick={onClose}
              icon={<ArrowLeftRegular />}
            >
              Back
            </Button>
            <Text size={500} weight="semibold" style={{ color: '#f1f5f9' }}>
              Scan Comparison
            </Text>
          </div>
          {!comparison && scans.length > 0 && (
            <Dialog open={showClearDialog} onOpenChange={(_, data) => setShowClearDialog(data.open)}>
              <DialogTrigger disableButtonEnhancement>
                <Button
                  appearance="subtle"
                  style={{
                    color: '#ef4444',
                    borderColor: 'rgba(239, 68, 68, 0.3)'
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
          )}
        </div>

        {!comparison && (
          <>
            <Text size={300} style={{ color: '#94a3b8', marginBottom: 24, display: 'block' }}>
              Select two scans to compare their diagnostic results and identify changes
            </Text>

            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16, marginBottom: 24 }}>
              <div>
                <Text size={300} weight="semibold" style={{ color: '#f1f5f9', display: 'block', marginBottom: 8 }}>
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
                        <Text size={200} style={{ color: '#94a3b8' }}>
                          {scan.task_count} tasks • {formatDuration(scan.duration_ms)} • {scan.success_count} passed
                        </Text>
                      </div>
                    </Option>
                  ))}
                </Dropdown>
              </div>

              <div>
                <Text size={300} weight="semibold" style={{ color: '#f1f5f9', display: 'block', marginBottom: 8 }}>
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
                          <Text size={200} style={{ color: '#94a3b8' }}>
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
          </>
        )}
      </div>

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