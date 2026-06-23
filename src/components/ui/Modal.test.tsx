import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { Modal } from './Modal'

describe('Modal overlay close behavior', () => {
  it('does not close when a drag starts inside the dialog and ends on the overlay', () => {
    const onClose = vi.fn()
    render(
      <Modal open onClose={onClose} title="Settings">
        <input aria-label="Custom endpoint" defaultValue="http://127.0.0.1:11434" />
      </Modal>
    )

    const overlay = document.querySelector<HTMLElement>('.modal-overlay')
    expect(overlay).toBeTruthy()

    fireEvent.pointerDown(screen.getByLabelText('Custom endpoint'))
    fireEvent.click(overlay!)

    expect(onClose).not.toHaveBeenCalled()
    expect(overlay).not.toHaveClass('closing')
  })

  it('closes when the press and click both happen on the overlay', () => {
    const onClose = vi.fn()
    render(
      <Modal open onClose={onClose} title="Settings">
        <button>Inside</button>
      </Modal>
    )

    const overlay = document.querySelector<HTMLElement>('.modal-overlay')
    expect(overlay).toBeTruthy()

    fireEvent.pointerDown(overlay!)
    fireEvent.click(overlay!)

    expect(overlay).toHaveClass('closing')
  })
})
