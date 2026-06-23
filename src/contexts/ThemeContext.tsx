import React, { createContext, useContext, useState, useEffect, ReactNode } from 'react'

// De-Fluented theme provider. The WindowsForum design tokens live statically
// in src/styles/colors_and_type.css and react to `html.is-dark` / `html.is-light`
// (and `prefers-color-scheme` when neither class is set). This provider just
// owns the theme mode and toggles those classes.

type ThemeMode = 'dark' | 'light' | 'auto'

interface ThemeContextType {
  themeMode: ThemeMode
  setThemeMode: (mode: ThemeMode) => void
  /** Effective theme after resolving Auto against the OS preference. */
  isDark: boolean
}

const ThemeContext = createContext<ThemeContextType | undefined>(undefined)

const applyThemeClass = (isDark: boolean) => {
  const root = document.documentElement
  root.setAttribute('data-theme', isDark ? 'dark' : 'light')
  root.classList.toggle('is-dark', isDark)
  root.classList.toggle('is-light', !isDark)
}

export const useTheme = () => {
  const context = useContext(ThemeContext)
  if (!context) {
    throw new Error('useTheme must be used within a ThemeProvider')
  }
  return context
}

interface ThemeProviderProps {
  children: ReactNode
  initialMode?: ThemeMode
  onModeChange?: (mode: ThemeMode) => void
}

export const ThemeProvider: React.FC<ThemeProviderProps> = ({
  children,
  initialMode = 'dark',
  onModeChange,
}) => {
  const [themeMode, setThemeModeInternal] = useState<ThemeMode>(initialMode)
  const [systemPrefersDark, setSystemPrefersDark] = useState(() =>
    window.matchMedia('(prefers-color-scheme: dark)').matches
  )

  useEffect(() => {
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)')
    const handleChange = (e: MediaQueryListEvent) => setSystemPrefersDark(e.matches)
    mediaQuery.addEventListener('change', handleChange)
    return () => mediaQuery.removeEventListener('change', handleChange)
  }, [])

  const setThemeMode = (mode: ThemeMode) => {
    setThemeModeInternal(mode)
    onModeChange?.(mode)
  }

  // `initialMode` only seeds the state: the provider mounts after settings have
  // loaded (App gates on settingsLoaded), and later changes to the persisted
  // theme flow through setThemeMode at the Save handler — no prop-follow needed.
  const isDark = themeMode === 'dark' || (themeMode === 'auto' && systemPrefersDark)

  useEffect(() => { applyThemeClass(isDark) }, [isDark])

  const value: ThemeContextType = { themeMode, setThemeMode, isDark }

  return (
    <ThemeContext.Provider value={value}>
      {children}
    </ThemeContext.Provider>
  )
}
