//! Deterministic visual fixtures for the shell's validation and QA modes.
//!
//! Nothing here participates in a production run: every item is reachable only
//! from the deterministic-visual code paths, so the whole tree can later be
//! gated behind a cargo feature by cfg-ing this one module declaration.

pub(crate) mod issues;

pub(crate) use issues::fixture_258_issues;
