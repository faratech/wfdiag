import { describe, it, expect } from 'vitest'
import { fuzzyScore } from './fuzzy'

describe('fuzzyScore', () => {
  it('matches exact substrings', () => {
    const r = fuzzyScore('scan', 'Run Quick Scan')
    expect(r).not.toBeNull()
    expect(r!.indices).toEqual([10, 11, 12, 13])
  })

  it('matches non-contiguous subsequences', () => {
    expect(fuzzyScore('rqs', 'Run Quick Scan')).not.toBeNull()
  })

  it('returns null when characters are missing', () => {
    expect(fuzzyScore('xyz', 'Run Quick Scan')).toBeNull()
  })

  it('is case-insensitive', () => {
    expect(fuzzyScore('QUICK', 'run quick scan')).not.toBeNull()
  })

  it('ranks word-start matches above mid-word matches', () => {
    const wordStart = fuzzyScore('mon', 'Go to Live Monitor')!
    const midWord = fuzzyScore('mon', 'Daemon list')!
    expect(wordStart.score).toBeGreaterThan(midWord.score)
  })

  it('ranks tight matches above scattered ones', () => {
    const tight = fuzzyScore('net', 'Network Adapter')!
    const scattered = fuzzyScore('net', 'Nine extra tools')!
    expect(tight.score).toBeGreaterThan(scattered.score)
  })

  it('matches everything with an empty query', () => {
    expect(fuzzyScore('', 'anything')).toEqual({ score: 0, indices: [] })
  })
})
