import React, { createContext, useContext, useCallback, useRef, useState, ReactNode } from 'react'

export type ToastType = 'success' | 'error' | 'warning' | 'info'

interface ToastItem {
  id: number
  title: string
  message?: string
  type: ToastType
  durationMs: number
  exiting?: boolean
}

interface ToastContextType {
  showToast: (title: string, message?: string, type?: ToastType) => void
  showSuccess: (title: string, message?: string) => void
  showError: (title: string, message?: string) => void
  showWarning: (title: string, message?: string) => void
  showInfo: (title: string, message?: string) => void
}

const ToastContext = createContext<ToastContextType | undefined>(undefined)

export const useToast = () => {
  const context = useContext(ToastContext)
  if (!context) {
    throw new Error('useToast must be used within a ToastProvider')
  }
  return context
}

const ICONS: Record<ToastType, string> = {
  success: 'fa-circle-check',
  error: 'fa-circle-xmark',
  warning: 'fa-triangle-exclamation',
  info: 'fa-circle-info',
}

const MAX_TOASTS = 4

interface ToastProviderProps {
  children: ReactNode
}

let nextId = 1

export const ToastProvider: React.FC<ToastProviderProps> = ({ children }) => {
  const [toasts, setToasts] = useState<ToastItem[]>([])
  // Per-toast dismiss timers so hover can pause and resume the countdown
  const timersRef = useRef<Map<number, { handle: number; deadline: number; remaining: number }>>(new Map())

  const remove = useCallback((id: number) => {
    timersRef.current.delete(id)
    setToasts(prev => prev.filter(t => t.id !== id))
  }, [])

  // Play the exit animation first; actual removal happens on animationend
  const dismiss = useCallback((id: number) => {
    const timer = timersRef.current.get(id)
    if (timer) {
      clearTimeout(timer.handle)
      timersRef.current.delete(id)
    }
    setToasts(prev => prev.map(t => (t.id === id ? { ...t, exiting: true } : t)))
  }, [])

  const armTimer = useCallback((id: number, ms: number) => {
    const handle = window.setTimeout(() => dismiss(id), ms)
    timersRef.current.set(id, { handle, deadline: Date.now() + ms, remaining: ms })
  }, [dismiss])

  const pauseTimer = useCallback((id: number) => {
    const timer = timersRef.current.get(id)
    if (!timer) return
    clearTimeout(timer.handle)
    timer.remaining = Math.max(0, timer.deadline - Date.now())
  }, [])

  const resumeTimer = useCallback((id: number) => {
    const timer = timersRef.current.get(id)
    if (!timer) return
    timer.deadline = Date.now() + timer.remaining
    timer.handle = window.setTimeout(() => dismiss(id), timer.remaining)
  }, [dismiss])

  const showToast = useCallback(
    (title: string, message?: string, type: ToastType = 'info') => {
      const id = nextId++
      const durationMs = type === 'error' ? 8000 : 5000
      setToasts(prev => {
        const next = [...prev, { id, title, message, type, durationMs }]
        // Cap the stack; drop the oldest non-exiting toast
        while (next.length > MAX_TOASTS) {
          const oldest = next.shift()
          if (oldest) {
            const timer = timersRef.current.get(oldest.id)
            if (timer) clearTimeout(timer.handle)
            timersRef.current.delete(oldest.id)
          }
        }
        return next
      })
      armTimer(id, durationMs)
    },
    [armTimer]
  )

  const showSuccess = useCallback((t: string, m?: string) => showToast(t, m, 'success'), [showToast])
  const showError = useCallback((t: string, m?: string) => showToast(t, m, 'error'), [showToast])
  const showWarning = useCallback((t: string, m?: string) => showToast(t, m, 'warning'), [showToast])
  const showInfo = useCallback((t: string, m?: string) => showToast(t, m, 'info'), [showToast])

  const value: ToastContextType = { showToast, showSuccess, showError, showWarning, showInfo }

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="toast-stack" aria-live="polite">
        {toasts.map(t => (
          <div
            key={t.id}
            className={`toast toast-${t.type} ${t.exiting ? 'exiting' : ''}`}
            role={t.type === 'error' ? 'alert' : 'status'}
            onMouseEnter={() => pauseTimer(t.id)}
            onMouseLeave={() => resumeTimer(t.id)}
            onAnimationEnd={e => {
              if (t.exiting && (e as React.AnimationEvent).animationName === 'toast-out') remove(t.id)
            }}
          >
            <i className={`fa-solid ${ICONS[t.type]}`} aria-hidden="true" />
            <div className="toast-body">
              <strong>{t.title}</strong>
              {t.message && <div style={{ opacity: 0.85, fontSize: 12 }}>{t.message}</div>}
            </div>
            <button className="toast-close" onClick={() => dismiss(t.id)} aria-label="Dismiss">
              <i className="fa-solid fa-xmark" aria-hidden="true" />
            </button>
            {!t.exiting && <span className="toast-timer" style={{ animationDuration: `${t.durationMs}ms` }} />}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  )
}
