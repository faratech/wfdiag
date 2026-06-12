import js from '@eslint/js'
import tseslint from 'typescript-eslint'
import reactHooks from 'eslint-plugin-react-hooks'
import globals from 'globals'

export default tseslint.config(
  { ignores: ['dist/', 'node_modules/', 'src-tauri/', 'release/', 'scripts/', 'msix_layout/'] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  reactHooks.configs.flat['recommended-latest'],
  {
    files: ['src/**/*.{ts,tsx}'],
    languageOptions: {
      globals: { ...globals.browser, __BUILD_TIME__: 'readonly' },
    },
    rules: {
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
      ],
      // Existing code has scattered `any` (Tauri event payloads etc.); tighten next cycle
      '@typescript-eslint/no-explicit-any': 'warn',
      // New compiler-powered rule; flags pre-existing sync-setState-in-effect
      // patterns in ThemeContext/AIContext/SettingsDialog — refactor next cycle
      'react-hooks/set-state-in-effect': 'warn',
    },
  },
  {
    files: ['vite.config.ts', 'vitest.config.ts', 'eslint.config.js'],
    languageOptions: { globals: globals.node },
  },
)
