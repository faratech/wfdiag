import React, { useEffect, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'

const inTauri = () => '__TAURI_INTERNALS__' in window

interface TitlebarProps {
  isDark: boolean
  onToggleTheme: () => void
}

/**
 * Custom titlebar for the frameless window (decorations: false). Sits directly
 * on the wallpaper. Left: brand mark + name. Right: theme toggle, then the
 * window controls. Dragging + double-click-maximize come from
 * data-tauri-drag-region (applied only to the bar itself; child buttons stay
 * clickable). Outside Tauri it still renders the bar + theme toggle so the
 * chrome and theme switch are usable in a plain browser dev session.
 */
export const Titlebar: React.FC<TitlebarProps> = ({ isDark, onToggleTheme }) => {
  const [maximized, setMaximized] = useState(false)
  const tauri = inTauri()

  useEffect(() => {
    if (!tauri) return
    const win = getCurrentWindow()
    let unlisten: (() => void) | undefined
    let cancelled = false
    win.isMaximized().then(setMaximized).catch(() => {})
    win.onResized(() => {
      win.isMaximized().then(setMaximized).catch(() => {})
    }).then(fn => {
      // Cleanup may already have run while onResized() was still in flight
      // (e.g. React StrictMode's dev double-invoke) — tear the just-created
      // listener down immediately instead of leaking a duplicate handler.
      if (cancelled) {
        fn()
      } else {
        unlisten = fn
      }
    }).catch(() => {})
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [tauri])

  const win = () => getCurrentWindow()

  return (
    <div className="titlebar" data-tauri-drag-region>
      <div className="tb-left" data-tauri-drag-region>
        <span className="tb-mark" aria-hidden="true"><img src="/wf-ds/icon-only.png" alt="" /></span>
        <span className="tb-title">WindowsForum Diagnostics</span>
      </div>
      <div className="tb-spacer" data-tauri-drag-region />
      <div className="tb-controls">
        <button title="Switch theme" aria-label="Switch theme" onClick={onToggleTheme}>
          <i className={`fa-solid ${isDark ? 'fa-sun' : 'fa-moon'}`} aria-hidden="true" />
        </button>
        {tauri && (
          <>
            <button onClick={() => win().minimize()} aria-label="Minimize" style={{ fontSize: 11 }}>
              <i className="fa-solid fa-minus" aria-hidden="true" />
            </button>
            <button onClick={() => win().toggleMaximize()} aria-label={maximized ? 'Restore' : 'Maximize'} style={{ fontSize: 10 }}>
              <i className={`fa-regular ${maximized ? 'fa-window-restore' : 'fa-square'}`} aria-hidden="true" />
            </button>
            <button className="close" onClick={() => win().close()} aria-label="Close" style={{ fontSize: 13 }}>
              <i className="fa-solid fa-xmark" aria-hidden="true" />
            </button>
          </>
        )}
      </div>
    </div>
  )
}
