import React from 'react'
import {
  makeStyles,
  tokens,
  Title3,
  Text,
  Caption1
} from '@fluentui/react-components'

const useStyles = makeStyles({
  container: {
    marginBottom: tokens.spacingVerticalM,
    marginTop: tokens.spacingVerticalXL,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
    paddingBottom: tokens.spacingVerticalS,
  },
  headerRow: {
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'baseline',
  },
  titleContainer: {
    display: 'flex',
    flexDirection: 'column',
    gap: tokens.spacingVerticalXS,
  },
  title: {
    color: tokens.colorNeutralForeground1,
    fontWeight: 600,
  },
  description: {
    color: tokens.colorNeutralForeground3,
  },
  meta: {
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase200,
  }
})

// Section descriptions for user clarity
const SECTION_DESCRIPTIONS: Record<string, string> = {
  Hardware: 'CPU, memory, graphics, and physical device status',
  System: 'Windows configuration, drivers, services, and logs',
  Storage: 'Disk health, partitions, and available space',
  Network: 'Adapters, connections, and network configuration',
}

interface SectionHeaderProps {
  title: string
  count: number
  successCount: number
  /** Optional custom description override */
  description?: string
}

export const SectionHeader: React.FC<SectionHeaderProps> = ({
  title,
  count,
  successCount,
  description
}) => {
  const styles = useStyles()
  const sectionDescription = description || SECTION_DESCRIPTIONS[title]

  return (
    <div className={styles.container}>
      <div className={styles.headerRow}>
        <div className={styles.titleContainer}>
          <Title3 className={styles.title}>{title}</Title3>
          {sectionDescription && (
            <Caption1 className={styles.description}>{sectionDescription}</Caption1>
          )}
        </div>
        <Text className={styles.meta}>
          {successCount}/{count} Passed
        </Text>
      </div>
    </div>
  )
}