// Minimal barrel after the de-Fluent rewrite. The screens live in src/screens/*;
// only shared types, the error boundary, and the two dialogs remain here.

export { ErrorBoundary } from './ErrorBoundary'
export { SettingsDialog } from './SettingsDialog'
export { AboutDialog } from './AboutDialog'
export type { TabValue, SettingsData } from './types'
