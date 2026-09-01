//! Compatibility shim: deterministic evidence assembly now lives in
//! `wfdiag-native-ai-report` so both shells compile one implementation.

pub use wfdiag_native_ai_report::evidence::*;
