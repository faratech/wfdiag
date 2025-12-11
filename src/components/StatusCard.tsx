import React from 'react'
import {
  Card,
  Text,
  Badge,
  Button,
  Divider,
  tokens,
  makeStyles,
  shorthands,
  InfoLabel,
  Caption1,
  Body1,
  Title3
} from '@fluentui/react-components'
import {
  CheckmarkCircle20Regular,
  ErrorCircle20Regular,
  Warning20Regular,
  Info20Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  card: {
    ...shorthands.padding(tokens.spacingVerticalL),
    background: 'rgba(30, 41, 59, 0.6)',
    backdropFilter: 'blur(10px)',
    ...shorthands.border('1px', 'solid', 'rgba(255, 255, 255, 0.1)'),
  },
  header: {
    display: 'flex',
    alignItems: 'flex-start',
    justifyContent: 'space-between',
    marginBottom: tokens.spacingVerticalXL,
  },
  statusIcon: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
  },
  content: {
    marginBottom: tokens.spacingVerticalL,
    ...shorthands.gap(tokens.spacingVerticalS),
    display: 'flex',
    flexDirection: 'column',
  },
  actions: {
    display: 'flex',
    ...shorthands.gap(tokens.spacingHorizontalS),
    flexWrap: 'wrap',
  },
  infoSection: {
    marginTop: tokens.spacingVerticalL,
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalL),
    backgroundColor: 'rgba(59, 130, 246, 0.1)',
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    ...shorthands.border('1px', 'solid', 'rgba(59, 130, 246, 0.3)'),
  }
})

export interface StatusCardProps {
  title: string
  description?: string
  status: 'success' | 'warning' | 'error' | 'info'
  details?: string[]
  actions?: Array<{
    label: string
    onClick: () => void
    primary?: boolean
    disabled?: boolean
  }>
  badge?: string
  infoMessage?: string
}

export const StatusCard: React.FC<StatusCardProps> = ({
  title,
  description,
  status,
  details,
  actions,
  badge,
  infoMessage
}) => {
  const styles = useStyles()

  const getStatusIcon = () => {
    switch (status) {
      case 'success':
        return <CheckmarkCircle20Regular primaryFill="#10B981" />
      case 'warning':
        return <Warning20Regular primaryFill="#F59E0B" />
      case 'error':
        return <ErrorCircle20Regular primaryFill="#EF4444" />
      case 'info':
      default:
        return <Info20Regular primaryFill="#3B82F6" />
    }
  }

  const getStatusColor = (): any => {
    switch (status) {
      case 'success': return 'success'
      case 'warning': return 'warning'
      case 'error': return 'danger'
      case 'info':
      default: return 'informative'
    }
  }

  return (
    <Card className={styles.card}>
      <div className={styles.header}>
        <div className={styles.statusIcon}>
          {getStatusIcon()}
          <Title3>{title}</Title3>
        </div>
        {badge && (
          <Badge
            appearance="filled"
            color={getStatusColor()}
            size="medium"
          >
            {badge}
          </Badge>
        )}
      </div>

      {description && (
        <Body1 className={styles.content}>
          {description}
        </Body1>
      )}

      {details && details.length > 0 && (
        <>
          <Divider />
          <div style={{ marginTop: tokens.spacingVerticalM }}>
            {details.map((detail, index) => (
              <Caption1
                key={index}
                style={{
                  display: 'block',
                  marginBottom: tokens.spacingVerticalXS,
                  color: tokens.colorNeutralForeground3
                }}
              >
                • {detail}
              </Caption1>
            ))}
          </div>
        </>
      )}

      {infoMessage && (
        <div className={styles.infoSection}>
          <InfoLabel
            info={
              <Text size={200}>
                {infoMessage}
              </Text>
            }
          >
            <Caption1 style={{ color: '#60A5FA' }}>
              <Info20Regular style={{ marginRight: tokens.spacingHorizontalXS }} />
              Additional Information
            </Caption1>
          </InfoLabel>
        </div>
      )}

      {actions && actions.length > 0 && (
        <>
          <Divider style={{ marginTop: tokens.spacingVerticalM, marginBottom: tokens.spacingVerticalM }} />
          <div className={styles.actions}>
            {actions.map((action, index) => (
              <Button
                key={index}
                appearance={action.primary ? 'primary' : 'secondary'}
                onClick={action.onClick}
                disabled={action.disabled}
                size="small"
              >
                {action.label}
              </Button>
            ))}
          </div>
        </>
      )}
    </Card>
  )
}