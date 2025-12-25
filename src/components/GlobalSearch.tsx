import React, { useState, useRef, useEffect, useCallback } from 'react'
import {
  makeStyles,
  shorthands,
  tokens,
  SearchBox,
  Text,
  Caption1,
  Badge,
  mergeClasses,
} from '@fluentui/react-components'
import {
  Search20Regular,
  Stethoscope20Regular,
  Warning20Regular,
  History20Regular,
  Settings20Regular,
  Dismiss20Regular,
} from '@fluentui/react-icons'
import type { SearchResult } from '../contexts/AppContext'
import type { TabValue } from './TabNavigation'

const useStyles = makeStyles({
  container: {
    position: 'relative',
    width: '100%',
    maxWidth: '400px',
  },

  searchBox: {
    width: '100%',
  },

  dropdown: {
    position: 'absolute',
    top: '100%',
    left: 0,
    right: 0,
    marginTop: tokens.spacingVerticalXS,
    backgroundColor: tokens.colorNeutralBackground1,
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke1),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    boxShadow: tokens.shadow16,
    zIndex: 1000,
    maxHeight: '400px',
    overflowY: 'auto',
  },

  dropdownHidden: {
    display: 'none',
  },

  section: {
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
  },

  sectionHeader: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalS),
    ...shorthands.padding(tokens.spacingVerticalXS, tokens.spacingHorizontalM),
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase200,
    fontWeight: tokens.fontWeightSemibold,
    textTransform: 'uppercase',
    letterSpacing: '0.05em',
    backgroundColor: tokens.colorNeutralBackground2,
  },

  resultItem: {
    display: 'flex',
    alignItems: 'flex-start',
    ...shorthands.gap(tokens.spacingHorizontalM),
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
    cursor: 'pointer',
    transitionProperty: 'background-color',
    transitionDuration: tokens.durationFaster,
    ...shorthands.borderRadius(tokens.borderRadiusSmall),

    ':hover': {
      backgroundColor: tokens.colorNeutralBackground1Hover,
    },

    ':focus': {
      backgroundColor: tokens.colorNeutralBackground1Hover,
      outlineColor: tokens.colorBrandStroke1,
      outlineStyle: 'solid',
      outlineWidth: '2px',
      outlineOffset: '-2px',
    },
  },

  resultItemSelected: {
    backgroundColor: tokens.colorNeutralBackground1Selected,
  },

  resultIcon: {
    marginTop: '2px',
    color: tokens.colorNeutralForeground3,
    flexShrink: 0,
  },

  resultContent: {
    flex: 1,
    minWidth: 0,
  },

  resultTitle: {
    display: 'block',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
  },

  resultDescription: {
    display: 'block',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    color: tokens.colorNeutralForeground3,
    marginTop: tokens.spacingVerticalXXS,
  },

  noResults: {
    ...shorthands.padding(tokens.spacingVerticalL, tokens.spacingHorizontalM),
    textAlign: 'center',
    color: tokens.colorNeutralForeground3,
  },

  recentSearches: {
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
  },

  recentItem: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    ...shorthands.padding(tokens.spacingVerticalXS, tokens.spacingHorizontalS),
    ...shorthands.borderRadius(tokens.borderRadiusSmall),
    cursor: 'pointer',

    ':hover': {
      backgroundColor: tokens.colorNeutralBackground1Hover,
    },
  },

  clearButton: {
    opacity: 0,
    transitionProperty: 'opacity',
    transitionDuration: tokens.durationFaster,

    '.recentItem:hover &': {
      opacity: 1,
    },
  },
})

export interface GlobalSearchProps {
  value: string
  onChange: (value: string) => void
  results: SearchResult[]
  onResultSelect: (result: SearchResult) => void
  onNavigate?: (tab: TabValue) => void
  placeholder?: string
  isSearching?: boolean
}

const RECENT_SEARCHES_KEY = 'wfdiag-recent-searches'
const MAX_RECENT_SEARCHES = 5

export const GlobalSearch: React.FC<GlobalSearchProps> = ({
  value,
  onChange,
  results,
  onResultSelect,
  onNavigate,
  placeholder = 'Search diagnostics, issues, history...',
  // isSearching can be used for showing a loading indicator
}) => {
  const styles = useStyles()
  const containerRef = useRef<HTMLDivElement>(null)
  const [isOpen, setIsOpen] = useState(false)
  const [selectedIndex, setSelectedIndex] = useState(-1)
  const [recentSearches, setRecentSearches] = useState<string[]>([])

  // Load recent searches from localStorage
  useEffect(() => {
    try {
      const saved = localStorage.getItem(RECENT_SEARCHES_KEY)
      if (saved) {
        setRecentSearches(JSON.parse(saved))
      }
    } catch {
      // Ignore localStorage errors
    }
  }, [])

  // Save search to recent
  const saveToRecent = useCallback((query: string) => {
    if (!query.trim()) return
    setRecentSearches((prev) => {
      const filtered = prev.filter((s) => s.toLowerCase() !== query.toLowerCase())
      const updated = [query, ...filtered].slice(0, MAX_RECENT_SEARCHES)
      try {
        localStorage.setItem(RECENT_SEARCHES_KEY, JSON.stringify(updated))
      } catch {
        // Ignore localStorage errors
      }
      return updated
    })
  }, [])

  // Remove from recent
  const removeFromRecent = (query: string) => {
    setRecentSearches((prev) => {
      const updated = prev.filter((s) => s !== query)
      try {
        localStorage.setItem(RECENT_SEARCHES_KEY, JSON.stringify(updated))
      } catch {
        // Ignore localStorage errors
      }
      return updated
    })
  }

  // Close dropdown when clicking outside
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setIsOpen(false)
      }
    }

    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [])

  // Handle keyboard navigation
  const handleKeyDown = (event: React.KeyboardEvent) => {
    if (!isOpen) {
      if (event.key === 'ArrowDown' || event.key === 'Enter') {
        setIsOpen(true)
      }
      return
    }

    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault()
        setSelectedIndex((prev) => Math.min(prev + 1, results.length - 1))
        break
      case 'ArrowUp':
        event.preventDefault()
        setSelectedIndex((prev) => Math.max(prev - 1, -1))
        break
      case 'Enter':
        event.preventDefault()
        if (selectedIndex >= 0 && results[selectedIndex]) {
          handleResultClick(results[selectedIndex])
        }
        break
      case 'Escape':
        event.preventDefault()
        setIsOpen(false)
        setSelectedIndex(-1)
        break
    }
  }

  const handleResultClick = (result: SearchResult) => {
    saveToRecent(value)
    onResultSelect(result)
    if (result.navigateTo && onNavigate) {
      onNavigate(result.navigateTo)
    }
    setIsOpen(false)
    onChange('')
  }

  const handleRecentClick = (query: string) => {
    onChange(query)
    setIsOpen(true)
  }

  const getResultIcon = (type: SearchResult['type']) => {
    switch (type) {
      case 'diagnostic':
        return <Stethoscope20Regular />
      case 'issue':
        return <Warning20Regular />
      case 'history':
        return <History20Regular />
      case 'setting':
        return <Settings20Regular />
      default:
        return <Search20Regular />
    }
  }

  const getTypeBadge = (type: SearchResult['type']) => {
    const colorMap: Record<string, 'informative' | 'warning' | 'success' | 'subtle'> = {
      diagnostic: 'informative',
      issue: 'warning',
      history: 'success',
      setting: 'subtle',
    }
    return (
      <Badge appearance="tint" color={colorMap[type] || 'subtle'} size="small">
        {type}
      </Badge>
    )
  }

  // Group results by type
  const groupedResults = results.reduce(
    (acc, result) => {
      if (!acc[result.type]) acc[result.type] = []
      acc[result.type].push(result)
      return acc
    },
    {} as Record<string, SearchResult[]>
  )

  const showDropdown = isOpen && (value.length > 0 || recentSearches.length > 0)

  return (
    <div ref={containerRef} className={styles.container}>
      <SearchBox
        className={styles.searchBox}
        placeholder={placeholder}
        value={value}
        onChange={(_, data) => {
          onChange(data.value)
          setIsOpen(true)
          setSelectedIndex(-1)
        }}
        onFocus={() => setIsOpen(true)}
        onKeyDown={handleKeyDown}
        appearance="filled-lighter"
      />

      <div className={mergeClasses(styles.dropdown, !showDropdown && styles.dropdownHidden)}>
        {value.length > 0 ? (
          results.length > 0 ? (
            Object.entries(groupedResults).map(([type, typeResults]) => (
              <div key={type}>
                <div className={styles.sectionHeader}>
                  {getResultIcon(type as SearchResult['type'])}
                  <span>{type.charAt(0).toUpperCase() + type.slice(1)}s</span>
                  <Badge appearance="outline" size="small">
                    {typeResults.length}
                  </Badge>
                </div>
                <div className={styles.section}>
                  {typeResults.map((result, index) => {
                    const globalIndex = results.indexOf(result)
                    return (
                      <div
                        key={index}
                        className={mergeClasses(
                          styles.resultItem,
                          globalIndex === selectedIndex && styles.resultItemSelected
                        )}
                        onClick={() => handleResultClick(result)}
                        role="option"
                        aria-selected={globalIndex === selectedIndex}
                        tabIndex={-1}
                      >
                        <span className={styles.resultIcon}>{getResultIcon(result.type)}</span>
                        <div className={styles.resultContent}>
                          <Text className={styles.resultTitle} weight="medium">
                            {result.title}
                          </Text>
                          {result.description && (
                            <Caption1 className={styles.resultDescription}>{result.description}</Caption1>
                          )}
                        </div>
                        {getTypeBadge(result.type)}
                      </div>
                    )
                  })}
                </div>
              </div>
            ))
          ) : (
            <div className={styles.noResults}>
              <Search20Regular style={{ marginBottom: tokens.spacingVerticalS, opacity: 0.5 }} />
              <Text block>No results found for "{value}"</Text>
              <Caption1 style={{ marginTop: tokens.spacingVerticalXS }}>
                Try searching for task names, errors, or issues
              </Caption1>
            </div>
          )
        ) : recentSearches.length > 0 ? (
          <div className={styles.recentSearches}>
            <Caption1
              style={{
                display: 'block',
                marginBottom: tokens.spacingVerticalS,
                color: tokens.colorNeutralForeground3,
              }}
            >
              Recent Searches
            </Caption1>
            {recentSearches.map((query, index) => (
              <div
                key={index}
                className={`${styles.recentItem} recentItem`}
                onClick={() => handleRecentClick(query)}
              >
                <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalS }}>
                  <History20Regular style={{ color: tokens.colorNeutralForeground3 }} />
                  <Text>{query}</Text>
                </div>
                <button
                  className={styles.clearButton}
                  onClick={(e) => {
                    e.stopPropagation()
                    removeFromRecent(query)
                  }}
                  style={{
                    border: 'none',
                    background: 'none',
                    cursor: 'pointer',
                    padding: tokens.spacingHorizontalXS,
                  }}
                >
                  <Dismiss20Regular style={{ color: tokens.colorNeutralForeground3 }} />
                </button>
              </div>
            ))}
          </div>
        ) : null}
      </div>
    </div>
  )
}
