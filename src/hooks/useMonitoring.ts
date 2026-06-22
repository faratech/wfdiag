/**
 * Shared monitoring hook for system stats and process data
 * Consolidates lifecycle management from SystemMonitoring.tsx and ProcessesTab.tsx
 */

import { useState, useEffect, useRef, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen, UnlistenFn } from '@tauri-apps/api/event'
import type { SystemStats, ProcessInfo } from '../types/monitoring'
import * as logger from '../utils/logger'

export interface UseMonitoringOptions {
  /** Auto-start monitoring when hook mounts (default: false) */
  autoStart?: boolean
  /** Callback when new stats are received */
  onStats?: (stats: SystemStats) => void
  /** Component name for logging */
  componentName?: string
  /** Include per-process GPU/NPU adapter stats. This is intentionally opt-in. */
  includeProcessAdapterStats?: boolean
}

export interface UseMonitoringResult {
  /** Whether monitoring is currently active */
  isActive: boolean
  /** Latest system stats (null if not yet received) */
  stats: SystemStats | null
  /** All processes from latest stats */
  processes: ProcessInfo[]
  /** Whether the hook is loading/initializing */
  isLoading: boolean
  /** Start monitoring */
  start: () => Promise<void>
  /** Stop monitoring */
  stop: () => Promise<void>
  /** Toggle monitoring on/off */
  toggle: () => Promise<void>
  /** Manually refresh stats */
  refresh: () => Promise<void>
}

export function useMonitoring(options: UseMonitoringOptions = {}): UseMonitoringResult {
  const {
    autoStart = false,
    onStats,
    componentName = 'useMonitoring',
    includeProcessAdapterStats = false,
  } = options

  const [isActive, setIsActive] = useState(false)
  const [stats, setStats] = useState<SystemStats | null>(null)
  const [isLoading, setIsLoading] = useState(false)

  // Refs for cleanup and preventing double-initialization
  const unlistenRef = useRef<UnlistenFn | null>(null)
  const leaseRef = useRef<number | null>(null)
  const hasAutoStarted = useRef(false)
  const isMountedRef = useRef(true)

  // Start monitoring
  const start = useCallback(async () => {
    if (isActive || leaseRef.current !== null) return

    try {
      setIsLoading(true)
      const leaseId = await invoke<number>('start_monitoring', { includeProcessAdapterStats })
      leaseRef.current = leaseId

      if (!isMountedRef.current) {
        // Component unmounted during async call
        await invoke('stop_monitoring', { leaseId }).catch(err =>
          logger.error(componentName, 'Failed to stop monitoring after unmount', err)
        )
        if (leaseRef.current === leaseId) {
          leaseRef.current = null
        }
        return
      }

      // Set up event listener
      const unlisten = await listen<SystemStats>('system-stats', (event) => {
        if (!isMountedRef.current) return

        const newStats = event.payload
        setStats(newStats)
        onStats?.(newStats)
      })

      // The component may have unmounted while listen() was in flight. If so, cleanup
      // already ran and saw a null ref — tear the listener down here so it doesn't leak
      // and keep firing on an unmounted view.
      if (!isMountedRef.current) {
        unlisten()
        return
      }
      // Defensively drop any previous listener before overwriting the ref.
      if (unlistenRef.current) {
        unlistenRef.current()
      }
      unlistenRef.current = unlisten
      setIsActive(true)
    } catch (error) {
      const leaseId = leaseRef.current
      if (leaseId !== null) {
        leaseRef.current = null
        await invoke('stop_monitoring', { leaseId }).catch(err =>
          logger.error(componentName, 'Failed to stop monitoring after start error', err)
        )
      }
      logger.error(componentName, 'Failed to start monitoring', error)
    } finally {
      if (isMountedRef.current) {
        setIsLoading(false)
      }
    }
  }, [isActive, onStats, componentName, includeProcessAdapterStats])

  // Stop monitoring
  const stop = useCallback(async () => {
    if (!isActive && leaseRef.current === null) return

    // Clean up event listener
    if (unlistenRef.current) {
      unlistenRef.current()
      unlistenRef.current = null
    }

    try {
      const leaseId = leaseRef.current
      leaseRef.current = null
      await invoke('stop_monitoring', leaseId !== null ? { leaseId } : {})
    } catch (error) {
      logger.error(componentName, 'Failed to stop monitoring', error)
    }

    if (isMountedRef.current) {
      setIsActive(false)
    }
  }, [isActive, componentName])

  // Toggle monitoring
  const toggle = useCallback(async () => {
    if (isActive) {
      await stop()
    } else {
      await start()
    }
  }, [isActive, start, stop])

  // Manually refresh stats
  const refresh = useCallback(async () => {
    try {
      const currentStats = await invoke<SystemStats>('get_current_stats')
      if (currentStats && isMountedRef.current) {
        setStats(currentStats)
        onStats?.(currentStats)
      }
    } catch (error) {
      logger.debug(componentName, 'Could not fetch stats', error)
    }
  }, [onStats, componentName])

  // Auto-start on mount if requested
  useEffect(() => {
    if (autoStart && !hasAutoStarted.current) {
      hasAutoStarted.current = true
      start()
    }
  }, [autoStart, start])

  // Cleanup on unmount
  useEffect(() => {
    isMountedRef.current = true

    return () => {
      isMountedRef.current = false

      // Clean up event listener
      if (unlistenRef.current) {
        unlistenRef.current()
        unlistenRef.current = null
      }

      // Stop only the monitoring stream this hook started. A later hook mount
      // has a newer lease and the backend will ignore this stale cleanup.
      const leaseId = leaseRef.current
      if (leaseId !== null) {
        leaseRef.current = null
        invoke('stop_monitoring', { leaseId }).catch(error =>
          logger.error(componentName, 'Failed to stop monitoring on cleanup', error)
        )
      }
    }
  }, [componentName])

  // Extract processes from stats
  const processes = stats?.top_processes ?? []

  return {
    isActive,
    stats,
    processes,
    isLoading,
    start,
    stop,
    toggle,
    refresh,
  }
}

export default useMonitoring
