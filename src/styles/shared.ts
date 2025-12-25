import { makeStyles, tokens, shorthands } from '@fluentui/react-components'
import { AI_ACCENT_GRADIENT, AI_ACCENT_GRADIENT_HOVER } from '../theme'

/**
 * Card styles - theme-aware backgrounds with proper elevation
 */
export const useCardStyles = makeStyles({
  // Standard card with theme-aware background
  card: {
    backgroundColor: tokens.colorNeutralBackground1,
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke1),
    ...shorthands.borderRadius(tokens.borderRadiusLarge),
    boxShadow: tokens.shadow4,
    ...shorthands.padding(tokens.spacingVerticalL, tokens.spacingHorizontalL),
    transitionProperty: 'box-shadow, transform',
    transitionDuration: tokens.durationNormal,
    transitionTimingFunction: tokens.curveEasyEase,
  },

  // Elevated card (for hover states or emphasis)
  cardElevated: {
    backgroundColor: tokens.colorNeutralBackground1,
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke1),
    ...shorthands.borderRadius(tokens.borderRadiusLarge),
    boxShadow: tokens.shadow8,
    ...shorthands.padding(tokens.spacingVerticalL, tokens.spacingHorizontalL),
  },

  // Subtle card (secondary emphasis)
  cardSubtle: {
    backgroundColor: tokens.colorNeutralBackground2,
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke2),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    boxShadow: tokens.shadow2,
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalM),
  },

  // Interactive card with hover effect
  cardInteractive: {
    cursor: 'pointer',
    ':hover': {
      boxShadow: tokens.shadow8,
      transform: 'translateY(-2px)',
      ...shorthands.borderColor(tokens.colorNeutralStroke1Hover),
    },
    ':active': {
      transform: 'translateY(0)',
      boxShadow: tokens.shadow4,
    },
  },
})

/**
 * Button styles - includes AI gradient for special actions
 */
export const useButtonStyles = makeStyles({
  // AI accent gradient button (for primary CTAs)
  gradientButton: {
    background: AI_ACCENT_GRADIENT,
    color: '#ffffff',
    fontWeight: tokens.fontWeightSemibold,
    ...shorthands.border('none'),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalL),
    cursor: 'pointer',
    transitionProperty: 'background, transform, box-shadow',
    transitionDuration: tokens.durationNormal,
    transitionTimingFunction: tokens.curveEasyEase,
    ':hover': {
      background: AI_ACCENT_GRADIENT_HOVER,
      transform: 'translateY(-1px)',
      boxShadow: '0 4px 12px rgba(51, 95, 227, 0.3)',
    },
    ':active': {
      transform: 'translateY(0)',
    },
    ':disabled': {
      opacity: 0.5,
      cursor: 'not-allowed',
      transform: 'none',
    },
  },

  // Icon-only button (for toolbars)
  iconButton: {
    minWidth: '32px',
    width: '32px',
    height: '32px',
    ...shorthands.padding('0'),
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
  },
})

/**
 * Layout styles - common flex patterns and spacing
 */
export const useLayoutStyles = makeStyles({
  // Flex row with gap
  row: {
    display: 'flex',
    flexDirection: 'row',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalM),
  },

  // Flex row with space-between
  rowSpaceBetween: {
    display: 'flex',
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
  },

  // Flex column with gap
  column: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalM),
  },

  // Grid layout for cards
  grid: {
    display: 'grid',
    gridTemplateColumns: 'repeat(auto-fill, minmax(300px, 1fr))',
    ...shorthands.gap(tokens.spacingHorizontalL),
  },

  // Full height scrollable container
  scrollContainer: {
    flex: 1,
    overflowY: 'auto',
    overflowX: 'hidden',
    ...shorthands.padding(tokens.spacingVerticalL),
  },

  // Centered content
  centered: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
  },
})

/**
 * Status indicator styles - semantic colors
 */
export const useStatusStyles = makeStyles({
  success: {
    backgroundColor: tokens.colorPaletteGreenBackground2,
    color: tokens.colorPaletteGreenForeground2,
    ...shorthands.border('1px', 'solid', tokens.colorPaletteGreenBorder2),
  },

  warning: {
    backgroundColor: tokens.colorPaletteYellowBackground2,
    color: tokens.colorPaletteYellowForeground2,
    ...shorthands.border('1px', 'solid', tokens.colorPaletteYellowBorder2),
  },

  error: {
    backgroundColor: tokens.colorPaletteRedBackground2,
    color: tokens.colorPaletteRedForeground2,
    ...shorthands.border('1px', 'solid', tokens.colorPaletteRedBorder2),
  },

  info: {
    backgroundColor: tokens.colorPaletteBlueBackground2,
    color: tokens.colorPaletteBlueForeground2,
    ...shorthands.border('1px', 'solid', tokens.colorPaletteBlueForeground2),
  },

  // Badge variant (pill-shaped)
  badge: {
    display: 'inline-flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
    ...shorthands.padding(tokens.spacingVerticalXXS, tokens.spacingHorizontalS),
    ...shorthands.borderRadius(tokens.borderRadiusCircular),
    fontSize: tokens.fontSizeBase200,
    fontWeight: tokens.fontWeightSemibold,
  },
})

/**
 * Typography styles - consistent text styling
 */
export const useTypographyStyles = makeStyles({
  title: {
    fontSize: tokens.fontSizeBase500,
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground1,
    lineHeight: tokens.lineHeightBase500,
  },

  subtitle: {
    fontSize: tokens.fontSizeBase400,
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground1,
    lineHeight: tokens.lineHeightBase400,
  },

  body: {
    fontSize: tokens.fontSizeBase300,
    fontWeight: tokens.fontWeightRegular,
    color: tokens.colorNeutralForeground1,
    lineHeight: tokens.lineHeightBase300,
  },

  caption: {
    fontSize: tokens.fontSizeBase200,
    fontWeight: tokens.fontWeightRegular,
    color: tokens.colorNeutralForeground2,
    lineHeight: tokens.lineHeightBase200,
  },

  muted: {
    color: tokens.colorNeutralForeground3,
  },
})

/**
 * Sidebar/Navigation styles
 */
export const useSidebarStyles = makeStyles({
  sidebar: {
    width: '72px',
    backgroundColor: tokens.colorNeutralBackground2,
    ...shorthands.borderRight('1px', 'solid', tokens.colorNeutralStroke1),
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    ...shorthands.padding(tokens.spacingVerticalM, '0'),
    ...shorthands.gap(tokens.spacingVerticalXS),
  },

  sidebarExpanded: {
    width: '240px',
    alignItems: 'stretch',
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalM),
  },

  navItem: {
    width: '48px',
    height: '48px',
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    cursor: 'pointer',
    color: tokens.colorNeutralForeground2,
    backgroundColor: 'transparent',
    ...shorthands.border('none'),
    transitionProperty: 'background-color, color',
    transitionDuration: tokens.durationFaster,
    ':hover': {
      backgroundColor: tokens.colorNeutralBackground2Hover,
      color: tokens.colorNeutralForeground1,
    },
  },

  navItemActive: {
    backgroundColor: tokens.colorNeutralBackground2Selected,
    color: tokens.colorBrandForeground1,
    ':hover': {
      backgroundColor: tokens.colorNeutralBackground2Selected,
    },
  },
})
