import {
  createDarkTheme,
  createLightTheme,
  Theme,
  BrandVariants,
  tokens,
  makeStyles,
  shorthands
} from '@fluentui/react-components'

// WF Diagnostics optimized brand colors
const wfDiagnosticsBrand: BrandVariants = {
  10: '#030712',   // Darkest
  20: '#0F1629',   // Deep navy
  30: '#1A2341',   // Dark navy
  40: '#243054',   // Navy
  50: '#2E3D67',   // Medium navy
  60: '#384B7A',   // Light navy
  70: '#4F69FF',   // Primary brand (vibrant blue)
  80: '#6B7FFF',   // Light primary
  90: '#8B9EFF',   // Lighter primary
  100: '#A8B7FF',  // Pastel primary
  110: '#C3CEFF',  // Very light primary
  120: '#DCE2FF',  // Ultra light primary
  130: '#EEF1FF',  // Near white blue
  140: '#F5F7FF',  // Off white
  150: '#FAFBFF',  // Almost white
  160: '#FFFFFF',  // Pure white
}

// Create custom dark theme with enterprise-grade colors
export const wfDarkTheme: Theme = {
  ...createDarkTheme(wfDiagnosticsBrand),
  // Premium enterprise color palette
  colorNeutralBackground1: '#0B0E19', // Deep, rich background
  colorNeutralBackground2: '#12161F', // Card backgrounds with subtle elevation
  colorNeutralBackground3: '#1A1F2E', // Elevated elements
  colorNeutralBackground4: '#242936', // Hover backgrounds
  colorNeutralBackground5: '#2D3342', // Active backgrounds
  colorNeutralBackground6: '#363C4F', // Selected items

  // Modern borders with subtle glow
  colorNeutralStroke1: 'rgba(99, 115, 255, 0.12)', // Subtle borders with brand accent
  colorNeutralStroke2: 'rgba(99, 115, 255, 0.06)', // Faint dividers
  colorNeutralStroke3: 'rgba(99, 115, 255, 0.20)', // Focus borders with glow

  // Refined text hierarchy
  colorNeutralForeground1: '#F5F7FA', // Primary text - crisp white
  colorNeutralForeground2: '#E1E4EA', // Secondary text
  colorNeutralForeground3: '#A8B0BF', // Muted text
  colorNeutralForeground4: '#6B7280', // Disabled text

  // Brand colors with better contrast
  colorBrandForeground1: '#6B7FFF', // Brand text
  colorBrandForeground2: '#8B9EFF', // Brand text hover
  colorBrandBackground: '#5B6EFF', // Primary brand - more vibrant
  colorBrandBackgroundHover: '#7088FF', // Brand hover with lift
  colorBrandBackgroundPressed: '#4A5DCC', // Brand pressed

  // Enhanced status colors
  colorPaletteGreenBackground3: '#10B981', // Success
  colorPaletteYellowBackground3: '#F59E0B', // Warning
  colorPaletteRedBackground3: '#EF4444', // Error
}

// Light theme (for future use)
export const wfLightTheme: Theme = createLightTheme(wfDiagnosticsBrand)

// Shared styles using Fluent UI's makeStyles
export const useAppStyles = makeStyles({
  // Premium card design with refined depth and polish
  glassCard: {
    background: 'linear-gradient(145deg, rgba(18, 22, 31, 0.95), rgba(26, 31, 46, 0.90))',
    backdropFilter: 'blur(32px) saturate(180%)',
    ...shorthands.border('1px', 'solid', 'rgba(99, 115, 255, 0.10)'),
    ...shorthands.borderRadius('12px'),
    ...shorthands.padding('20px', '24px'),
    boxShadow: `
      0 0 0 1px rgba(99, 115, 255, 0.05),
      0 2px 8px rgba(0, 0, 0, 0.15),
      0 4px 16px rgba(0, 0, 0, 0.10),
      inset 0 1px 0 rgba(255, 255, 255, 0.03)
    `,
    transition: 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)',
    position: 'relative',
    ...shorthands.overflow('hidden'),

    '&::before': {
      content: '""',
      position: 'absolute',
      top: 0,
      left: 0,
      right: 0,
      height: '1px',
      background: 'linear-gradient(90deg, transparent, rgba(99, 115, 255, 0.15), transparent)',
      opacity: 0.8,
    },

    ':hover': {
      ...shorthands.border('1px', 'solid', 'rgba(99, 115, 255, 0.15)'),
      boxShadow: `
        0 0 0 1px rgba(99, 115, 255, 0.08),
        0 4px 12px rgba(0, 0, 0, 0.20),
        0 8px 24px rgba(0, 0, 0, 0.15),
        inset 0 1px 0 rgba(255, 255, 255, 0.05)
      `,
      transform: 'translateY(-2px)',
    }
  },

  // Modern enterprise button design
  gradientButton: {
    background: `linear-gradient(135deg, ${tokens.colorBrandBackground} 0%, #6B7FFF 100%)`,
    ...shorthands.border('1px', 'solid', 'rgba(107, 127, 255, 0.2)'),
    color: '#FFFFFF',
    ...shorthands.padding('10px', '20px'),
    ...shorthands.borderRadius('8px'),
    fontWeight: tokens.fontWeightSemibold,
    fontSize: '14px',
    cursor: 'pointer',
    boxShadow: `
      0 0 0 1px rgba(107, 127, 255, 0.1),
      0 2px 8px rgba(91, 110, 255, 0.25),
      inset 0 1px 0 rgba(255, 255, 255, 0.1)
    `,
    transition: 'all 0.2s cubic-bezier(0.4, 0, 0.2, 1)',
    position: 'relative',
    ...shorthands.overflow('hidden'),

    '&::before': {
      content: '""',
      position: 'absolute',
      top: 0,
      left: '-100%',
      width: '100%',
      height: '100%',
      background: 'linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.1), transparent)',
      transition: 'left 0.5s',
    },

    ':hover': {
      background: `linear-gradient(135deg, ${tokens.colorBrandBackgroundHover} 0%, #7B8FFF 100%)`,
      ...shorthands.border('1px', 'solid', 'rgba(107, 127, 255, 0.3)'),
      boxShadow: `
        0 0 0 1px rgba(107, 127, 255, 0.15),
        0 4px 12px rgba(91, 110, 255, 0.35),
        inset 0 1px 0 rgba(255, 255, 255, 0.15)
      `,
      transform: 'translateY(-2px)',

      '&::before': {
        left: '100%',
      }
    },

    ':active': {
      transform: 'translateY(0)',
      boxShadow: `
        0 0 0 1px rgba(107, 127, 255, 0.15),
        0 2px 6px rgba(91, 110, 255, 0.30),
        inset 0 1px 2px rgba(0, 0, 0, 0.1)
      `,
    }
  },

  // Refined status badges with glow effects
  statusBadge: {
    display: 'inline-flex',
    alignItems: 'center',
    ...shorthands.gap('6px'),
    ...shorthands.padding('6px', '12px'),
    ...shorthands.borderRadius('6px'),
    fontSize: '13px',
    fontWeight: tokens.fontWeightSemibold,
    letterSpacing: '0.01em',
    transition: 'all 0.2s ease',
  },

  statusSuccess: {
    backgroundColor: 'rgba(16, 185, 129, 0.15)',
    color: '#34D399',
    ...shorthands.border('1px', 'solid', 'rgba(16, 185, 129, 0.25)'),
    boxShadow: '0 0 12px rgba(16, 185, 129, 0.15), inset 0 1px 0 rgba(255, 255, 255, 0.05)',
  },

  statusWarning: {
    backgroundColor: 'rgba(245, 158, 11, 0.15)',
    color: '#FBB040',
    ...shorthands.border('1px', 'solid', 'rgba(245, 158, 11, 0.25)'),
    boxShadow: '0 0 12px rgba(245, 158, 11, 0.15), inset 0 1px 0 rgba(255, 255, 255, 0.05)',
  },

  statusError: {
    backgroundColor: 'rgba(239, 68, 68, 0.15)',
    color: '#F87171',
    ...shorthands.border('1px', 'solid', 'rgba(239, 68, 68, 0.25)'),
    boxShadow: '0 0 12px rgba(239, 68, 68, 0.15), inset 0 1px 0 rgba(255, 255, 255, 0.05)',
  },

  statusInfo: {
    backgroundColor: 'rgba(59, 130, 246, 0.15)',
    color: '#60A5FA',
    ...shorthands.border('1px', 'solid', 'rgba(59, 130, 246, 0.25)'),
    boxShadow: '0 0 12px rgba(59, 130, 246, 0.15), inset 0 1px 0 rgba(255, 255, 255, 0.05)',
  },

  // Health score circle
  healthScoreCircle: {
    width: '80px',
    height: '80px',
    ...shorthands.borderRadius('50%'),
    position: 'relative',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    background: `conic-gradient(
      from 0deg,
      ${tokens.colorPaletteGreenBackground1} 0deg,
      ${tokens.colorPaletteGreenBackground1} var(--score-angle),
      ${tokens.colorNeutralBackground3} var(--score-angle)
    )`,

    '&::before': {
      content: '""',
      position: 'absolute',
      width: '60px',
      height: '60px',
      ...shorthands.borderRadius('50%'),
      backgroundColor: tokens.colorNeutralBackground2,
    }
  },

  // Refined scrollable container with elegant scrollbar
  scrollableContainer: {
    overflowY: 'auto',
    overflowX: 'hidden',

    '::-webkit-scrollbar': {
      width: '10px',
    },

    '::-webkit-scrollbar-track': {
      backgroundColor: 'rgba(26, 31, 46, 0.4)',
      ...shorthands.border('1px', 'solid', 'rgba(99, 115, 255, 0.08)'),
      ...shorthands.borderRadius('6px'),
      margin: '4px',
    },

    '::-webkit-scrollbar-thumb': {
      backgroundColor: 'rgba(99, 115, 255, 0.3)',
      ...shorthands.borderRadius('6px'),
      ...shorthands.border('1px', 'solid', 'rgba(99, 115, 255, 0.2)'),
      boxShadow: 'inset 0 1px 0 rgba(255, 255, 255, 0.1)',
      transition: 'background-color 0.2s ease',

      ':hover': {
        backgroundColor: 'rgba(99, 115, 255, 0.5)',
        boxShadow: `
          inset 0 1px 0 rgba(255, 255, 255, 0.15),
          0 0 8px rgba(99, 115, 255, 0.3)
        `,
      },

      ':active': {
        backgroundColor: 'rgba(99, 115, 255, 0.6)',
      }
    }
  },

  // Elegant modern progress bar with glow
  progressContainer: {
    width: '100%',
    height: '10px',
    backgroundColor: 'rgba(26, 31, 46, 0.6)',
    ...shorthands.border('1px', 'solid', 'rgba(99, 115, 255, 0.10)'),
    ...shorthands.borderRadius('8px'),
    ...shorthands.overflow('hidden'),
    boxShadow: 'inset 0 1px 3px rgba(0, 0, 0, 0.3)',
    position: 'relative',
  },

  progressBar: {
    height: '100%',
    background: 'linear-gradient(90deg, #5B6EFF 0%, #7B8FFF 50%, #8B9EFF 100%)',
    ...shorthands.borderRadius('8px'),
    boxShadow: `
      0 0 16px rgba(91, 110, 255, 0.5),
      inset 0 1px 0 rgba(255, 255, 255, 0.2)
    `,
    transition: 'width 0.4s cubic-bezier(0.4, 0, 0.2, 1)',
    position: 'relative',

    '&::before': {
      content: '""',
      position: 'absolute',
      top: 0,
      left: 0,
      right: 0,
      height: '50%',
      background: 'linear-gradient(180deg, rgba(255, 255, 255, 0.2) 0%, transparent 100%)',
      borderTopLeftRadius: '8px',
      borderTopRightRadius: '8px',
    },

    '&::after': {
      content: '""',
      position: 'absolute',
      top: 0,
      left: '-100%',
      width: '100%',
      height: '100%',
      background: 'linear-gradient(90deg, transparent 0%, rgba(255, 255, 255, 0.3) 50%, transparent 100%)',
      animation: 'shimmer 2s ease-in-out infinite',
    }
  },

  // Collapsible section
  collapsibleSection: {
    marginTop: tokens.spacingVerticalM,

    '& .collapsible-header': {
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'space-between',
      cursor: 'pointer',
      ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
      ...shorthands.borderRadius(tokens.borderRadiusMedium),
      backgroundColor: 'rgba(30, 41, 59, 0.3)',
      transition: 'all 0.2s ease',

      ':hover': {
        backgroundColor: 'rgba(30, 41, 59, 0.5)',
      }
    },

    '& .collapsible-content': {
      ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalM),
      marginTop: tokens.spacingVerticalS,
    }
  },

  // Category icon
  categoryIcon: {
    width: '48px',
    height: '48px',
    ...shorthands.borderRadius(tokens.borderRadiusCircular),
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    color: tokens.colorNeutralBackground1,
    fontSize: '24px',
    flexShrink: 0,
  },

  // Result grid
  resultGrid: {
    display: 'grid',
    gridTemplateColumns: 'minmax(150px, 1fr) 2fr',
    ...shorthands.gap(tokens.spacingHorizontalM),
    alignItems: 'start',
    ...shorthands.padding(tokens.spacingVerticalXS, '0'),

    '& .result-label': {
      color: tokens.colorNeutralForeground3,
      fontSize: tokens.fontSizeBase200,
      display: 'flex',
      alignItems: 'center',
      ...shorthands.gap(tokens.spacingHorizontalXS),
    },

    '& .result-value': {
      color: tokens.colorNeutralForeground1,
      fontSize: tokens.fontSizeBase300,
      wordBreak: 'break-word',
    }
  },

  // Scan animation
  scanAnimation: {
    '& .icon-spin': {
      animation: 'spin 2s linear infinite',
    }
  },

  // Tooltip
  tooltip: {
    position: 'relative',

    '& .tooltip-text': {
      visibility: 'hidden',
      opacity: '0',
      position: 'absolute',
      bottom: '125%',
      left: '50%',
      transform: 'translateX(-50%)',
      backgroundColor: tokens.colorNeutralBackground1,
      color: tokens.colorNeutralForeground1,
      ...shorthands.padding(tokens.spacingVerticalXS, tokens.spacingHorizontalS),
      ...shorthands.borderRadius(tokens.borderRadiusSmall),
      fontSize: tokens.fontSizeBase200,
      whiteSpace: 'nowrap',
      zIndex: '1000',
      boxShadow: tokens.shadow16,
      transition: 'opacity 0.3s',
    },

    ':hover .tooltip-text': {
      visibility: 'visible',
      opacity: '1',
    }
  }
})

// Export convenience functions for common patterns
export const getStatusStyles = (status: 'success' | 'warning' | 'error' | 'info') => {
  const styles = useAppStyles()
  const statusMap = {
    success: styles.statusSuccess,
    warning: styles.statusWarning,
    error: styles.statusError,
    info: styles.statusInfo,
  }
  return `${styles.statusBadge} ${statusMap[status]}`
}

// Modern animation keyframes with refined easing
export const animations = `
  @keyframes spin {
    from {
      transform: rotate(0deg);
    }
    to {
      transform: rotate(360deg);
    }
  }

  @keyframes shimmer {
    0% {
      transform: translateX(-100%);
      opacity: 0;
    }
    50% {
      opacity: 1;
    }
    100% {
      transform: translateX(200%);
      opacity: 0;
    }
  }

  @keyframes fadeIn {
    0% {
      opacity: 0;
      transform: translateY(12px);
    }
    100% {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @keyframes fadeInScale {
    0% {
      opacity: 0;
      transform: scale(0.95) translateY(8px);
    }
    100% {
      opacity: 1;
      transform: scale(1) translateY(0);
    }
  }

  @keyframes slideInRight {
    0% {
      opacity: 0;
      transform: translateX(20px);
    }
    100% {
      opacity: 1;
      transform: translateX(0);
    }
  }

  @keyframes pulse {
    0%, 100% {
      opacity: 1;
      transform: scale(1);
    }
    50% {
      opacity: 0.7;
      transform: scale(0.98);
    }
  }

  @keyframes glow {
    0%, 100% {
      box-shadow: 0 0 8px rgba(99, 115, 255, 0.3);
    }
    50% {
      box-shadow: 0 0 16px rgba(99, 115, 255, 0.6);
    }
  }

  @keyframes ripple {
    0% {
      transform: scale(0);
      opacity: 0.6;
    }
    100% {
      transform: scale(2);
      opacity: 0;
    }
  }
`