//! Native remediation action runtime for the Reactor shell.
//!
//! The UI can only stage catalog IDs into an immutable preview and later approve
//! that preview by its opaque ID. The worker owns the proposal store, consumes
//! each grant exactly once, revalidates current scan/catalog/issue evidence at
//! the execution boundary, and is the only place that can reach `RealRunner`.

#![deny(unsafe_code)]

use std::collections::{HashMap, HashSet, VecDeque, hash_map::RandomState};
use std::future::Future;
use std::hash::{BuildHasher, Hash, Hasher};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio_util::sync::CancellationToken;
use wfdiag_native_remediation::remediation::{
    FixCompletionStatus, FixResult, RealRunner, RemediationSummary, RemediationTier,
    execute_authorized, find, remediations,
};

use crate::ui_wake_support::NotifySenderExt;

pub const ACTION_PROPOSAL_TTL_MS: u64 = 10 * 60 * 1_000;
pub const MAX_BATCH_ACTIONS: usize = 5;
const MAX_PENDING_PROPOSALS: usize = 100;
const MAX_ACTION_HISTORY: usize = 50;
const ACTION_CATALOG_SCHEMA_VERSION: u32 = 1;

/// One catalog action requested by an app-owned UI flow or validated AI plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionRequest {
    pub remediation_id: String,
    pub issue_id: Option<String>,
}

/// A detected issue and its sole catalog-authorized remediation, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedIssueRemediation {
    pub issue_id: String,
    pub remediation_id: Option<String>,
}

/// Authoritative app snapshot captured at a prepare or approve boundary.
///
/// Callers must rebuild this from the currently committed diagnostic session;
/// a proposal never carries mutable references back into component state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionSnapshot {
    pub scan_fingerprint: String,
    pub catalog_fingerprint: String,
    pub detected_issues: Vec<DetectedIssueRemediation>,
    pub is_admin: bool,
}

/// Preparation input. Optional expected fingerprints bind an AI fix plan to
/// the evidence/catalog versions from which it was generated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionPrepareInput {
    pub actions: Vec<ActionRequest>,
    pub expected_scan_fingerprint: Option<String>,
    pub expected_catalog_fingerprint: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalScope {
    Exact,
    Batch,
}

/// Immutable, catalog-derived data that the UI must review before approval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionPreview {
    pub remediation: RemediationSummary,
    pub issue_id: Option<String>,
    pub steps: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionProposal {
    pub proposal_id: String,
    pub approval_scope: ApprovalScope,
    pub actions: Vec<ActionPreview>,
    pub scan_fingerprint: String,
    pub catalog_fingerprint: String,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

/// Repair proposals require the caller to enter the explicit confirmed path
/// after showing the immutable proposal. Non-Repair proposals use `Reviewed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionApproval {
    Reviewed,
    RepairConfirmed,
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // consumed by the next main.rs lifecycle-projection integration
pub struct ActionExecutionItem {
    pub remediation: RemediationSummary,
    pub issue_id: Option<String>,
    pub result: Result<FixResult, String>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // consumed by the next main.rs lifecycle-projection integration
pub struct ActionExecution {
    pub run_id: String,
    pub proposal_id: String,
    pub items: Vec<ActionExecutionItem>,
    /// Full terminal projection. `items` remains for the already-integrated
    /// Reactor status path; new UI should render this summary instead.
    pub summary: ActionRunSummary,
}

/// Per-catalog-item lifecycle. This deliberately mirrors the shipping
/// action-broker projection so Reactor can reuse the same UI semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionItemStatus {
    Pending,
    Running,
    Succeeded,
    Partial,
    Failed,
    Cancelled,
    Skipped,
}

/// Aggregate lifecycle for one approved proposal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionRunStatus {
    Running,
    CancelRequested,
    Succeeded,
    Partial,
    Failed,
    Cancelled,
}

impl ActionRunStatus {
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Partial | Self::Failed | Self::Cancelled
        )
    }
}

/// One action's live/terminal projection. `FixResult` is preserved verbatim,
/// including per-step succeeded/already-satisfied/failed/cancelled detail and
/// the requires-restart flag.
#[derive(Clone, Debug)]
#[allow(dead_code)] // full shipping-compatible projection is not rendered yet
pub struct ActionItemRun {
    pub remediation_id: String,
    pub label: String,
    pub cancellable: bool,
    pub status: ActionItemStatus,
    pub result: Option<FixResult>,
    pub error: Option<String>,
}

/// Addressable, rehydratable run state compatible with the shipping React
/// `ActionRunSummary` shape.
#[derive(Clone, Debug)]
#[allow(dead_code)] // full shipping-compatible projection is not rendered yet
pub struct ActionRunSummary {
    pub run_id: String,
    pub proposal_id: String,
    /// Audit metadata only. No public method accepts this as authority.
    pub authorization_id: String,
    pub status: ActionRunStatus,
    pub actions: Vec<ActionItemRun>,
    pub current_index: Option<usize>,
    pub approved_at_ms: u64,
    pub completed_at_ms: Option<u64>,
    pub scan_fingerprint: String,
    pub catalog_fingerprint: String,
}

impl ActionRunSummary {
    #[must_use]
    pub fn requires_restart(&self) -> bool {
        self.actions.iter().any(|action| {
            action
                .result
                .as_ref()
                .is_some_and(|result| result.requires_restart)
        })
    }
}

/// A live update from the action runtime. The request ID correlates initial
/// authorization with the component's existing pending-request identity;
/// rehydrated history intentionally contains only the stable summary.
#[derive(Clone, Debug)]
pub struct ActionRunEvent {
    // Stable correlation metadata for consumers that perform stale-event
    // rejection. The current component subscribes once per runtime and only
    // needs the summary; other support-module consumers still require it.
    #[allow(dead_code)]
    pub request_id: u64,
    pub summary: ActionRunSummary,
}

/// Atomic state returned when a remounted view subscribes. Subscribe and
/// snapshot happen under one lock, so no run transition can be lost between
/// the history read and live delivery.
#[derive(Debug)]
pub struct ActionRuntimeSnapshot {
    pub pending_proposals: Vec<ActionProposal>,
    pub history: Vec<ActionRunSummary>,
    pub active_run: Option<ActionRunSummary>,
}

/// Typed worker events drained by the component.
#[derive(Clone, Debug)]
pub enum ActionWorkerEvent {
    Prepared {
        request_id: u64,
        proposal: ActionProposal,
    },
    Done {
        request_id: u64,
        execution: ActionExecution,
    },
    Failed {
        request_id: u64,
        message: String,
    },
    NeedsRepairConfirmation {
        request_id: u64,
        proposal: ActionProposal,
    },
}

impl ActionWorkerEvent {
    /// The originating execute's identity, for stale-event rejection.
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Prepared { request_id, .. }
            | Self::Done { request_id, .. }
            | Self::Failed { request_id, .. }
            | Self::NeedsRepairConfirmation { request_id, .. } => *request_id,
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn stable_fingerprint<T: Hash + ?Sized>(value: &T) -> String {
    struct Fnv64(u64);

    impl Hasher for Fnv64 {
        fn write(&mut self, bytes: &[u8]) {
            for byte in bytes {
                self.0 ^= u64::from(*byte);
                self.0 = self.0.wrapping_mul(0x100000001b3);
            }
        }

        fn finish(&self) -> u64 {
            self.0
        }
    }

    let mut hasher = Fnv64(0xcbf29ce484222325);
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Fingerprint of every security-relevant catalog field and immutable preview
/// step. Callers put this value into each authoritative snapshot.
#[must_use]
pub fn current_action_catalog_fingerprint() -> String {
    let material = remediations()
        .iter()
        .map(|spec| {
            format!(
                "{}|{}|{}|{:?}|{}|{}|{}|{}|{}|{}",
                spec.id,
                spec.label,
                spec.description,
                spec.tier,
                spec.admin_required,
                spec.requires_restart,
                spec.long_running,
                spec.maintenance,
                spec.cancellable(),
                spec.preview_steps().join("\u{1e}")
            )
        })
        .collect::<Vec<_>>();
    stable_fingerprint(&(ACTION_CATALOG_SCHEMA_VERSION, material))
}

fn opaque_id(prefix: &'static str) -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let thread_id = std::thread::current().id();
    let process_id = std::process::id();

    // RandomState is independently keyed from the operating system. The
    // timestamp/counter guarantee distinct input while the hidden keys keep
    // proposal IDs opaque without adding another package dependency.
    let first = RandomState::new().hash_one((prefix, stamp, sequence, process_id, thread_id));
    let second = RandomState::new().hash_one((first, stamp, sequence.rotate_left(23), process_id));
    format!("{prefix}_{first:016x}{second:016x}")
}

fn validate_snapshot(snapshot: &ActionSnapshot) -> Result<(), String> {
    if snapshot.scan_fingerprint.trim().is_empty() {
        return Err("The authoritative scan fingerprint is missing".to_string());
    }
    if snapshot.catalog_fingerprint != current_action_catalog_fingerprint() {
        return Err("The authoritative remediation catalog fingerprint is stale".to_string());
    }

    let mut issue_ids = HashSet::new();
    for issue in &snapshot.detected_issues {
        if issue.issue_id.trim().is_empty() {
            return Err("A detected issue is missing its catalog ID".to_string());
        }
        if !issue_ids.insert(issue.issue_id.as_str()) {
            return Err(format!(
                "Detected issue '{}' appears more than once in the authoritative snapshot",
                issue.issue_id
            ));
        }
        if issue
            .remediation_id
            .as_deref()
            .is_some_and(|remediation_id| remediation_id.trim().is_empty())
        {
            return Err(format!(
                "Detected issue '{}' has an empty remediation ID",
                issue.issue_id
            ));
        }
    }
    Ok(())
}

fn build_proposal(
    input: ActionPrepareInput,
    snapshot: &ActionSnapshot,
    created_at_ms: u64,
) -> Result<ActionProposal, String> {
    validate_snapshot(snapshot)?;
    if input.actions.is_empty() {
        return Err("At least one action is required".to_string());
    }
    if input.actions.len() > MAX_BATCH_ACTIONS {
        return Err(format!(
            "At most {MAX_BATCH_ACTIONS} actions can be approved together"
        ));
    }
    if input
        .expected_scan_fingerprint
        .as_deref()
        .is_some_and(|expected| expected != snapshot.scan_fingerprint)
    {
        return Err("The scan changed after this plan was created".to_string());
    }
    if input
        .expected_catalog_fingerprint
        .as_deref()
        .is_some_and(|expected| expected != snapshot.catalog_fingerprint)
    {
        return Err("The remediation catalog changed after this plan was created".to_string());
    }

    let batch = input.actions.len() > 1;
    let mut seen = HashSet::new();
    let mut actions = Vec::with_capacity(input.actions.len());
    for request in input.actions {
        if !seen.insert(request.remediation_id.clone()) {
            return Err(format!(
                "Remediation '{}' appears more than once",
                request.remediation_id
            ));
        }
        let spec = find(&request.remediation_id)
            .ok_or_else(|| format!("Unknown remediation '{}'", request.remediation_id))?;
        if batch && !spec.batch_eligible() {
            return Err(format!(
                "'{}' requires exact approval and cannot be included in a batch",
                spec.label
            ));
        }

        match request.issue_id.as_deref() {
            Some(issue_id) => {
                let issue = snapshot
                    .detected_issues
                    .iter()
                    .find(|issue| issue.issue_id == issue_id)
                    .ok_or_else(|| {
                        format!("Issue '{issue_id}' is not detected in the current scan")
                    })?;
                if issue.remediation_id.as_deref() != Some(spec.id) {
                    return Err(format!(
                        "Remediation '{}' is not mapped to issue '{issue_id}'",
                        spec.id
                    ));
                }
            }
            None if !spec.maintenance => {
                return Err(format!(
                    "Remediation '{}' requires a currently detected issue",
                    spec.id
                ));
            }
            None => {}
        }

        actions.push(ActionPreview {
            remediation: spec.summary(),
            issue_id: request.issue_id,
            steps: spec.preview_steps(),
        });
    }

    Ok(ActionProposal {
        proposal_id: opaque_id("proposal"),
        approval_scope: if batch {
            ApprovalScope::Batch
        } else {
            ApprovalScope::Exact
        },
        actions,
        scan_fingerprint: snapshot.scan_fingerprint.clone(),
        catalog_fingerprint: snapshot.catalog_fingerprint.clone(),
        created_at_ms,
        expires_at_ms: created_at_ms.saturating_add(ACTION_PROPOSAL_TTL_MS),
    })
}

struct StoredProposal {
    proposal: ActionProposal,
    consumed: bool,
}

#[derive(Debug)]
struct ActionGrant {
    authorization_id: String,
    proposal: ActionProposal,
}

#[derive(Debug)]
enum AuthorizationError {
    RepairConfirmationRequired(ActionProposal),
    Rejected(String),
}

#[derive(Default)]
struct ActionBroker {
    proposals: HashMap<String, StoredProposal>,
}

impl ActionBroker {
    fn prepare(
        &mut self,
        input: ActionPrepareInput,
        snapshot: &ActionSnapshot,
        current_time_ms: u64,
    ) -> Result<ActionProposal, String> {
        self.proposals.retain(|_, stored| {
            !stored.consumed && stored.proposal.expires_at_ms > current_time_ms
        });
        if self.proposals.len() >= MAX_PENDING_PROPOSALS {
            return Err(
                "Too many action previews are pending; finish or dismiss one first".to_string(),
            );
        }
        let proposal = build_proposal(input, snapshot, current_time_ms)?;
        self.proposals.insert(
            proposal.proposal_id.clone(),
            StoredProposal {
                proposal: proposal.clone(),
                consumed: false,
            },
        );
        Ok(proposal)
    }

    fn authorize(
        &mut self,
        proposal_id: &str,
        snapshot: &ActionSnapshot,
        approval: ActionApproval,
        approved_at_ms: u64,
    ) -> Result<ActionGrant, AuthorizationError> {
        validate_snapshot(snapshot).map_err(AuthorizationError::Rejected)?;
        let stored = self.proposals.get_mut(proposal_id).ok_or_else(|| {
            AuthorizationError::Rejected(
                "Action preview was not found, was used, or expired".to_string(),
            )
        })?;
        if stored.consumed {
            return Err(AuthorizationError::Rejected(
                "This action preview has already been approved".to_string(),
            ));
        }
        if stored.proposal.expires_at_ms <= approved_at_ms {
            return Err(AuthorizationError::Rejected(
                "This action preview expired. Review the action again.".to_string(),
            ));
        }
        if stored.proposal.scan_fingerprint != snapshot.scan_fingerprint {
            return Err(AuthorizationError::Rejected(
                "The scan changed after this action was reviewed".to_string(),
            ));
        }
        if stored.proposal.catalog_fingerprint != snapshot.catalog_fingerprint {
            return Err(AuthorizationError::Rejected(
                "The remediation catalog changed after this action was reviewed".to_string(),
            ));
        }

        for action in &stored.proposal.actions {
            let spec = find(&action.remediation.id).ok_or_else(|| {
                AuthorizationError::Rejected(format!(
                    "Remediation '{}' is no longer in the catalog",
                    action.remediation.id
                ))
            })?;
            if spec.summary() != action.remediation || spec.preview_steps() != action.steps {
                return Err(AuthorizationError::Rejected(format!(
                    "Remediation '{}' changed after this action was reviewed",
                    action.remediation.id
                )));
            }
            match action.issue_id.as_deref() {
                Some(issue_id) => {
                    let still_valid = snapshot.detected_issues.iter().any(|issue| {
                        issue.issue_id == issue_id
                            && issue.remediation_id.as_deref() == Some(spec.id)
                    });
                    if !still_valid {
                        return Err(AuthorizationError::Rejected(format!(
                            "Issue '{issue_id}' changed after this action was reviewed"
                        )));
                    }
                }
                None if !spec.maintenance => {
                    return Err(AuthorizationError::Rejected(format!(
                        "Remediation '{}' is no longer available as maintenance",
                        spec.id
                    )));
                }
                None => {}
            }
        }
        if stored
            .proposal
            .actions
            .iter()
            .any(|action| action.remediation.admin_required)
            && !snapshot.is_admin
        {
            return Err(AuthorizationError::Rejected(
                "This action requires administrator rights".to_string(),
            ));
        }
        if stored
            .proposal
            .actions
            .iter()
            .any(|action| action.remediation.tier == RemediationTier::Repair)
            && approval != ActionApproval::RepairConfirmed
        {
            return Err(AuthorizationError::RepairConfirmationRequired(
                stored.proposal.clone(),
            ));
        }

        // The private grant is created and the proposal consumed in one
        // operation; no executable catalog ID is returned to the caller.
        stored.consumed = true;
        Ok(ActionGrant {
            authorization_id: opaque_id("grant"),
            proposal: stored.proposal.clone(),
        })
    }

    fn discard(&mut self, proposal_id: &str) {
        if self
            .proposals
            .get(proposal_id)
            .is_some_and(|stored| !stored.consumed)
        {
            self.proposals.remove(proposal_id);
        }
    }

    fn pending(&mut self, current_time_ms: u64) -> Vec<ActionProposal> {
        self.proposals.retain(|_, stored| {
            !stored.consumed && stored.proposal.expires_at_ms > current_time_ms
        });
        let mut proposals = self
            .proposals
            .values()
            .map(|stored| stored.proposal.clone())
            .collect::<Vec<_>>();
        proposals.sort_by_key(|proposal| proposal.created_at_ms);
        proposals
    }
}

enum ActionCommand {
    Prepare {
        request_id: u64,
        input: ActionPrepareInput,
        snapshot: ActionSnapshot,
    },
    Approve {
        request_id: u64,
        proposal_id: String,
        snapshot: ActionSnapshot,
        approval: ActionApproval,
    },
    Discard {
        proposal_id: String,
    },
}

struct ActionRunRecord {
    request_id: u64,
    summary: ActionRunSummary,
    cancel: CancellationToken,
}

#[derive(Debug)]
struct AuthorizedRun {
    request_id: u64,
    run_id: String,
    grant: ActionGrant,
    cancel: CancellationToken,
}

#[derive(Default)]
struct ActionRuntimeState {
    broker: ActionBroker,
    runs: VecDeque<ActionRunRecord>,
    active_run_id: Option<String>,
    subscribers: Vec<std_mpsc::Sender<ActionRunEvent>>,
}

impl ActionRuntimeState {
    fn publish(&mut self, event: ActionRunEvent) {
        self.subscribers
            .retain(|subscriber| subscriber.send_and_wake(event.clone()).is_ok());
    }

    fn publish_summary(&mut self, request_id: u64, summary: ActionRunSummary) {
        self.publish(ActionRunEvent {
            request_id,
            summary,
        });
    }

    fn record_grant(
        &mut self,
        request_id: u64,
        grant: ActionGrant,
        approved_at_ms: u64,
    ) -> Result<AuthorizedRun, String> {
        if self.active_run_id.is_some() {
            return Err("Another system action is already running".to_string());
        }

        let run_id = opaque_id("run");
        let cancel = CancellationToken::new();
        let summary = ActionRunSummary {
            run_id: run_id.clone(),
            proposal_id: grant.proposal.proposal_id.clone(),
            authorization_id: grant.authorization_id.clone(),
            status: ActionRunStatus::Running,
            actions: grant
                .proposal
                .actions
                .iter()
                .map(|action| ActionItemRun {
                    remediation_id: action.remediation.id.clone(),
                    label: action.remediation.label.clone(),
                    cancellable: action.remediation.cancellable,
                    status: ActionItemStatus::Pending,
                    result: None,
                    error: None,
                })
                .collect(),
            current_index: None,
            approved_at_ms,
            completed_at_ms: None,
            scan_fingerprint: grant.proposal.scan_fingerprint.clone(),
            catalog_fingerprint: grant.proposal.catalog_fingerprint.clone(),
        };
        self.active_run_id = Some(run_id.clone());
        self.runs.push_back(ActionRunRecord {
            request_id,
            summary: summary.clone(),
            cancel: cancel.clone(),
        });
        while self.runs.len() > MAX_ACTION_HISTORY {
            self.runs.pop_front();
        }
        self.publish_summary(request_id, summary);
        Ok(AuthorizedRun {
            request_id,
            run_id,
            grant,
            cancel,
        })
    }

    fn authorize_run(
        &mut self,
        request_id: u64,
        proposal_id: &str,
        snapshot: &ActionSnapshot,
        approval: ActionApproval,
        approved_at_ms: u64,
    ) -> Result<AuthorizedRun, AuthorizationError> {
        // Reserve the one mutation slot before consuming the proposal. A
        // rejected concurrent approval therefore remains reviewable.
        if self.active_run_id.is_some() {
            return Err(AuthorizationError::Rejected(
                "Another system action is already running".to_string(),
            ));
        }
        let grant = self
            .broker
            .authorize(proposal_id, snapshot, approval, approved_at_ms)?;
        self.record_grant(request_id, grant, approved_at_ms)
            .map_err(AuthorizationError::Rejected)
    }

    fn run(&self, run_id: &str) -> Option<&ActionRunRecord> {
        self.runs
            .iter()
            .find(|record| record.summary.run_id == run_id)
    }

    fn run_mut(&mut self, run_id: &str) -> Option<&mut ActionRunRecord> {
        self.runs
            .iter_mut()
            .find(|record| record.summary.run_id == run_id)
    }

    fn history(&self) -> Vec<ActionRunSummary> {
        self.runs
            .iter()
            .rev()
            .map(|record| record.summary.clone())
            .collect()
    }

    fn active_run(&self) -> Option<ActionRunSummary> {
        self.active_run_id
            .as_deref()
            .and_then(|run_id| self.run(run_id))
            .map(|record| record.summary.clone())
    }

    fn cancel_run(&mut self, run_id: &str) -> Result<ActionRunSummary, String> {
        let (request_id, summary) = {
            let record = self
                .run_mut(run_id)
                .ok_or_else(|| "Action run was not found".to_string())?;
            if record.summary.status.terminal() {
                return Ok(record.summary.clone());
            }
            if let Some(index) = record.summary.current_index
                && !record.summary.actions[index].cancellable
            {
                return Err("The current action cannot be stopped safely once started".to_string());
            }
            record.cancel.cancel();
            record.summary.status = ActionRunStatus::CancelRequested;
            (record.request_id, record.summary.clone())
        };
        self.publish_summary(request_id, summary.clone());
        Ok(summary)
    }

    fn fail_run(&mut self, run_id: &str, message: String) -> Option<ActionRunSummary> {
        let (request_id, summary) = {
            let record = self.run_mut(run_id)?;
            if record.summary.status.terminal() {
                return Some(record.summary.clone());
            }
            let failed_index = record.summary.current_index.or_else(|| {
                record
                    .summary
                    .actions
                    .iter()
                    .position(|action| action.status == ActionItemStatus::Pending)
            });
            if let Some(index) = failed_index {
                let action = &mut record.summary.actions[index];
                action.status = ActionItemStatus::Failed;
                action.error = Some(message);
            }
            for action in &mut record.summary.actions {
                if action.status == ActionItemStatus::Pending {
                    action.status = ActionItemStatus::Skipped;
                }
            }
            record.summary.status = completed_run_status(&record.summary.actions, false);
            record.summary.current_index = None;
            record.summary.completed_at_ms = Some(now_ms());
            (record.request_id, record.summary.clone())
        };
        if self.active_run_id.as_deref() == Some(run_id) {
            self.active_run_id = None;
        }
        self.publish_summary(request_id, summary.clone());
        Some(summary)
    }

    fn snapshot(&mut self) -> ActionRuntimeSnapshot {
        ActionRuntimeSnapshot {
            pending_proposals: self.broker.pending(now_ms()),
            history: self.history(),
            active_run: self.active_run(),
        }
    }
}

fn lock_state(state: &Mutex<ActionRuntimeState>) -> MutexGuard<'_, ActionRuntimeState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

type ExecutionFuture<'a> = Pin<Box<dyn Future<Output = Result<FixResult, String>> + Send + 'a>>;

trait AuthorizedActionExecutor: Send + Sync {
    fn execute<'a>(
        &'a self,
        remediation_id: &'a str,
        cancel: &'a CancellationToken,
    ) -> ExecutionFuture<'a>;
}

struct RealCatalogExecutor;

impl AuthorizedActionExecutor for RealCatalogExecutor {
    fn execute<'a>(
        &'a self,
        remediation_id: &'a str,
        cancel: &'a CancellationToken,
    ) -> ExecutionFuture<'a> {
        Box::pin(async move { execute_authorized(remediation_id, &RealRunner, cancel).await })
    }
}

#[cfg(test)]
async fn execute_grant(
    grant: ActionGrant,
    executor: &dyn AuthorizedActionExecutor,
) -> ActionExecution {
    let state = Arc::new(Mutex::new(ActionRuntimeState::default()));
    let authorized = lock_state(&state)
        .record_grant(0, grant, now_ms())
        .expect("fresh test execution state cannot have an active run");
    execute_authorized_run(state, authorized, executor).await
}

fn cancellation_applies(actions: &[ActionItemRun], requested: bool) -> bool {
    let has_cancelled_item = actions
        .iter()
        .any(|action| action.status == ActionItemStatus::Cancelled);
    let has_unfinished_item = actions.iter().any(|action| {
        matches!(
            action.status,
            ActionItemStatus::Pending | ActionItemStatus::Running
        )
    });
    has_cancelled_item || (requested && has_unfinished_item)
}

fn completed_run_status(actions: &[ActionItemRun], was_cancelled: bool) -> ActionRunStatus {
    if was_cancelled {
        return ActionRunStatus::Cancelled;
    }
    let succeeded = actions
        .iter()
        .filter(|action| action.status == ActionItemStatus::Succeeded)
        .count();
    let partial = actions
        .iter()
        .any(|action| action.status == ActionItemStatus::Partial);
    let failed = actions
        .iter()
        .any(|action| action.status == ActionItemStatus::Failed);
    if partial || (failed && succeeded > 0) {
        ActionRunStatus::Partial
    } else if failed {
        ActionRunStatus::Failed
    } else {
        ActionRunStatus::Succeeded
    }
}

async fn execute_authorized_run(
    state: Arc<Mutex<ActionRuntimeState>>,
    authorized: AuthorizedRun,
    executor: &dyn AuthorizedActionExecutor,
) -> ActionExecution {
    let run_id = authorized.run_id.clone();
    let proposal_id = authorized.grant.proposal.proposal_id.clone();
    let actions = authorized.grant.proposal.actions;
    let mut items = Vec::with_capacity(actions.len());

    for (index, action) in actions.into_iter().enumerate() {
        if authorized.cancel.is_cancelled() {
            break;
        }
        {
            let mut state = lock_state(&state);
            let update = state.run_mut(&run_id).map(|record| {
                record.summary.current_index = Some(index);
                record.summary.actions[index].status = ActionItemStatus::Running;
                (record.request_id, record.summary.clone())
            });
            if let Some((request_id, summary)) = update {
                state.publish_summary(request_id, summary);
            } else {
                break;
            }
        }

        let result = executor
            .execute(&action.remediation.id, &authorized.cancel)
            .await;
        let stop = {
            let mut state = lock_state(&state);
            let update = state.run_mut(&run_id).map(|record| {
                let stop = match &result {
                    Ok(result) => {
                        record.summary.actions[index].status = match result.completion_status {
                            FixCompletionStatus::Succeeded => ActionItemStatus::Succeeded,
                            FixCompletionStatus::Partial => ActionItemStatus::Partial,
                            FixCompletionStatus::Failed => ActionItemStatus::Failed,
                            FixCompletionStatus::Cancelled => ActionItemStatus::Cancelled,
                        };
                        record.summary.actions[index].result = Some(result.clone());
                        result.completion_status != FixCompletionStatus::Succeeded
                    }
                    Err(error) => {
                        record.summary.actions[index].status = ActionItemStatus::Failed;
                        record.summary.actions[index].error = Some(error.clone());
                        true
                    }
                };
                (stop, record.request_id, record.summary.clone())
            });
            match update {
                Some((stop, request_id, summary)) => {
                    state.publish_summary(request_id, summary);
                    stop
                }
                None => true,
            }
        };
        items.push(ActionExecutionItem {
            remediation: action.remediation,
            issue_id: action.issue_id,
            result,
        });
        if stop {
            break;
        }
    }

    let summary = {
        let mut state = lock_state(&state);
        let update = state.run_mut(&run_id).map(|record| {
            // A late cancel must not rewrite a fully-completed successful run.
            let was_cancelled =
                cancellation_applies(&record.summary.actions, authorized.cancel.is_cancelled());
            for action in &mut record.summary.actions {
                if action.status == ActionItemStatus::Pending {
                    action.status = if was_cancelled {
                        ActionItemStatus::Cancelled
                    } else {
                        ActionItemStatus::Skipped
                    };
                }
            }
            record.summary.status = completed_run_status(&record.summary.actions, was_cancelled);
            record.summary.current_index = None;
            record.summary.completed_at_ms = Some(now_ms());
            (record.request_id, record.summary.clone())
        });
        let (request_id, summary) = update.expect("authorized run must remain in bounded history");
        if state.active_run_id.as_deref() == Some(run_id.as_str()) {
            state.active_run_id = None;
        }
        state.publish_summary(request_id, summary.clone());
        summary
    };

    ActionExecution {
        run_id,
        proposal_id,
        items,
        summary,
    }
}

/// UI-thread handle for the native action runtime.
pub struct NativeActionRuntime {
    /// Option so Drop can release the sender BEFORE joining the worker.
    commands: Option<std_mpsc::Sender<ActionCommand>>,
    state: Arc<Mutex<ActionRuntimeState>>,
    command_worker: Option<JoinHandle<()>>,
    execution_worker: Option<JoinHandle<()>>,
}

impl NativeActionRuntime {
    /// Start the worker.
    ///
    /// # Errors
    /// When the worker thread cannot be spawned.
    pub fn start() -> std::io::Result<(Self, std_mpsc::Receiver<ActionWorkerEvent>)> {
        let (commands, command_rx) = std_mpsc::channel::<ActionCommand>();
        let (events, event_rx) = std_mpsc::channel::<ActionWorkerEvent>();
        let (executions, execution_rx) = std_mpsc::channel::<AuthorizedRun>();
        let state = Arc::new(Mutex::new(ActionRuntimeState::default()));

        let command_state = Arc::clone(&state);
        let command_events = events.clone();
        let command_worker = std::thread::Builder::new()
            .name("wfdiag-reactor-actions".to_string())
            .spawn(move || {
                while let Ok(command) = command_rx.recv() {
                    match command {
                        ActionCommand::Prepare {
                            request_id,
                            input,
                            snapshot,
                        } => match lock_state(&command_state).broker.prepare(
                            input,
                            &snapshot,
                            now_ms(),
                        ) {
                            Ok(proposal) => {
                                let _ = command_events.send_and_wake(ActionWorkerEvent::Prepared {
                                    request_id,
                                    proposal,
                                });
                            }
                            Err(message) => {
                                let _ = command_events.send_and_wake(ActionWorkerEvent::Failed {
                                    request_id,
                                    message,
                                });
                            }
                        },
                        ActionCommand::Approve {
                            request_id,
                            proposal_id,
                            snapshot,
                            approval,
                        } => {
                            let authorized = lock_state(&command_state).authorize_run(
                                request_id,
                                &proposal_id,
                                &snapshot,
                                approval,
                                now_ms(),
                            );
                            match authorized {
                                Ok(grant) => {
                                    if let Err(error) = executions.send(grant) {
                                        let run_id = error.0.run_id;
                                        let message =
                                            "The native remediation executor stopped".to_string();
                                        lock_state(&command_state)
                                            .fail_run(&run_id, message.clone());
                                        let _ = command_events.send_and_wake(
                                            ActionWorkerEvent::Failed {
                                                request_id,
                                                message,
                                            },
                                        );
                                    }
                                }
                                Err(AuthorizationError::RepairConfirmationRequired(proposal)) => {
                                    let _ = command_events.send_and_wake(
                                        ActionWorkerEvent::NeedsRepairConfirmation {
                                            request_id,
                                            proposal,
                                        },
                                    );
                                }
                                Err(AuthorizationError::Rejected(message)) => {
                                    let _ =
                                        command_events.send_and_wake(ActionWorkerEvent::Failed {
                                            request_id,
                                            message,
                                        });
                                }
                            }
                        }
                        ActionCommand::Discard { proposal_id } => {
                            lock_state(&command_state).broker.discard(&proposal_id);
                        }
                    }
                }
            })?;

        let execution_state = Arc::clone(&state);
        let execution_worker = match std::thread::Builder::new()
            .name("wfdiag-reactor-action-executor".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                while let Ok(authorized) = execution_rx.recv() {
                    let request_id = authorized.request_id;
                    match &runtime {
                        Ok(runtime) => {
                            let execution = runtime.block_on(execute_authorized_run(
                                Arc::clone(&execution_state),
                                authorized,
                                &RealCatalogExecutor,
                            ));
                            let _ = events.send_and_wake(ActionWorkerEvent::Done {
                                request_id,
                                execution,
                            });
                        }
                        Err(_) => {
                            let message =
                                "Could not start the native remediation executor".to_string();
                            lock_state(&execution_state)
                                .fail_run(&authorized.run_id, message.clone());
                            let _ = events.send_and_wake(ActionWorkerEvent::Failed {
                                request_id,
                                message,
                            });
                        }
                    }
                }
            }) {
            Ok(worker) => worker,
            Err(error) => {
                drop(commands);
                let _ = command_worker.join();
                return Err(error);
            }
        };
        Ok((
            Self {
                commands: Some(commands),
                state,
                command_worker: Some(command_worker),
                execution_worker: Some(execution_worker),
            },
            event_rx,
        ))
    }

    /// Stage catalog IDs into a reviewable immutable proposal.
    ///
    /// Returns `false` only when the worker queue is unavailable.
    pub fn prepare(
        &self,
        request_id: u64,
        input: ActionPrepareInput,
        snapshot: ActionSnapshot,
    ) -> bool {
        self.commands.as_ref().is_some_and(|commands| {
            commands
                .send(ActionCommand::Prepare {
                    request_id,
                    input,
                    snapshot,
                })
                .is_ok()
        })
    }

    /// Approve an opaque proposal against a freshly captured authoritative
    /// snapshot. Repair proposals require `ActionApproval::RepairConfirmed`.
    ///
    /// Returns `false` only when the worker queue is unavailable.
    pub fn approve(
        &self,
        request_id: u64,
        proposal_id: String,
        snapshot: ActionSnapshot,
        approval: ActionApproval,
    ) -> bool {
        self.commands.as_ref().is_some_and(|commands| {
            commands
                .send(ActionCommand::Approve {
                    request_id,
                    proposal_id,
                    snapshot,
                    approval,
                })
                .is_ok()
        })
    }

    /// Drop an unused proposal after the review UI is dismissed.
    pub fn discard(&self, proposal_id: String) -> bool {
        self.commands.as_ref().is_some_and(|commands| {
            commands
                .send(ActionCommand::Discard { proposal_id })
                .is_ok()
        })
    }

    /// Signal cancellation without waiting behind the currently executing
    /// remediation. Cancellation is accepted only before a non-cancellable
    /// catalog action starts, matching the shipping broker's safety rule.
    #[allow(dead_code)] // main.rs integration follows this isolated runtime change
    pub fn cancel(&self, run_id: &str) -> Result<ActionRunSummary, String> {
        lock_state(&self.state).cancel_run(run_id)
    }

    /// Current projection for one addressable run.
    #[must_use]
    #[allow(dead_code)] // main.rs integration follows this isolated runtime change
    pub fn get_status(&self, run_id: &str) -> Option<ActionRunSummary> {
        lock_state(&self.state)
            .run(run_id)
            .map(|record| record.summary.clone())
    }

    /// Newest-first bounded in-memory audit history, retained across view
    /// unmount/remount for the lifetime of the native application.
    #[must_use]
    #[allow(dead_code)] // direct-query compatibility; main uses subscribe_run_events snapshot
    pub fn list_history(&self) -> Vec<ActionRunSummary> {
        lock_state(&self.state).history()
    }

    /// Unconsumed, unexpired proposals for staged-action rehydration.
    #[must_use]
    #[allow(dead_code)] // direct-query compatibility; main uses subscribe_run_events snapshot
    pub fn list_pending_proposals(&self) -> Vec<ActionProposal> {
        lock_state(&self.state).broker.pending(now_ms())
    }

    /// Atomically attach a live observer and capture pending/history/active
    /// state. The caller can render the snapshot, then drain events without a
    /// race window between those operations.
    pub fn subscribe_run_events(
        &self,
    ) -> (std_mpsc::Receiver<ActionRunEvent>, ActionRuntimeSnapshot) {
        let (sender, receiver) = std_mpsc::channel();
        let mut state = lock_state(&self.state);
        state.subscribers.push(sender);
        let snapshot = state.snapshot();
        (receiver, snapshot)
    }
}

impl Drop for NativeActionRuntime {
    fn drop(&mut self) {
        // Release the command sender BEFORE reaping: the command worker
        // drains, exits, and its closure drops the last `executions` sender,
        // which is what disconnects the execution worker's receive loop.
        // Without that disconnect the executor reaper would wait on a worker
        // that can never observe shutdown.
        self.commands = None;
        // Best-effort cancellation of an in-flight run. A step already started
        // and flagged non-cancellable keeps running to completion, which is
        // exactly why the joins happen on a detached reaper rather than here.
        let active_run_id = lock_state(&self.state).active_run_id.clone();
        if let Some(run_id) = active_run_id {
            // "Not found" and "cannot be stopped safely" are both fine here.
            let _ = lock_state(&self.state).cancel_run(&run_id);
        }
        let command_worker = self.command_worker.take();
        let execution_worker = self.execution_worker.take();
        // Independent reapers keep graceful close off the UI thread: the
        // executor may be inside one more blocking step even after
        // cancellation, and the command worker exits on its own once the
        // sender above drops, disconnecting the executor's receive loop.
        if let Some(worker) = command_worker {
            crate::teardown_support::reap_worker(worker);
        }
        if let Some(worker) = execution_worker {
            crate::teardown_support::reap_worker(worker);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Duration;
    use wfdiag_native_remediation::remediation::{RemediationStepResult, RemediationStepStatus};

    fn snapshot(scan_fingerprint: &str, is_admin: bool) -> ActionSnapshot {
        ActionSnapshot {
            scan_fingerprint: scan_fingerprint.to_string(),
            catalog_fingerprint: current_action_catalog_fingerprint(),
            detected_issues: Vec::new(),
            is_admin,
        }
    }

    fn issue_snapshot(
        scan_fingerprint: &str,
        issue_id: &str,
        remediation_id: &str,
    ) -> ActionSnapshot {
        ActionSnapshot {
            detected_issues: vec![DetectedIssueRemediation {
                issue_id: issue_id.to_string(),
                remediation_id: Some(remediation_id.to_string()),
            }],
            ..snapshot(scan_fingerprint, true)
        }
    }

    fn request(remediation_id: &str, issue_id: Option<&str>) -> ActionPrepareInput {
        ActionPrepareInput {
            actions: vec![ActionRequest {
                remediation_id: remediation_id.to_string(),
                issue_id: issue_id.map(str::to_string),
            }],
            expected_scan_fingerprint: None,
            expected_catalog_fingerprint: None,
        }
    }

    fn preview(remediation_id: &str) -> ActionPreview {
        let spec = find(remediation_id).expect("test remediation must exist");
        ActionPreview {
            remediation: spec.summary(),
            issue_id: None,
            steps: spec.preview_steps(),
        }
    }

    fn grant(remediation_ids: &[&str]) -> ActionGrant {
        ActionGrant {
            authorization_id: opaque_id("test_grant"),
            proposal: ActionProposal {
                proposal_id: opaque_id("test_proposal"),
                approval_scope: if remediation_ids.len() == 1 {
                    ApprovalScope::Exact
                } else {
                    ApprovalScope::Batch
                },
                actions: remediation_ids.iter().map(|id| preview(id)).collect(),
                scan_fingerprint: "scan-a".to_string(),
                catalog_fingerprint: current_action_catalog_fingerprint(),
                created_at_ms: 1_000,
                expires_at_ms: 1_000 + ACTION_PROPOSAL_TTL_MS,
            },
        }
    }

    fn fix_result(
        completion_status: FixCompletionStatus,
        requires_restart: bool,
        steps: Vec<RemediationStepResult>,
    ) -> FixResult {
        FixResult {
            success: matches!(completion_status, FixCompletionStatus::Succeeded),
            message: format!("{completion_status:?}"),
            actions_taken: steps.iter().map(|step| step.action.clone()).collect(),
            requires_restart,
            completion_status,
            steps,
        }
    }

    fn step(action: &str, status: RemediationStepStatus) -> RemediationStepResult {
        RemediationStepResult {
            action: action.to_string(),
            status,
            detail: Some(format!("{action} detail")),
        }
    }

    struct SequenceExecutor {
        outcomes: Mutex<VecDeque<Result<FixResult, String>>>,
        calls: Mutex<Vec<String>>,
    }

    impl SequenceExecutor {
        fn new(outcomes: impl IntoIterator<Item = Result<FixResult, String>>) -> Self {
            Self {
                outcomes: Mutex::new(outcomes.into_iter().collect()),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl AuthorizedActionExecutor for SequenceExecutor {
        fn execute<'a>(
            &'a self,
            remediation_id: &'a str,
            _cancel: &'a CancellationToken,
        ) -> ExecutionFuture<'a> {
            self.calls.lock().unwrap().push(remediation_id.to_string());
            let outcome = self
                .outcomes
                .lock()
                .unwrap()
                .pop_front()
                .expect("test executor needs one outcome per call");
            Box::pin(async move { outcome })
        }
    }

    fn rejected(result: Result<ActionGrant, AuthorizationError>) -> String {
        match result {
            Err(AuthorizationError::Rejected(message)) => message,
            Err(AuthorizationError::RepairConfirmationRequired(_)) => {
                panic!("expected a hard rejection, got a Repair confirmation request")
            }
            Ok(_) => panic!("expected authorization to be rejected"),
        }
    }

    #[test]
    fn proposal_expires_at_the_ttl_boundary() {
        let mut broker = ActionBroker::default();
        let current = snapshot("scan-a", true);
        let proposal = broker
            .prepare(request("flush_dns", None), &current, 1_000)
            .unwrap();

        let message = rejected(broker.authorize(
            &proposal.proposal_id,
            &current,
            ActionApproval::Reviewed,
            1_000 + ACTION_PROPOSAL_TTL_MS,
        ));
        assert!(message.contains("expired"));
    }

    #[test]
    fn proposal_grant_is_one_use_and_replay_is_rejected() {
        let mut broker = ActionBroker::default();
        let current = snapshot("scan-a", true);
        let proposal = broker
            .prepare(request("flush_dns", None), &current, 1_000)
            .unwrap();

        broker
            .authorize(
                &proposal.proposal_id,
                &current,
                ActionApproval::Reviewed,
                1_001,
            )
            .unwrap();
        let message = rejected(broker.authorize(
            &proposal.proposal_id,
            &current,
            ActionApproval::Reviewed,
            1_002,
        ));
        assert!(message.contains("already been approved"));
    }

    #[test]
    fn approval_rejects_stale_scan_and_catalog_snapshots() {
        let current = snapshot("scan-a", true);

        let mut scan_broker = ActionBroker::default();
        let scan_proposal = scan_broker
            .prepare(request("flush_dns", None), &current, 1_000)
            .unwrap();
        let changed_scan = snapshot("scan-b", true);
        let message = rejected(scan_broker.authorize(
            &scan_proposal.proposal_id,
            &changed_scan,
            ActionApproval::Reviewed,
            1_001,
        ));
        assert!(message.contains("scan changed"));

        let mut catalog_broker = ActionBroker::default();
        let catalog_proposal = catalog_broker
            .prepare(request("flush_dns", None), &current, 2_000)
            .unwrap();
        let mut changed_catalog = current.clone();
        changed_catalog.catalog_fingerprint = "stale-catalog".to_string();
        let message = rejected(catalog_broker.authorize(
            &catalog_proposal.proposal_id,
            &changed_catalog,
            ActionApproval::Reviewed,
            2_001,
        ));
        assert!(message.contains("catalog"));
    }

    #[test]
    fn preparation_and_approval_reject_issue_remediation_mismatches() {
        let current = issue_snapshot("scan-a", "cpu-pressure", "open_task_manager");
        let mut broker = ActionBroker::default();
        let error = broker
            .prepare(
                request("open_disk_cleanup", Some("cpu-pressure")),
                &current,
                1_000,
            )
            .unwrap_err();
        assert!(error.contains("not mapped"));

        let proposal = broker
            .prepare(
                request("open_task_manager", Some("cpu-pressure")),
                &current,
                1_001,
            )
            .unwrap();
        let changed = issue_snapshot("scan-a", "cpu-pressure", "open_disk_cleanup");
        let message = rejected(broker.authorize(
            &proposal.proposal_id,
            &changed,
            ActionApproval::Reviewed,
            1_002,
        ));
        assert!(message.contains("Issue 'cpu-pressure' changed"));

        let unknown = broker
            .prepare(request("not_in_the_catalog", None), &current, 1_003)
            .unwrap_err();
        assert!(unknown.contains("Unknown remediation"));
    }

    #[test]
    fn repair_requires_explicit_confirmation_without_consuming_preview() {
        let mut broker = ActionBroker::default();
        let current = snapshot("scan-a", true);
        let proposal = broker
            .prepare(request("empty_recycle_bin", None), &current, 1_000)
            .unwrap();

        match broker.authorize(
            &proposal.proposal_id,
            &current,
            ActionApproval::Reviewed,
            1_001,
        ) {
            Err(AuthorizationError::RepairConfirmationRequired(review)) => {
                assert_eq!(review, proposal);
            }
            Err(AuthorizationError::Rejected(message)) => {
                panic!("unexpected rejection: {message}")
            }
            Ok(_) => panic!("Repair was authorized without explicit confirmation"),
        }

        broker
            .authorize(
                &proposal.proposal_id,
                &current,
                ActionApproval::RepairConfirmed,
                1_002,
            )
            .unwrap();
        let message = rejected(broker.authorize(
            &proposal.proposal_id,
            &current,
            ActionApproval::RepairConfirmed,
            1_003,
        ));
        assert!(message.contains("already been approved"));
    }

    #[test]
    fn approval_rechecks_administrator_state() {
        let mut broker = ActionBroker::default();
        let elevated = snapshot("scan-a", true);
        let proposal = broker
            .prepare(request("sfc_scannow", None), &elevated, 1_000)
            .unwrap();
        let not_elevated = snapshot("scan-a", false);

        let message = rejected(broker.authorize(
            &proposal.proposal_id,
            &not_elevated,
            ActionApproval::RepairConfirmed,
            1_001,
        ));
        assert!(message.contains("administrator"));
    }

    #[derive(Default)]
    struct RecordingExecutor {
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl AuthorizedActionExecutor for RecordingExecutor {
        fn execute<'a>(
            &'a self,
            remediation_id: &'a str,
            _cancel: &'a CancellationToken,
        ) -> ExecutionFuture<'a> {
            self.calls.lock().unwrap().push(remediation_id.to_string());
            let result = FixResult {
                success: true,
                message: "recorded".to_string(),
                actions_taken: vec![remediation_id.to_string()],
                requires_restart: false,
                completion_status: FixCompletionStatus::Succeeded,
                steps: Vec::new(),
            };
            Box::pin(async move { Ok(result) })
        }
    }

    #[test]
    fn successful_grant_executes_once_through_injected_executor() {
        let mut broker = ActionBroker::default();
        let current = snapshot("scan-a", true);
        let proposal = broker
            .prepare(request("flush_dns", None), &current, 1_000)
            .unwrap();
        let grant = broker
            .authorize(
                &proposal.proposal_id,
                &current,
                ActionApproval::Reviewed,
                1_001,
            )
            .unwrap();
        let executor = RecordingExecutor::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let execution = runtime.block_on(execute_grant(grant, &executor));

        assert_eq!(execution.proposal_id, proposal.proposal_id);
        assert_eq!(execution.items.len(), 1);
        assert!(execution.items[0].result.is_ok());
        assert_eq!(execution.summary.status, ActionRunStatus::Succeeded);
        assert_eq!(
            execution.summary.actions[0].status,
            ActionItemStatus::Succeeded
        );
        assert!(execution.summary.completed_at_ms.is_some());
        assert_eq!(executor.calls.lock().unwrap().as_slice(), ["flush_dns"]);

        let message = rejected(broker.authorize(
            &proposal.proposal_id,
            &current,
            ActionApproval::Reviewed,
            1_002,
        ));
        assert!(message.contains("already been approved"));
    }

    #[test]
    fn structured_progress_preserves_steps_partial_and_restart_semantics() {
        let state = Arc::new(Mutex::new(ActionRuntimeState::default()));
        let (updates, update_rx) = std_mpsc::channel();
        let authorized = {
            let mut state = lock_state(&state);
            state.subscribers.push(updates);
            state
                .record_grant(42, grant(&["flush_dns", "clear_icon_cache"]), 2_000)
                .unwrap()
        };
        let initial = update_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(initial.request_id, 42);
        assert_eq!(initial.summary.status, ActionRunStatus::Running);
        assert_eq!(initial.summary.current_index, None);
        assert!(
            initial
                .summary
                .actions
                .iter()
                .all(|action| action.status == ActionItemStatus::Pending)
        );

        let executor = SequenceExecutor::new([
            Ok(fix_result(
                FixCompletionStatus::Succeeded,
                false,
                vec![step("Flush DNS", RemediationStepStatus::AlreadySatisfied)],
            )),
            Ok(fix_result(
                FixCompletionStatus::Partial,
                true,
                vec![
                    step("Stop Explorer", RemediationStepStatus::Succeeded),
                    step("Delete cache", RemediationStepStatus::Failed),
                ],
            )),
        ]);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let execution = runtime.block_on(execute_authorized_run(
            Arc::clone(&state),
            authorized,
            &executor,
        ));

        assert_eq!(execution.summary.status, ActionRunStatus::Partial);
        assert_eq!(execution.summary.current_index, None);
        assert_eq!(
            execution
                .summary
                .actions
                .iter()
                .map(|action| action.status)
                .collect::<Vec<_>>(),
            [ActionItemStatus::Succeeded, ActionItemStatus::Partial]
        );
        assert_eq!(
            execution.summary.actions[0].result.as_ref().unwrap().steps[0].status,
            RemediationStepStatus::AlreadySatisfied
        );
        assert_eq!(
            execution.summary.actions[1].result.as_ref().unwrap().steps[1].status,
            RemediationStepStatus::Failed
        );
        assert!(execution.summary.requires_restart());
        assert!(lock_state(&state).active_run_id.is_none());

        let events = update_rx.try_iter().collect::<Vec<_>>();
        assert!(events.iter().any(|event| {
            event.summary.current_index == Some(0)
                && event.summary.actions[0].status == ActionItemStatus::Running
        }));
        assert!(events.iter().any(|event| {
            event.summary.current_index == Some(1)
                && event.summary.actions[1].status == ActionItemStatus::Running
        }));
        assert_eq!(
            events.last().unwrap().summary.status,
            ActionRunStatus::Partial
        );
    }

    #[test]
    fn failure_after_success_is_partial_and_later_actions_are_skipped() {
        let state = Arc::new(Mutex::new(ActionRuntimeState::default()));
        let authorized = lock_state(&state)
            .record_grant(
                7,
                grant(&["flush_dns", "clear_icon_cache", "open_task_manager"]),
                2_000,
            )
            .unwrap();
        let executor = SequenceExecutor::new([
            Ok(fix_result(
                FixCompletionStatus::Succeeded,
                false,
                vec![step("Flush DNS", RemediationStepStatus::Succeeded)],
            )),
            Err("cache command failed".to_string()),
        ]);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let execution = runtime.block_on(execute_authorized_run(
            Arc::clone(&state),
            authorized,
            &executor,
        ));

        assert_eq!(execution.summary.status, ActionRunStatus::Partial);
        assert_eq!(
            execution
                .summary
                .actions
                .iter()
                .map(|action| action.status)
                .collect::<Vec<_>>(),
            [
                ActionItemStatus::Succeeded,
                ActionItemStatus::Failed,
                ActionItemStatus::Skipped,
            ]
        );
        assert_eq!(
            execution.summary.actions[1].error.as_deref(),
            Some("cache command failed")
        );
        assert_eq!(execution.items.len(), 2);
        assert_eq!(
            executor.calls.lock().unwrap().as_slice(),
            ["flush_dns", "clear_icon_cache"]
        );
    }

    #[test]
    fn infrastructure_failure_after_completed_work_is_partial() {
        let mut state = ActionRuntimeState::default();
        let authorized = state
            .record_grant(
                71,
                grant(&["flush_dns", "clear_icon_cache", "open_task_manager"]),
                2_000,
            )
            .unwrap();
        {
            let record = state.run_mut(&authorized.run_id).unwrap();
            record.summary.actions[0].status = ActionItemStatus::Succeeded;
            record.summary.current_index = Some(1);
            record.summary.actions[1].status = ActionItemStatus::Running;
        }

        let summary = state
            .fail_run(&authorized.run_id, "executor stopped".to_string())
            .unwrap();

        assert_eq!(summary.status, ActionRunStatus::Partial);
        assert_eq!(
            summary
                .actions
                .iter()
                .map(|action| action.status)
                .collect::<Vec<_>>(),
            [
                ActionItemStatus::Succeeded,
                ActionItemStatus::Failed,
                ActionItemStatus::Skipped,
            ]
        );
        assert_eq!(
            summary.actions[1].error.as_deref(),
            Some("executor stopped")
        );
        assert!(state.active_run_id.is_none());
    }

    #[test]
    fn cancellation_before_execution_marks_every_unfinished_action_cancelled() {
        let state = Arc::new(Mutex::new(ActionRuntimeState::default()));
        let authorized = lock_state(&state)
            .record_grant(8, grant(&["flush_dns", "clear_icon_cache"]), 2_000)
            .unwrap();
        let run_id = authorized.run_id.clone();
        let cancelled = lock_state(&state).cancel_run(&run_id).unwrap();
        assert_eq!(cancelled.status, ActionRunStatus::CancelRequested);
        let executor = RecordingExecutor::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let execution = runtime.block_on(execute_authorized_run(
            Arc::clone(&state),
            authorized,
            &executor,
        ));

        assert_eq!(execution.summary.status, ActionRunStatus::Cancelled);
        assert!(
            execution
                .summary
                .actions
                .iter()
                .all(|action| action.status == ActionItemStatus::Cancelled)
        );
        assert!(execution.items.is_empty());
        assert!(executor.calls.lock().unwrap().is_empty());
    }

    struct WaitForCancellationExecutor {
        started: Mutex<Option<std_mpsc::Sender<()>>>,
    }

    impl AuthorizedActionExecutor for WaitForCancellationExecutor {
        fn execute<'a>(
            &'a self,
            remediation_id: &'a str,
            cancel: &'a CancellationToken,
        ) -> ExecutionFuture<'a> {
            if let Some(started) = self.started.lock().unwrap().take() {
                let _ = started.send(());
            }
            let action = remediation_id.to_string();
            Box::pin(async move {
                cancel.cancelled().await;
                Ok(fix_result(
                    FixCompletionStatus::Cancelled,
                    false,
                    vec![step(&action, RemediationStepStatus::Cancelled)],
                ))
            })
        }
    }

    #[test]
    fn in_flight_cancel_is_immediate_and_publishes_cancel_requested_then_terminal() {
        let state = Arc::new(Mutex::new(ActionRuntimeState::default()));
        let (subscriber, updates) = std_mpsc::channel();
        let authorized = {
            let mut state = lock_state(&state);
            state.subscribers.push(subscriber);
            state.record_grant(9, grant(&["flush_dns"]), 2_000).unwrap()
        };
        let run_id = authorized.run_id.clone();
        let (started_tx, started_rx) = std_mpsc::channel();
        let execution_state = Arc::clone(&state);
        let execution_thread = std::thread::spawn(move || {
            let executor = WaitForCancellationExecutor {
                started: Mutex::new(Some(started_tx)),
            };
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(execute_authorized_run(
                execution_state,
                authorized,
                &executor,
            ))
        });

        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let cancel_projection = lock_state(&state).cancel_run(&run_id).unwrap();
        assert_eq!(cancel_projection.status, ActionRunStatus::CancelRequested);
        let execution = execution_thread.join().unwrap();
        assert_eq!(execution.summary.status, ActionRunStatus::Cancelled);
        assert_eq!(
            execution.summary.actions[0].status,
            ActionItemStatus::Cancelled
        );

        let statuses = updates
            .try_iter()
            .map(|event| event.summary.status)
            .collect::<Vec<_>>();
        assert!(statuses.contains(&ActionRunStatus::CancelRequested));
        assert_eq!(statuses.last(), Some(&ActionRunStatus::Cancelled));
    }

    #[test]
    fn cancellation_rejects_a_started_non_cancellable_action() {
        let mut state = ActionRuntimeState::default();
        let authorized = state
            .record_grant(10, grant(&["sfc_scannow"]), 2_000)
            .unwrap();
        let run_id = authorized.run_id.clone();
        {
            let record = state.run_mut(&run_id).unwrap();
            record.summary.current_index = Some(0);
            record.summary.actions[0].status = ActionItemStatus::Running;
        }
        let error = state.cancel_run(&run_id).unwrap_err();
        assert!(error.contains("cannot be stopped safely"));
        assert!(!authorized.cancel.is_cancelled());
        assert_eq!(
            state.run(&run_id).unwrap().summary.status,
            ActionRunStatus::Running
        );
    }

    #[test]
    fn late_cancel_does_not_rewrite_a_successful_terminal_run() {
        let state = Arc::new(Mutex::new(ActionRuntimeState::default()));
        let authorized = lock_state(&state)
            .record_grant(11, grant(&["flush_dns"]), 2_000)
            .unwrap();
        let run_id = authorized.run_id.clone();
        let executor = RecordingExecutor::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let execution = runtime.block_on(execute_authorized_run(
            Arc::clone(&state),
            authorized,
            &executor,
        ));
        assert_eq!(execution.summary.status, ActionRunStatus::Succeeded);

        let after_cancel = lock_state(&state).cancel_run(&run_id).unwrap();
        assert_eq!(after_cancel.status, ActionRunStatus::Succeeded);
        assert_eq!(after_cancel.actions[0].status, ActionItemStatus::Succeeded);
    }

    #[test]
    fn concurrent_run_rejection_does_not_consume_the_second_proposal() {
        let current = snapshot("scan-a", true);
        let mut state = ActionRuntimeState::default();
        let first = state
            .broker
            .prepare(request("flush_dns", None), &current, 1_000)
            .unwrap();
        let second = state
            .broker
            .prepare(request("flush_dns", None), &current, 1_001)
            .unwrap();
        let active = state
            .authorize_run(
                1,
                &first.proposal_id,
                &current,
                ActionApproval::Reviewed,
                1_002,
            )
            .unwrap();

        let rejected = state.authorize_run(
            2,
            &second.proposal_id,
            &current,
            ActionApproval::Reviewed,
            1_003,
        );
        assert!(matches!(
            rejected,
            Err(AuthorizationError::Rejected(message)) if message.contains("already running")
        ));
        assert!(
            state
                .broker
                .pending(1_004)
                .iter()
                .any(|proposal| proposal.proposal_id == second.proposal_id)
        );

        state.fail_run(&active.run_id, "test completion".to_string());
        assert!(
            state
                .authorize_run(
                    2,
                    &second.proposal_id,
                    &current,
                    ActionApproval::Reviewed,
                    1_005,
                )
                .is_ok()
        );
    }

    #[test]
    fn history_is_newest_first_bounded_and_atomic_snapshot_keeps_pending_state() {
        let mut state = ActionRuntimeState::default();
        let current = snapshot("scan-a", true);
        let pending = state
            .broker
            .prepare(request("flush_dns", None), &current, now_ms())
            .unwrap();
        let mut run_ids = Vec::new();
        for index in 0..=MAX_ACTION_HISTORY {
            let authorized = state
                .record_grant(index as u64, grant(&["flush_dns"]), index as u64)
                .unwrap();
            run_ids.push(authorized.run_id.clone());
            state.fail_run(&authorized.run_id, format!("failure {index}"));
        }
        assert_eq!(state.runs.len(), MAX_ACTION_HISTORY);
        assert!(state.run(&run_ids[0]).is_none());

        let (subscriber, updates) = std_mpsc::channel();
        state.subscribers.push(subscriber);
        let captured = state.snapshot();
        assert_eq!(captured.history.len(), MAX_ACTION_HISTORY);
        assert_eq!(captured.history[0].run_id, *run_ids.last().unwrap());
        assert_eq!(captured.history.last().unwrap().run_id, run_ids[1]);
        assert!(captured.active_run.is_none());
        assert!(
            captured
                .pending_proposals
                .iter()
                .any(|proposal| proposal.proposal_id == pending.proposal_id)
        );

        let live = state
            .record_grant(999, grant(&["flush_dns"]), 9_999)
            .unwrap();
        let update = updates.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(update.request_id, 999);
        assert_eq!(update.summary.run_id, live.run_id);
        assert_eq!(update.summary.status, ActionRunStatus::Running);
    }

    fn item(status: ActionItemStatus) -> ActionItemRun {
        ActionItemRun {
            remediation_id: "test".to_string(),
            label: "Test".to_string(),
            cancellable: true,
            status,
            result: None,
            error: None,
        }
    }

    #[test]
    fn aggregate_status_table_matches_shipping_semantics() {
        assert_eq!(
            completed_run_status(&[item(ActionItemStatus::Succeeded)], false),
            ActionRunStatus::Succeeded
        );
        assert_eq!(
            completed_run_status(&[item(ActionItemStatus::Failed)], false),
            ActionRunStatus::Failed
        );
        assert_eq!(
            completed_run_status(
                &[
                    item(ActionItemStatus::Succeeded),
                    item(ActionItemStatus::Failed),
                ],
                false,
            ),
            ActionRunStatus::Partial
        );
        assert_eq!(
            completed_run_status(&[item(ActionItemStatus::Partial)], false),
            ActionRunStatus::Partial
        );
        assert_eq!(
            completed_run_status(&[item(ActionItemStatus::Pending)], true),
            ActionRunStatus::Cancelled
        );
        assert!(!cancellation_applies(
            &[item(ActionItemStatus::Succeeded)],
            true
        ));
        assert!(cancellation_applies(
            &[
                item(ActionItemStatus::Succeeded),
                item(ActionItemStatus::Pending),
            ],
            true,
        ));
    }

    #[test]
    fn public_runtime_rehydrates_pending_proposals_without_executing_them() {
        let (runtime, legacy_events) = NativeActionRuntime::start().unwrap();
        let (run_events, initial) = runtime.subscribe_run_events();
        assert!(initial.pending_proposals.is_empty());
        assert!(initial.history.is_empty());
        assert!(initial.active_run.is_none());
        assert!(runtime.prepare(123, request("flush_dns", None), snapshot("scan-a", true),));
        let prepared = match legacy_events.recv_timeout(Duration::from_secs(2)) {
            Ok(ActionWorkerEvent::Prepared { proposal, .. }) => proposal,
            Ok(other) => panic!("expected Prepared, got {other:?}"),
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for preparation"),
            Err(RecvTimeoutError::Disconnected) => panic!("action worker disconnected"),
        };
        assert!(
            runtime
                .list_pending_proposals()
                .iter()
                .any(|proposal| proposal.proposal_id == prepared.proposal_id)
        );
        assert!(runtime.list_history().is_empty());
        assert!(run_events.try_recv().is_err());
        assert!(runtime.discard(prepared.proposal_id));
    }
}
