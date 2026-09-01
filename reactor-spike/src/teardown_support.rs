//! Shared teardown policy for the native worker runtimes.
//!
//! Workers own blocking receive loops and (in several cases) blocking
//! `block_on` sections that cannot observe cancellation until their current
//! step finishes. Joining such a worker on the UI thread therefore risks
//! stalling graceful close for as long as the slowest in-flight operation
//! (a provider probe, a vendor CLI, a system remediation step). Every runtime
//! `Drop` releases its command sender FIRST so the worker's receive loop
//! disconnects, cancels any active request without holding the slot lock, and
//! then reaps the join handle here instead of joining inline.

use std::thread::JoinHandle;

/// Join `worker` on a detached thread rather than the calling (UI) thread.
pub(crate) fn reap_worker(worker: JoinHandle<()>) {
    let _ = std::thread::Builder::new()
        .name("wfdiag-reactor-reaper".to_string())
        .spawn(move || {
            let _ = worker.join();
        });
}
