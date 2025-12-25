import React from 'react'
import {
  makeStyles,
  shorthands,
  tokens,
  Text
} from '@fluentui/react-components'
import { Sparkle24Regular } from '@fluentui/react-icons'

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
  }
})

interface InsightPanelProps {
  title?: string
  content: string
}

export const InsightPanel: React.FC<InsightPanelProps> = ({
  title = "System Analysis",
  content
}) => {
  const styles = useStyles()

  return (
    <div className={styles.container}>
      <Sparkle24Regular className={styles.icon} />
      <div className={styles.content}>
        <Text className={styles.title}>{title}</Text>
        <Text className={styles.text}>{content}</Text>
      </div>
    </div>
  )
}