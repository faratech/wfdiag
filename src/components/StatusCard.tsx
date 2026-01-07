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
import { Info20Regular } from '@fluentui/react-icons'
import { getStatusIcon } from '../utils/statusHelpers'
import { getStatusBadgeColor, StatusVariant } from '../types/status'

const useStyles = makeStyles({
  card: {
    ...shorthands.padding(tokens.spacingVerticalL),
    background: 'var(--theme-card-gradient)',
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke1),
    boxShadow: tokens.shadow4,
    transitionProperty: 'box-shadow, border-color',
    transitionDuration: tokens.durationNormal,
    transitionTimingFunction: tokens.curveEasyEase,
    ':hover': {
      boxShadow: tokens.shadow8,
      ...shorthands.borderColor(tokens.colorNeutralStroke1Hover),
    },
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
    backgroundColor: tokens.colorNeutralBackground3,
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke2),
  }
})

export interface StatusCardProps {
  title: string
  description?: string
  status: StatusVariant
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

  return (
    <Card className={styles.card}>
      <div className={styles.header}>
        <div className={styles.statusIcon}>
          {getStatusIcon(status)}
          <Title3>{title}</Title3>
        </div>
        {badge && (
          <Badge
            appearance="filled"
            color={getStatusBadgeColor(status)}
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
            <Caption1 style={{ color: tokens.colorBrandForeground1 }}>
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