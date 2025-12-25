## Project Overview

This project is a security-hardened, modern diagnostic tool for Windows systems called "WF Diagnostics". It is built with Tauri v2 and React, and developed by Fara Technologies LLC for WindowsForum.com. The application features a comprehensive set of diagnostic tasks, real-time system monitoring, encrypted data storage, and AI-powered analysis via OpenAI integration.

The frontend is a React application built with Vite and TypeScript, using the Fluent UI component library. The backend is a pure Rust application that exposes a set of Tauri commands to the frontend. The backend handles the diagnostic tasks, system monitoring, data encryption, and communication with external services like OpenAI.

## Building and Running

### Prerequisites

*   **Windows 10/11** (x64)
*   **Rust**: `winget install Rustlang.Rust.GNU`
*   **Node.js**: `winget install OpenJS.NodeJS`

### Development

1.  Clone the repository: `git clone https://github.com/faratech/wfdiag.git`
2.  Navigate to the project directory: `cd wfdiag`
3.  Install dependencies: `npm install`
4.  Run in development mode: `npm run tauri dev`

### Production Build

1.  Follow the development setup steps.
2.  Build for production: `npm run tauri build`

## Development Conventions

*   **Frontend:** The frontend is a React application written in TypeScript. It uses the Fluent UI component library for its design system. The main application logic is in `src/App.tsx`.
*   **Backend:** The backend is a Rust application that uses the Tauri framework. The main application logic is in `src-tauri/src/lib.rs`. The backend exposes a set of commands to the frontend using the `#[tauri::command]` macro.
*   **State Management:** The application state is managed in the Rust backend and exposed to the frontend through Tauri commands.
*   **API:** The backend exposes a comprehensive API to the frontend for running diagnostics, managing data, and interacting with external services. The API is defined in `src-tauri/src/lib.rs`.
*   **Security:** The application has a strong focus on security, with features like command injection prevention, encrypted data storage, and secure authentication. The security features are implemented in `src-tauri/src/security.rs` and `src-tauri/src/encrypted_storage.rs`.
