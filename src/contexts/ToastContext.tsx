import React, { createContext, useContext, useCallback, ReactNode } from 'react'
import {
  Toaster,
  useToastController,
  ToastTitle,
  ToastBody,
  Toast,
  ToastIntent,
  useId,
} from '@fluentui/react-components'

export type ToastType = 'success' | 'error' | 'warning' | 'info'

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

interface ToastProviderProps {
  children: ReactNode
}

export const ToastProvider: React.FC<ToastProviderProps> = ({ children }) => {
  const toasterId = useId('toaster')
  const { dispatchToast } = useToastController(toasterId)

  const showToast = useCallback(
    (title: string, message?: string, type: ToastType = 'info') => {
      const intent: ToastIntent = type === 'error' ? 'error'
        : type === 'success' ? 'success'
        : type === 'warning' ? 'warning'
        : 'info'

      dispatchToast(
        <Toast>
          <ToastTitle>{title}</ToastTitle>
          {message && <ToastBody>{message}</ToastBody>}
        </Toast>,
        { intent, timeout: type === 'error' ? 8000 : 5000 }
      )
    },
    [dispatchToast]
  )

  const showSuccess = useCallback(
    (title: string, message?: string) => showToast(title, message, 'success'),
    [showToast]
  )

  const showError = useCallback(
    (title: string, message?: string) => showToast(title, message, 'error'),
    [showToast]
  )

  const showWarning = useCallback(
    (title: string, message?: string) => showToast(title, message, 'warning'),
    [showToast]
  )

  const showInfo = useCallback(
    (title: string, message?: string) => showToast(title, message, 'info'),
    [showToast]
  )

  const value: ToastContextType = {
    showToast,
    showSuccess,
    showError,
    showWarning,
    showInfo,
  }

  return (
    <ToastContext.Provider value={value}>
      <Toaster toasterId={toasterId} position="top-end" />
      {children}
    </ToastContext.Provider>
  )
}