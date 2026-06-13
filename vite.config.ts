import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Generate build timestamp in YYMMDDHH format
const now = new Date()
const buildTime = `${String(now.getFullYear()).slice(-2)}${String(now.getMonth() + 1).padStart(2, '0')}${String(now.getDate()).padStart(2, '0')}${String(now.getHours()).padStart(2, '0')}`

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 3000,
    strictPort: true,
    fs: {
      // Restrict file system access
      strict: true,
      // Deny access to sensitive files and directories
      deny: [
        '**/.env',
        '**/.env.*',
        '**/secrets/**',
        '**/private/**',
        '**/.git/**',
        '**/node_modules/**',
        '**/.ssh/**',
        '**/.aws/**',
        '**/config.json',
        '**/credentials.json',
        '**/*.pem',
        '**/*.key',
        '**/*.pfx',
        '**/*.p12',
        // Prevent directory traversal attacks
        '**/../**',
        '**/./.**',
        '**//.**',
        '**/\\.**'
      ],
      // Only allow serving files from the project root
      allow: ['.']
    },
    // Security headers
    headers: {
      'X-Content-Type-Options': 'nosniff',
      'X-Frame-Options': 'DENY',
      'X-XSS-Protection': '1; mode=block',
      'Content-Security-Policy': "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline';"
    },
    // Restrict CORS in development
    cors: {
      origin: false
    }
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: ['es2021', 'chrome100', 'safari13'],
    // Vite 8 bundles with Rolldown and no longer ships esbuild; use the
    // built-in (Oxc) minifier instead of the old 'esbuild' setting.
    minify: !process.env.TAURI_DEBUG,
    sourcemap: !!process.env.TAURI_DEBUG,
  },
  define: {
    __BUILD_TIME__: JSON.stringify(buildTime),
  },
})