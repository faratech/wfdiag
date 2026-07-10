#!/usr/bin/env node

import { spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)
const root = path.resolve(__dirname, '..')

const args = process.argv.slice(2)
const versionArg = args.find(arg => !arg.startsWith('-'))
const flags = args.filter(arg => arg.startsWith('-'))
const version = versionArg ?? JSON.parse(readFileSync(path.join(root, 'version.json'), 'utf8')).version

if (!version) {
  console.error('No version supplied and version.json has no version field.')
  process.exit(1)
}

const script = path.join(root, 'scripts', 'bump-version.py')
const scriptArgs = [script, version, ...flags]
const candidates = process.platform === 'win32' ? ['python', 'python3'] : ['python3', 'python']

for (const python of candidates) {
  const result = spawnSync(python, scriptArgs, { stdio: 'inherit' })
  if (result.error && result.error.code === 'ENOENT') {
    continue
  }
  process.exit(result.status ?? 1)
}

console.error('Python was not found. Install Python 3 or run scripts/bump-version.py directly.')
process.exit(1)
