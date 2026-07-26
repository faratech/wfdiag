import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import type { ProcessPage } from '../types/monitoring'
import * as logger from '../utils/logger'

export type ProcessSortKey =
  | 'name'
  | 'pid'
  | 'cpu_percent'
  | 'memory_percent'
  | 'memory_mb'
  | 'status'
  | 'thread_count'

export type ProcessSortDirection = 'asc' | 'desc'

interface ProcessExplorerQuery {
  search: string
  sortBy: ProcessSortKey
  sortDirection: ProcessSortDirection
  offset: number
  limit: number
}

const REFRESH_MS = 2_000

export function useProcessExplorer(query: ProcessExplorerQuery) {
  const [page, setPage] = useState<ProcessPage | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [isRefreshing, setIsRefreshing] = useState(false)
  const [isPaused, setIsPaused] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const requestIdRef = useRef(0)
  const inFlightRef = useRef(false)
  const queuedRefreshRef = useRef(false)
  const queuedInitialRef = useRef(false)
  const disposedRef = useRef(false)
  const refreshRef = useRef<(showInitialLoading?: boolean) => Promise<void>>(async () => {})

  const refresh = useCallback(async (showInitialLoading = false) => {
    if (showInitialLoading || page === null) setIsLoading(true)
    else setIsRefreshing(true)

    // Enumeration can exceed the polling interval on busy machines. Coalesce
    // all overlapping ticks into one follow-up request so calls neither queue
    // behind the backend monitor lock nor continually invalidate each other.
    if (inFlightRef.current) {
      queuedRefreshRef.current = true
      queuedInitialRef.current ||= showInitialLoading
      if (showInitialLoading) requestIdRef.current += 1
      return
    }

    inFlightRef.current = true
    const requestId = ++requestIdRef.current

    try {
      const next = await invoke<ProcessPage>('list_processes', {
        query: {
          search: query.search,
          sort_by: query.sortBy,
          sort_direction: query.sortDirection,
          offset: query.offset,
          limit: query.limit,
        },
      })
      if (requestId !== requestIdRef.current) return
      setPage(next)
      setError(null)
    } catch (cause) {
      if (requestId !== requestIdRef.current) return
      const message = cause instanceof Error ? cause.message : String(cause)
      logger.error('useProcessExplorer', 'Failed to load processes', cause)
      setError(message)
    } finally {
      if (requestId === requestIdRef.current) {
        setIsLoading(false)
        setIsRefreshing(false)
      }
      inFlightRef.current = false
      if (queuedRefreshRef.current && !disposedRef.current) {
        const nextInitial = queuedInitialRef.current
        queuedRefreshRef.current = false
        queuedInitialRef.current = false
        void refreshRef.current(nextInitial)
      }
    }
  }, [page, query.limit, query.offset, query.search, query.sortBy, query.sortDirection])

  useEffect(() => {
    refreshRef.current = refresh
  }, [refresh])

  useEffect(() => {
    const debounce = window.setTimeout(() => void refresh(true), 180)
    return () => window.clearTimeout(debounce)
  }, [query.limit, query.offset, query.search, query.sortBy, query.sortDirection]) // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (isPaused) return
    const interval = window.setInterval(() => {
      if (!document.hidden) void refresh(false)
    }, REFRESH_MS)
    return () => window.clearInterval(interval)
  }, [isPaused, refresh])

  useEffect(() => {
    disposedRef.current = false
    return () => {
      disposedRef.current = true
      queuedRefreshRef.current = false
      queuedInitialRef.current = false
      requestIdRef.current += 1
    }
  }, [])

  return {
    page,
    isLoading,
    isRefreshing,
    isPaused,
    error,
    refresh: () => refresh(false),
    togglePaused: () => setIsPaused(value => !value),
  }
}
