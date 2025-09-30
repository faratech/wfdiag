import React from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useAppContext } from '../contexts/AppContext'
import { StatusCard } from '../components'
import * as logger from '../utils/logger'
import {
  Card,
  Title3,
  Text,
  tokens,
} from '@fluentui/react-components'
import {
  CheckmarkCircle20Regular,
  Info20Regular,
  Shield20Regular,
} from '@fluentui/react-icons'

export const IssuesTab: React.FC = () => {
  const { results, issues, fixingIssue, setFixingIssue, setIssues } = useAppContext()

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

  return (
    <div style={{ maxWidth: '1200px', margin: '0 auto' }}>
      <Card>
        <Title3>
          <Shield20Regular style={{ marginRight: tokens.spacingHorizontalS }} />
          System Health Check
        </Title3>

        {sortedIssues.length === 0 ? (
          <div style={{ textAlign: 'center', padding: tokens.spacingVerticalXXL }}>
            {Object.keys(results).length === 0 ? (
              <>
                <Info20Regular style={{ fontSize: '48px', color: tokens.colorNeutralForeground3, marginBottom: tokens.spacingVerticalM }} />
                <Title3 style={{ marginBottom: tokens.spacingVerticalM }}>No Scan Data Available</Title3>
                <Text style={{ display: 'block' }}>Please run a diagnostic scan first to check for system issues.</Text>
              </>
            ) : (
              <>
                <CheckmarkCircle20Regular style={{ fontSize: '48px', color: '#10B981', marginBottom: tokens.spacingVerticalM }} />
                <Title3 style={{ marginBottom: tokens.spacingVerticalM }}>No Issues Found</Title3>
                <Text style={{ display: 'block' }}>Your system appears to be healthy with no issues detected.</Text>
              </>
            )}
          </div>
        ) : (
          <div style={{ marginTop: tokens.spacingVerticalL }}>
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
      </Card>
    </div>
  )
}