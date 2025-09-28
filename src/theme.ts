import {
  createDarkTheme,
  createLightTheme,
  Theme,
  BrandVariants,
  tokens,
  makeStyles,
  shorthands
} from '@fluentui/react-components'

// WF Diagnostics brand colors
const wfDiagnosticsBrand: BrandVariants = {
  10: '#020305',
  20: '#0A1929',
  30: '#0F2642',
  40: '#143356',
  50: '#1A416B',
  60: '#1F4F80',
  70: '#2563EB', // Primary brand color
  80: '#3B82F6',
  90: '#60A5FA',
  100: '#93C5FD',
  110: '#BFDBFE',
  120: '#DBEAFE',
  130: '#EFF6FF',
  140: '#F0F9FF',
  150: '#F7FBFF',
  160: '#FAFCFF',
}

// Create custom dark theme extending Fluent UI's dark theme
export const wfDarkTheme: Theme = {
  ...createDarkTheme(wfDiagnosticsBrand),
  // Custom color overrides for specific UI elements
  colorNeutralBackground1: '#0F172A', // Main background
  colorNeutralBackground2: '#1E293B', // Card backgrounds
  colorNeutralBackground3: '#334155', // Elevated surfaces
  colorNeutralStroke1: 'rgba(255, 255, 255, 0.1)',
  colorNeutralStroke2: 'rgba(255, 255, 255, 0.05)',
}

// Light theme (for future use)
export const wfLightTheme: Theme = createLightTheme(wfDiagnosticsBrand)

// Shared styles using Fluent UI's makeStyles
export const useAppStyles = makeStyles({
  // Glass morphism effect for cards
  glassCard: {
    backgroundColor: 'rgba(30, 41, 59, 0.8)',
    backdropFilter: 'blur(10px)',
    ...shorthands.border('1px', 'solid', 'rgba(255, 255, 255, 0.1)'),
    ...shorthands.borderRadius(tokens.borderRadiusLarge),
    ...shorthands.padding(tokens.spacingVerticalL, tokens.spacingHorizontalL),
    boxShadow: tokens.shadow16,
    transition: 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)',

    ':hover': {
      backgroundColor: 'rgba(30, 41, 59, 0.9)',
      boxShadow: tokens.shadow28,
    }
  },

  // Gradient backgrounds for CTAs
  gradientButton: {
    background: 'linear-gradient(135deg, #3B82F6 0%, #8B5CF6 100%)',
    ...shorthands.border('none'),
    color: tokens.colorNeutralForegroundOnBrand,
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalL),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    fontWeight: tokens.fontWeightSemibold,
    cursor: 'pointer',
    transition: 'all 0.2s ease',

    ':hover': {
      transform: 'translateY(-2px)',
      boxShadow: '0 8px 24px rgba(59, 130, 246, 0.4)',
    },

    ':active': {
      transform: 'translateY(0)',
    }
  },

  // Status badges
  statusBadge: {
    display: 'inline-flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
    ...shorthands.padding(tokens.spacingVerticalXS, tokens.spacingHorizontalS),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    fontSize: tokens.fontSizeBase200,
    fontWeight: tokens.fontWeightMedium,
  },

  statusSuccess: {
    backgroundColor: 'rgba(16, 185, 129, 0.2)',
    color: '#10B981',
    ...shorthands.border('1px', 'solid', 'rgba(16, 185, 129, 0.3)'),
  },

  statusWarning: {
    backgroundColor: 'rgba(245, 158, 11, 0.2)',
    color: '#F59E0B',
    ...shorthands.border('1px', 'solid', 'rgba(245, 158, 11, 0.3)'),
  },

  statusError: {
    backgroundColor: 'rgba(239, 68, 68, 0.2)',
    color: '#EF4444',
    ...shorthands.border('1px', 'solid', 'rgba(239, 68, 68, 0.3)'),
  },

  statusInfo: {
    backgroundColor: 'rgba(59, 130, 246, 0.2)',
    color: '#3B82F6',
    ...shorthands.border('1px', 'solid', 'rgba(59, 130, 246, 0.3)'),
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

  // Scrollable container with custom scrollbar
  scrollableContainer: {
    overflowY: 'auto',
    overflowX: 'hidden',

    '::-webkit-scrollbar': {
      width: '8px',
    },

    '::-webkit-scrollbar-track': {
      backgroundColor: 'rgba(255, 255, 255, 0.05)',
      ...shorthands.borderRadius(tokens.borderRadiusSmall),
    },

    '::-webkit-scrollbar-thumb': {
      backgroundColor: 'rgba(255, 255, 255, 0.2)',
      ...shorthands.borderRadius(tokens.borderRadiusSmall),

      ':hover': {
        backgroundColor: 'rgba(255, 255, 255, 0.3)',
      }
    }
  },

  // Modern progress bar
  progressContainer: {
    width: '100%',
    height: '8px',
    backgroundColor: 'rgba(255, 255, 255, 0.1)',
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    ...shorthands.overflow('hidden'),
  },

  progressBar: {
    height: '100%',
    background: 'linear-gradient(90deg, #3B82F6 0%, #8B5CF6 100%)',
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    transition: 'width 0.3s ease',
    position: 'relative',

    '&::after': {
      content: '""',
      position: 'absolute',
      top: '0',
      left: '0',
      right: '0',
      bottom: '0',
      background: 'linear-gradient(90deg, transparent 0%, rgba(255, 255, 255, 0.3) 50%, transparent 100%)',
      animation: 'shimmer 2s infinite',
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

// Animation keyframes
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
    }
    100% {
      transform: translateX(200%);
    }
  }

  @keyframes fadeIn {
    from {
      opacity: 0;
      transform: translateY(10px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @keyframes pulse {
    0%, 100% {
      opacity: 1;
    }
    50% {
      opacity: 0.5;
    }
  }
`