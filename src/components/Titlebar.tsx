import React, { useEffect, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'

const inTauri = () => '__TAURI_INTERNALS__' in window

/**
 * Custom titlebar for the frameless window (decorations: false). Dragging and
 * double-click-maximize come from the data-tauri-drag-region attribute, which
 * Tauri applies only to the element itself — child buttons stay clickable.
 * Renders nothing in a plain-browser dev session.
 */
export const Titlebar: React.FC = () => {
  const [maximized, setMaximized] = useState(false)

  useEffect(() => {
    if (!inTauri()) return
    const win = getCurrentWindow()
    let unlisten: (() => void) | undefined
    win.isMaximized().then(setMaximized).catch(() => {})
    win.onResized(() => {
      win.isMaximized().then(setMaximized).catch(() => {})
    }).then(fn => { unlisten = fn }).catch(() => {})
    return () => unlisten?.()
  }, [])

  if (!inTauri()) return null

  const win = () => getCurrentWindow()

  return (
    <div className="titlebar" data-tauri-drag-region>
      <div className="tb-left" data-tauri-drag-region>
        <span className="tb-mark" aria-hidden="true">WF</span>
        <span className="tb-title">WindowsForum Diagnostic Tool</span>
      </div>
      <div className="tb-controls">
        <button onClick={() => win().minimize()} aria-label="Minimize">
          <i className="fa-solid fa-window-minimize" aria-hidden="true" />
        </button>
        <button onClick={() => win().toggleMaximize()} aria-label={maximized ? 'Restore' : 'Maximize'}>
          <i className={`fa-regular ${maximized ? 'fa-window-restore' : 'fa-window-maximize'}`} aria-hidden="true" />
        </button>
        <button className="close" onClick={() => win().close()} aria-label="Close">
          <i className="fa-solid fa-xmark" aria-hidden="true" />
        </button>
      </div>
    </div>
  )
}
