//! Trusted action broker for catalog-backed system changes.
//!
//! The frontend and AI may name remediation IDs, but they never provide a
//! program, argument, path, or authorization boolean. Preparation resolves an
//! immutable preview from the compile-time catalog. Approval atomically
//! consumes a one-use internal grant and reserves the app's sole mutation
//! slot before execution starts.

use crate::issue_catalog::{DetectCtx, Issue};
use crate::remediation::{FixCompletionStatus, FixResult, RemediationSummary, RunKind};
use crate::state::{AppState, DiagnosticSession};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, State};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub const PROPOSAL_TTL_MS: u64 = 10 * 60 * 1_000;
pub const MAX_BATCH_ACTIONS: usize = 5;
const MAX_PENDING_PROPOSALS: usize = 100;
const MAX_ACTION_HISTORY: usize = 50;
const CATALOG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionRequest {
    pub remediation_id: String,
    #[serde(default)]
    pub issue_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionPrepareInput {
    pub actions: Vec<ActionRequest>,
    #[serde(default)]
    pub expected_scan_fingerprint: Option<String>,
    #[serde(default)]
    pub expected_catalog_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    Exact,
    Batch,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionPreview {
    pub remediation: RemediationSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<String>,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionProposal {
    pub proposal_id: String,
    pub approval_scope: ApprovalScope,
    pub actions: Vec<ActionPreview>,
    pub scan_fingerprint: String,
    pub catalog_fingerprint: String,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionItemStatus {
    Pending,
    Running,
    Succeeded,
    Partial,
    Failed,
    Cancelled,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionRunStatus {
    Running,
    CancelRequested,
    Succeeded,
    Partial,
    Failed,
    Cancelled,
}

impl ActionRunStatus {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Partial | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItemRun {
    pub remediation_id: String,
    pub label: String,
    pub cancellable: bool,
    pub status: ActionItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<FixResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRunSummary {
    pub run_id: String,
    pub proposal_id: String,
    /// Backend-generated one-use authorization identifier. It is audit
    /// metadata, not a capability accepted by any IPC command.
    pub authorization_id: String,
    pub status: ActionRunStatus,
    pub actions: Vec<ActionItemRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_index: Option<usize>,
    pub approved_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
    pub scan_fingerprint: String,
    pub catalog_fingerprint: String,
}

struct StoredProposal {
    proposal: ActionProposal,
    consumed: bool,
}

struct ActionRunRecord {
    summary: ActionRunSummary,
    cancel: CancellationToken,
}

/// Private, non-deserializable grant. Moving it into `AuthorizedRun` is its
/// one permitted consumption; callers can never manufacture one from IPC.
#[derive(Debug)]
struct ActionGrant {
    authorization_id: String,
    proposal: ActionProposal,
}

#[derive(Debug)]
struct AuthorizedRun {
    run_id: String,
    grant: ActionGrant,
    cancel: CancellationToken,
}

#[derive(Default)]
pub struct ActionBrokerState {
    proposals: HashMap<String, StoredProposal>,
    runs: VecDeque<ActionRunRecord>,
    active_run_id: Option<String>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Stable FNV-1a hash. These fingerprints detect stale evidence/catalog
/// changes; authorization secrecy comes from an unguessable UUID and the
/// private grant type, not from this hash.
fn stable_fingerprint<T: Hash>(value: &T) -> String {
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

pub(crate) fn scan_fingerprint(session: Option<&DiagnosticSession>) -> String {
    let Some(session) = session else {
        return stable_fingerprint(&("no_scan", CATALOG_SCHEMA_VERSION));
    };
    let mut selected = session.selected_tasks.clone();
    selected.sort();
    let mut results: Vec<_> = session.results.iter().collect();
    results.sort_by_key(|(id, _)| *id);
    let material: Vec<_> = results
        .into_iter()
        .map(|(id, result)| {
            (
                id.as_str(),
                result.success,
                result.output.as_str(),
                result.error.as_deref(),
            )
        })
        .collect();
    stable_fingerprint(&(session.session_id.as_str(), selected, material))
}

pub(crate) fn catalog_fingerprint() -> String {
    let mut material = Vec::new();
    for spec in crate::remediation::remediations() {
        let run = match &spec.run {
            RunKind::Spawn { program, args } => {
                format!("spawn|{}|{}", program, args.join("\u{1f}"))
            }
            RunKind::Steps {
                steps,
                timeout_secs,
                success_msg,
            } => format!(
                "steps|{}|{}|{}",
                timeout_secs,
                success_msg,
                steps
                    .iter()
                    .map(|step| format!(
                        "{}|{}|{}|{}",
                        step.program,
                        step.args.join("\u{1f}"),
                        step.ignore_failure,
                        step.action_label
                    ))
                    .collect::<Vec<_>>()
                    .join("\u{1e}")
            ),
            RunKind::Custom { .. } => format!("custom-v{}", CATALOG_SCHEMA_VERSION),
        };
        material.push(format!(
            "{}|{}|{}|{:?}|{}|{}|{}|{}|{}",
            spec.id,
            spec.label,
            spec.description,
            spec.tier,
            spec.admin_required,
            spec.requires_restart,
            spec.long_running,
            spec.maintenance,
            run
        ));
    }
    stable_fingerprint(&(CATALOG_SCHEMA_VERSION, material))
}

fn detected_issues(session: Option<&DiagnosticSession>) -> Vec<Issue> {
    let temp_file_count = count_temp_entries();
    detected_issues_with(session, temp_file_count)
}

/// Blocking %TEMP% enumeration, kept SEPARATE from detection so callers
/// holding the scan-session lock can run it beforehand (see action_approve).
fn count_temp_entries() -> Option<usize> {
    std::fs::read_dir(std::env::temp_dir()).ok().map(|entries| entries.count())
}

fn detected_issues_with(
    session: Option<&DiagnosticSession>,
    temp_file_count: Option<usize>,
) -> Vec<Issue> {
    let Some(session) = session else {
        return Vec::new();
    };
    crate::issue_catalog::detect_all(&DetectCtx {
        results: &session.results,
        now: crate::timestamp::Timestamp::now(),
        temp_file_count,
    })
}

fn build_proposal(
    input: ActionPrepareInput,
    issues: &[Issue],
    current_scan_fingerprint: String,
    current_catalog_fingerprint: String,
    created_at_ms: u64,
) -> Result<ActionProposal, String> {
    if input.actions.is_empty() {
        return Err("At least one action is required".to_string());
    }
    if input.actions.len() > MAX_BATCH_ACTIONS {
        return Err(format!(
            "At most {} actions can be approved together",
            MAX_BATCH_ACTIONS
        ));
    }
    if input
        .expected_scan_fingerprint
        .as_deref()
        .is_some_and(|expected| expected != current_scan_fingerprint)
    {
        return Err(
            "The scan changed after this plan was created. Generate a new plan.".to_string(),
        );
    }
    if input
        .expected_catalog_fingerprint
        .as_deref()
        .is_some_and(|expected| expected != current_catalog_fingerprint)
    {
        return Err("The remediation catalog changed. Review the action again.".to_string());
    }

    let batch = input.actions.len() > 1;
    let mut seen = std::collections::HashSet::new();
    let mut actions = Vec::with_capacity(input.actions.len());
    for request in input.actions {
        if !seen.insert(request.remediation_id.clone()) {
            return Err(format!(
                "Remediation '{}' appears more than once",
                request.remediation_id
            ));
        }
        let spec = crate::remediation::find(&request.remediation_id)
            .ok_or_else(|| format!("Unknown remediation '{}'", request.remediation_id))?;
        if batch && !spec.batch_eligible() {
            return Err(format!(
                "'{}' requires exact approval and cannot be included in a batch",
                spec.label
            ));
        }
        match request.issue_id.as_deref() {
            Some(issue_id) => {
                let issue = issues
                    .iter()
                    .find(|issue| issue.detected && issue.id == issue_id)
                    .ok_or_else(|| {
                        format!(
                            "Issue '{}' is no longer detected in the current scan",
                            issue_id
                        )
                    })?;
                if issue
                    .remediation
                    .as_ref()
                    .is_none_or(|remediation| remediation.id != spec.id)
                {
                    return Err(format!(
                        "Remediation '{}' is not mapped to issue '{}'",
                        spec.id, issue_id
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
        proposal_id: format!("proposal_{}", uuid::Uuid::new_v4().simple()),
        approval_scope: if batch {
            ApprovalScope::Batch
        } else {
            ApprovalScope::Exact
        },
        actions,
        scan_fingerprint: current_scan_fingerprint,
        catalog_fingerprint: current_catalog_fingerprint,
        created_at_ms,
        expires_at_ms: created_at_ms.saturating_add(PROPOSAL_TTL_MS),
    })
}

impl ActionBrokerState {
    fn insert_proposal(
        &mut self,
        proposal: ActionProposal,
        current_time_ms: u64,
    ) -> Result<(), String> {
        // Completed runs are the audit record. Consumed and expired previews
        // are no longer capabilities and must not exhaust this bound.
        self.proposals.retain(|_, stored| {
            !stored.consumed && stored.proposal.expires_at_ms > current_time_ms
        });
        if self.proposals.len() >= MAX_PENDING_PROPOSALS {
            return Err(
                "Too many action previews are pending; finish or dismiss one first".to_string(),
            );
        }
        self.proposals.insert(
            proposal.proposal_id.clone(),
            StoredProposal {
                proposal,
                consumed: false,
            },
        );
        Ok(())
    }

    fn authorize(
        &mut self,
        proposal_id: &str,
        current_scan_fingerprint: &str,
        current_catalog_fingerprint: &str,
        current_issues: &[Issue],
        is_admin: bool,
        approved_at_ms: u64,
    ) -> Result<AuthorizedRun, String> {
        let stored = self
            .proposals
            .get_mut(proposal_id)
            .ok_or_else(|| "Action preview was not found or has expired".to_string())?;
        if stored.consumed {
            return Err("This action preview has already been approved".to_string());
        }
        if stored.proposal.expires_at_ms <= approved_at_ms {
            return Err("This action preview expired. Review the action again.".to_string());
        }
        if stored.proposal.scan_fingerprint != current_scan_fingerprint {
            return Err("The scan changed after this action was reviewed".to_string());
        }
        if stored.proposal.catalog_fingerprint != current_catalog_fingerprint {
            return Err(
                "The remediation catalog changed after this action was reviewed".to_string(),
            );
        }
        for action in &stored.proposal.actions {
            let Some(issue_id) = action.issue_id.as_deref() else {
                continue;
            };
            let still_valid = current_issues.iter().any(|issue| {
                issue.detected
                    && issue.id == issue_id
                    && issue
                        .remediation
                        .as_ref()
                        .is_some_and(|remediation| remediation.id == action.remediation.id)
            });
            if !still_valid {
                return Err(format!(
                    "Issue '{}' changed after this action was reviewed",
                    issue_id
                ));
            }
        }
        if stored
            .proposal
            .actions
            .iter()
            .any(|action| action.remediation.admin_required)
            && !is_admin
        {
            return Err("This action requires administrator rights".to_string());
        }
        if self.active_run_id.is_some() {
            return Err("Another system action is already running".to_string());
        }

        // Grant creation, consumption, and mutation-slot reservation happen
        // while holding this single lock. There is no replayable gap.
        stored.consumed = true;
        let grant = ActionGrant {
            authorization_id: format!("grant_{}", uuid::Uuid::new_v4().simple()),
            proposal: stored.proposal.clone(),
        };
        let run_id = format!("run_{}", uuid::Uuid::new_v4().simple());
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
            summary,
            cancel: cancel.clone(),
        });
        while self.runs.len() > MAX_ACTION_HISTORY {
            self.runs.pop_front();
        }
        Ok(AuthorizedRun {
            run_id,
            grant,
            cancel,
        })
    }

    fn run_mut(&mut self, run_id: &str) -> Option<&mut ActionRunRecord> {
        self.runs
            .iter_mut()
            .find(|record| record.summary.run_id == run_id)
    }

    fn run(&self, run_id: &str) -> Option<&ActionRunRecord> {
        self.runs
            .iter()
            .find(|record| record.summary.run_id == run_id)
    }
}

async fn stage_proposal(
    state: &AppState,
    request: ActionPrepareInput,
) -> Result<ActionProposal, String> {
    stage_proposal_from_parts(&state.action_broker, &state.current_session, request).await
}

async fn stage_proposal_from_parts(
    broker: &Arc<Mutex<ActionBrokerState>>,
    current_session: &Arc<Mutex<Option<DiagnosticSession>>>,
    request: ActionPrepareInput,
) -> Result<ActionProposal, String> {
    let session = current_session.lock().await.clone();
    let proposal = build_proposal(
        request,
        &detected_issues(session.as_ref()),
        scan_fingerprint(session.as_ref()),
        catalog_fingerprint(),
        now_ms(),
    )?;
    broker
        .lock()
        .await
        .insert_proposal(proposal.clone(), now_ms())?;
    Ok(proposal)
}

/// Runtime-parts variant for the read-only chat tool executor, which owns
/// cloned state Arcs rather than a Tauri `State<AppState>`.
pub(crate) async fn stage_exact_proposal_from_parts(
    broker: &Arc<Mutex<ActionBrokerState>>,
    current_session: &Arc<Mutex<Option<DiagnosticSession>>>,
    request: ActionPrepareInput,
) -> Result<ActionProposal, String> {
    if request.actions.len() != 1 {
        return Err("AI may stage exactly one action at a time".to_string());
    }
    stage_proposal_from_parts(broker, current_session, request).await
}

#[tauri::command]
pub async fn action_prepare(
    state: State<'_, AppState>,
    request: ActionPrepareInput,
) -> Result<ActionProposal, String> {
    stage_proposal(state.inner(), request).await
}

#[tauri::command]
pub async fn action_get_proposal(
    state: State<'_, AppState>,
    proposal_id: String,
) -> Result<ActionProposal, String> {
    let mut broker = state.action_broker.lock().await;
    let current_time = now_ms();
    let expired = broker
        .proposals
        .get(&proposal_id)
        .is_some_and(|stored| stored.consumed || stored.proposal.expires_at_ms <= current_time);
    if expired {
        broker.proposals.remove(&proposal_id);
    }
    broker
        .proposals
        .get(&proposal_id)
        .map(|stored| stored.proposal.clone())
        .ok_or_else(|| "Action preview was not found, was used, or expired".to_string())
}

#[tauri::command]
pub async fn action_list_pending_proposals(
    state: State<'_, AppState>,
) -> Result<Vec<ActionProposal>, String> {
    let mut broker = state.action_broker.lock().await;
    let current_time = now_ms();
    broker
        .proposals
        .retain(|_, stored| !stored.consumed && stored.proposal.expires_at_ms > current_time);
    let mut proposals: Vec<_> = broker
        .proposals
        .values()
        .map(|stored| stored.proposal.clone())
        .collect();
    proposals.sort_by_key(|proposal| proposal.created_at_ms);
    Ok(proposals)
}

#[tauri::command]
pub async fn action_discard_proposal(
    state: State<'_, AppState>,
    proposal_id: String,
) -> Result<(), String> {
    let mut broker = state.action_broker.lock().await;
    let Some(stored) = broker.proposals.get(&proposal_id) else {
        return Ok(());
    };
    if stored.consumed {
        return Err("An approved action preview cannot be dismissed".to_string());
    }
    broker.proposals.remove(&proposal_id);
    Ok(())
}

#[tauri::command]
pub async fn action_approve(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    proposal_id: String,
) -> Result<ActionRunSummary, String> {
    let is_admin = crate::get_system_info().await?.is_admin;
    // The %TEMP% walk is blocking IO: run it BEFORE taking the scan snapshot
    // so no directory enumeration happens while the session mutex is held.
    let temp_file_count = count_temp_entries();
    // Hold the scan snapshot through grant creation so a scan update cannot
    // land between fingerprint validation and authorization consumption.
    let session = state.current_session.lock().await;
    let current_scan = scan_fingerprint(session.as_ref());
    let current_issues = detected_issues_with(session.as_ref(), temp_file_count);
    let current_catalog = catalog_fingerprint();
    let authorized = state.action_broker.lock().await.authorize(
        &proposal_id,
        &current_scan,
        &current_catalog,
        &current_issues,
        is_admin,
        now_ms(),
    )?;
    let initial = state
        .action_broker
        .lock()
        .await
        .run(&authorized.run_id)
        .expect("authorized run was not recorded")
        .summary
        .clone();
    let broker = state.action_broker.clone();
    tauri::async_runtime::spawn(execute_authorized(app, broker, authorized));
    Ok(initial)
}

async fn snapshot(
    broker: &Arc<Mutex<ActionBrokerState>>,
    run_id: &str,
) -> Option<ActionRunSummary> {
    broker
        .lock()
        .await
        .run(run_id)
        .map(|record| record.summary.clone())
}

fn emit_status(app: &tauri::AppHandle, summary: &ActionRunSummary) {
    let _ = app.emit("action://status", summary);
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

async fn execute_authorized(
    app: tauri::AppHandle,
    broker: Arc<Mutex<ActionBrokerState>>,
    authorized: AuthorizedRun,
) {
    let run_id = authorized.run_id.clone();
    // Moving the private grant into this task consumes the only authorization.
    let actions = authorized.grant.proposal.actions;
    for (index, action) in actions.iter().enumerate() {
        if authorized.cancel.is_cancelled() {
            break;
        }
        {
            let mut state = broker.lock().await;
            let Some(record) = state.run_mut(&run_id) else {
                return;
            };
            record.summary.current_index = Some(index);
            record.summary.actions[index].status = ActionItemStatus::Running;
            emit_status(&app, &record.summary);
        }

        let outcome = crate::remediation::execute_authorized(
            &action.remediation.id,
            &crate::remediation::RealRunner,
            &authorized.cancel,
        )
        .await;

        let stop = {
            let mut state = broker.lock().await;
            let Some(record) = state.run_mut(&run_id) else {
                return;
            };
            let stop = match outcome {
                Ok(result) => {
                    record.summary.actions[index].status = match result.completion_status {
                        FixCompletionStatus::Succeeded => ActionItemStatus::Succeeded,
                        FixCompletionStatus::Partial => ActionItemStatus::Partial,
                        FixCompletionStatus::Failed => ActionItemStatus::Failed,
                        FixCompletionStatus::Cancelled => ActionItemStatus::Cancelled,
                    };
                    let stop = result.completion_status != FixCompletionStatus::Succeeded;
                    record.summary.actions[index].result = Some(result);
                    stop
                }
                Err(error) => {
                    record.summary.actions[index].status = ActionItemStatus::Failed;
                    record.summary.actions[index].error = Some(error);
                    true
                }
            };
            emit_status(&app, &record.summary);
            stop
        };
        if stop {
            break;
        }
    }

    let final_summary = {
        let mut state = broker.lock().await;
        let Some(record) = state.run_mut(&run_id) else {
            return;
        };
        // A late cancel click must not rewrite an action that already finished
        // successfully as cancelled. It only cancels unfinished work.
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
        let summary = record.summary.clone();
        state.active_run_id = None;
        summary
    };
    emit_status(&app, &final_summary);
}

#[tauri::command]
pub async fn action_cancel(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<ActionRunSummary, String> {
    let mut broker = state.action_broker.lock().await;
    let record = broker
        .run_mut(&run_id)
        .ok_or_else(|| "Action run was not found".to_string())?;
    if record.summary.status.terminal() {
        return Ok(record.summary.clone());
    }
    if let Some(index) = record.summary.current_index {
        let remediation_id = &record.summary.actions[index].remediation_id;
        if !crate::remediation::find(remediation_id).is_some_and(|spec| spec.cancellable()) {
            return Err("The current action cannot be stopped safely once started".to_string());
        }
    }
    record.cancel.cancel();
    record.summary.status = ActionRunStatus::CancelRequested;
    Ok(record.summary.clone())
}

#[tauri::command]
pub async fn action_get_status(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<ActionRunSummary, String> {
    snapshot(&state.action_broker, &run_id)
        .await
        .ok_or_else(|| "Action run was not found".to_string())
}

#[tauri::command]
pub async fn action_list_history(
    state: State<'_, AppState>,
) -> Result<Vec<ActionRunSummary>, String> {
    let broker = state.action_broker.lock().await;
    Ok(broker
        .runs
        .iter()
        .rev()
        .map(|record| record.summary.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::TaskResult;
    use crate::issue_catalog::{IssueSeverity, IssueStatus};
    use std::collections::HashMap;

    fn single(remediation_id: &str) -> ActionPrepareInput {
        ActionPrepareInput {
            actions: vec![ActionRequest {
                remediation_id: remediation_id.to_string(),
                issue_id: None,
            }],
            expected_scan_fingerprint: None,
            expected_catalog_fingerprint: None,
        }
    }

    fn proposal(remediation_id: &str, now: u64) -> ActionProposal {
        build_proposal(
            single(remediation_id),
            &[],
            "scan-a".to_string(),
            "catalog-a".to_string(),
            now,
        )
        .unwrap()
    }

    fn issue(id: &str, remediation_id: &str) -> Issue {
        Issue {
            id: id.to_string(),
            category: "System".to_string(),
            severity: IssueSeverity::Warning,
            status: IssueStatus::Detected,
            title: "Detected issue".to_string(),
            description: "Detected for an action broker test".to_string(),
            recommendation: "Review the catalog action".to_string(),
            detected: true,
            source_tasks: None,
            remediation: crate::remediation::summary(remediation_id),
        }
    }

    fn run_item(status: ActionItemStatus) -> ActionItemRun {
        ActionItemRun {
            remediation_id: "flush_dns".into(),
            label: "Flush DNS cache".into(),
            cancellable: true,
            status,
            result: None,
            error: None,
        }
    }

    #[test]
    fn proposal_is_immutable_catalog_data_with_ten_minute_expiry() {
        let proposal = proposal("flush_dns", 1_000);
        assert_eq!(proposal.approval_scope, ApprovalScope::Exact);
        assert_eq!(proposal.expires_at_ms, 1_000 + PROPOSAL_TTL_MS);
        assert_eq!(proposal.actions[0].remediation.id, "flush_dns");
        assert_eq!(proposal.actions[0].steps, vec!["ipconfig /flushdns"]);
    }

    #[test]
    fn batch_rejects_high_risk_duplicates_and_overflow() {
        let duplicate = ActionPrepareInput {
            actions: vec![
                ActionRequest {
                    remediation_id: "flush_dns".into(),
                    issue_id: None,
                },
                ActionRequest {
                    remediation_id: "flush_dns".into(),
                    issue_id: None,
                },
            ],
            expected_scan_fingerprint: None,
            expected_catalog_fingerprint: None,
        };
        assert!(build_proposal(duplicate, &[], "s".into(), "c".into(), 0).is_err());

        let high_risk = ActionPrepareInput {
            actions: vec![
                ActionRequest {
                    remediation_id: "flush_dns".into(),
                    issue_id: None,
                },
                ActionRequest {
                    remediation_id: "sfc_scannow".into(),
                    issue_id: None,
                },
            ],
            expected_scan_fingerprint: None,
            expected_catalog_fingerprint: None,
        };
        assert!(build_proposal(high_risk, &[], "s".into(), "c".into(), 0).is_err());

        let overflow = ActionPrepareInput {
            actions: (0..=MAX_BATCH_ACTIONS)
                .map(|index| ActionRequest {
                    remediation_id: format!("action-{index}"),
                    issue_id: None,
                })
                .collect(),
            expected_scan_fingerprint: None,
            expected_catalog_fingerprint: None,
        };
        assert!(build_proposal(overflow, &[], "s".into(), "c".into(), 0).is_err());
    }

    #[test]
    fn request_schema_rejects_frontend_programs_and_arguments() {
        let untrusted = serde_json::json!({
            "actions": [{
                "remediationId": "flush_dns",
                "program": "powershell",
                "args": ["-Command", "Remove-Item C:\\\\*"]
            }]
        });
        assert!(serde_json::from_value::<ActionPrepareInput>(untrusted).is_err());
    }

    #[tokio::test]
    async fn chat_staging_is_exact_only() {
        let broker = Arc::new(Mutex::new(ActionBrokerState::default()));
        let session = Arc::new(Mutex::new(None));
        let request = ActionPrepareInput {
            actions: vec![
                ActionRequest {
                    remediation_id: "flush_dns".into(),
                    issue_id: None,
                },
                ActionRequest {
                    remediation_id: "flush_dns".into(),
                    issue_id: None,
                },
            ],
            expected_scan_fingerprint: None,
            expected_catalog_fingerprint: None,
        };
        assert!(
            stage_exact_proposal_from_parts(&broker, &session, request)
                .await
                .unwrap_err()
                .contains("exactly one")
        );
        assert!(broker.lock().await.proposals.is_empty());
    }

    #[test]
    fn authorization_is_one_use_stale_safe_and_reserves_one_slot() {
        let mut broker = ActionBrokerState::default();
        let first = proposal("flush_dns", 1_000);
        let first_id = first.proposal_id.clone();
        broker.insert_proposal(first, 1_000).unwrap();
        let run = broker
            .authorize(&first_id, "scan-a", "catalog-a", &[], true, 1_001)
            .unwrap();
        assert!(run.grant.authorization_id.starts_with("grant_"));
        assert!(
            broker
                .authorize(&first_id, "scan-a", "catalog-a", &[], true, 1_002)
                .unwrap_err()
                .contains("already been approved")
        );

        let second = proposal("flush_dns", 1_000);
        let second_id = second.proposal_id.clone();
        broker.insert_proposal(second, 1_000).unwrap();
        assert!(
            broker
                .authorize(&second_id, "scan-a", "catalog-a", &[], true, 1_003)
                .unwrap_err()
                .contains("already running")
        );
    }

    #[test]
    fn authorization_rejects_expiry_and_changed_evidence() {
        let mut broker = ActionBrokerState::default();
        let expired = proposal("flush_dns", 1_000);
        let expired_id = expired.proposal_id.clone();
        broker.insert_proposal(expired, 1_000).unwrap();
        assert!(
            broker
                .authorize(
                    &expired_id,
                    "scan-a",
                    "catalog-a",
                    &[],
                    true,
                    1_000 + PROPOSAL_TTL_MS
                )
                .unwrap_err()
                .contains("expired")
        );

        let stale = proposal("flush_dns", 2_000);
        let stale_id = stale.proposal_id.clone();
        broker.insert_proposal(stale, 2_000).unwrap();
        assert!(
            broker
                .authorize(&stale_id, "scan-b", "catalog-a", &[], true, 2_001)
                .unwrap_err()
                .contains("scan changed")
        );

        let mut catalog_broker = ActionBrokerState::default();
        let stale_catalog = proposal("flush_dns", 3_000);
        let stale_catalog_id = stale_catalog.proposal_id.clone();
        catalog_broker
            .insert_proposal(stale_catalog, 3_000)
            .unwrap();
        assert!(
            catalog_broker
                .authorize(&stale_catalog_id, "scan-a", "catalog-b", &[], true, 3_001,)
                .unwrap_err()
                .contains("catalog changed")
        );

        let mut admin_broker = ActionBrokerState::default();
        let admin = proposal("sfc_scannow", 4_000);
        let admin_id = admin.proposal_id.clone();
        admin_broker.insert_proposal(admin, 4_000).unwrap();
        assert!(
            admin_broker
                .authorize(&admin_id, "scan-a", "catalog-a", &[], false, 4_001)
                .unwrap_err()
                .contains("administrator")
        );
    }

    #[test]
    fn authorization_rechecks_issue_mapping_and_prunes_consumed_preview() {
        let detected = issue("dns_issue", "flush_dns");
        let input = ActionPrepareInput {
            actions: vec![ActionRequest {
                remediation_id: "flush_dns".into(),
                issue_id: Some(detected.id.clone()),
            }],
            expected_scan_fingerprint: None,
            expected_catalog_fingerprint: None,
        };
        let staged = build_proposal(
            input,
            std::slice::from_ref(&detected),
            "scan-a".into(),
            "catalog-a".into(),
            1_000,
        )
        .unwrap();
        let proposal_id = staged.proposal_id.clone();
        let mut broker = ActionBrokerState::default();
        broker.insert_proposal(staged, 1_000).unwrap();

        assert!(
            broker
                .authorize(&proposal_id, "scan-a", "catalog-a", &[], true, 1_001)
                .unwrap_err()
                .contains("Issue 'dns_issue' changed")
        );
        broker
            .authorize(
                &proposal_id,
                "scan-a",
                "catalog-a",
                std::slice::from_ref(&detected),
                true,
                1_002,
            )
            .unwrap();

        let next = proposal("flush_dns", 1_003);
        broker.insert_proposal(next, 1_003).unwrap();
        assert!(!broker.proposals.contains_key(&proposal_id));
    }

    #[test]
    fn terminal_status_preserves_partial_and_ignores_too_late_cancel() {
        assert_eq!(
            completed_run_status(&[run_item(ActionItemStatus::Partial)], false),
            ActionRunStatus::Partial
        );
        assert!(!cancellation_applies(
            &[run_item(ActionItemStatus::Succeeded)],
            true
        ));
        assert!(cancellation_applies(
            &[
                run_item(ActionItemStatus::Succeeded),
                run_item(ActionItemStatus::Pending),
            ],
            true
        ));
    }

    #[test]
    fn scan_fingerprint_changes_with_evidence() {
        let mut results = HashMap::new();
        results.insert(
            "os_info".to_string(),
            TaskResult {
                success: true,
                output: "Windows 11".to_string(),
                error: None,
                duration_ms: 1,
            },
        );
        let mut session = DiagnosticSession {
            session_id: "scan-1".to_string(),
            start_time: SystemTime::now(),
            scan_kind: crate::state::ScanKind::Targeted,
            selected_tasks: vec!["os_info".to_string()],
            results,
        };
        let before = scan_fingerprint(Some(&session));
        session.results.get_mut("os_info").unwrap().output = "Windows 12".to_string();
        assert_ne!(before, scan_fingerprint(Some(&session)));
    }
}
