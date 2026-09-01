//! AI prompt templates.
//!
//! The builders moved to `wfdiag_native_ai_analysis::prompts` so the native
//! shell and the Tauri backend share one copy of the diagnostic prompt, JSON
//! compaction, and Unicode-safe budget logic.
pub use wfdiag_native_ai_analysis::prompts::*;
