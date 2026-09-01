//! Pure, UI-neutral projections shared by `WFDiag` shells.
//!
//! Everything here is framework-neutral and host-portable: no `windows`,
//! no `windows_reactor`, no async runtime. That keeps the observable
//! contracts of the History diff, the Markdown-lite parser and its link
//! policy, process-identity reconciliation, and monitor graph geometry
//! testable on every platform the engine builds for.

pub mod json_diff;
pub mod markdown;
pub mod process_identity;
pub mod render;
