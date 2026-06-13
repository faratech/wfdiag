import { useEffect, useRef } from 'react'
import { useAppContext } from '../contexts/AppContext'
import { useScanner } from './useScanner'
import type { TabValue } from '../components/types'

const TAB_ORDER: TabValue[] = ['diagnostics', 'monitoring', 'processes', 'ai', 'issues', 'history']

interface GlobalShortcutOptions {
  onTogglePalette: () => void
  onShowHelp: () => void
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  const tag = target.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable
}

/**
 * Single app-wide keydown listener (mounted once from AppContent). Modals and
 * the palette handle their own Escape inside their focus traps and stop
 * propagation, so there is no overlay registry to coordinate here.
 */
export function useGlobalShortcuts({ onTogglePalette, onShowHelp }: GlobalShortcutOptions) {
  const { setSelectedTab, isRunning } = useAppContext()
  const { runQuickScan, runFullScan } = useScanner()

  // Keep the handler stable; read latest values through a ref (synced after
  // each render so the listener never holds stale closures)
  const stateRef = useRef({ setSelectedTab, isRunning, runQuickScan, runFullScan, onTogglePalette, onShowHelp })
  useEffect(() => {
    stateRef.current = { setSelectedTab, isRunning, runQuickScan, runFullScan, onTogglePalette, onShowHelp }
  })

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const s = stateRef.current
      const inEditable = isEditableTarget(e.target)

      // Ctrl+K works everywhere, including inputs
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        s.onTogglePalette()
        return
      }
      if (inEditable) return

      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key >= '1' && e.key <= '6') {
        e.preventDefault()
        s.setSelectedTab(TAB_ORDER[Number(e.key) - 1])
        return
      }
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === '/') {
        e.preventDefault()
        s.onShowHelp()
        return
      }
      if (e.ctrlKey && e.shiftKey && !e.altKey && e.key.toLowerCase() === 'q' && !s.isRunning) {
        e.preventDefault()
        s.runQuickScan()
        return
      }
      if (e.ctrlKey && e.shiftKey && !e.altKey && e.key.toLowerCase() === 'f' && !s.isRunning) {
        e.preventDefault()
        s.runFullScan()
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])
}
