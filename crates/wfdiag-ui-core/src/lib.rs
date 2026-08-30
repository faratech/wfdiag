//! Framework-neutral UI contracts and event delivery for `WFDiag`.
//!
//! The event bus deliberately separates lossless UI events from high-frequency
//! snapshots. See [`event_bus`] for the delivery guarantees.

#![forbid(unsafe_code)]

pub mod contract;
pub mod event_bus;

pub use contract::*;
pub use event_bus::*;
