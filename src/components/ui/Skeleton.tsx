import React from 'react'

export interface SkeletonProps {
  variant?: 'text' | 'block'
  width?: number | string
  height?: number | string
  /** Render this many stacked skeletons */
  count?: number
  style?: React.CSSProperties
}

export const Skeleton: React.FC<SkeletonProps> = ({ variant = 'text', width, height, count = 1, style }) => (
  <>
    {Array.from({ length: count }, (_, i) => (
      <div
        key={i}
        className={`skeleton ${variant}`}
        aria-hidden="true"
        style={{ width, height, marginBottom: count > 1 && i < count - 1 ? 8 : undefined, ...style }}
      />
    ))}
  </>
)
