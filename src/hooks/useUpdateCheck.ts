import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useToast } from '../contexts/ToastContext'
import * as logger from '../utils/logger'

export interface UpdateInfo {
  version: string
  html_url: string
  published_at?: string
  notes_excerpt?: string
}

export const CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000
const LAST_CHECK_KEY = 'updateCheck.lastRun'
const START_DELAY_MS = 5000

/** Pure throttle decision, exported for tests. */
export function shouldCheck(lastRun: string | null, now: number, interval = CHECK_INTERVAL_MS): boolean {
  if (!lastRun) return true
  const last = Number(lastRun)
  return !Number.isFinite(last) || now - last >= interval
}

/**
 * Once per 24h (and a few seconds after startup, to stay off the critical
 * path), asks the backend whether a newer GitHub release exists. The backend
 * is silent for Store installs and on any network failure, so this never
 * produces an error surface — just an info toast and the returned UpdateInfo
 * for the About dialog's download button.
 */
export function useUpdateCheck(): UpdateInfo | null {
  const { showInfo } = useToast()
  const [update, setUpdate] = useState<UpdateInfo | null>(null)

  useEffect(() => {
    if (!shouldCheck(localStorage.getItem(LAST_CHECK_KEY), Date.now())) return
    const timer = setTimeout(async () => {
      try {
        const info = await invoke<UpdateInfo | null>('check_for_update')
        localStorage.setItem(LAST_CHECK_KEY, String(Date.now()))
        if (info) {
          setUpdate(info)
          showInfo(`Update available: v${info.version}`, 'Open About to download the new version')
        }
      } catch (e) {
        logger.debug('useUpdateCheck', 'Update check failed', String(e))
      }
    }, START_DELAY_MS)
    return () => clearTimeout(timer)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return update
}
