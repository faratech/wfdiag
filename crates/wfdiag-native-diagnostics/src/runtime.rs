use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::Mutex;
use uuid::Uuid;
use wfdiag_ui_core::{
    DiagnosticTaskResult, TaskProgress, TaskProgressStatus, UiEvent, UiEventReceiver, UiEventSink,
    ui_event_bus,
};

/// Metadata used to validate a requested diagnostic scan before it replaces
/// the current usable session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticTask {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub admin_required: bool,
}

/// Framework-neutral output from one diagnostic executor invocation.
/// This is also the canonical input consumed by native issue detection.
pub use wfdiag_native_issues::TaskResult as DiagnosticOutput;

pub type DiagnosticFuture<'a> = Pin<Box<dyn Future<Output = DiagnosticOutput> + Send + 'a>>;

/// Injectable task runner used by the portable scan coordinator.
pub trait DiagnosticExecutor: Send + Sync {
    fn available_tasks(&self) -> Vec<DiagnosticTask>;
    fn execute(&self, task_id: String) -> DiagnosticFuture<'_>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanKind {
    Quick,
    Full,
    #[default]
    Targeted,
}

/// Durable in-memory evidence for the current scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticSession {
    pub session_id: String,
    pub start_time: SystemTime,
    #[serde(default)]
    pub scan_kind: ScanKind,
    pub selected_tasks: Vec<String>,
    pub results: HashMap<String, DiagnosticOutput>,
}

#[derive(Debug, Default)]
struct CoordinatorState {
    current: Option<DiagnosticSession>,
    previous: Option<DiagnosticSession>,
    cancelled: HashSet<String>,
    active_runners: HashSet<String>,
}

/// Complete results returned after a scan runner drains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticRunResult {
    pub session_id: String,
    pub completed: Vec<(String, DiagnosticOutput)>,
    pub cancelled: bool,
}

/// Errors from validation or session ownership checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticRuntimeError {
    NoValidTasks,
    NoActiveSession,
    SessionMismatch { expected: String, actual: String },
    AlreadyRunning { session_id: String },
}

impl std::fmt::Display for DiagnosticRuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoValidTasks => formatter.write_str("no valid diagnostic tasks were provided"),
            Self::NoActiveSession => formatter.write_str("no active diagnostic session"),
            Self::SessionMismatch { expected, actual } => {
                write!(
                    formatter,
                    "session id mismatch: expected {expected}, got {actual}"
                )
            }
            Self::AlreadyRunning { session_id } => {
                write!(
                    formatter,
                    "diagnostics are already running for session {session_id}"
                )
            }
        }
    }
}

impl std::error::Error for DiagnosticRuntimeError {}

/// UI-framework-neutral diagnostic session coordinator.
///
/// The UI owns the receiver and may drain it from a `DispatcherQueue` callback.
/// The coordinator owns no runtime and can be driven from the application's
/// existing Tokio worker.
#[derive(Clone)]
pub struct DiagnosticRuntime {
    executor: Arc<dyn DiagnosticExecutor>,
    sink: Box<dyn UiEventSink>,
    state: Arc<Mutex<CoordinatorState>>,
}

impl std::fmt::Debug for DiagnosticRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DiagnosticRuntime")
            .finish_non_exhaustive()
    }
}

impl DiagnosticRuntime {
    /// Create a coordinator over an arbitrary event sink.
    #[must_use]
    pub fn new(executor: Arc<dyn DiagnosticExecutor>, sink: Box<dyn UiEventSink>) -> Self {
        Self {
            executor,
            sink,
            state: Arc::new(Mutex::new(CoordinatorState::default())),
        }
    }

    /// Create a coordinator and its native UI event receiver.
    #[must_use]
    pub fn with_event_bus(
        executor: Arc<dyn DiagnosticExecutor>,
        lossless_capacity: NonZeroUsize,
    ) -> (Self, UiEventReceiver) {
        let (publisher, receiver) = ui_event_bus(lossless_capacity);
        (Self::new(executor, Box::new(publisher)), receiver)
    }

    /// Return the executor's current task catalog for native task selection.
    #[must_use]
    pub fn available_tasks(&self) -> Vec<DiagnosticTask> {
        self.executor.available_tasks()
    }

    /// Validate a task selection and transactionally replace the visible
    /// session while retaining the previous usable evidence for rollback.
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticRuntimeError::NoValidTasks`] when none of the
    /// requested identifiers exist in the executor's task catalog.
    pub async fn start_session(
        &self,
        task_ids: Vec<String>,
        scan_kind: ScanKind,
    ) -> Result<String, DiagnosticRuntimeError> {
        let available: HashSet<String> = self
            .executor
            .available_tasks()
            .into_iter()
            .map(|task| task.id)
            .collect();
        let selected_tasks: Vec<String> = task_ids
            .into_iter()
            .filter(|task_id| available.contains(task_id))
            .collect();
        if selected_tasks.is_empty() {
            return Err(DiagnosticRuntimeError::NoValidTasks);
        }

        let session_id = format!("scan_{}", Uuid::new_v4().simple());
        let session = DiagnosticSession {
            session_id: session_id.clone(),
            start_time: SystemTime::now(),
            scan_kind,
            selected_tasks,
            results: HashMap::new(),
        };

        let mut state = self.state.lock().await;
        state.previous = state.current.replace(session);
        Ok(session_id)
    }

    /// Run the tasks selected when the session was created.
    ///
    /// Queued tasks observe cancellation before execution; already-running
    /// tasks finish. Only results belonging to the still-current session may
    /// mutate its evidence, preventing a draining old scan from contaminating
    /// a newly started one.
    ///
    /// # Errors
    ///
    /// Returns a session ownership error when the requested session is not
    /// current, or [`DiagnosticRuntimeError::AlreadyRunning`] when it already
    /// has an active runner.
    #[allow(clippy::too_many_lines)]
    pub async fn run_session(
        &self,
        session_id: String,
        max_concurrent: usize,
    ) -> Result<DiagnosticRunResult, DiagnosticRuntimeError> {
        let selected_tasks = {
            let mut state = self.state.lock().await;
            let session = state
                .current
                .as_ref()
                .ok_or(DiagnosticRuntimeError::NoActiveSession)?;
            if session.session_id != session_id {
                return Err(DiagnosticRuntimeError::SessionMismatch {
                    expected: session.session_id.clone(),
                    actual: session_id,
                });
            }
            let selected = session.selected_tasks.clone();
            if !state.active_runners.insert(session_id.clone()) {
                return Err(DiagnosticRuntimeError::AlreadyRunning { session_id });
            }
            selected
        };

        let max_concurrent = max_concurrent.clamp(1, 16);
        let task_metadata: Arc<HashMap<String, DiagnosticTask>> = Arc::new(
            self.executor
                .available_tasks()
                .into_iter()
                .map(|task| (task.id.clone(), task))
                .collect(),
        );

        let futures = selected_tasks.into_iter().map(|task_id| {
            let runtime = self.clone();
            let task_metadata = Arc::clone(&task_metadata);
            let session_id = session_id.clone();
            async move {
                if runtime.state.lock().await.cancelled.contains(&session_id) {
                    return None;
                }

                let Some(task) = task_metadata.get(&task_id) else {
                    let _ = runtime
                        .publish_progress(
                            &session_id,
                            &task_id,
                            TaskProgressStatus::Failed,
                            None,
                            Some(false),
                        )
                        .await;
                    return None;
                };

                if runtime
                    .publish_progress(
                        &session_id,
                        &task_id,
                        TaskProgressStatus::Running,
                        Some(task.name.clone()),
                        None,
                    )
                    .await
                    .is_err()
                {
                    return None;
                }

                let output = runtime.executor.execute(task_id.clone()).await;

                {
                    let mut state = runtime.state.lock().await;
                    if let Some(session) = state.current.as_mut()
                        && session.session_id == session_id
                    {
                        session.results.insert(task_id.clone(), output.clone());
                    }
                }

                let result_event = UiEvent::DiagnosticResult(DiagnosticTaskResult {
                    session_id: session_id.clone(),
                    task_id: task_id.clone(),
                    success: output.success,
                    output: output.output.clone(),
                    error: output.error.clone(),
                    duration_ms: output.duration_ms,
                });
                if runtime.sink.publish(result_event).await.is_err() {
                    return None;
                }

                if runtime
                    .publish_progress(
                        &session_id,
                        &task_id,
                        TaskProgressStatus::Completed,
                        None,
                        Some(output.success),
                    )
                    .await
                    .is_err()
                {
                    return None;
                }

                Some((task_id, output))
            }
        });

        let completed: Vec<(String, DiagnosticOutput)> = stream::iter(futures)
            .buffer_unordered(max_concurrent)
            .filter_map(async |result| result)
            .collect()
            .await;

        let cancelled = {
            let mut state = self.state.lock().await;
            let cancelled = state.cancelled.remove(&session_id);
            state.active_runners.remove(&session_id);
            if state
                .current
                .as_ref()
                .is_some_and(|session| session.session_id == session_id)
            {
                let incomplete = state.current.as_ref().is_some_and(|session| {
                    session
                        .selected_tasks
                        .iter()
                        .any(|task_id| !session.results.contains_key(task_id))
                });
                if incomplete {
                    state.current = state.previous.take();
                } else {
                    state.previous = None;
                }
            }
            cancelled
        };

        Ok(DiagnosticRunResult {
            session_id,
            completed,
            cancelled,
        })
    }

    /// Request task-granular cancellation and immediately restore the prior
    /// complete scan if the requested session is the visible replacement.
    ///
    /// # Errors
    ///
    /// Returns a session ownership error when no matching current session
    /// exists.
    pub async fn cancel_session(&self, session_id: &str) -> Result<(), DiagnosticRuntimeError> {
        let mut state = self.state.lock().await;
        let Some(current) = state.current.as_ref() else {
            return Err(DiagnosticRuntimeError::NoActiveSession);
        };
        if current.session_id != session_id {
            return Err(DiagnosticRuntimeError::SessionMismatch {
                expected: current.session_id.clone(),
                actual: session_id.to_string(),
            });
        }

        state.cancelled.insert(session_id.to_string());
        let runner_active = state.active_runners.contains(session_id);
        let incomplete = state.current.as_ref().is_some_and(|session| {
            session
                .selected_tasks
                .iter()
                .any(|task_id| !session.results.contains_key(task_id))
        });
        if runner_active || incomplete {
            state.current = state.previous.take();
        }
        Ok(())
    }

    /// Return a snapshot only when the caller still owns the visible session.
    ///
    /// # Errors
    ///
    /// Returns a session ownership error when no matching current session
    /// exists.
    pub async fn session_results(
        &self,
        session_id: &str,
    ) -> Result<HashMap<String, DiagnosticOutput>, DiagnosticRuntimeError> {
        let state = self.state.lock().await;
        let current = state
            .current
            .as_ref()
            .ok_or(DiagnosticRuntimeError::NoActiveSession)?;
        if current.session_id != session_id {
            return Err(DiagnosticRuntimeError::SessionMismatch {
                expected: current.session_id.clone(),
                actual: session_id.to_string(),
            });
        }
        Ok(current.results.clone())
    }

    #[must_use]
    pub async fn current_session(&self) -> Option<DiagnosticSession> {
        self.state.lock().await.current.clone()
    }

    async fn publish_progress(
        &self,
        session_id: &str,
        task_id: &str,
        status: TaskProgressStatus,
        task_name: Option<String>,
        success: Option<bool>,
    ) -> Result<(), ()> {
        self.sink
            .publish(UiEvent::TaskProgress(TaskProgress {
                session_id: session_id.to_string(),
                task_id: task_id.to_string(),
                status,
                task_name,
                success,
            }))
            .await
            .map(|_| ())
            .map_err(|_| ())
    }
}

/// Windows native coordinator constructor kept separate so portable tests do
/// not need a Windows target.
#[cfg(windows)]
pub struct NativeDiagnosticRuntime;

#[cfg(windows)]
impl NativeDiagnosticRuntime {
    /// Create native collectors and a typed receiver for the owning UI thread.
    #[must_use]
    pub fn start(lossless_capacity: NonZeroUsize) -> (DiagnosticRuntime, UiEventReceiver) {
        DiagnosticRuntime::with_event_bus(
            Arc::new(crate::NativeDiagnosticExecutor),
            lossless_capacity,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use tokio::sync::Semaphore;

    struct FakeExecutor {
        tasks: Vec<DiagnosticTask>,
        executed: Arc<StdMutex<Vec<String>>>,
        slow_started: Arc<Semaphore>,
        slow_release: Arc<Semaphore>,
    }

    impl FakeExecutor {
        fn new() -> Self {
            Self {
                tasks: ["base", "slow", "queued"]
                    .into_iter()
                    .map(|id| DiagnosticTask {
                        id: id.to_string(),
                        name: format!("Task {id}"),
                        description: format!("Test task {id}"),
                        category: "Test".to_string(),
                        admin_required: false,
                    })
                    .collect(),
                executed: Arc::new(StdMutex::new(Vec::new())),
                slow_started: Arc::new(Semaphore::new(0)),
                slow_release: Arc::new(Semaphore::new(0)),
            }
        }
    }

    impl DiagnosticExecutor for FakeExecutor {
        fn available_tasks(&self) -> Vec<DiagnosticTask> {
            self.tasks.clone()
        }

        fn execute(&self, task_id: String) -> DiagnosticFuture<'_> {
            Box::pin(async move {
                self.executed
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(task_id.clone());
                if task_id == "slow" {
                    self.slow_started.add_permits(1);
                    self.slow_release
                        .acquire()
                        .await
                        .expect("test release semaphore remains open")
                        .forget();
                }
                DiagnosticOutput {
                    success: true,
                    output: format!("{{\"task\":\"{task_id}\"}}"),
                    error: None,
                    duration_ms: 7,
                }
            })
        }
    }

    fn capacity() -> NonZeroUsize {
        NonZeroUsize::new(32).expect("test capacity is non-zero")
    }

    #[tokio::test]
    async fn validates_selection_and_delivers_complete_results_without_a_ui_framework() {
        let executor = Arc::new(FakeExecutor::new());
        let (runtime, receiver) = DiagnosticRuntime::with_event_bus(executor, capacity());

        assert_eq!(
            runtime
                .start_session(vec!["missing".into()], ScanKind::Quick)
                .await,
            Err(DiagnosticRuntimeError::NoValidTasks)
        );

        let session_id = runtime
            .start_session(vec!["base".into(), "missing".into()], ScanKind::Quick)
            .await
            .unwrap();
        let run = runtime.run_session(session_id.clone(), 1).await.unwrap();
        assert_eq!(run.completed.len(), 1);
        assert!(!run.cancelled);
        assert_eq!(
            runtime.session_results(&session_id).await.unwrap()["base"].output,
            "{\"task\":\"base\"}"
        );

        assert_eq!(
            receiver.drain(),
            vec![
                UiEvent::DiagnosticResult(DiagnosticTaskResult {
                    session_id: session_id.clone(),
                    task_id: "base".into(),
                    success: true,
                    output: "{\"task\":\"base\"}".into(),
                    error: None,
                    duration_ms: 7,
                }),
                UiEvent::TaskProgress(TaskProgress {
                    session_id,
                    task_id: "base".into(),
                    status: TaskProgressStatus::Completed,
                    task_name: None,
                    success: Some(true),
                }),
            ]
        );
    }

    #[tokio::test]
    async fn cancellation_restores_previous_evidence_and_skips_queued_tasks() {
        let executor = Arc::new(FakeExecutor::new());
        let executed = Arc::clone(&executor.executed);
        let started = Arc::clone(&executor.slow_started);
        let release = Arc::clone(&executor.slow_release);
        let (runtime, receiver) = DiagnosticRuntime::with_event_bus(executor, capacity());

        let previous_id = runtime
            .start_session(vec!["base".into()], ScanKind::Quick)
            .await
            .unwrap();
        runtime.run_session(previous_id.clone(), 1).await.unwrap();
        let _ = receiver.drain();

        let replacement_id = runtime
            .start_session(vec!["slow".into(), "queued".into()], ScanKind::Full)
            .await
            .unwrap();
        let running = {
            let runtime = runtime.clone();
            let session_id = replacement_id.clone();
            tokio::spawn(async move { runtime.run_session(session_id, 1).await })
        };

        started
            .acquire()
            .await
            .expect("slow task announces start")
            .forget();
        runtime.cancel_session(&replacement_id).await.unwrap();
        release.add_permits(1);

        let result = running.await.unwrap().unwrap();
        assert!(result.cancelled);
        assert_eq!(
            runtime.current_session().await.unwrap().session_id,
            previous_id
        );
        assert_eq!(
            executed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_slice(),
            ["base", "slow"]
        );
    }

    #[tokio::test]
    async fn rejects_a_second_runner_for_the_same_session() {
        let executor = Arc::new(FakeExecutor::new());
        let started = Arc::clone(&executor.slow_started);
        let release = Arc::clone(&executor.slow_release);
        let (runtime, _receiver) = DiagnosticRuntime::with_event_bus(executor, capacity());
        let session_id = runtime
            .start_session(vec!["slow".into()], ScanKind::Targeted)
            .await
            .unwrap();

        let first = {
            let runtime = runtime.clone();
            let session_id = session_id.clone();
            tokio::spawn(async move { runtime.run_session(session_id, 1).await })
        };
        started
            .acquire()
            .await
            .expect("slow task announces start")
            .forget();

        assert_eq!(
            runtime.run_session(session_id.clone(), 1).await,
            Err(DiagnosticRuntimeError::AlreadyRunning {
                session_id: session_id.clone()
            })
        );
        release.add_permits(1);
        first.await.unwrap().unwrap();
    }
}
