//! The `WFDiag` application service: one facade over every engine runtime.
//!
//! # Why this crate exists
//!
//! The native `WinUI` shell owned seventeen worker runtimes directly, each with
//! its own paired receiver, wait task, request id, pending flag, and error
//! slot, and each re-implementing its own staleness guard. That is where the
//! shell's hardest bugs lived: a stale reply overwriting newer evidence, a
//! rollback that forgot one projection, a scan that hung because nothing timed
//! a worker out. The owner's requirement is that the engine must drive *any*
//! GUI, or none at all.
//!
//! [`AppService`] is that engine. It owns every runtime, takes one
//! [`AppCommand`] in, and produces one [`AppEvent`] stream out. Every
//! staleness comparison happens inside [`AppService::drain`], in one place,
//! against the newtypes in [`ids`]; a host never holds a request id and never
//! compares one.
//!
//! # Threading model
//!
//! * The core is **single-threaded and host-owned**. [`AppService::dispatch`]
//!   and [`AppService::drain`] take `&mut self`, so the service lives on one
//!   thread — the UI thread, for a GUI shell.
//! * `dispatch` **never blocks**. It validates against local state machines,
//!   hands work to a worker or to the scan executor's Tokio runtime, and
//!   returns a [`DispatchOutcome`] synchronously.
//! * Workers **wake** the host through the callback installed with
//!   [`AppEventReceiver::set_wake_handler`]. The diagnostic and monitor event
//!   buses and the settings runtime call it themselves; the crates that answer
//!   on a bare `oneshot` are covered by one watcher thread that ticks every
//!   50 ms *only while a reply is outstanding*.
//! * `drain` is the only reader of worker output, the only writer of
//!   [`AppSnapshot`], and the only judge of staleness.
//!
//! # Headless
//!
//! Everything environmental is a port ([`AppPorts`]). [`AppPorts::mock`] builds
//! a complete in-memory bundle, so the integration tests in `tests/` drive the
//! real service — real workers, real threads, real guards — on Linux with no
//! Windows and no GUI. `wfdiag-native-monitor` is a `#![cfg(windows)]` crate
//! and is reached only through [`ports::monitor::MonitorPort`], whose
//! [`ports::monitor::NoopMonitor`] answers off Windows.
//!
//! # Deviations worth knowing
//!
//! * The host wake callback is [`AppWakeHandler`], not `wfdiag_ui_core::UiWakeHandler`:
//!   the latter's invoke method is private to that crate, so it can only be
//!   handed *to* `ui-core`, never called by this one. The service installs a
//!   `UiWakeHandler` on the event buses that forwards into the same queue.
//! * The shell's 500 ms cosmetic pause between "scan complete" and "history
//!   saved" is not reproduced: it is presentation, and a headless host should
//!   not wait for it.

#![deny(unsafe_code)]
#![deny(missing_docs)]

pub mod command;
pub mod config;
pub mod domain;
pub mod event;
pub mod ids;
pub mod ports;
mod replies;
pub mod service;
pub mod snapshot;
mod workers;

pub use command::{
    AppCommand, DispatchOutcome, ProviderCredentialCommand, RejectReason, UpdateCheckReason,
    WorkerKind,
};
pub use config::AppConfig;
pub use event::{
    AppEvent, AppEventReceiver, AppWakeHandler, ExportEvent, ExtensionEvent, HistoryEvent,
    HistoryRequest, IssuesEvent, MonitorEvent, ProviderEvent, ScanEvent, SettingsEvent,
    SystemEvent, UpdateEvent,
};
pub use ids::{Epoch, Generation, RequestId};
pub use ports::{AppPorts, ElevationPort, EnvironmentPort, UpdateThrottlePort};
pub use service::{AppService, AppStartError, ShutdownReport};
pub use snapshot::{
    AppSnapshot, HistorySnapshot, MonitorSnapshot, UpdateSnapshot, WorkerUnavailable,
};
pub use workers::WorkerStopRecord;
