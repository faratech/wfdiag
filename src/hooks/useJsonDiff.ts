/**
 * Hook for comparing JSON data and finding differences
 */

/** Any value that can result from `JSON.parse`. */
export type JsonValue = string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue }

export interface JsonDifference {
  path: string
  type: 'added' | 'removed' | 'modified' | 'type_changed'
  oldValue?: JsonValue
  newValue?: JsonValue
}

export function useJsonDiff() {

  const compareJson = (obj1: JsonValue | undefined, obj2: JsonValue | undefined, path: string = ''): JsonDifference[] => {
    const differences: JsonDifference[] = []
    
    // Handle null/undefined cases
    if (obj1 === obj2) return differences
    if (obj1 === null || obj1 === undefined || obj2 === null || obj2 === undefined) {
      differences.push({
        path: path || 'root',
        type: 'modified',
        oldValue: obj1,
        newValue: obj2
      })
      return differences
    }
    
    // Different types
    if (typeof obj1 !== typeof obj2) {
      differences.push({
        path: path || 'root',
        type: 'type_changed',
        oldValue: obj1,
        newValue: obj2
      })
      return differences
    }
    
    // Arrays
    if (Array.isArray(obj1) && Array.isArray(obj2)) {
      const maxLength = Math.max(obj1.length, obj2.length)
      for (let i = 0; i < maxLength; i++) {
        const currentPath = `${path}[${i}]`
        if (i >= obj1.length) {
          differences.push({
            path: currentPath,
            type: 'added',
            newValue: obj2[i]
          })
        } else if (i >= obj2.length) {
          differences.push({
            path: currentPath,
            type: 'removed',
            oldValue: obj1[i]
          })
        } else {
          differences.push(...compareJson(obj1[i], obj2[i], currentPath))
        }
      }
      return differences
    }
    
    // Objects
    if (typeof obj1 === 'object' && typeof obj2 === 'object') {
      const o1 = obj1 as { [key: string]: JsonValue }
      const o2 = obj2 as { [key: string]: JsonValue }
      const allKeys = new Set([...Object.keys(o1), ...Object.keys(o2)])

      for (const key of allKeys) {
        const currentPath = path ? `${path}.${key}` : key

        if (!(key in o1)) {
          differences.push({
            path: currentPath,
            type: 'added',
            newValue: o2[key]
          })
        } else if (!(key in o2)) {
          differences.push({
            path: currentPath,
            type: 'removed',
            oldValue: o1[key]
          })
        } else {
          differences.push(...compareJson(o1[key], o2[key], currentPath))
        }
      }
      return differences
    }
    
    // Primitives
    if (obj1 !== obj2) {
      differences.push({
        path: path || 'root',
        type: 'modified',
        oldValue: obj1,
        newValue: obj2
      })
    }
    
    return differences
  }
  
  const findJsonDifferences = (json1: string, json2: string): JsonDifference[] | null => {
    try {
      const parsed1 = JSON.parse(json1) as JsonValue
      const parsed2 = JSON.parse(json2) as JsonValue
      return compareJson(parsed1, parsed2)
    } catch {
      return null
    }
  }
  
  const formatDifference = (diff: JsonDifference): string => {
    switch (diff.type) {
      case 'added':
        return `Added: ${diff.path} = ${JSON.stringify(diff.newValue)}`
      case 'removed':
        return `Removed: ${diff.path} = ${JSON.stringify(diff.oldValue)}`
      case 'modified':
        return `Changed: ${diff.path} from ${JSON.stringify(diff.oldValue)} to ${JSON.stringify(diff.newValue)}`
      case 'type_changed':
        return `Type changed: ${diff.path} from ${typeof diff.oldValue} to ${typeof diff.newValue}`
      default:
        return `Unknown change at ${diff.path}`
    }
  }
  
  return {
    findJsonDifferences,
    formatDifference,
    compareJson
  }
}