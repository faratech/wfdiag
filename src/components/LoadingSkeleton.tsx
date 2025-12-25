import React from 'react'
import {
  makeStyles,
  shorthands,
  tokens,
} from '@fluentui/react-components'

const useStyles = makeStyles({
  skeleton: {
    backgroundColor: tokens.colorNeutralBackground3,
    ...shorthands.borderRadius(tokens.borderRadiusSmall),
    position: 'relative',
    overflow: 'hidden',

    '&::after': {
      content: '""',
      position: 'absolute',
      top: 0,
      left: 0,
      right: 0,
      bottom: 0,
      background: `linear-gradient(90deg, transparent, ${tokens.colorNeutralBackground2}, transparent)`,
      animationName: {
        '0%': { transform: 'translateX(-100%)' },
        '100%': { transform: 'translateX(100%)' },
      },
      animationDuration: '1.5s',
      animationIterationCount: 'infinite',
      animationTimingFunction: 'ease-in-out',
    },
  },

  skeletonNoAnimation: {
    '&::after': {
      display: 'none',
    },
  },

  text: {
    height: '16px',
    marginBottom: tokens.spacingVerticalS,
  },

  textSmall: {
    height: '12px',
  },

  textLarge: {
    height: '24px',
  },

  circular: {
    ...shorthands.borderRadius('50%'),
  },

  card: {
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
  },

  container: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalS),
  },
})

export interface LoadingSkeletonProps {
  variant?: 'text' | 'circular' | 'rectangular' | 'card'
  width?: string | number
  height?: string | number
  rows?: number
  animated?: boolean
  className?: string
}

export const LoadingSkeleton: React.FC<LoadingSkeletonProps> = ({
  variant = 'text',
  width,
  height,
  rows = 1,
  animated = true,
  className,
}) => {
  const styles = useStyles()

  const getVariantClass = () => {
    switch (variant) {
      case 'circular':
        return styles.circular
      case 'card':
        return styles.card
      case 'text':
      case 'rectangular':
      default:
        return ''
    }
  }

  const getDefaultDimensions = () => {
    switch (variant) {
      case 'circular':
        return { width: 40, height: 40 }
      case 'card':
        return { width: '100%', height: 120 }
      case 'text':
        return { width: '100%', height: 16 }
      case 'rectangular':
      default:
        return { width: '100%', height: 100 }
    }
  }

  const defaults = getDefaultDimensions()
  const finalWidth = width ?? defaults.width
  const finalHeight = height ?? defaults.height

  const skeletonStyle = {
    width: typeof finalWidth === 'number' ? `${finalWidth}px` : finalWidth,
    height: typeof finalHeight === 'number' ? `${finalHeight}px` : finalHeight,
  }

  if (rows > 1) {
    return (
      <div className={styles.container}>
        {Array.from({ length: rows }).map((_, index) => (
          <div
            key={index}
            className={`${styles.skeleton} ${getVariantClass()} ${!animated ? styles.skeletonNoAnimation : ''} ${className || ''}`}
            style={{
              ...skeletonStyle,
              // Vary width for text rows for more natural look
              width: variant === 'text' && index === rows - 1 ? '70%' : skeletonStyle.width,
            }}
          />
        ))}
      </div>
    )
  }

  return (
    <div
      className={`${styles.skeleton} ${getVariantClass()} ${!animated ? styles.skeletonNoAnimation : ''} ${className || ''}`}
      style={skeletonStyle}
    />
  )
}

// Pre-composed skeleton layouts for common use cases
export const CardSkeleton: React.FC<{ className?: string }> = ({ className }) => {
  const styles = useStyles()

  return (
    <div
      className={`${styles.skeleton} ${styles.card} ${className || ''}`}
      style={{
        padding: tokens.spacingVerticalL,
        display: 'flex',
        flexDirection: 'column',
        gap: tokens.spacingVerticalM,
      }}
    >
      <LoadingSkeleton variant="circular" width={48} height={48} />
      <LoadingSkeleton variant="text" width="60%" />
      <LoadingSkeleton variant="text" rows={2} />
    </div>
  )
}

export const TableRowSkeleton: React.FC<{ columns?: number; className?: string }> = ({
  columns = 4,
  className,
}) => {
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: `repeat(${columns}, 1fr)`,
        gap: tokens.spacingHorizontalM,
        padding: tokens.spacingVerticalM,
      }}
      className={className}
    >
      {Array.from({ length: columns }).map((_, index) => (
        <LoadingSkeleton key={index} variant="text" width={index === 0 ? '80%' : '60%'} />
      ))}
    </div>
  )
}

export const MetricSkeleton: React.FC<{ className?: string }> = ({ className }) => {
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: tokens.spacingVerticalS,
        padding: tokens.spacingVerticalM,
      }}
      className={className}
    >
      <LoadingSkeleton variant="text" width={60} height={32} />
      <LoadingSkeleton variant="text" width={80} height={12} />
    </div>
  )
}
