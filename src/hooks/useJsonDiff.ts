/**
 * Hook for comparing JSON data and finding differences
 */

export interface JsonDifference {
  path: string
  type: 'added' | 'removed' | 'modified' | 'type_changed'
  oldValue?: any
  newValue?: any
}

export function useJsonDiff() {
  
  const compareJson = (obj1: any, obj2: any, path: string = ''): JsonDifference[] => {
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
      const allKeys = new Set([...Object.keys(obj1), ...Object.keys(obj2)])
      
      for (const key of allKeys) {
        const currentPath = path ? `${path}.${key}` : key
        
        if (!(key in obj1)) {
          differences.push({
            path: currentPath,
            type: 'added',
            newValue: obj2[key]
          })
        } else if (!(key in obj2)) {
          differences.push({
            path: currentPath,
            type: 'removed',
            oldValue: obj1[key]
          })
        } else {
          differences.push(...compareJson(obj1[key], obj2[key], currentPath))
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
      const parsed1 = JSON.parse(json1)
      const parsed2 = JSON.parse(json2)
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