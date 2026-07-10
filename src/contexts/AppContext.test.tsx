import React from 'react'
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { AppProvider, useAppContext } from './AppContext'

const invokeMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

const Consumer: React.FC = () => {
  const { navRailCollapsed, settingsLoaded } = useAppContext()
  return (
    <div>
      <span data-testid="nav">{String(navRailCollapsed)}</span>
      <span data-testid="settings">{String(settingsLoaded)}</span>
    </div>
  )
}

beforeEach(() => {
  localStorage.clear()
  invokeMock.mockReset()
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'load_settings') return Promise.resolve({})
    return Promise.reject(new Error(`unexpected command ${cmd}`))
  })
})

describe('AppContext', () => {
  it('falls back when navRailCollapsed localStorage is malformed', async () => {
    localStorage.setItem('navRailCollapsed', '{bad json')

    render(
      <AppProvider>
        <Consumer />
      </AppProvider>
    )

    expect(screen.getByTestId('nav')).toHaveTextContent('false')
    await waitFor(() => expect(screen.getByTestId('settings')).toHaveTextContent('true'))
  })
})
