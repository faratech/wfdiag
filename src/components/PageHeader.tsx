import React from 'react'
import {
  makeStyles,
  shorthands,
  tokens,
  Text,
  Caption1,
} from '@fluentui/react-components'
import { GlobalSearch } from './GlobalSearch'
import type { SearchResult } from '../contexts/AppContext'
import type { TabValue } from './TabNavigation'

const useStyles = makeStyles({
  header: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    ...shorthands.padding(tokens.spacingVerticalL, tokens.spacingHorizontalXL),
    backgroundColor: tokens.colorNeutralBackground2,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
    position: 'sticky',
    top: 0,
    zIndex: 100,
    '@media (max-width: 768px)': {
      flexDirection: 'column',
      alignItems: 'stretch',
      ...shorthands.gap(tokens.spacingVerticalM),
    },
  },

  titleSection: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
  },

  iconContainer: {
    width: '40px',
    height: '40px',
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    backgroundColor: tokens.colorBrandBackground,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    color: 'white',
    flexShrink: 0,
  },

  titleText: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalXXS),
  },

  title: {
    fontSize: tokens.fontSizeBase500,
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground1,
    lineHeight: tokens.lineHeightBase500,
  },

  description: {
    color: tokens.colorNeutralForeground3,
  },

  actionsSection: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
    '@media (max-width: 768px)': {
      flexDirection: 'column',
      alignItems: 'stretch',
    },
  },

  searchContainer: {
    '@media (max-width: 768px)': {
      width: '100%',
      maxWidth: 'none',
    },
  },

  actions: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalS),
    flexShrink: 0,
  },

  breadcrumbs: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalS),
    marginBottom: tokens.spacingVerticalS,
  },

  breadcrumbItem: {
    color: tokens.colorNeutralForeground3,
    textDecoration: 'none',
    cursor: 'pointer',
    ':hover': {
      color: tokens.colorBrandForeground1,
      textDecoration: 'underline',
    },
  },

  breadcrumbSeparator: {
    color: tokens.colorNeutralForeground4,
  },

  breadcrumbCurrent: {
    color: tokens.colorNeutralForeground1,
    fontWeight: tokens.fontWeightMedium,
  },
})

export interface PageHeaderProps {
  title: string
  description?: string
  icon?: React.ReactNode
  actions?: React.ReactNode
  showSearch?: boolean
  searchValue?: string
  onSearchChange?: (value: string) => void
  searchResults?: SearchResult[]
  onSearchResultSelect?: (result: SearchResult) => void
  onNavigate?: (tab: TabValue) => void
  searchPlaceholder?: string
  breadcrumbs?: Array<{ label: string; onClick?: () => void }>
}

export const PageHeader: React.FC<PageHeaderProps> = ({
  title,
  description,
  icon,
  actions,
  showSearch = false,
  searchValue = '',
  onSearchChange,
  searchResults = [],
  onSearchResultSelect,
  onNavigate,
  searchPlaceholder,
  breadcrumbs,
}) => {
  const styles = useStyles()

  return (
    <header className={styles.header}>
      <div>
        {breadcrumbs && breadcrumbs.length > 0 && (
          <nav className={styles.breadcrumbs} aria-label="Breadcrumb">
            {breadcrumbs.map((crumb, index) => (
              <React.Fragment key={index}>
                {index > 0 && <span className={styles.breadcrumbSeparator}>/</span>}
                {index === breadcrumbs.length - 1 ? (
                  <span className={styles.breadcrumbCurrent}>{crumb.label}</span>
                ) : (
                  <span
                    className={styles.breadcrumbItem}
                    onClick={crumb.onClick}
                    role="link"
                    tabIndex={0}
                  >
                    {crumb.label}
                  </span>
                )}
              </React.Fragment>
            ))}
          </nav>
        )}
        <div className={styles.titleSection}>
          {icon && <div className={styles.iconContainer}>{icon}</div>}
          <div className={styles.titleText}>
            <Text className={styles.title}>{title}</Text>
            {description && <Caption1 className={styles.description}>{description}</Caption1>}
          </div>
        </div>
      </div>

      <div className={styles.actionsSection}>
        {showSearch && onSearchChange && onSearchResultSelect && (
          <div className={styles.searchContainer}>
            <GlobalSearch
              value={searchValue}
              onChange={onSearchChange}
              results={searchResults}
              onResultSelect={onSearchResultSelect}
              onNavigate={onNavigate}
              placeholder={searchPlaceholder}
            />
          </div>
        )}
        {actions && <div className={styles.actions}>{actions}</div>}
      </div>
    </header>
  )
}
