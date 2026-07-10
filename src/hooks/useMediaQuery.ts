import { useCallback, useSyncExternalStore } from 'react'

/**
 * Reactive media-query match. Guarded for jsdom (tests have no matchMedia):
 * falls back to false and stays inert there. Backed by useSyncExternalStore so
 * React owns the subscription — no effect/setState dance.
 */
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (onChange: () => void) => {
      const mql = window.matchMedia?.(query)
      if (!mql) return () => {}
      mql.addEventListener('change', onChange)
      return () => mql.removeEventListener('change', onChange)
    },
    [query]
  )

  // `?.` short-circuits the whole chain to undefined when matchMedia is absent
  const getSnapshot = useCallback(() => window.matchMedia?.(query).matches ?? false, [query])

  return useSyncExternalStore(subscribe, getSnapshot, () => false)
}
