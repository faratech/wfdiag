import React from 'react'
import {
  makeStyles,
  tokens,
  Title3,
  Text
} from '@fluentui/react-components'

const useStyles = makeStyles({
  container: {
    marginBottom: tokens.spacingVerticalM,
    marginTop: tokens.spacingVerticalXL,
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'baseline',
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
    paddingBottom: tokens.spacingVerticalS,
  },
  title: {
    color: tokens.colorNeutralForeground1,
    fontWeight: 600,
  },
  meta: {
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase200,
  }
})

interface SectionHeaderProps {
  title: string
  count: number
  successCount: number
}

export const SectionHeader: React.FC<SectionHeaderProps> = ({
  title,
  count,
  successCount
}) => {
  const styles = useStyles()

  return (
    <div className={styles.container}>
      <Title3 className={styles.title}>{title}</Title3>
      <Text className={styles.meta}>
        {successCount}/{count} Passed
      </Text>
    </div>
  )
}