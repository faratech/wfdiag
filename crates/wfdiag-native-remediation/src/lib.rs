//! UI-framework-neutral remediation: catalog, approval broker, run runtime.
//!
//! The vetted, tiered catalog (`OpenTool` | `AutoSafe` | Repair), the
//! injectable command runner, the approval gate, and the worker/run projection
//! compile once here and are used identically by the shipping Tauri backend
//! and the native Reactor shell.
//!
//! SECURITY (#185): [`remediation::execute_authorized`] is `pub(crate)`. Every
//! caller outside this crate must go through [`broker::RealCatalogExecutor`],
//! which only accepts a [`broker::AuthorizedAction`] borrowed from an
//! [`broker::ActionGrant`], and [`broker::ActionBroker::authorize`] refuses to
//! mint a grant over a `RemediationTier::Repair` preview without
//! [`broker::ActionApproval::RepairConfirmed`].

#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]

use wfdiag_remediation_catalog as remediation_catalog;

pub use remediation_catalog::{RemediationMetadata, RemediationSummary, RemediationTier};

/// The catalog and the execution engine.
pub mod remediation;

/// Proposal staging, fingerprints, and the one-use approval grant.
pub mod broker;

/// Workers, run projection, and cancellation over the broker's grants.
pub mod runtime;

/// Administrator relaunch via the `runas` verb.
#[allow(unsafe_code)]
pub mod elevation;
