import React, {
  ReactNode,
  useCallback,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { createPortal } from 'react-dom'
import { Kbd } from './Kbd'

export interface TooltipProps {
  content: ReactNode
  /** Optional shortcut hint rendered as key chips, e.g. "Ctrl+K" */
  shortcut?: string
  side?: 'top' | 'right'
  children: ReactNode
}

/** Portaled tooltip; appears on hover and on keyboard focus of the child. */
export const Tooltip: React.FC<TooltipProps> = ({ content, shortcut, side = 'top', children }) => {
  const id = useId()
  const wrapRef = useRef<HTMLSpanElement>(null)
  const [open, setOpen] = useState(false)
  const [style, setStyle] = useState<React.CSSProperties | null>(null)

  const updatePosition = useCallback(() => {
    const trigger = wrapRef.current
    if (!trigger) return

    const rect = trigger.getBoundingClientRect()
    const gap = 7
    if (side === 'right') {
      setStyle({
        left: rect.right + gap,
        top: rect.top + rect.height / 2,
        transform: 'translateY(-50%)',
      })
      return
    }

    setStyle({
      left: rect.left + rect.width / 2,
      top: rect.top - gap,
      transform: 'translate(-50%, -100%)',
    })
  }, [side])

  useLayoutEffect(() => {
    if (!open) return

    updatePosition()
    window.addEventListener('resize', updatePosition)
    window.addEventListener('scroll', updatePosition, true)
    return () => {
      window.removeEventListener('resize', updatePosition)
      window.removeEventListener('scroll', updatePosition, true)
    }
  }, [open, updatePosition])

  return (
    <span
      ref={wrapRef}
      className={`tooltip-wrap ${side === 'right' ? 'tip-right' : ''}`}
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
      onFocus={() => setOpen(true)}
      onBlur={() => setOpen(false)}
    >
      {children}
      {open && style && typeof document !== 'undefined' && createPortal(
        <span id={id} className="tooltip-portal" role="tooltip" style={style}>
          {content}
          {shortcut && <Kbd keys={shortcut} />}
        </span>,
        document.body
      )}
    </span>
  )
}
