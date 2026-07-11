import React from 'react'

interface ChartCardProps {
  title: string
  value: string
  sub: string
  series: number[]
  max: number
  hint: string
}

/**
 * Monochrome SVG sparkline card. All monitor charts render in the brand accent
 * (--app-chart) — the per-metric rainbow is dropped per the design system's
 * "no non-brand hues" rule. The path is drawn in a fixed 300×72 viewBox and
 * stretched with preserveAspectRatio="none"; a non-scaling stroke keeps the
 * line crisp at any card width.
 */
export const ChartCard: React.FC<ChartCardProps> = ({ title, value, sub, series, max, hint }) => {
  const w = 300
  const h = 72
  const n = series.length
  const pts = n > 1
    ? series.map((v, i) => `${(i * (w / (n - 1))).toFixed(1)},${(h - 4 - Math.min(1, v / max) * (h - 12)).toFixed(1)}`)
    : ['0,68', '300,68']
  const line = 'M' + pts.join(' L')
  const area = `${line} L300,72 L0,72 Z`
  return (
    <div className="chart-card">
      <div className="ch-head">
        <div>
          <div className="ch-title">{title}</div>
          <div className="ch-hint">{hint}</div>
        </div>
        <div className="ch-val">{value}<span className="ch-sub">{sub}</span></div>
      </div>
      <svg className="chart-svg" viewBox="0 0 300 72" preserveAspectRatio="none">
        <path className="area" d={area} />
        <path className="line" d={line} vectorEffect="non-scaling-stroke" />
      </svg>
      <div className="chart-axis">
        <span>-60s</span><span>-45</span><span>-30</span><span>-15</span><span>now</span>
      </div>
    </div>
  )
}
