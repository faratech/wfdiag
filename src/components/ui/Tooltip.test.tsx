import { describe, it, expect } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { Tooltip } from './Tooltip'

describe('Tooltip', () => {
  it('renders the bubble through a body portal', async () => {
    render(
      <Tooltip content="Run the essential checks" shortcut="Ctrl+Shift+Q">
        <button>Quick Scan</button>
      </Tooltip>
    )

    fireEvent.mouseEnter(screen.getByText('Quick Scan').parentElement!)

    const tooltip = await screen.findByRole('tooltip')
    expect(tooltip).toHaveTextContent('Run the essential checks')
    expect(tooltip.parentElement).toBe(document.body)
  })

  it('hides the portaled bubble after mouse leave', async () => {
    render(
      <Tooltip content="Diagnostics" side="right">
        <button>Nav item</button>
      </Tooltip>
    )

    const trigger = screen.getByText('Nav item').parentElement!
    fireEvent.mouseEnter(trigger)
    expect(await screen.findByRole('tooltip')).toBeInTheDocument()

    fireEvent.mouseLeave(trigger)
    expect(screen.queryByRole('tooltip')).not.toBeInTheDocument()
  })
})
