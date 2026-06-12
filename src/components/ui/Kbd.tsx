import React from 'react'

/** Renders a shortcut like "Ctrl+K" as separate key chips. */
export const Kbd: React.FC<{ keys: string }> = ({ keys }) => (
  <>
    {keys.split('+').map(k => (
      <kbd key={k} className="kbd">{k}</kbd>
    ))}
  </>
)
