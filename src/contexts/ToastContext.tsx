import React, { createContext, useContext, useCallback, useState, ReactNode } from 'react'

export type ToastType = 'success' | 'error' | 'warning' | 'info'

interface ToastItem {
  id: number
  title: string
  message?: string
  type: ToastType
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

interface ToastProviderProps {
  children: ReactNode
}

let nextId = 1

export const ToastProvider: React.FC<ToastProviderProps> = ({ children }) => {
  const [toasts, setToasts] = useState<ToastItem[]>([])

  const remove = useCallback((id: number) => {
    setToasts(prev => prev.filter(t => t.id !== id))
  }, [])

  const showToast = useCallback(
    (title: string, message?: string, type: ToastType = 'info') => {
      const id = nextId++
      setToasts(prev => [...prev, { id, title, message, type }])
      window.setTimeout(() => remove(id), type === 'error' ? 8000 : 5000)
    },
    [remove]
  )

  const showSuccess = useCallback((t: string, m?: string) => showToast(t, m, 'success'), [showToast])
  const showError = useCallback((t: string, m?: string) => showToast(t, m, 'error'), [showToast])
  const showWarning = useCallback((t: string, m?: string) => showToast(t, m, 'warning'), [showToast])
  const showInfo = useCallback((t: string, m?: string) => showToast(t, m, 'info'), [showToast])

  const value: ToastContextType = { showToast, showSuccess, showError, showWarning, showInfo }

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="toast-stack">
        {toasts.map((t, i) => (
          <div
            key={t.id}
            className={`toast toast-${t.type}`}
            style={{ bottom: 36 + i * 56 }}
            onClick={() => remove(t.id)}
          >
            <i className={`fa-solid ${ICONS[t.type]}`} />
            <div>
              <strong>{t.title}</strong>
              {t.message && <div style={{ opacity: 0.85, fontSize: 12 }}>{t.message}</div>}
            </div>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  )
}
