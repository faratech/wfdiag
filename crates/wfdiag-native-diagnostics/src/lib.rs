//! Native diagnostic collection and UI-framework-neutral scan orchestration.
//!
//! The coordinator is portable and executor-driven. The Windows executor
//! reuses `WFDiag`'s existing native collectors without taking a dependency on
//! Tauri, Wry, `WebView2`, or Windows Reactor.

#![deny(unsafe_code)]

mod runtime;

pub use runtime::*;

// FFI-heavy collectors keep `unsafe` scoped to themselves; the workspace (and
// this crate) denies `unsafe_code` everywhere else.
#[cfg(windows)]
#[allow(unsafe_code)]
pub mod native_diagnostics;

#[cfg(windows)]
pub mod catalog;

#[cfg(windows)]
pub use wfdiag_native_monitor as native_monitor;

#[cfg(windows)]
mod windows_executor {
    use super::{DiagnosticExecutor, DiagnosticFuture, DiagnosticOutput, DiagnosticTask, catalog};

    /// Runs the same native diagnostic collectors used by the shipping Tauri
    /// backend, but without linking any desktop UI framework.
    #[derive(Debug, Default)]
    pub struct NativeDiagnosticExecutor;

    impl DiagnosticExecutor for NativeDiagnosticExecutor {
        fn available_tasks(&self) -> Vec<DiagnosticTask> {
            catalog::get_all_tasks()
                .into_iter()
                .map(|task| DiagnosticTask {
                    id: task.id,
                    name: task.name,
                    description: task.description,
                    category: task.category,
                    admin_required: task.admin_required,
                })
                .collect()
        }

        fn execute(&self, task_id: String) -> DiagnosticFuture<'_> {
            Box::pin(async move {
                let result = catalog::run_diagnostic_task(&task_id).await;
                DiagnosticOutput {
                    success: result.success,
                    output: result.output,
                    error: result.error,
                    duration_ms: result.duration_ms,
                }
            })
        }
    }
}

#[cfg(windows)]
pub use windows_executor::NativeDiagnosticExecutor;
