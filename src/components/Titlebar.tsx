import React, { useState, useEffect } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { makeStyles, shorthands, tokens } from '@fluentui/react-components'
import {
  Subtract20Regular,
  Square20Regular,
  SquareMultiple20Regular,
  Dismiss20Regular,
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  titlebar: {
    height: '32px',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    background: 'linear-gradient(135deg, rgba(11, 14, 25, 0.98) 0%, rgba(18, 22, 31, 0.95) 100%)',
    backdropFilter: 'blur(20px)',
    ...shorthands.borderBottom('1px', 'solid', 'rgba(99, 115, 255, 0.08)'),
    boxShadow: '0 1px 3px rgba(0, 0, 0, 0.2), inset 0 1px 0 rgba(255, 255, 255, 0.02)',
    userSelect: 'none',
    WebkitUserSelect: 'none',
    WebkitAppRegion: 'drag',
  },
  titleSection: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
    ...shorthands.padding('0', tokens.spacingHorizontalM),
  },
  appIcon: {
    width: '16px',
    height: '16px',
    ...shorthands.borderRadius('3px'),
    flexShrink: 0,
  },
  title: {
    fontSize: tokens.fontSizeBase200,
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground1,
    letterSpacing: '0.01em',
    textShadow: '0 1px 2px rgba(0, 0, 0, 0.3)',
  },
  windowControls: {
    display: 'flex',
    height: '100%',
    WebkitAppRegion: 'no-drag',
  },
  controlButton: {
    width: '46px',
    height: '100%',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    background: 'transparent',
    ...shorthands.border('none'),
    color: tokens.colorNeutralForeground2,
    cursor: 'pointer',
    ...shorthands.transition('all', '0.15s', 'cubic-bezier(0.4, 0, 0.2, 1)'),
    position: 'relative',

    '&::before': {
      content: '""',
      position: 'absolute',
      top: 0,
      left: 0,
      right: 0,
      bottom: 0,
      background: 'transparent',
      ...shorthands.transition('background', '0.15s', 'cubic-bezier(0.4, 0, 0.2, 1)'),
    },

    '& svg': {
      position: 'relative',
      zIndex: 1,
      ...shorthands.transition('color', '0.15s', 'cubic-bezier(0.4, 0, 0.2, 1)'),
    },

    ':hover': {
      color: tokens.colorNeutralForeground1,

      '&::before': {
        background: 'rgba(99, 115, 255, 0.15)',
      },
    },

    ':active': {
      '&::before': {
        background: 'rgba(99, 115, 255, 0.25)',
      },
    },
  },
  closeButton: {
    ':hover': {
      color: '#FFFFFF',

      '&::before': {
        background: '#EF4444',
      },
    },

    ':active': {
      '&::before': {
        background: '#DC2626',
      },
    },
  },
})

export const Titlebar: React.FC = () => {
  const styles = useStyles()
  const [isMaximized, setIsMaximized] = useState(false)
  const appWindow = getCurrentWindow()

  useEffect(() => {
    // Check initial maximized state
    const checkMaximized = async () => {
      const maximized = await appWindow.isMaximized()
      setIsMaximized(maximized)
    }
    checkMaximized()

    // Listen for window state changes
    const unlisten = appWindow.onResized(() => {
      checkMaximized()
    })

    return () => {
      unlisten.then(fn => fn())
    }
  }, [])

  const handleMinimize = async () => {
    await appWindow.minimize()
  }

  const handleMaximize = async () => {
    await appWindow.toggleMaximize()
    const maximized = await appWindow.isMaximized()
    setIsMaximized(maximized)
  }

  const handleClose = async () => {
    await appWindow.close()
  }

  return (
    <div className={styles.titlebar}>
      <div className={styles.titleSection}>
        <img
          src="/icon.png"
          alt="App Icon"
          className={styles.appIcon}
          onError={(e) => {
            // Hide icon if not found
            (e.target as HTMLImageElement).style.display = 'none'
          }}
        />
        <span className={styles.title}>WindowsForum Diagnostic Tool</span>
      </div>

      <div className={styles.windowControls}>
        <button
          className={styles.controlButton}
          onClick={handleMinimize}
          title="Minimize"
        >
          <Subtract20Regular />
        </button>

        <button
          className={styles.controlButton}
          onClick={handleMaximize}
          title={isMaximized ? "Restore" : "Maximize"}
        >
          {isMaximized ? <SquareMultiple20Regular /> : <Square20Regular />}
        </button>

        <button
          className={`${styles.controlButton} ${styles.closeButton}`}
          onClick={handleClose}
          title="Close"
        >
          <Dismiss20Regular />
        </button>
      </div>
    </div>
  )
}