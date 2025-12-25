import React, { createContext, useContext, useState, useEffect, ReactNode } from 'react'
import { FluentProvider, Theme } from '@fluentui/react-components'
import { wfDarkTheme, wfLightTheme, THEME_GRADIENTS, AI_ACCENT_GRADIENT } from '../theme'

type ThemeMode = 'dark' | 'light' | 'auto'

interface ThemeContextType {
  themeMode: ThemeMode
  setThemeMode: (mode: ThemeMode) => void
  currentTheme: Theme
  isDark: boolean
}

const ThemeContext = createContext<ThemeContextType | undefined>(undefined)

// Sync Fluent tokens to CSS custom properties for use in CSS files
const syncCSSCustomProperties = (theme: Theme, isDark: boolean) => {
  const root = document.documentElement

  // Set data-theme attribute for CSS selectors
  root.setAttribute('data-theme', isDark ? 'dark' : 'light')

  // Map key Fluent tokens to CSS custom properties
  // These allow CSS files to use theme-aware colors
  const cssVars: Record<string, string> = {
    // Backgrounds
    '--theme-background': theme.colorNeutralBackground1 ?? '',
    '--theme-background-2': theme.colorNeutralBackground2 ?? '',
    '--theme-background-3': theme.colorNeutralBackground3 ?? '',
    '--theme-background-4': theme.colorNeutralBackground4 ?? '',
    '--theme-background-hover': theme.colorNeutralBackground1Hover ?? '',
    '--theme-background-pressed': theme.colorNeutralBackground1Pressed ?? '',

    // Foregrounds / Text
    '--theme-foreground': theme.colorNeutralForeground1 ?? '',
    '--theme-foreground-2': theme.colorNeutralForeground2 ?? '',
    '--theme-foreground-3': theme.colorNeutralForeground3 ?? '',
    '--theme-foreground-disabled': theme.colorNeutralForegroundDisabled ?? '',

    // Borders / Strokes
    '--theme-stroke': theme.colorNeutralStroke1 ?? '',
    '--theme-stroke-2': theme.colorNeutralStroke2 ?? '',
    '--theme-stroke-hover': theme.colorNeutralStroke1Hover ?? '',

    // Brand colors
    '--theme-brand-background': theme.colorBrandBackground ?? '',
    '--theme-brand-background-hover': theme.colorBrandBackgroundHover ?? '',
    '--theme-brand-foreground': theme.colorBrandForeground1 ?? '',

    // Subtle backgrounds (for hover states)
    '--theme-subtle-background': theme.colorSubtleBackground ?? '',
    '--theme-subtle-background-hover': theme.colorSubtleBackgroundHover ?? '',
    '--theme-subtle-background-pressed': theme.colorSubtleBackgroundPressed ?? '',

    // Shadows
    '--theme-shadow-2': theme.shadow2 ?? '',
    '--theme-shadow-4': theme.shadow4 ?? '',
    '--theme-shadow-8': theme.shadow8 ?? '',
    '--theme-shadow-16': theme.shadow16 ?? '',

    // Border radius
    '--theme-radius-small': theme.borderRadiusSmall ?? '',
    '--theme-radius-medium': theme.borderRadiusMedium ?? '',
    '--theme-radius-large': theme.borderRadiusLarge ?? '',
    '--theme-radius-xlarge': theme.borderRadiusXLarge ?? '',

    // Spacing
    '--theme-spacing-xs': theme.spacingHorizontalXS ?? '',
    '--theme-spacing-s': theme.spacingHorizontalS ?? '',
    '--theme-spacing-m': theme.spacingHorizontalM ?? '',
    '--theme-spacing-l': theme.spacingHorizontalL ?? '',
    '--theme-spacing-xl': theme.spacingHorizontalXL ?? '',
  }

  // Apply all CSS custom properties
  Object.entries(cssVars).forEach(([property, value]) => {
    if (value) {
      root.style.setProperty(property, value)
    }
  })

  // Apply theme-specific gradients (key for polish)
  const gradients = isDark ? THEME_GRADIENTS.dark : THEME_GRADIENTS.light
  root.style.setProperty('--theme-card-gradient', gradients.card)
  root.style.setProperty('--theme-card-gradient-hover', gradients.cardHover)
  root.style.setProperty('--theme-hero-gradient', gradients.hero)
  root.style.setProperty('--theme-sidebar-gradient', gradients.sidebar)
  root.style.setProperty('--theme-accent-gradient', AI_ACCENT_GRADIENT)
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

  // Sync CSS custom properties whenever theme changes
  useEffect(() => {
    syncCSSCustomProperties(currentTheme, isDark)
  }, [currentTheme, isDark])

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
