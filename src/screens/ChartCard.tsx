import React from 'react'

interface ChartCardProps {
  title: string
  value: string
  sub: string
  series: number[]
  color: string
  max: number
  hint: string
}

// SVG sparkline card — ported verbatim from the theme-demo ChartCard.
export const ChartCard: React.FC<ChartCardProps> = ({ title, value, sub, series, color, max, hint }) => {
  const w = 320
  const h = 80
  const pts = series.length
    ? series.map((v, i) => [
        (i / Math.max(1, series.length - 1)) * w,
        h - Math.min(v, max) / max * (h - 4) - 2,
      ])
    : [[0, h - 2], [w, h - 2]]
  const path = pts.map((p, i) => `${i === 0 ? 'M' : 'L'}${p[0].toFixed(1)},${p[1].toFixed(1)}`).join(' ')
  const area = `${path} L${w},${h} L0,${h} Z`
  const cur = pts[pts.length - 1]
  return (
    <div className="chart-card">
      <div className="ch-head">
        <div>
          <div className="ch-title">{title}</div>
          <div style={{ fontSize: 11, color: 'var(--wf-text-muted)' }}>{hint}</div>
        </div>
        <div style={{ textAlign: 'right' }}>
          <span className="ch-val" style={{ color }}>{value}</span>
          <span className="ch-sub">{sub}</span>
        </div>
      </div>
      <svg className="chart-svg" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none">
        <defs>
          <linearGradient id={`g-${title}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={color} stopOpacity="0.32" />
            <stop offset="100%" stopColor={color} stopOpacity="0.02" />
          </linearGradient>
        </defs>
        <path d={area} fill={`url(#g-${title})`} />
        <path d={path} fill="none" stroke={color} strokeWidth="1.6" />
        <circle cx={cur[0]} cy={cur[1]} r="3" fill={color} />
        <circle cx={cur[0]} cy={cur[1]} r="6" fill={color} opacity="0.2" />
      </svg>
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 10, color: 'var(--wf-text-muted)', marginTop: 4, fontFamily: 'var(--wf-font-mono)' }}>
        <span>-60s</span><span>-45</span><span>-30</span><span>-15</span><span>now</span>
      </div>
    </div>
  )
}
