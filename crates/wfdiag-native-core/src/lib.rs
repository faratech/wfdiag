//! Shared engine primitives for `WFDiag`.
//!
//! Everything here is UI-framework-neutral and dependency-light: it is the
//! bottom of the crate graph, depended on by the diagnostics, monitor,
//! history, issues, and remediation engines as well as both shells. These
//! files previously lived in `src-tauri/src` and were compiled a second,
//! third, and fourth time into each consumer through `#[path]` includes,
//! which minted structurally identical but *distinct* types (four separate
//! `Timestamp`s). Compiling them once here makes those types the same type.
//!
//! - [`error`] — [`DiagError`](error::DiagError), the JSON-serializable
//!   error enum every engine surface returns.
//! - [`timestamp`] — a dependency-free UTC [`Timestamp`](timestamp::Timestamp)
//!   that serializes as ISO 8601, plus CIM/WMI datetime parsing.
//! - [`fs_atomic`] — crash-safe file writes (private staging file, fsync,
//!   atomic replace) used by every durable store.
//! - [`security`] — the trusted-program allowlist, the whitelisted
//!   [`SecureCommandExecutor`](security::SecureCommandExecutor), and OEM/UTF-16
//!   console-output decoding.
//! - `wmi` (Windows only) — the native WMI wrapper built on COM
//!   (`IWbemLocator`/`IWbemServices`) rather than the `wmi` crate.

pub mod error;
pub mod fs_atomic;
pub mod security;
pub mod timestamp;

// FFI-heavy module: `unsafe` stays scoped to it, the workspace denies
// `unsafe_code` everywhere else.
#[cfg(windows)]
#[allow(unsafe_code)]
pub mod wmi;
