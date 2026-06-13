import { useEffect, useState } from 'react'

/**
 * Reactive media-query match. Guarded for jsdom (tests have no matchMedia):
 * falls back to false and stays inert there.
 */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState<boolean>(
    () => window.matchMedia?.(query).matches ?? false
  )

  useEffect(() => {
    const mql = window.matchMedia?.(query)
    if (!mql) return
    const onChange = (event: MediaQueryListEvent) => setMatches(event.matches)
    setMatches(mql.matches)
    mql.addEventListener('change', onChange)
    return () => mql.removeEventListener('change', onChange)
  }, [query])

  return matches
}
