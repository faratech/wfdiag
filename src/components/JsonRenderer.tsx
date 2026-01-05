import React from 'react'
import { makeStyles, shorthands, tokens } from '@fluentui/react-components'
import { EGUI_COLORS } from '../theme'

interface JsonRendererProps {
  value: unknown
  maxDepth?: number
}

const useStyles = makeStyles({
  container: {
    fontSize: '11px',
    lineHeight: '1.5',
  },
  grid: {
    display: 'grid',
    gridTemplateColumns: 'auto 1fr',
    columnGap: '16px',
    rowGap: '6px',
  },
  keyLabel: {
    color: tokens.colorNeutralForeground3,
    whiteSpace: 'nowrap',
  },
  valueCell: {
    minWidth: 0,
  },
  null: {
    color: tokens.colorNeutralForeground4,
    fontStyle: 'italic',
  },
  empty: {
    color: tokens.colorNeutralForeground4,
    fontStyle: 'italic',
  },
  number: {
    color: EGUI_COLORS.NUMBER,
  },
  boolTrue: {
    color: EGUI_COLORS.BOOLEAN_TRUE,
  },
  boolFalse: {
    color: EGUI_COLORS.BOOLEAN_FALSE,
  },
  string: {
    wordBreak: 'break-word',
  },
  arrayItem: {
    display: 'flex',
    ...shorthands.gap('4px'),
    marginTop: '4px',
  },
  arrayBorder: {
    color: tokens.colorNeutralForeground4,
    paddingLeft: '8px',
    paddingRight: '4px',
  },
  arrayContent: {
    flex: 1,
    minWidth: 0,
  },
})

// Fields to skip in display (system/internal fields)
const SKIP_FIELDS = new Set([
  'PSComputerName',
  'Scope',
  'Options',
  'ClassPath',
  'Properties',
  'SystemProperties',
  'Qualifiers',
  'Site',
  'Container',
  'CSName',
  'SystemName',
  'PSShowComputerName',
  'CimClass',
  'CimInstanceProperties',
  'CimSystemProperties',
  'CreationClassName',
  'SystemCreationClassName',
])

// Field priority for sorting (lower = higher priority)
function getFieldPriority(key: string): number {
  switch (key) {
    case 'Name':
    case 'Caption':
    case 'DisplayName':
      return 0
    case 'Description':
      return 1
    case 'Status':
    case 'State':
      return 2
    case 'Version':
    case 'DriverVersion':
      return 3
    case 'Size':
    case 'Capacity':
    case 'FreeSpace':
      return 4
    default:
      return 50
  }
}

// Common field name mappings for better display
const FIELD_NAME_MAPPINGS: Record<string, string> = {
  IPAddress: 'IP Address',
  MACAddress: 'MAC',
  DHCPEnabled: 'DHCP',
  FreeSpace: 'Free',
  MaxClockSpeed: 'Max Speed',
  CurrentClockSpeed: 'Speed',
  NumberOfCores: 'Cores',
  NumberOfLogicalProcessors: 'Threads',
  DriverVersion: 'Version',
  TotalPhysicalMemory: 'Total RAM',
}

function formatFieldName(key: string): string {
  // Check for direct mapping
  if (FIELD_NAME_MAPPINGS[key]) {
    return FIELD_NAME_MAPPINGS[key]
  }

  // Convert CamelCase to spaced
  let result = ''
  for (let i = 0; i < key.length; i++) {
    const char = key[i]
    if (char === char.toUpperCase() && i > 0 && char !== char.toLowerCase()) {
      result += ' '
    }
    result += char
  }
  return result
}

function shouldSkipField(key: string): boolean {
  return key.startsWith('__') || SKIP_FIELDS.has(key)
}

function isEmptyValue(value: unknown): boolean {
  if (value === null || value === undefined) return true
  if (typeof value === 'string' && value.trim() === '') return true
  if (Array.isArray(value) && value.length === 0) return true
  return false
}

const JsonValue: React.FC<{
  value: unknown
  depth: number
  maxDepth: number
  styles: ReturnType<typeof useStyles>
}> = ({ value, depth, maxDepth, styles }) => {
  // Null
  if (value === null || value === undefined) {
    return <span className={styles.null}>null</span>
  }

  // Boolean
  if (typeof value === 'boolean') {
    return (
      <span className={value ? styles.boolTrue : styles.boolFalse}>
        {value ? 'True' : 'False'}
      </span>
    )
  }

  // Number
  if (typeof value === 'number') {
    return <span className={styles.number}>{value.toString()}</span>
  }

  // String
  if (typeof value === 'string') {
    if (value === '') {
      return <span className={styles.empty}>(empty)</span>
    }
    // Truncate very long strings
    const display = value.length > 200 ? `${value.slice(0, 197)}...` : value
    return <span className={styles.string}>{display}</span>
  }

  // Array
  if (Array.isArray(value)) {
    if (value.length === 0) {
      return <span className={styles.empty}>[]</span>
    }

    return (
      <div>
        {value.map((item, i) => (
          <div key={i} className={styles.arrayItem}>
            {depth > 0 && <span className={styles.arrayBorder}>|</span>}
            <div className={styles.arrayContent}>
              <JsonValue value={item} depth={depth + 1} maxDepth={maxDepth} styles={styles} />
            </div>
          </div>
        ))}
      </div>
    )
  }

  // Object
  if (typeof value === 'object') {
    const obj = value as Record<string, unknown>
    const keys = Object.keys(obj)

    if (keys.length === 0) {
      return <span className={styles.empty}>{'{}'}</span>
    }

    // Sort keys by priority then alphabetically
    const sortedKeys = keys
      .filter(k => !shouldSkipField(k) && !isEmptyValue(obj[k]))
      .sort((a, b) => {
        const priorityDiff = getFieldPriority(a) - getFieldPriority(b)
        if (priorityDiff !== 0) return priorityDiff
        return a.localeCompare(b)
      })

    if (sortedKeys.length === 0) {
      return <span className={styles.empty}>(no data)</span>
    }

    // Prevent excessive nesting
    if (depth >= maxDepth) {
      return <span className={styles.empty}>[nested object]</span>
    }

    return (
      <div className={styles.grid}>
        {sortedKeys.map(key => (
          <React.Fragment key={key}>
            <span className={styles.keyLabel}>{formatFieldName(key)}</span>
            <div className={styles.valueCell}>
              <JsonValue value={obj[key]} depth={depth + 1} maxDepth={maxDepth} styles={styles} />
            </div>
          </React.Fragment>
        ))}
      </div>
    )
  }

  // Fallback for unknown types
  return <span>{String(value)}</span>
}

export const JsonRenderer: React.FC<JsonRendererProps> = ({ value, maxDepth = 5 }) => {
  const styles = useStyles()

  return (
    <div className={styles.container}>
      <JsonValue value={value} depth={0} maxDepth={maxDepth} styles={styles} />
    </div>
  )
}

// Helper to safely parse JSON string
export function parseJsonSafe(str: string): unknown {
  try {
    return JSON.parse(str)
  } catch {
    // Try to extract JSON from mixed content
    const jsonMatch = str.match(/\[[\s\S]*\]|\{[\s\S]*\}/)
    if (jsonMatch) {
      try {
        return JSON.parse(jsonMatch[0])
      } catch {
        return str
      }
    }
    return str
  }
}
