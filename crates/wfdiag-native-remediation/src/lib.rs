//! UI-framework-neutral remediation execution.
//!
//! The vetted, tiered catalog (OpenTool | AutoSafe | Repair), the injectable
//! command runner, and the tier confirm gate compile once here and are used
//! identically by the shipping Tauri backend and the native Reactor shell.
//! The included module is the shipping backend's own source; edit it there.

// The included shipping module legitimately contains `unsafe` (the recycle
// bin WinRT/Win32 call); src-tauri's lint policy applies there.
#![allow(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]

use wfdiag_remediation_catalog as remediation_catalog;

pub use remediation_catalog::{RemediationMetadata, RemediationSummary, RemediationTier};

// Included verbatim from the shipping backend. Its `crate::security` and
// `crate::issue_catalog` references resolve through the shims below.
#[path = "../../../src-tauri/src/remediation.rs"]
pub mod remediation;

/// Adapter shim: the trusted-program allowlist and console-output decoding
/// the real runner uses.
#[path = "../../../src-tauri/src/security.rs"]
pub mod security;

/// Test-only shim: one regression test asserts the catalog's remediation
/// mapping stays exhaustive against the canonical issue catalog.
#[cfg(test)]
pub(crate) mod issue_catalog {
    pub use wfdiag_native_issues::catalog;
}
