import React from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useAppContext } from '../contexts/AppContext'
import { StatusCard, PageHeader, EmptyState } from '../components'
import * as logger from '../utils/logger'
import {
  Card,
  tokens,
  makeStyles,
  shorthands,
} from '@fluentui/react-components'
import {
  Shield20Regular,
  Stethoscope20Regular,
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  container: {
    maxWidth: '1200px',
    margin: '0 auto',
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalL),
  },
  issuesList: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalM),
  },
})

export const IssuesTab: React.FC = () => {
  const styles = useStyles()
  const {
    results,
    issues,
    fixingIssue,
    setFixingIssue,
    setIssues,
    setSelectedTab,
  } = useAppContext()

  const handleFixIssue = async (issueId: string) => {
    setFixingIssue(issueId)
    try {
      const result = await invoke<any>('fix_issue', { issueId })
      if (result.success) {
        setTimeout(async () => {
          try {
            const updatedIssues = await invoke<any[]>('detect_issues')
            setIssues(updatedIssues)
          } catch (error) {
            logger.error('IssuesTab', 'Failed to detect issues', error)
          }
        }, 2000)
      }
    } catch (error) {
      logger.error('IssuesTab', 'Failed to fix issue', error)
    } finally {
      setFixingIssue(null)
    }
  }

  const sortedIssues = [...issues].sort((a, b) => {
    if (a.detected !== b.detected) {
      return a.detected ? -1 : 1;
    }
    const severityOrder: Record<string, number> = { Critical: 0, Warning: 1, Info: 2, Ok: 3 };
    return (severityOrder[a.severity] || 999) - (severityOrder[b.severity] || 999);
  })

  const detectedIssuesCount = issues.filter(i => i.detected).length
  const criticalCount = issues.filter(i => i.severity === 'Critical').length

  return (
    <div className={styles.container}>
      {/* Page Header */}
      <PageHeader
        title="System Issues"
        description={detectedIssuesCount > 0
          ? `${detectedIssuesCount} issue${detectedIssuesCount !== 1 ? 's' : ''} detected${criticalCount > 0 ? ` (${criticalCount} critical)` : ''}`
          : 'Review and fix detected system issues'
        }
        icon={<Shield20Regular />}
      />

      {/* Empty States */}
      {sortedIssues.length === 0 ? (
        <Card>
          {Object.keys(results).length === 0 ? (
            <EmptyState
              title="No Scan Data Available"
              description="Run a diagnostic scan first to check for system issues."
              variant="info"
              icon={<Stethoscope20Regular style={{ width: 40, height: 40 }} />}
              primaryAction={{
                label: 'Run Diagnostics',
                onClick: () => setSelectedTab('diagnostics'),
              }}
            />
          ) : (
            <EmptyState
              title="No Issues Found"
              description="Your system appears to be healthy with no issues detected."
              variant="success"
            />
          )}
        </Card>
      ) : (
        <div className={styles.issuesList}>
          {sortedIssues.map((issue, index) => (
            <StatusCard
              key={index}
              status={issue.severity === 'Ok' ? 'success' :
                      issue.severity === 'Critical' ? 'error' :
                      issue.severity === 'Warning' ? 'warning' : 'info'}
              title={issue.title}
              description={issue.description}
              badge={issue.category}
              details={issue.recommendation ? [issue.recommendation] : undefined}
              actions={issue.detected && issue.id ? [{
                label: fixingIssue === issue.id ? 'Fixing...' : 'Fix Issue',
                onClick: () => handleFixIssue(issue.id!),
                disabled: fixingIssue !== null,
                primary: true
              }] : undefined}
            />
          ))}
        </div>
      )}
    </div>
  )
}