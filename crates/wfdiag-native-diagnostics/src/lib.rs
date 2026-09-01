//! Native diagnostic collection and UI-framework-neutral scan orchestration.
//!
//! The coordinator is portable and executor-driven. The Windows executor
//! reuses `WFDiag`'s existing native collectors without taking a dependency on
//! Tauri, Wry, `WebView2`, or Windows Reactor.

#![deny(unsafe_code)]

mod runtime;

pub use runtime::*;

#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "../../../src-tauri/src/native_diagnostics.rs"]
mod native_diagnostics;

// The shared primitives compile once in `wfdiag-native-core`. The
// `#[path]`-included shipping modules above and below still spell them
// `crate::security` / `crate::timestamp` / `crate::wmi_native`, so those names
// stay, pointing at the one implementation.
#[cfg(windows)]
#[allow(unused_imports)]
mod security {
    pub use wfdiag_native_core::security::*;
}
#[cfg(windows)]
#[allow(unused_imports)]
mod timestamp {
    pub use wfdiag_native_core::timestamp::*;
}
#[cfg(windows)]
#[allow(unused_imports)]
mod wmi_native {
    pub use wfdiag_native_core::wmi::*;
}

#[cfg(windows)]
pub use wfdiag_native_monitor as native_monitor;

#[cfg(windows)]
#[allow(dead_code, unsafe_code)]
#[path = "../../../src-tauri/src/diagnostics.rs"]
mod diagnostics_impl;

#[cfg(windows)]
mod windows_executor {
    use super::{
        DiagnosticExecutor, DiagnosticFuture, DiagnosticOutput, DiagnosticTask, diagnostics_impl,
    };

    /// Runs the same native diagnostic collectors used by the shipping Tauri
    /// backend, but without linking any desktop UI framework.
    #[derive(Debug, Default)]
    pub struct NativeDiagnosticExecutor;

    impl DiagnosticExecutor for NativeDiagnosticExecutor {
        fn available_tasks(&self) -> Vec<DiagnosticTask> {
            diagnostics_impl::get_all_tasks()
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
                let result = diagnostics_impl::run_diagnostic_task(&task_id).await;
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
