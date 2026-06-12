import { describe, it, expect } from 'vitest'
import {
  formatBytes,
  formatCpuTime,
  formatUptime,
  formatDuration,
  formatMemoryMB,
  formatPercent,
} from './formatters'

describe('formatBytes', () => {
  it('formats zero', () => {
    expect(formatBytes(0)).toBe('0 B')
  })

  it('keeps sub-KB values in bytes', () => {
    expect(formatBytes(1023)).toBe('1023 B')
  })

  it('formats each unit boundary', () => {
    expect(formatBytes(1024)).toBe('1.0 KB')
    expect(formatBytes(1024 * 1024)).toBe('1.0 MB')
    expect(formatBytes(1024 * 1024 * 1024)).toBe('1.00 GB')
  })

  it('respects the decimals argument', () => {
    expect(formatBytes(1536, 2)).toBe('1.50 KB')
  })
})

describe('formatCpuTime', () => {
  it('formats minutes and seconds without hours', () => {
    expect(formatCpuTime(330)).toBe('5:30')
  })

  it('formats hours with zero-padded fields', () => {
    expect(formatCpuTime(3600 + 23 * 60 + 45)).toBe('1:23:45')
  })

  it('pads single-digit seconds', () => {
    expect(formatCpuTime(61)).toBe('1:01')
  })
})

describe('formatUptime', () => {
  it('handles fresh boot', () => {
    expect(formatUptime(0)).toBe('Just started')
  })

  it('shows minutes only when under an hour', () => {
    expect(formatUptime(120)).toBe('2 mins')
  })

  it('drops minutes once days are shown', () => {
    expect(formatUptime(5 * 86400 + 3 * 3600 + 10 * 60)).toBe('5 days, 3 hours')
  })

  it('uses singular units', () => {
    expect(formatUptime(86400 + 3600)).toBe('1 day, 1 hour')
  })
})

describe('formatDuration', () => {
  it('keeps sub-second values in ms', () => {
    expect(formatDuration(999)).toBe('999ms')
  })

  it('formats seconds with one decimal', () => {
    expect(formatDuration(5200)).toBe('5.2s')
  })

  it('rolls 59.95s+ into the minutes branch instead of showing 60.0s', () => {
    expect(formatDuration(59950)).toBe('1m 0s')
  })

  it('formats minutes and seconds', () => {
    expect(formatDuration(83000)).toBe('1m 23s')
  })
})

describe('formatMemoryMB', () => {
  it('keeps sub-GB values in MB', () => {
    expect(formatMemoryMB(256)).toBe('256.0 MB')
  })

  it('converts to GB at 1024 MB', () => {
    expect(formatMemoryMB(1536)).toBe('1.5 GB')
  })
})

describe('formatPercent', () => {
  it('formats with default decimals', () => {
    expect(formatPercent(42.345)).toBe('42.3%')
  })
})
