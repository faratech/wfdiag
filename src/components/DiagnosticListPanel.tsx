import React, { useMemo } from 'react'
import { makeStyles, shorthands, tokens } from '@fluentui/react-components'
import {
  EGUI_COLORS,
  CATEGORY_ICONS,
  TASK_ICONS,
  LIST_PANEL_WIDTH,
  LIST_ITEM_HEIGHT,
  STATUS_DOT_SIZE,
} from '../theme'

export interface DiagnosticItem {
  id: string
  name: string
  category: string
  success: boolean
  error?: string
  output: string
  durationMs: number
}

interface DiagnosticListPanelProps {
  diagnostics: DiagnosticItem[]
  selectedId: string | null
  onSelect: (id: string) => void
  isDarkMode?: boolean
}

const useStyles = makeStyles({
  panel: {
    width: `${LIST_PANEL_WIDTH}px`,
    minWidth: `${LIST_PANEL_WIDTH}px`,
    height: '100%',
    display: 'flex',
    flexDirection: 'column',
    borderRight: `1px solid ${tokens.colorNeutralStroke2}`,
    backgroundColor: tokens.colorNeutralBackground2,
    overflow: 'hidden',
  },
  summaryHeader: {
    ...shorthands.padding('12px'),
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'center',
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
    flexShrink: 0,
  },
  summaryText: {
    fontSize: '13px',
    fontWeight: 600,
    color: tokens.colorNeutralForeground1,
  },
  failedCount: {
    fontSize: '13px',
    fontWeight: 600,
    color: EGUI_COLORS.ERROR,
  },
  listContainer: {
    flex: 1,
    overflowY: 'auto',
    overflowX: 'hidden',
    ...shorthands.padding('8px', '0'),
  },
  categoryHeader: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap('6px'),
    ...shorthands.padding('8px', '12px', '4px'),
    marginTop: '4px',
  },
  categoryIcon: {
    fontSize: '14px',
  },
  categoryName: {
    fontSize: '12px',
    fontWeight: 600,
    color: tokens.colorNeutralForeground1,
  },
  categoryFailedBadge: {
    fontSize: '11px',
    color: EGUI_COLORS.ERROR,
    marginLeft: 'auto',
  },
  listItem: {
    display: 'flex',
    alignItems: 'center',
    height: `${LIST_ITEM_HEIGHT}px`,
    ...shorthands.padding('0', '12px'),
    cursor: 'pointer',
    ...shorthands.borderRadius('4px'),
    ...shorthands.gap('8px'),
    marginLeft: '4px',
    marginRight: '4px',
    transitionProperty: 'background-color',
    transitionDuration: tokens.durationFaster,
    ':hover': {
      backgroundColor: 'var(--hover-bg)',
    },
  },
  listItemSelected: {
    backgroundColor: 'var(--selected-bg)',
    ':hover': {
      backgroundColor: 'var(--selected-bg)',
    },
  },
  statusDot: {
    width: `${STATUS_DOT_SIZE}px`,
    height: `${STATUS_DOT_SIZE}px`,
    ...shorthands.borderRadius('50%'),
    flexShrink: 0,
  },
  statusDotSuccess: {
    backgroundColor: EGUI_COLORS.SUCCESS,
  },
  statusDotError: {
    backgroundColor: EGUI_COLORS.ERROR,
  },
  taskIcon: {
    fontSize: '12px',
    flexShrink: 0,
  },
  taskName: {
    fontSize: '12px',
    color: tokens.colorNeutralForeground1,
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    flex: 1,
  },
  emptyState: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    height: '100%',
    ...shorthands.padding('24px'),
    textAlign: 'center',
  },
  emptyTitle: {
    fontSize: '16px',
    fontWeight: 600,
    color: tokens.colorNeutralForeground1,
    marginBottom: '8px',
  },
  emptyDescription: {
    fontSize: '13px',
    color: tokens.colorNeutralForeground3,
  },
})

// Category display order
const CATEGORY_ORDER: Record<string, number> = {
  Hardware: 0,
  Storage: 1,
  Network: 2,
  System: 3,
}

function getCategoryOrder(category: string): number {
  return CATEGORY_ORDER[category] ?? 99
}

export const DiagnosticListPanel: React.FC<DiagnosticListPanelProps> = ({
  diagnostics,
  selectedId,
  onSelect,
  isDarkMode = false,
}) => {
  const styles = useStyles()

  // Group diagnostics by category
  const groupedDiagnostics = useMemo(() => {
    const groups: Record<string, DiagnosticItem[]> = {}

    // Sort diagnostics by category order, then by name
    const sorted = [...diagnostics].sort((a, b) => {
      const orderDiff = getCategoryOrder(a.category) - getCategoryOrder(b.category)
      if (orderDiff !== 0) return orderDiff
      return a.name.localeCompare(b.name)
    })

    for (const item of sorted) {
      if (!groups[item.category]) {
        groups[item.category] = []
      }
      groups[item.category].push(item)
    }

    return groups
  }, [diagnostics])

  // Calculate summary stats
  const totalCount = diagnostics.length
  const failedCount = diagnostics.filter(d => !d.success).length

  // CSS custom properties for theme-aware hover/selected colors
  const cssVars = {
    '--hover-bg': isDarkMode ? EGUI_COLORS.HOVER_DARK : EGUI_COLORS.HOVER_LIGHT,
    '--selected-bg': isDarkMode ? EGUI_COLORS.SELECTED_DARK : EGUI_COLORS.SELECTED_LIGHT,
  } as React.CSSProperties

  if (diagnostics.length === 0) {
    return (
      <div className={styles.panel}>
        <div className={styles.emptyState}>
          <div className={styles.emptyTitle}>No Results</div>
          <div className={styles.emptyDescription}>
            Run a scan to see diagnostics
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className={styles.panel} style={cssVars}>
      {/* Summary header */}
      <div className={styles.summaryHeader}>
        <span className={styles.summaryText}>
          {totalCount} diagnostic{totalCount !== 1 ? 's' : ''}
        </span>
        {failedCount > 0 && (
          <span className={styles.failedCount}>
            {failedCount} failed
          </span>
        )}
      </div>

      {/* Category-grouped list */}
      <div className={styles.listContainer}>
        {Object.entries(groupedDiagnostics)
          .sort(([a], [b]) => getCategoryOrder(a) - getCategoryOrder(b))
          .map(([category, items]) => {
            const categoryFailed = items.filter(i => !i.success).length
            const categoryIcon = CATEGORY_ICONS[category] || '\u{1F4C4}'

            return (
              <div key={category}>
                {/* Category header */}
                <div className={styles.categoryHeader}>
                  <span className={styles.categoryIcon}>{categoryIcon}</span>
                  <span className={styles.categoryName}>{category}</span>
                  {categoryFailed > 0 && (
                    <span className={styles.categoryFailedBadge}>
                      * {categoryFailed}
                    </span>
                  )}
                </div>

                {/* Items in this category */}
                {items.map(item => {
                  const isSelected = selectedId === item.id
                  const taskIcon = TASK_ICONS[item.id] || '\u{1F4C4}'

                  return (
                    <div
                      key={item.id}
                      className={`${styles.listItem} ${isSelected ? styles.listItemSelected : ''}`}
                      onClick={() => onSelect(item.id)}
                      role="button"
                      tabIndex={0}
                      onKeyDown={e => {
                        if (e.key === 'Enter' || e.key === ' ') {
                          e.preventDefault()
                          onSelect(item.id)
                        }
                      }}
                    >
                      <div
                        className={`${styles.statusDot} ${
                          item.success ? styles.statusDotSuccess : styles.statusDotError
                        }`}
                      />
                      <span className={styles.taskIcon}>{taskIcon}</span>
                      <span className={styles.taskName}>{item.name}</span>
                    </div>
                  )
                })}
              </div>
            )
          })}
      </div>
    </div>
  )
}
