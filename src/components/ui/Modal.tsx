import React, { useEffect, useRef, useState, ReactNode, useCallback } from 'react'

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

export interface ModalProps {
  open: boolean
  onClose: () => void
  title: string
  width?: number
  children: ReactNode
  footer?: ReactNode
}

/**
 * Shared modal: scale/fade animation, focus trap, Escape-to-close, and focus
 * restoration to the element that opened it. Visual output matches the
 * previous hand-rolled dialogs (wf-block + modal-card + wf-block-header).
 */
export const Modal: React.FC<ModalProps> = (props) =>
  props.open ? <ModalInner {...props} /> : null

/**
 * Mounted only while open (Modal gates on it), so `closing` always starts false
 * — the close animation can never be stuck on from a prior cycle. The element
 * unmounts after the out-animation completes (handleAnimationEnd → onClose →
 * parent sets open=false), so the animation always plays in full.
 */
const ModalInner: React.FC<ModalProps> = ({ onClose, title, width = 520, children, footer }) => {
  const [closing, setClosing] = useState(false)
  const cardRef = useRef<HTMLDivElement>(null)
  const restoreRef = useRef<HTMLElement | null>(null)
  const pointerDownOnOverlayRef = useRef(false)

  useEffect(() => {
    restoreRef.current = document.activeElement as HTMLElement | null
    // Focus the first focusable element once the card is in the DOM
    requestAnimationFrame(() => {
      const first = cardRef.current?.querySelector<HTMLElement>(FOCUSABLE)
      ;(first ?? cardRef.current)?.focus()
    })
  }, [])

  const requestClose = useCallback(() => setClosing(true), [])

  const handleOverlayPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    pointerDownOnOverlayRef.current = e.target === e.currentTarget
  }

  const handleOverlayClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.target === e.currentTarget && pointerDownOnOverlayRef.current) {
      requestClose()
    }
    pointerDownOnOverlayRef.current = false
  }

  const handleAnimationEnd = (e: React.AnimationEvent) => {
    if (closing && e.animationName === 'overlay-out') {
      onClose()
      restoreRef.current?.focus()
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    e.stopPropagation()
    if (e.key === 'Escape') {
      // Modals own Escape while focus is trapped inside; the global shortcut
      // listener must not also act on it
      requestClose()
      return
    }
    if (e.key !== 'Tab') return
    const nodes = cardRef.current?.querySelectorAll<HTMLElement>(FOCUSABLE)
    if (!nodes || nodes.length === 0) return
    const list = Array.from(nodes)
    const first = list[0]
    const last = list[list.length - 1]
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault()
      last.focus()
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault()
      first.focus()
    }
  }

  return (
    <div
      className={`modal-overlay ${closing ? 'closing' : ''}`}
      onPointerDown={handleOverlayPointerDown}
      onClick={handleOverlayClick}
      onKeyDown={handleKeyDown}
      onAnimationEnd={handleAnimationEnd}
    >
      <div
        ref={cardRef}
        className="wf-block modal-card"
        style={{ maxWidth: width, width: '92%' }}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        onClick={e => e.stopPropagation()}
      >
        <header className="wf-block-header">
          <span className="accent-bar" />
          <span>{title}</span>
          <button
            className="btn-icon"
            style={{ marginLeft: 'auto' }}
            onClick={requestClose}
            aria-label="Close"
            title="Close"
          >
            <i className="fa-solid fa-xmark" aria-hidden="true" />
          </button>
        </header>
        <div className="modal-body">{children}</div>
        {footer && <div className="modal-footer">{footer}</div>}
      </div>
    </div>
  )
}
