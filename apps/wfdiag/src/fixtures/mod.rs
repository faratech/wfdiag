//! Deterministic visual fixtures for the shell's validation and QA modes.
//!
//! Fixture *data* ([`issues`], the data constructors in [`visual`]) is
//! compiled only with the `validation` feature. The two plumbing enums
//! (`VisualState`, `LiveTestFixture`) stay in every build so their pure
//! parsers keep the production default on one code path.
//!
//! # Environment access is forbidden outside [`knobs`] (#186, #212)
//!
//! [`knobs`] is the ONE module in `apps/wfdiag` allowed to call
//! `std::env::var`, `std::env::var_os`, or `std::env::args*`, and every such
//! call there is compiled out unless the `validation` cargo feature is on. Do
//! not reintroduce an environment read anywhere else in the shell: add a knob
//! accessor here instead, so a shipping build keeps its "no knobs at all"
//! shape. The only allowed exceptions live in `main.rs`
//! (`--wfdiag-elevated-relaunch`, real production behaviour) and
//! `platform/crash.rs` (`%LOCALAPPDATA%` for the crash-log directory); both
//! are documented at their call sites. Engine crates own their own variables
//! and are out of scope for this rule.

#[cfg(feature = "validation")]
pub(crate) mod issues;
pub(crate) mod knobs;
pub(crate) mod visual;

#[cfg(feature = "validation")]
pub(crate) use issues::fixture_258_issues;
