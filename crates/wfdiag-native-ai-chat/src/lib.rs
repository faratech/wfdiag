//! UI-framework-neutral AI chat state, tool loop, and event contracts.
//!
//! The native Reactor shell and the shipping Tauri shell both use this crate
//! for canonical conversation state, provider-neutral request/response types,
//! bounded tool execution, cancellation, and history projection. UI shells
//! supply only provider, tool, and event adapters. This crate has no `Tauri`,
//! `Wry`, `WebView`, or `Reactor` dependency.

#![deny(unsafe_code)]
#![allow(clippy::missing_errors_doc)]

mod contract;
mod engine;
mod model;

pub use contract::*;
pub use engine::*;
pub use model::*;
