/**
 * Tiny fuzzy matcher for the command palette. Case-insensitive subsequence
 * match with bonuses for word-start hits, consecutive runs, and matches near
 * the beginning. Returns matched indices so the UI can highlight them.
 */
export interface FuzzyResult {
  score: number
  indices: number[]
}

const WORD_SEPARATORS = new Set([' ', '-', '_', '/', '.', ':'])

export function fuzzyScore(query: string, target: string): FuzzyResult | null {
  const q = query.toLowerCase()
  const t = target.toLowerCase()
  if (!q) return { score: 0, indices: [] }
  if (q.length > t.length) return null

  const indices: number[] = []
  let score = 0
  let ti = 0
  let prevMatch = -2

  for (let qi = 0; qi < q.length; qi++) {
    const ch = q[qi]
    let found = -1
    for (; ti < t.length; ti++) {
      if (t[ti] === ch) {
        found = ti
        break
      }
    }
    if (found === -1) return null

    let bonus = 1
    if (found === 0) bonus += 8
    else if (WORD_SEPARATORS.has(t[found - 1])) bonus += 6
    if (found === prevMatch + 1) bonus += 4
    // Earlier matches are slightly better
    bonus += Math.max(0, 3 - found * 0.05)

    score += bonus
    indices.push(found)
    prevMatch = found
    ti = found + 1
  }

  // Prefer tighter overall matches
  const spread = indices[indices.length - 1] - indices[0] + 1
  score += Math.max(0, q.length * 2 - (spread - q.length))

  return { score, indices }
}
