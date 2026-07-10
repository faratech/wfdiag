import { describe, it, expect } from 'vitest'
import { formatDuration, formatBytesMb, parseOutput, toKeyValues } from './util'

describe('formatDuration', () => {
  it('returns an em dash for zero and negative values', () => {
    expect(formatDuration(0)).toBe('—')
    expect(formatDuration(-5)).toBe('—')
  })

  it('formats sub-second values in ms', () => {
    expect(formatDuration(450)).toBe('450 ms')
  })

  it('formats seconds with one decimal', () => {
    expect(formatDuration(5300)).toBe('5.3s')
  })

  it('formats minutes and seconds', () => {
    expect(formatDuration(83000)).toBe('1m 23s')
  })
})

describe('formatBytesMb', () => {
  it('keeps sub-GB values in MB', () => {
    expect(formatBytesMb(512)).toBe('512.0 MB')
  })

  it('converts to GB at 1024 MB', () => {
    expect(formatBytesMb(2048)).toBe('2.00 GB')
  })
})

describe('parseOutput', () => {
  it('parses JSON objects', () => {
    expect(parseOutput('{"a": 1}')).toEqual({ a: 1 })
  })

  it('parses JSON arrays with surrounding whitespace', () => {
    expect(parseOutput('  [1, 2]  ')).toEqual([1, 2])
  })

  it('returns null for malformed JSON', () => {
    expect(parseOutput('{not json')).toBeNull()
  })

  it('returns null for plain text and empty strings', () => {
    expect(parseOutput('plain text output')).toBeNull()
    expect(parseOutput('')).toBeNull()
    expect(parseOutput('   ')).toBeNull()
  })
})

describe('toKeyValues', () => {
  it('returns empty array for non-objects', () => {
    expect(toKeyValues(null)).toEqual([])
    expect(toKeyValues('string')).toEqual([])
  })

  it('flattens flat objects', () => {
    expect(toKeyValues({ a: 1, b: 'x' })).toEqual([
      ['a', '1'],
      ['b', 'x'],
    ])
  })

  it('flattens nested objects with the joined key prefix', () => {
    expect(toKeyValues({ outer: { inner: 'v' } })).toEqual([['outer · inner', 'v']])
  })

  it('joins arrays and stringifies object elements', () => {
    expect(toKeyValues({ list: [1, 'two', { x: 1 }] })).toEqual([
      ['list', '1, two, {"x":1}'],
    ])
  })

  it('stringifies null values', () => {
    expect(toKeyValues({ a: null })).toEqual([['a', 'null']])
  })
})
