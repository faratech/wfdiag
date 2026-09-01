//! Pure state machines shared by every shell.
//!
//! Nothing in this module performs I/O, reads the clock, or touches a worker
//! runtime: every environmental input is a parameter. That is what makes the
//! scan transaction, the issue guard, and the startup gate unit-testable on a
//! Linux CI box with no Windows and no GUI.

pub mod actions;
pub mod ai_intent;
pub mod catalog;
pub mod consent;
pub mod history;
pub mod invalidation;
pub mod issues;
pub mod providers;
pub mod scan;
pub mod startup;
pub mod subscriptions;
pub mod update;
