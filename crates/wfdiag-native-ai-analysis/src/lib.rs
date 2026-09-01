//! UI-framework-neutral one-shot AI analysis, issue prioritization, fix-plan
//! generation, and their prompt/budget policy for `WFDiag`.
//!
//! Each worker owns a standard thread with a persistent Tokio runtime. A shell
//! submits immutable snapshots and drains typed events over `std::sync::mpsc`;
//! credential resolution, optional `WindowsForum` grounding, cache access, and
//! provider calls never run on the UI thread. Nothing here knows about
//! `Tauri`, `Wry`, `WebView`, or `Reactor` — shells supply a wake callback and
//! the provider/settings ports.

#![deny(unsafe_code)]

mod analysis;
mod fix_plan;
pub mod prompts;

use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

pub use analysis::{
    AnalysisCacheIdentity, AnalysisRoute, AnalysisWorkerEvent, DiagnosticAnalysisGeneration,
    IssuePrioritizationGeneration, NativeAnalysisRuntime, diagnostic_output_hash,
    one_shot_data_budget, one_shot_effective_data_budget, one_shot_grounding_budget,
};
pub use fix_plan::{
    FixPlanEntry, FixPlanGeneration, FixPlanRoute, FixPlanWorkerEvent, NativeFixPlanRuntime,
    PLAN_SYSTEM, ValidatedFixPlan, initial_fix_plan_route,
};
pub use wfdiag_native_ai_chat::{GroundingTrace, GroundingTraceSource};

/// Thread-safe UI notification invoked after a worker event is queued.
///
/// This replaces the native shell's process-global wake hook: the runtimes are
/// portable library code, so the shell passes its own callback at `start`.
pub type WakeHandler = Arc<dyn Fn() + Send + Sync + 'static>;

/// Queue one worker event and, only on successful delivery, schedule a UI
/// wake. A disconnected channel must not wake a component that can no longer
/// receive the event.
fn send_event<T>(events: &Sender<T>, wake: &WakeHandler, event: T) -> bool {
    if events.send(event).is_err() {
        return false;
    }
    wake();
    true
}

/// Join `worker` on a detached thread rather than the calling (UI) thread.
///
/// An in-flight request that ignores cancellation (a hung vendor CLI, a slow
/// provider probe) must not extend graceful close.
fn reap_worker(worker: JoinHandle<()>) {
    let _ = std::thread::Builder::new()
        .name("wfdiag-ai-analysis-reaper".to_string())
        .spawn(move || {
            let _ = worker.join();
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    #[test]
    fn delivery_wakes_the_shell_and_a_dead_channel_does_not() {
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&woken);
        let wake: WakeHandler = Arc::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });

        let (sender, receiver) = mpsc::channel();
        assert!(send_event(&sender, &wake, 42_u8));
        assert_eq!(receiver.recv().unwrap(), 42);
        assert_eq!(woken.load(Ordering::Relaxed), 1);

        drop(receiver);
        assert!(!send_event(&sender, &wake, 7_u8));
        assert_eq!(woken.load(Ordering::Relaxed), 1);
    }
}
