import React, { createContext, useContext, useState, useEffect, ReactNode } from 'react'
import { FluentProvider, Theme } from '@fluentui/react-components'
import { wfDarkTheme, wfLightTheme } from '../theme'

type ThemeMode = 'dark' | 'light' | 'auto'

interface ThemeContextType {
  themeMode: ThemeMode
  setThemeMode: (mode: ThemeMode) => void
  currentTheme: Theme
  isDark: boolean
}

const ThemeContext = createContext<ThemeContextType | undefined>(undefined)

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
  onModeChange
}) => {
  const [themeMode, setThemeModeInternal] = useState<ThemeMode>(initialMode)
  const [systemPrefersDark, setSystemPrefersDark] = useState(() =>
    window.matchMedia('(prefers-color-scheme: dark)').matches
  )

  // Listen for system theme changes
  useEffect(() => {
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)')

    const handleChange = (e: MediaQueryListEvent) => {
      setSystemPrefersDark(e.matches)
    }

    // Add listener for live updates
    mediaQuery.addEventListener('change', handleChange)

    return () => {
      mediaQuery.removeEventListener('change', handleChange)
    }
  }, [])

  // Update theme mode and notify parent
  const setThemeMode = (mode: ThemeMode) => {
    setThemeModeInternal(mode)
    onModeChange?.(mode)
  }

  // Sync with external mode changes (from settings)
  useEffect(() => {
    if (initialMode !== themeMode) {
      setThemeModeInternal(initialMode)
    }
  }, [initialMode])

  // Determine actual theme based on mode
  const isDark = themeMode === 'dark' || (themeMode === 'auto' && systemPrefersDark)
  const currentTheme = isDark ? wfDarkTheme : wfLightTheme

  const value: ThemeContextType = {
    themeMode,
    setThemeMode,
    currentTheme,
    isDark
  }

  return (
    <ThemeContext.Provider value={value}>
      <FluentProvider theme={currentTheme}>
        {children}
      </FluentProvider>
    </ThemeContext.Provider>
  )
}
