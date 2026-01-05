# Repository Guidelines

## Project Structure & Module Organization
- `src/` holds the React + TypeScript frontend (main entry: `src/App.tsx`).
- `src-tauri/` contains the Rust backend and Tauri commands (see `src-tauri/src/lib.rs`).
- `public/` hosts static assets for Vite; build output lands in `dist/`.
- `scripts/` includes automation like version sync (`scripts/update-version.js`).
- `docs/`, `release/`, and `releases/` store documentation and release artifacts.

## Build, Test, and Development Commands
- `npm install` installs frontend dependencies.
- `npm run tauri dev` runs the Vite dev server plus Tauri in dev mode.
- `npm run dev` runs the frontend only (limited without Tauri APIs).
- `npm run build` compiles TypeScript and builds the frontend bundle.
- `npm run tauri build` creates production installers (exe/MSI/NSIS, MSIX optional).
- `npx tsc --noEmit` performs TypeScript type checks.
- `cd src-tauri && cargo check` verifies Rust builds; `cargo fmt` and `cargo clippy` for formatting and linting.

## Coding Style & Naming Conventions
- TypeScript uses 2-space indentation, single quotes, and no semicolons (match existing files like `src/App.tsx`).
- Prefer descriptive React component names in PascalCase and hooks in `useX` form.
- Rust follows standard rustfmt output and idiomatic snake_case for functions and modules.

## Testing Guidelines
- There is no dedicated test framework in the repo; most validation is manual.
- When adding tests, keep them close to the relevant module (e.g., Rust unit tests under `src-tauri/src/`).
- Document any new test commands in this file.

## Commit & Pull Request Guidelines
- Commit subjects are short, imperative sentences; optional scope prefixes like `docs:` appear in history.
- Include a clear PR description with rationale and user impact.
- Attach screenshots or screen recordings for UI changes (e.g., `src/` updates).

## Security & Configuration Tips
- Do not commit API keys or secrets; keep them in local environment or OS-managed storage.
- Tauri permissions are defined in `src-tauri/capabilities/default.json`—update carefully when adding features.
