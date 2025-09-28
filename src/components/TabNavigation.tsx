import React from 'react'
import {
  Tab,
  TabList,
  SelectTabData,
  Badge,
  makeStyles,
  tokens,
  shorthands
} from '@fluentui/react-components'
import {
  Stethoscope20Regular,
  ChartMultiple20Regular,
  Brain20Regular,
  Warning20Regular,
  History20Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  container: {
    backgroundColor: 'rgba(15, 23, 42, 0.8)',
    borderBottom: `1px solid ${tokens.colorNeutralStroke1}`,
    backdropFilter: 'blur(10px)',
    position: 'sticky',
    top: '72px', // Height of NavigationHeader
    zIndex: 999,
  },

  tabList: {
    maxWidth: '1400px',
    ...shorthands.margin('0', 'auto'),
    ...shorthands.padding('0', tokens.spacingHorizontalXXL),
  },

  tab: {
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalL),
    fontSize: tokens.fontSizeBase300,
    fontWeight: tokens.fontWeightRegular,
    color: tokens.colorNeutralForeground3,
    position: 'relative',

    ':hover': {
      color: tokens.colorNeutralForeground1,
      backgroundColor: 'rgba(59, 130, 246, 0.1)',
    },

    '&[data-selected="true"]': {
      color: tokens.colorBrandForeground1,
      fontWeight: tokens.fontWeightSemibold,

      '&::after': {
        content: '""',
        position: 'absolute',
        bottom: 0,
        left: tokens.spacingHorizontalL,
        right: tokens.spacingHorizontalL,
        height: '3px',
        backgroundColor: tokens.colorBrandBackground,
        ...shorthands.borderRadius(tokens.borderRadiusSmall),
      }
    }
  },

  tabContent: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalS),
  },

  badge: {
    marginLeft: tokens.spacingHorizontalXS,
  }
})

export type TabValue = 'diagnostics' | 'monitoring' | 'ai' | 'issues' | 'history'

export interface TabNavigationProps {
  selectedTab: TabValue
  onTabSelect: (tab: TabValue) => void
  issueCount?: number
  isMonitoringActive?: boolean
  hasAIKey?: boolean
}

export const TabNavigation: React.FC<TabNavigationProps> = ({
  selectedTab,
  onTabSelect,
  issueCount = 0,
  isMonitoringActive = false,
  hasAIKey = false
}) => {
  const styles = useStyles()

  const handleTabSelect = (_: any, data: SelectTabData) => {
    onTabSelect(data.value as TabValue)
  }

  const tabs = [
    {
      value: 'diagnostics',
      icon: <Stethoscope20Regular />,
      label: 'Diagnostics',
      badge: null
    },
    {
      value: 'monitoring',
      icon: <ChartMultiple20Regular />,
      label: 'System Monitor',
      badge: isMonitoringActive ? (
        <Badge
          appearance="filled"
          color="success"
          size="extra-small"
          shape="circular"
        />
      ) : null
    },
    {
      value: 'ai',
      icon: <Brain20Regular />,
      label: 'AI Analysis',
      badge: hasAIKey ? (
        <Badge
          appearance="tint"
          color="informative"
          size="extra-small"
        >
          Ready
        </Badge>
      ) : null
    },
    {
      value: 'issues',
      icon: <Warning20Regular />,
      label: 'Issues',
      badge: issueCount > 0 ? (
        <Badge
          appearance="filled"
          color={issueCount > 5 ? 'danger' : 'warning'}
          size="small"
        >
          {issueCount}
        </Badge>
      ) : null
    },
    {
      value: 'history',
      icon: <History20Regular />,
      label: 'History',
      badge: null
    }
  ]

  return (
    <nav className={styles.container} role="navigation" aria-label="Main navigation">
      <TabList
        className={styles.tabList}
        selectedValue={selectedTab}
        onTabSelect={handleTabSelect}
        size="large"
      >
        {tabs.map(tab => (
          <Tab
            key={tab.value}
            value={tab.value}
            className={styles.tab}
            data-selected={selectedTab === tab.value}
          >
            <div className={styles.tabContent}>
              {tab.icon}
              <span>{tab.label}</span>
              {tab.badge && <span className={styles.badge}>{tab.badge}</span>}
            </div>
          </Tab>
        ))}
      </TabList>
    </nav>
  )
}