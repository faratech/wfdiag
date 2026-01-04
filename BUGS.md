# Codebase Issues & Bugs Report

## 1. Logic Bugs & Runtime Errors

### Backend (Rust)
*   ~~**Potential Panic in `openai_integration.rs`**:~~ *(Fixed in PR #7)*
    - ~~**Location**: `check_phi_silica_available` function.~~
    - ~~**Issue**: `addr.parse().unwrap()` is used on a hardcoded address string. While currently "safe" because the string is constant, any future change to an invalid string will cause the backend to panic.~~
    - ~~**Recommendation**: Use `addr.parse().map_err(...)` or `expect` with a clear message.~~

*   ~~**Potential Panic in `lib.rs`**:~~ *(Fixed in PR #7)*
    - ~~**Location**: `copy_minidumps_to_desktop` command.~~
    - ~~**Issue**: `native_diagnostics::NativeDiagnostics::new().unwrap()` is called. If initialization fails, the command will crash.~~
    - ~~**Recommendation**: Propagate the error using `?` or map it to a string error for the frontend.~~

*   ~~**Non-Atomic File Writes (Data Integrity)**:~~ *(Fixed in PR #7)*
    - ~~**Location**: `src-tauri/src/encrypted_storage.rs` (`store` method).~~
    - ~~**Issue**: The method uses `fs::write` directly to the target file path. If the application crashes or power is lost during this write operation, the file will be corrupted (partial write).~~
    - ~~**Recommendation**: Write to a temporary file first (e.g., `filename.tmp`), ensure it's flushed to disk, and then atomically rename it to the target filename.~~

### Frontend (React/TypeScript)
*   ~~**Silent Failures in AI Context**:~~ *(Fixed)*
    - ~~**Location**: `src/contexts/AIContext.tsx`~~
    - ~~**Issue**: Several `useEffect` hooks and async operations use `.catch(console.error)` which only logs errors to the developer console. Users may be unaware if background updates or status checks fail.~~
    - ~~**Recommendation**: Expose error states to the UI via toast notifications or status indicators.~~

*   ~~**Potential Race/Leak in `useScanner`**:~~ *(Fixed)*
    - ~~**Location**: `src/hooks/useScanner.ts`.~~
    - ~~**Issue**: The auto-save logic likely uses a `setTimeout` that isn't cleared if the component unmounts or the scan stops unexpectedly. This can lead to trying to save state that is no longer valid.~~
    - ~~**Recommendation**: Ensure the timeout ID is stored in a `useRef` and cleared in the cleanup function of a `useEffect`.~~

## 2. Incomplete Features (TODOs/FIXMEs)

*   ~~**Missing Service Enumeration**:~~ *(Fixed)*
    - ~~**Location**: `src-tauri/src/windows_native.rs` (Line ~370)~~
    - ~~**Issue**: The `get_services` function contains `// TODO: Implement with EnumServicesStatusExW` and returns **hardcoded placeholder data** (specifically just "Windows Update").~~
    - ~~**Impact**: The "Services" diagnostic task is effectively non-functional and provides misleading information.~~
    - ~~**Priority**: High.~~

## 3. Error Handling Gaps

*   ~~**Rust `unwrap()` Usage**:~~ *(Critical cases fixed)*
    - ~~There are multiple instances of `unwrap()` in the backend which should be converted to proper error handling (`Result<T, E>`) to prevent application crashes.~~
    - ~~Specific areas: `openai_integration.rs`, `lib.rs`, `native_diagnostics.rs`.~~
    - *Note: Fixed critical cases including disk health defaults (now "Unknown" instead of "Healthy"), version parsing with logging, JSON serialization with error logging, and ScanStorage graceful recovery.*

*   ~~**Input Validation (Security)**:~~ *(Fixed)*
    - ~~**Location**: `src-tauri/src/lib.rs` (Command `save_results_to_file`).~~
    - ~~**Issue**: The `path` argument comes directly from the frontend and is used in `fs::write(&path, content)`. While the frontend might use a dialog, a compromised frontend or malicious actor could invoke this command with arbitrary paths (e.g., overwriting system files if running as Admin).~~
    - ~~**Recommendation**: Validate that the path is within allowed directories or user-selected paths.~~

## 4. Security & Architecture

*   **Unsafe Code Blocks**:
    - **Context**: The application relies heavily on `unsafe` Rust code to interact with Windows APIs (`windows-rs`).
    - **Locations**: `windows_native.rs`, `phi_silica.rs`, `dpapi.rs`.
    - **Risk**: While necessary for the domain, these blocks require manual memory management and are prone to segmentation faults if not handled correctly. `phi_silica.rs` involves manual DLL loading and function pointer casting, which is high-risk.

*   **Frontend/Backend Protocol Tight Coupling**:
    - **Issue**: The frontend `useScanner.ts` and backend `diagnostics.rs` share implicit knowledge of Task IDs. If a Task ID is renamed in Rust but not TypeScript, the diagnostic will fail silently or with a generic "Task not found" error.
    - **Recommendation**: Generate TypeScript types from Rust structs to ensure type safety across the bridge.

## 5. Other Findings

*   **Hardcoded Configuration**:
    - `openai_integration.rs` contains hardcoded ports (`5001`) and addresses (`127.0.0.1`) for the local AI server. This should ideally be configurable via `AppSettings`.