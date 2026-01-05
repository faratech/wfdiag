import {
  createDarkTheme,
  createLightTheme,
  Theme,
  BrandVariants,
} from '@fluentui/react-components'

// Brand colors based on AI Dev Gallery gradient palette
// Gradient: #7cb6e9 → #335fe3 → #8e5df5 → #ee9bbf
const wfBrand: BrandVariants = {
  10: '#050a14',
  20: '#0d1a33',
  30: '#142952',
  40: '#1a3a71',
  50: '#214b90',
  60: '#335fe3',   // Primary blue from gradient
  70: '#5077e8',   // Hover state
  80: '#6d8fed',   // Active
  90: '#8aa7f2',
  100: '#a7bff7',
  110: '#c4d7fc',
  120: '#d8e5fd',
  130: '#e9effd',
  140: '#f4f7fe',
  150: '#fafbff',
  160: '#ffffff',
}

// Create themes with custom overrides for polish
export const wfDarkTheme: Theme = {
  ...createDarkTheme(wfBrand),
  // Slightly lighter backgrounds for better contrast
  colorNeutralBackground1: '#1a1d24',
  colorNeutralBackground2: '#141720',
  colorNeutralBackground3: '#0f1218',
  colorNeutralBackground4: '#0a0c10',
}

export const wfLightTheme: Theme = {
  ...createLightTheme(wfBrand),
  // Clean white backgrounds with subtle blue tint
  colorNeutralBackground1: '#ffffff',
  colorNeutralBackground2: '#f6f8fb',
  colorNeutralBackground3: '#eef1f6',
  colorNeutralBackground4: '#e4e8f0',
  // Strong text contrast for readability
  colorNeutralForeground1: '#11141a',
  colorNeutralForeground2: '#2d3748',
  colorNeutralForeground3: '#4a5568',
  // More visible strokes for card definition
  colorNeutralStroke1: '#c4cad6',
  colorNeutralStroke2: '#d8dde6',
  // Stronger shadows for light mode
  shadow4: '0 2px 8px rgba(0, 0, 0, 0.08), 0 1px 2px rgba(0, 0, 0, 0.06)',
  shadow8: '0 4px 16px rgba(0, 0, 0, 0.10), 0 2px 4px rgba(0, 0, 0, 0.06)',
  shadow16: '0 8px 24px rgba(0, 0, 0, 0.12), 0 4px 8px rgba(0, 0, 0, 0.06)',
}

// AI Dev Gallery-style accent gradient for special elements
export const AI_ACCENT_GRADIENT = 'linear-gradient(135deg, #7cb6e9 0%, #335fe3 35%, #8e5df5 70%, #ee9bbf 100%)'
export const AI_ACCENT_GRADIENT_HOVER = 'linear-gradient(135deg, #8ec4ed 0%, #4570e8 35%, #9d70f7 70%, #f2aecb 100%)'

// Theme-specific card gradients (like AI Dev Gallery)
export const THEME_GRADIENTS = {
  dark: {
    // Subtle blue-purple overlay for cards
    card: 'linear-gradient(145deg, rgba(200, 174, 196, 0.08) 0%, rgba(50, 134, 238, 0.08) 100%)',
    cardHover: 'linear-gradient(145deg, rgba(200, 174, 196, 0.14) 0%, rgba(50, 134, 238, 0.14) 100%)',
    // Hero/feature card gradient
    hero: 'linear-gradient(135deg, rgba(50, 134, 238, 0.15) 0%, rgba(200, 174, 196, 0.1) 50%, transparent 100%)',
    // Sidebar gradient
    sidebar: 'linear-gradient(180deg, rgba(20, 23, 32, 0.98) 0%, rgba(15, 18, 24, 0.95) 100%)',
  },
  light: {
    // Clean white cards with very subtle blue tint - like AI Dev Gallery
    card: '#ffffff',
    cardHover: '#f8faff',
    // Hero/feature card gradient - subtle blue accent
    hero: 'linear-gradient(135deg, rgba(51, 95, 227, 0.06) 0%, rgba(142, 93, 245, 0.04) 50%, rgba(238, 155, 191, 0.02) 100%)',
    // Sidebar - clean light gray
    sidebar: '#f6f8fb',
  }
}

// Semantic status colors
export const STATUS_COLORS = {
  success: {
    background: 'var(--colorPaletteGreenBackground2)',
    foreground: 'var(--colorPaletteGreenForeground2)',
    border: 'var(--colorPaletteGreenBorder2)',
  },
  warning: {
    background: 'var(--colorPaletteYellowBackground2)',
    foreground: 'var(--colorPaletteYellowForeground2)',
    border: 'var(--colorPaletteYellowBorder2)',
  },
  error: {
    background: 'var(--colorPaletteRedBackground2)',
    foreground: 'var(--colorPaletteRedForeground2)',
    border: 'var(--colorPaletteRedBorder2)',
  },
  info: {
    background: 'var(--colorPaletteBlueBackground2)',
    foreground: 'var(--colorPaletteBlueForeground2)',
    border: 'var(--colorPaletteBlueForeground2)',
  },
}

// Layout constants for NavRail and page structure
export const NAV_RAIL_WIDTH_COLLAPSED = 72
export const NAV_RAIL_WIDTH_EXPANDED = 240
export const PAGE_HEADER_HEIGHT = 64
export const CONTENT_MAX_WIDTH = 1400
export const NAV_RAIL_BREAKPOINT = 1024 // Auto-collapse below this width

// NavRail gradient backgrounds
export const NAV_RAIL_GRADIENT = {
  dark: 'linear-gradient(180deg, #1a1d24 0%, #141720 100%)',
  light: 'linear-gradient(180deg, #ffffff 0%, #f6f8fb 100%)',
}

// egui-compatible color scheme (matches src-tauri/src/ui/colors.rs)
export const EGUI_COLORS = {
  // Status colors
  SUCCESS: 'rgb(80, 180, 80)',
  ERROR: 'rgb(220, 80, 80)',
  WARNING: 'rgb(245, 158, 11)',
  INFO: 'rgb(59, 130, 246)',
  PRIMARY: 'rgb(99, 102, 241)',
  LINK: 'rgb(96, 165, 250)',

  // JSON value colors
  NUMBER: 'rgb(100, 149, 237)',
  BOOLEAN_TRUE: 'rgb(80, 180, 80)',
  BOOLEAN_FALSE: 'rgb(180, 100, 100)',

  // Status badge backgrounds (20% opacity)
  SUCCESS_BG: 'rgba(80, 180, 80, 0.2)',
  ERROR_BG: 'rgba(220, 80, 80, 0.2)',
  WARNING_BG: 'rgba(245, 158, 11, 0.2)',
  INFO_BG: 'rgba(59, 130, 246, 0.2)',

  // Selection backgrounds
  SELECTED_DARK: 'rgba(59, 130, 246, 0.31)',
  SELECTED_LIGHT: 'rgba(59, 130, 246, 0.16)',
  HOVER_DARK: 'rgba(128, 128, 128, 0.12)',
  HOVER_LIGHT: 'rgba(0, 0, 0, 0.06)',

  // AI panel
  AI_PANEL_BG: 'rgba(99, 102, 241, 0.08)',
  AI_PANEL_STROKE: 'rgba(99, 102, 241, 0.2)',
}

// Category icons matching egui nav.rs
export const CATEGORY_ICONS: Record<string, string> = {
  Hardware: '\u{1F5A5}\uFE0F', // Monitor
  Storage: '\u{1F4BE}',       // Floppy disk
  Network: '\u{1F310}',       // Globe
  System: '\u2699\uFE0F',     // Gear
}

// Task-specific icons matching diagnostics.rs get_task_icon()
export const TASK_ICONS: Record<string, string> = {
  processor: '\u{1F532}',
  physical_memory: '\u{1F4CA}',
  device_memory: '\u{1F4CA}',
  baseboard: '\u{1F5A5}\uFE0F',
  bios: '\u26A1',
  npu_info: '\u{1F9E0}',
  disk_drive: '\u{1F4BF}',
  disk_partition: '\u{1F4C1}',
  logical_disk: '\u{1F4BE}',
  network_adapter: '\u{1F4E1}',
  ipconfig: '\u{1F310}',
  services: '\u2699\uFE0F',
  system_driver: '\u{1F527}',
  drivers_list: '\u{1F527}',
  comp_system: '\u{1F4BB}',
  os_info: '\u{1F4BB}',
  systeminfo: '\u{1F4BB}',
  event_logs: '\u{1F4CB}',
  installed_programs: '\u{1F4E6}',
  store_apps: '\u{1F4E6}',
  processes: '\u{1F4C8}',
  windows_update: '\u{1F504}',
}

// Navigation icons for NavRail (egui style)
export const NAV_ICONS: Record<string, string> = {
  diagnostics: '\u{1F50D}', // Magnifying glass
  monitoring: '\u{1F4CA}',  // Chart
  ai: '\u2728',             // Sparkles
  issues: '\u26A0\uFE0F',   // Warning
  history: '\u{1F4DC}',     // Scroll
}

// Spacing constants matching egui components.rs
export const SPACING = {
  SM: 8,
  MD: 12,
  LG: 16,
  XL: 24,
}

// List panel dimensions
export const LIST_PANEL_WIDTH = 260
export const LIST_ITEM_HEIGHT = 32
export const STATUS_DOT_SIZE = 8
