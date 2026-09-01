//! The trusted action broker: the ONLY way to reach catalog execution.
//!
//! A caller can name catalog IDs, never a program, an argument, a path, or an
//! authorization boolean. Preparation resolves an immutable, catalog-derived
//! preview; approval revalidates the whole preview against a freshly captured
//! authoritative snapshot and atomically consumes a one-use [`ActionGrant`].
//!
//! SECURITY (#185): [`crate::remediation::execute_authorized`] is `pub(crate)`.
//! The only public route into it is [`RealCatalogExecutor`], whose signature
//! demands an [`AuthorizedAction`], and an `AuthorizedAction` can only be
//! borrowed from an `ActionGrant` that [`ActionBroker::authorize`] minted. That
//! function refuses every `RemediationTier::Repair` preview unless the caller
//! presents [`ActionApproval::RepairConfirmed`], so no caller — shell, IPC
//! command, or model-driven tool — can reach a Repair without the explicit
//! second confirmation.

use std::collections::{HashMap, HashSet, hash_map::RandomState};
use std::fmt;
use std::future::Future;
use std::hash::{BuildHasher, Hash, Hasher};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::remediation::{FixResult, RealRunner, RemediationTier, find, remediations};

/// Wire forms shared with both shells. They are defined once in the UI
/// contract crate so the webview payloads and the native shell's projections
/// cannot drift apart.
pub use wfdiag_ui_core::contract::{ActionPreview, ActionProposal, ApprovalScope};

/// A staged preview stays approvable for ten minutes.
pub const ACTION_PROPOSAL_TTL_MS: u64 = 10 * 60 * 1_000;
/// Batch approval covers at most five low-impact catalog actions.
pub const MAX_BATCH_ACTIONS: usize = 5;
/// Bumped whenever the fingerprint material below changes meaning.
pub const ACTION_CATALOG_SCHEMA_VERSION: u32 = 1;

const MAX_PENDING_PROPOSALS: usize = 100;

/// One catalog action requested by a UI flow or a validated AI plan.
///
/// `deny_unknown_fields` is load-bearing: an IPC caller that tries to smuggle
/// a `program`/`args` pair alongside the ID is rejected by the deserializer.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionRequest {
    pub remediation_id: String,
    #[serde(default)]
    pub issue_id: Option<String>,
}

/// Preparation input. Optional expected fingerprints bind an AI fix plan to
/// the evidence/catalog versions from which it was generated.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionPrepareInput {
    pub actions: Vec<ActionRequest>,
    #[serde(default)]
    pub expected_scan_fingerprint: Option<String>,
    #[serde(default)]
    pub expected_catalog_fingerprint: Option<String>,
}

/// A detected issue and its sole catalog-authorized remediation, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedIssueRemediation {
    pub issue_id: String,
    pub remediation_id: Option<String>,
}

/// Authoritative application snapshot captured at a prepare or approve
/// boundary.
///
/// Callers must rebuild this from the currently committed diagnostic session;
/// a proposal never carries mutable references back into shell state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionSnapshot {
    pub scan_fingerprint: String,
    pub catalog_fingerprint: String,
    pub detected_issues: Vec<DetectedIssueRemediation>,
    pub is_admin: bool,
}

/// The approval a caller presents alongside a proposal ID.
///
/// Repair proposals require the caller to obtain a second, repair-specific
/// confirmation after showing the immutable preview. Every other tier only
/// needs `Reviewed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionApproval {
    Reviewed,
    RepairConfirmed,
}

/// Why an approval did not produce a grant.
#[derive(Debug)]
pub enum AuthorizationError {
    /// The preview contains a Repair-tier action and was NOT consumed. Show
    /// the returned proposal again and re-approve with
    /// [`ActionApproval::RepairConfirmed`].
    RepairConfirmationRequired(ActionProposal),
    Rejected(String),
}

impl fmt::Display for AuthorizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RepairConfirmationRequired(_) => {
                formatter.write_str("This repair needs an explicit confirmation")
            }
            Self::Rejected(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for AuthorizationError {}

/// Current wall-clock milliseconds. Both shells and the runtime share this
/// clock so proposal TTLs are compared against one definition of "now".
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Stable FNV-1a hash. These fingerprints detect stale evidence and catalog
/// changes; authorization secrecy comes from the unguessable opaque IDs and
/// the private grant type, not from this hash.
#[must_use]
pub fn stable_fingerprint<T: Hash + ?Sized>(value: &T) -> String {
    struct Fnv64(u64);

    impl Hasher for Fnv64 {
        fn write(&mut self, bytes: &[u8]) {
            for byte in bytes {
                self.0 ^= u64::from(*byte);
                self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
            }
        }

        fn finish(&self) -> u64 {
            self.0
        }
    }

    let mut hasher = Fnv64(0xcbf2_9ce4_8422_2325);
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

/// Unguessable, prefixed identifier for proposals, grants, and runs.
#[must_use]
pub fn opaque_id(prefix: &'static str) -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
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

/// A consumed approval. It cannot be constructed outside this module and it
/// cannot be deserialized, so no IPC caller can manufacture one.
#[derive(Debug)]
pub struct ActionGrant {
    authorization_id: String,
    proposal: ActionProposal,
}

impl ActionGrant {
    /// Audit metadata only. No public entry point accepts it as authority.
    #[must_use]
    pub fn authorization_id(&self) -> &str {
        &self.authorization_id
    }

    #[must_use]
    pub const fn proposal(&self) -> &ActionProposal {
        &self.proposal
    }

    /// The authorized actions, in approval order.
    #[must_use]
    pub fn actions(&self) -> impl ExactSizeIterator<Item = AuthorizedAction<'_>> {
        self.proposal
            .actions
            .iter()
            .map(|preview| AuthorizedAction { preview })
    }

    #[must_use]
    pub fn action(&self, index: usize) -> Option<AuthorizedAction<'_>> {
        self.proposal
            .actions
            .get(index)
            .map(|preview| AuthorizedAction { preview })
    }
}

#[cfg(test)]
impl ActionGrant {
    /// Test-only constructor for run-projection fixtures, which need grants
    /// over combinations the approval path deliberately refuses (a batch that
    /// mixes tiers, say). Never compiled into a shipping binary; the approval
    /// gate itself is exercised by `authorize`'s own tests above.
    pub(crate) fn for_tests(actions: Vec<ActionPreview>) -> Self {
        Self {
            authorization_id: opaque_id("test_grant"),
            proposal: ActionProposal {
                proposal_id: opaque_id("test_proposal"),
                approval_scope: if actions.len() == 1 {
                    ApprovalScope::Exact
                } else {
                    ApprovalScope::Batch
                },
                actions,
                scan_fingerprint: "scan-a".to_string(),
                catalog_fingerprint: current_action_catalog_fingerprint(),
                created_at_ms: 1_000,
                expires_at_ms: 1_000 + ACTION_PROPOSAL_TTL_MS,
            },
        }
    }
}

/// One catalog action inside a consumed grant.
///
/// This is the capability [`AuthorizedActionExecutor`] demands. It has a
/// private field and no public constructor, so an arbitrary catalog ID can
/// never reach the execution engine: the value exists only after
/// [`ActionBroker::authorize`] accepted the approval.
#[derive(Clone, Copy, Debug)]
pub struct AuthorizedAction<'grant> {
    preview: &'grant ActionPreview,
}

impl<'grant> AuthorizedAction<'grant> {
    #[must_use]
    pub const fn preview(self) -> &'grant ActionPreview {
        self.preview
    }

    #[must_use]
    pub fn remediation_id(self) -> &'grant str {
        &self.preview.remediation.id
    }
}

/// The staged-proposal store and the approval gate.
#[derive(Default)]
pub struct ActionBroker {
    proposals: HashMap<String, StoredProposal>,
}

impl ActionBroker {
    /// Resolve an immutable preview from the compile-time catalog and store it
    /// for later approval by its opaque ID.
    pub fn prepare(
        &mut self,
        input: ActionPrepareInput,
        snapshot: &ActionSnapshot,
        current_time_ms: u64,
    ) -> Result<ActionProposal, String> {
        self.retain_live(current_time_ms);
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

    /// Revalidate a stored preview against a fresh authoritative snapshot and,
    /// on success, consume it into a one-use [`ActionGrant`].
    ///
    /// THE REPAIR GATE. A preview containing any `RemediationTier::Repair`
    /// action is refused with
    /// [`AuthorizationError::RepairConfirmationRequired`] — leaving the
    /// preview unconsumed and still reviewable — unless `approval` is
    /// [`ActionApproval::RepairConfirmed`].
    pub fn authorize(
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

        // The grant is created and the proposal consumed in one operation; no
        // executable catalog ID is returned to the caller.
        stored.consumed = true;
        Ok(ActionGrant {
            authorization_id: opaque_id("grant"),
            proposal: stored.proposal.clone(),
        })
    }

    /// Drop an unused preview after its review UI is dismissed.
    ///
    /// # Errors
    /// When the preview was already approved: a consumed preview is an audit
    /// record, not a dismissable capability.
    pub fn discard(&mut self, proposal_id: &str) -> Result<(), String> {
        let Some(stored) = self.proposals.get(proposal_id) else {
            return Ok(());
        };
        if stored.consumed {
            return Err("An approved action preview cannot be dismissed".to_string());
        }
        self.proposals.remove(proposal_id);
        Ok(())
    }

    /// One unconsumed, unexpired preview by ID.
    pub fn get(&mut self, proposal_id: &str, current_time_ms: u64) -> Option<ActionProposal> {
        self.retain_live(current_time_ms);
        self.proposals
            .get(proposal_id)
            .map(|stored| stored.proposal.clone())
    }

    /// Every unconsumed, unexpired preview, oldest first.
    pub fn pending(&mut self, current_time_ms: u64) -> Vec<ActionProposal> {
        self.retain_live(current_time_ms);
        let mut proposals = self
            .proposals
            .values()
            .map(|stored| stored.proposal.clone())
            .collect::<Vec<_>>();
        proposals.sort_by_key(|proposal| proposal.created_at_ms);
        proposals
    }

    /// Completed runs are the audit record. Consumed and expired previews are
    /// no longer capabilities and must not exhaust the pending bound.
    fn retain_live(&mut self, current_time_ms: u64) {
        self.proposals.retain(|_, stored| {
            !stored.consumed && stored.proposal.expires_at_ms > current_time_ms
        });
    }
}

/// The future returned by an [`AuthorizedActionExecutor`].
pub type ExecutionFuture<'a> = Pin<Box<dyn Future<Output = Result<FixResult, String>> + Send + 'a>>;

/// Runs one authorized catalog action. Injectable so run-projection tests
/// never touch the system.
pub trait AuthorizedActionExecutor: Send + Sync {
    fn execute<'a>(
        &'a self,
        action: AuthorizedAction<'a>,
        cancel: &'a CancellationToken,
    ) -> ExecutionFuture<'a>;
}

/// The real engine. This is the crate's ONLY public route into
/// [`crate::remediation::execute_authorized`], and it is reachable only with
/// an [`AuthorizedAction`] borrowed from a consumed [`ActionGrant`].
pub struct RealCatalogExecutor;

impl AuthorizedActionExecutor for RealCatalogExecutor {
    fn execute<'a>(
        &'a self,
        action: AuthorizedAction<'a>,
        cancel: &'a CancellationToken,
    ) -> ExecutionFuture<'a> {
        Box::pin(async move {
            crate::remediation::execute_authorized(action.remediation_id(), &RealRunner, cancel)
                .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remediation::testing::RecordingRunner;
    use crate::remediation::{CommandRunner, FixCompletionStatus};

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

    fn rejected(result: Result<ActionGrant, AuthorizationError>) -> String {
        match result {
            Err(AuthorizationError::Rejected(message)) => message,
            Err(AuthorizationError::RepairConfirmationRequired(_)) => {
                panic!("expected a hard rejection, got a Repair confirmation request")
            }
            Ok(_) => panic!("expected authorization to be rejected"),
        }
    }

    /// Runs a grant through the REAL catalog engine but against a recording
    /// command runner: whatever this records is exactly what the machine would
    /// have had run at it.
    fn run_grant(grant: &ActionGrant, runner: &RecordingRunner) -> Vec<FixCompletionStatus> {
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        tokio_runtime.block_on(async {
            let cancel = CancellationToken::new();
            let mut statuses = Vec::new();
            for action in grant.actions() {
                let result = crate::remediation::execute_authorized(
                    action.remediation_id(),
                    runner as &dyn CommandRunner,
                    &cancel,
                )
                .await
                .unwrap();
                statuses.push(result.completion_status);
            }
            statuses
        })
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

        let grant = broker
            .authorize(
                &proposal.proposal_id,
                &current,
                ActionApproval::Reviewed,
                1_001,
            )
            .unwrap();
        assert!(grant.authorization_id().starts_with("grant_"));
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

    /// #185 regression, rewritten from `remediation::repair_requires_confirmation`
    /// against the broker, which is now the gate.
    #[test]
    fn repair_requires_confirmation_and_constructs_no_command() {
        let mut broker = ActionBroker::default();
        let current = snapshot("scan-a", true);
        let runner = RecordingRunner::default();

        for remediation_id in ["sfc_scannow", "dism_restorehealth", "network_reset"] {
            let proposal = broker
                .prepare(request(remediation_id, None), &current, 1_000)
                .unwrap();
            match broker.authorize(
                &proposal.proposal_id,
                &current,
                ActionApproval::Reviewed,
                1_001,
            ) {
                Err(AuthorizationError::RepairConfirmationRequired(review)) => {
                    assert_eq!(review, proposal);
                    assert_eq!(review.actions[0].remediation.tier, RemediationTier::Repair);
                }
                Err(AuthorizationError::Rejected(message)) => {
                    panic!("unexpected rejection for {remediation_id}: {message}")
                }
                Ok(_) => panic!("{remediation_id} was authorized without explicit confirmation"),
            }
            // The strongest guarantee: no grant exists, so no command can be
            // constructed for the machine at all.
            assert!(runner.calls().is_empty());
            // The refused preview is NOT consumed and stays reviewable.
            assert!(broker.get(&proposal.proposal_id, 1_002).is_some());
        }
    }

    /// #185 regression, rewritten from
    /// `remediation::destructive_cleanups_require_confirmation`.
    #[test]
    fn destructive_cleanups_require_confirmation() {
        let mut broker = ActionBroker::default();
        let current = snapshot("scan-a", true);
        for remediation_id in ["empty_recycle_bin", "clear_temp_files"] {
            let proposal = broker
                .prepare(request(remediation_id, None), &current, 1_000)
                .unwrap();
            assert!(matches!(
                broker.authorize(
                    &proposal.proposal_id,
                    &current,
                    ActionApproval::Reviewed,
                    1_001,
                ),
                Err(AuthorizationError::RepairConfirmationRequired(_))
            ));
        }
    }

    #[test]
    fn confirmed_repair_is_granted_once_and_then_replayed_in_vain() {
        let mut broker = ActionBroker::default();
        let current = snapshot("scan-a", true);
        let proposal = broker
            .prepare(request("empty_recycle_bin", None), &current, 1_000)
            .unwrap();

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

    /// #185 regression, rewritten from
    /// `remediation::auto_safe_ignores_the_confirmed_flag`: the non-Repair
    /// tiers need review only, and a granted `AutoSafe` action really does
    /// reach the engine with its catalog-constant argv.
    #[test]
    fn auto_safe_and_open_tool_need_only_review() {
        let mut broker = ActionBroker::default();
        let current = snapshot("scan-a", true);
        let runner = RecordingRunner::default();

        let auto_safe = broker
            .prepare(request("flush_dns", None), &current, 1_000)
            .unwrap();
        let grant = broker
            .authorize(
                &auto_safe.proposal_id,
                &current,
                ActionApproval::Reviewed,
                1_001,
            )
            .unwrap();
        assert_eq!(
            run_grant(&grant, &runner),
            vec![FixCompletionStatus::Succeeded]
        );
        assert_eq!(runner.calls(), vec!["run:ipconfig /flushdns".to_string()]);

        let detected = issue_snapshot("scan-a", "cpu-pressure", "open_task_manager");
        let open_tool = broker
            .prepare(
                request("open_task_manager", Some("cpu-pressure")),
                &detected,
                1_002,
            )
            .unwrap();
        let grant = broker
            .authorize(
                &open_tool.proposal_id,
                &detected,
                ActionApproval::Reviewed,
                1_003,
            )
            .unwrap();
        assert_eq!(
            run_grant(&grant, &runner),
            vec![FixCompletionStatus::Succeeded]
        );
        assert_eq!(
            runner.calls().last().unwrap(),
            &"spawn:taskmgr.exe ".to_string()
        );
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

    #[test]
    fn batch_rejects_duplicates_overflow_and_high_risk_members() {
        let current = snapshot("scan-a", true);
        let mut broker = ActionBroker::default();

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
        assert!(broker.prepare(duplicate, &current, 0).is_err());

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
        assert!(broker.prepare(high_risk, &current, 0).is_err());

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
        assert!(broker.prepare(overflow, &current, 0).is_err());
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

    #[test]
    fn discard_drops_a_preview_but_never_an_approved_one() {
        let mut broker = ActionBroker::default();
        let current = snapshot("scan-a", true);

        let unused = broker
            .prepare(request("flush_dns", None), &current, 1_000)
            .unwrap();
        broker.discard(&unused.proposal_id).unwrap();
        assert!(broker.get(&unused.proposal_id, 1_001).is_none());

        let approved = broker
            .prepare(request("flush_dns", None), &current, 1_002)
            .unwrap();
        broker
            .authorize(
                &approved.proposal_id,
                &current,
                ActionApproval::Reviewed,
                1_003,
            )
            .unwrap();
        assert!(broker.discard(&approved.proposal_id).is_err());
    }

    #[test]
    fn catalog_fingerprint_is_stable_and_covers_preview_steps() {
        assert_eq!(
            current_action_catalog_fingerprint(),
            current_action_catalog_fingerprint()
        );
        assert_ne!(
            current_action_catalog_fingerprint(),
            stable_fingerprint(&(ACTION_CATALOG_SCHEMA_VERSION, Vec::<String>::new()))
        );
    }

    #[test]
    fn opaque_ids_are_prefixed_and_distinct() {
        let first = opaque_id("proposal");
        let second = opaque_id("proposal");
        assert!(first.starts_with("proposal_"));
        assert_eq!(first.len(), "proposal_".len() + 32);
        assert_ne!(first, second);
    }

    #[test]
    fn a_stale_snapshot_fingerprint_is_refused_before_anything_is_staged() {
        let mut broker = ActionBroker::default();
        let mut stale = snapshot("scan-a", true);
        stale.catalog_fingerprint = "not-the-current-catalog".to_string();
        let error = broker
            .prepare(request("flush_dns", None), &stale, 1_000)
            .unwrap_err();
        assert!(error.contains("stale"));
    }
}
