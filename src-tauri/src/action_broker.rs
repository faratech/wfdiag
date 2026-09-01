//! Tauri IPC adapters over the shared action broker.
//!
//! Proposal staging, fingerprints, the one-use grant, the Repair gate, and the
//! run projection all live in `wfdiag_native_remediation::{broker, runtime}`.
//! This module only translates between them and Tauri: the app's session
//! snapshot, the `AppState` mutex, `action://status` emission, and the
//! `#[tauri::command]` surface.
//!
//! The frontend and AI may name remediation IDs, but they never provide a
//! program, argument, path, or authorization boolean.

use crate::issue_catalog::{DetectCtx, Issue};
use crate::state::{AppState, DiagnosticSession};
use std::collections::VecDeque;
use std::sync::Arc;
use tauri::{Emitter, State};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use wfdiag_native_remediation::broker::{
    ActionApproval, ActionBroker, ActionGrant, ActionSnapshot, AuthorizationError,
    AuthorizedActionExecutor, DetectedIssueRemediation, RealCatalogExecutor,
    current_action_catalog_fingerprint, now_ms, opaque_id, stable_fingerprint,
};
use wfdiag_native_remediation::runtime::{
    ActionItemStatus, ActionRunStatus, ActionRunSummary, cancellation_applies,
    completed_run_status, initial_run_summary, item_status_for,
};

/// Wire types the frontend and the chat tool executor already pin. They are
/// the shared contract types; re-exported here so `crate::action_broker::…`
/// keeps resolving.
pub use wfdiag_native_remediation::broker::{ActionPrepareInput, ActionProposal, ActionRequest};

const CATALOG_SCHEMA_VERSION: u32 = 1;
const MAX_ACTION_HISTORY: usize = 50;

/// The webview's review dialog IS this shell's repair confirmation surface:
/// `ConfirmFixModal` renders a repair-specific "Run repair once" button and
/// `action_approve` is reachable only after the user pressed it on a
/// backend-minted, opaque, one-use proposal. The approval value is a compile
/// time constant here — the webview cannot send one.
const WEBVIEW_REVIEW_APPROVAL: ActionApproval = ActionApproval::RepairConfirmed;

struct ActionRunRecord {
    summary: ActionRunSummary,
    cancel: CancellationToken,
}

/// A consumed grant bound to this shell's single reserved run slot.
#[derive(Debug)]
struct AuthorizedRun {
    run_id: String,
    grant: ActionGrant,
    cancel: CancellationToken,
}

/// Tauri-side state: the shared proposal store plus this shell's bounded run
/// history and its single mutation slot.
#[derive(Default)]
pub struct ActionBrokerState {
    broker: ActionBroker,
    runs: VecDeque<ActionRunRecord>,
    active_run_id: Option<String>,
}

impl ActionBrokerState {
    /// Reserve the one mutation slot, consume the preview through the shared
    /// broker, and record the run.
    fn authorize(
        &mut self,
        proposal_id: &str,
        snapshot: &ActionSnapshot,
        approval: ActionApproval,
        approved_at_ms: u64,
    ) -> Result<AuthorizedRun, AuthorizationError> {
        if self.active_run_id.is_some() {
            return Err(AuthorizationError::Rejected(
                "Another system action is already running".to_string(),
            ));
        }
        let grant = self
            .broker
            .authorize(proposal_id, snapshot, approval, approved_at_ms)?;
        let run_id = opaque_id("run");
        let cancel = CancellationToken::new();
        self.active_run_id = Some(run_id.clone());
        self.runs.push_back(ActionRunRecord {
            summary: initial_run_summary(run_id.clone(), &grant, approved_at_ms),
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
    current_action_catalog_fingerprint()
}

fn detected_issues(session: Option<&DiagnosticSession>) -> Vec<Issue> {
    let temp_file_count = count_temp_entries();
    detected_issues_with(session, temp_file_count)
}

/// Blocking %TEMP% enumeration, kept SEPARATE from detection so callers
/// holding the scan-session lock can run it beforehand (see `action_approve`).
fn count_temp_entries() -> Option<usize> {
    std::fs::read_dir(std::env::temp_dir())
        .ok()
        .map(|entries| entries.count())
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

/// The authoritative snapshot the shared broker validates every prepare and
/// approve against.
fn action_snapshot(issues: &[Issue], scan: String, is_admin: bool) -> ActionSnapshot {
    ActionSnapshot {
        scan_fingerprint: scan,
        catalog_fingerprint: catalog_fingerprint(),
        detected_issues: issues
            .iter()
            .filter(|issue| issue.detected)
            .map(|issue| DetectedIssueRemediation {
                issue_id: issue.id.clone(),
                remediation_id: issue
                    .remediation
                    .as_ref()
                    .map(|remediation| remediation.id.clone()),
            })
            .collect(),
        is_admin,
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
    // Staging never depends on elevation; approval re-checks it for real.
    let snapshot = action_snapshot(
        &detected_issues(session.as_ref()),
        scan_fingerprint(session.as_ref()),
        true,
    );
    broker
        .lock()
        .await
        .broker
        .prepare(request, &snapshot, now_ms())
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
    state
        .action_broker
        .lock()
        .await
        .broker
        .get(&proposal_id, now_ms())
        .ok_or_else(|| "Action preview was not found, was used, or expired".to_string())
}

#[tauri::command]
pub async fn action_list_pending_proposals(
    state: State<'_, AppState>,
) -> Result<Vec<ActionProposal>, String> {
    Ok(state.action_broker.lock().await.broker.pending(now_ms()))
}

#[tauri::command]
pub async fn action_discard_proposal(
    state: State<'_, AppState>,
    proposal_id: String,
) -> Result<(), String> {
    state
        .action_broker
        .lock()
        .await
        .broker
        .discard(&proposal_id)
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
    let snapshot = action_snapshot(
        &detected_issues_with(session.as_ref(), temp_file_count),
        scan_fingerprint(session.as_ref()),
        is_admin,
    );
    let authorized = state
        .action_broker
        .lock()
        .await
        .authorize(&proposal_id, &snapshot, WEBVIEW_REVIEW_APPROVAL, now_ms())
        .map_err(|error| error.to_string())?;
    let initial = state
        .action_broker
        .lock()
        .await
        .run(&authorized.run_id)
        .expect("authorized run was not recorded")
        .summary
        .clone();
    let broker = state.action_broker.clone();
    tauri::async_runtime::spawn(run_authorized_actions(app, broker, authorized));
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

async fn run_authorized_actions(
    app: tauri::AppHandle,
    broker: Arc<Mutex<ActionBrokerState>>,
    authorized: AuthorizedRun,
) {
    let run_id = authorized.run_id.clone();
    // The grant moved into this task; it is the only authorization in flight.
    let action_count = authorized.grant.proposal().actions.len();
    for index in 0..action_count {
        if authorized.cancel.is_cancelled() {
            break;
        }
        let action = authorized
            .grant
            .action(index)
            .expect("index came from the grant's own action count");
        {
            let mut state = broker.lock().await;
            let Some(record) = state.run_mut(&run_id) else {
                return;
            };
            record.summary.current_index = Some(index);
            record.summary.actions[index].status = ActionItemStatus::Running;
            emit_status(&app, &record.summary);
        }

        let outcome = RealCatalogExecutor
            .execute(action, &authorized.cancel)
            .await;

        let stop = {
            let mut state = broker.lock().await;
            let Some(record) = state.run_mut(&run_id) else {
                return;
            };
            let stop = match outcome {
                Ok(result) => {
                    // `item_status_for` maps Succeeded <-> Succeeded, so this
                    // is the shipping `completion_status != Succeeded` rule.
                    record.summary.actions[index].status =
                        item_status_for(result.completion_status);
                    let stop = record.summary.actions[index].status != ActionItemStatus::Succeeded;
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
    if let Some(index) = record.summary.current_index
        && !record.summary.actions[index].cancellable
    {
        return Err("The current action cannot be stopped safely once started".to_string());
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
    use std::time::SystemTime;
    use wfdiag_native_remediation::broker::{
        ACTION_PROPOSAL_TTL_MS as PROPOSAL_TTL_MS, ApprovalScope, MAX_BATCH_ACTIONS,
    };

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

    fn snapshot_for(issues: &[Issue], scan: &str, is_admin: bool) -> ActionSnapshot {
        action_snapshot(issues, scan.to_string(), is_admin)
    }

    fn stage(state: &mut ActionBrokerState, remediation_id: &str, now: u64) -> ActionProposal {
        state
            .broker
            .prepare(
                single(remediation_id),
                &snapshot_for(&[], "scan-a", true),
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

    #[test]
    fn proposal_is_immutable_catalog_data_with_ten_minute_expiry() {
        let mut state = ActionBrokerState::default();
        let proposal = stage(&mut state, "flush_dns", 1_000);
        assert_eq!(proposal.approval_scope, ApprovalScope::Exact);
        assert_eq!(proposal.expires_at_ms, 1_000 + PROPOSAL_TTL_MS);
        assert_eq!(proposal.actions[0].remediation.id, "flush_dns");
        assert_eq!(proposal.actions[0].steps, vec!["ipconfig /flushdns"]);
    }

    #[test]
    fn batch_rejects_high_risk_duplicates_and_overflow() {
        let mut state = ActionBrokerState::default();
        let current = snapshot_for(&[], "scan-a", true);
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
        assert!(state.broker.prepare(duplicate, &current, 0).is_err());

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
        assert!(state.broker.prepare(high_risk, &current, 0).is_err());

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
        assert!(state.broker.prepare(overflow, &current, 0).is_err());
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
        assert!(broker.lock().await.broker.pending(now_ms()).is_empty());
    }

    #[test]
    fn authorization_is_one_use_stale_safe_and_reserves_one_slot() {
        let mut state = ActionBrokerState::default();
        let current = snapshot_for(&[], "scan-a", true);
        let first = stage(&mut state, "flush_dns", 1_000);
        let run = state
            .authorize(&first.proposal_id, &current, WEBVIEW_REVIEW_APPROVAL, 1_001)
            .unwrap();
        assert!(run.grant.authorization_id().starts_with("grant_"));
        assert!(
            state
                .authorize(&first.proposal_id, &current, WEBVIEW_REVIEW_APPROVAL, 1_002,)
                .unwrap_err()
                .to_string()
                .contains("already been approved")
        );

        let second = stage(&mut state, "flush_dns", 1_000);
        assert!(
            state
                .authorize(
                    &second.proposal_id,
                    &current,
                    WEBVIEW_REVIEW_APPROVAL,
                    1_003,
                )
                .unwrap_err()
                .to_string()
                .contains("already running")
        );
    }

    #[test]
    fn authorization_rejects_expiry_and_changed_evidence() {
        let current = snapshot_for(&[], "scan-a", true);

        let mut expired_state = ActionBrokerState::default();
        let expired = stage(&mut expired_state, "flush_dns", 1_000);
        assert!(
            expired_state
                .authorize(
                    &expired.proposal_id,
                    &current,
                    WEBVIEW_REVIEW_APPROVAL,
                    1_000 + PROPOSAL_TTL_MS,
                )
                .unwrap_err()
                .to_string()
                .contains("expired")
        );

        let mut scan_state = ActionBrokerState::default();
        let stale = stage(&mut scan_state, "flush_dns", 2_000);
        assert!(
            scan_state
                .authorize(
                    &stale.proposal_id,
                    &snapshot_for(&[], "scan-b", true),
                    WEBVIEW_REVIEW_APPROVAL,
                    2_001,
                )
                .unwrap_err()
                .to_string()
                .contains("scan changed")
        );

        let mut catalog_state = ActionBrokerState::default();
        let stale_catalog = stage(&mut catalog_state, "flush_dns", 3_000);
        let mut changed_catalog = current.clone();
        changed_catalog.catalog_fingerprint = "catalog-b".to_string();
        assert!(
            catalog_state
                .authorize(
                    &stale_catalog.proposal_id,
                    &changed_catalog,
                    WEBVIEW_REVIEW_APPROVAL,
                    3_001,
                )
                .unwrap_err()
                .to_string()
                .contains("catalog")
        );

        let mut admin_state = ActionBrokerState::default();
        let admin = stage(&mut admin_state, "sfc_scannow", 4_000);
        assert!(
            admin_state
                .authorize(
                    &admin.proposal_id,
                    &snapshot_for(&[], "scan-a", false),
                    WEBVIEW_REVIEW_APPROVAL,
                    4_001,
                )
                .unwrap_err()
                .to_string()
                .contains("administrator")
        );
    }

    /// #185: the Repair gate is enforced by the shared broker, so an approval
    /// that only claims "reviewed" cannot start DISM/SFC/network-reset even
    /// through this shell's own state type.
    #[test]
    fn a_reviewed_only_approval_cannot_start_a_repair() {
        let mut state = ActionBrokerState::default();
        let current = snapshot_for(&[], "scan-a", true);
        let proposal = stage(&mut state, "sfc_scannow", 1_000);

        let refused = state.authorize(
            &proposal.proposal_id,
            &current,
            ActionApproval::Reviewed,
            1_001,
        );
        assert!(matches!(
            refused,
            Err(AuthorizationError::RepairConfirmationRequired(review)) if review == proposal
        ));
        // No run slot was reserved and the preview stays reviewable.
        assert!(state.active_run_id.is_none());
        assert!(state.runs.is_empty());
        assert!(state.broker.get(&proposal.proposal_id, 1_002).is_some());
    }

    #[test]
    fn authorization_rechecks_issue_mapping_and_prunes_consumed_preview() {
        let detected = issue("dns_issue", "flush_dns");
        let with_issue = snapshot_for(std::slice::from_ref(&detected), "scan-a", true);
        let input = ActionPrepareInput {
            actions: vec![ActionRequest {
                remediation_id: "flush_dns".into(),
                issue_id: Some(detected.id.clone()),
            }],
            expected_scan_fingerprint: None,
            expected_catalog_fingerprint: None,
        };
        let mut state = ActionBrokerState::default();
        let staged = state.broker.prepare(input, &with_issue, 1_000).unwrap();

        assert!(
            state
                .authorize(
                    &staged.proposal_id,
                    &snapshot_for(&[], "scan-a", true),
                    WEBVIEW_REVIEW_APPROVAL,
                    1_001,
                )
                .unwrap_err()
                .to_string()
                .contains("Issue 'dns_issue' changed")
        );
        state
            .authorize(
                &staged.proposal_id,
                &with_issue,
                WEBVIEW_REVIEW_APPROVAL,
                1_002,
            )
            .unwrap();
        assert!(state.broker.get(&staged.proposal_id, 1_003).is_none());
    }

    #[test]
    fn terminal_status_preserves_partial_and_ignores_too_late_cancel() {
        let mut state = ActionBrokerState::default();
        let current = snapshot_for(&[], "scan-a", true);
        let proposal = stage(&mut state, "flush_dns", 1_000);
        let run = state
            .authorize(
                &proposal.proposal_id,
                &current,
                WEBVIEW_REVIEW_APPROVAL,
                1_001,
            )
            .unwrap();
        let mut actions = state.run(&run.run_id).unwrap().summary.actions.clone();

        actions[0].status = ActionItemStatus::Partial;
        assert_eq!(
            completed_run_status(&actions, false),
            ActionRunStatus::Partial
        );
        actions[0].status = ActionItemStatus::Succeeded;
        assert!(!cancellation_applies(&actions, true));
        actions.push(actions[0].clone());
        actions[1].status = ActionItemStatus::Pending;
        assert!(cancellation_applies(&actions, true));
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
