//! The AI, remediation, and provider-setup read models.
//!
//! These live beside [`crate::AppSnapshot`] and follow the same rule: they are
//! written only inside [`crate::AppService::drain`], and everything in them is
//! already current. A host renders them; it never reconstructs them from the
//! event stream.

use crate::domain::ai_intent::PendingAiIntent;
use crate::domain::catalog::CatalogState;
use crate::domain::subscriptions::{AccountState, InstallPrompt};
use std::collections::BTreeMap;
use wfdiag_native_ai_analysis::ValidatedFixPlan;
use wfdiag_native_ai_chat::{ChatToolHistory, ProviderUse, SubscriptionInstallProgress};
use wfdiag_native_remediation::broker::ActionProposal;
use wfdiag_native_remediation::runtime::ActionRunSummary;

/// A cloud-fallback question the host must answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloudFallbackPrompt {
    /// The cloud provider that would run next, as a wire id.
    pub candidate: String,
    /// The local provider's failure.
    pub reason: String,
    /// Whether the answer is being persisted right now.
    pub saving: bool,
}

/// A Full Scan the model asked for. No scan has started.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FullScanRequest {
    /// The scan the request was attributed to.
    pub source_scan_id: String,
    /// Why the model wants more evidence.
    pub reason: String,
}

/// A remediation the model staged. Nothing was executed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedProposalRequest {
    /// The catalog id.
    pub remediation_id: String,
    /// The detected issue it was staged against.
    pub issue_id: Option<String>,
}

/// The agentic-chat read model.
#[derive(Clone, Debug, Default)]
pub struct ChatSnapshot {
    /// A turn is streaming.
    pub streaming: bool,
    /// The provider handling the current or last turn, as a wire id.
    pub provider: Option<String>,
    /// The assistant text accumulated for the current turn.
    pub text: String,
    /// The turn's tool activity.
    pub tools: ChatToolHistory,
    /// Remediations the model staged during the turn.
    pub proposals: Vec<StagedProposalRequest>,
    /// A Full Scan the model asked for.
    pub full_scan_request: Option<FullScanRequest>,
    /// A cloud-fallback question awaiting an answer.
    pub cloud_fallback: Option<CloudFallbackPrompt>,
    /// The engine's finish reason for the last turn.
    pub finish_reason: Option<String>,
    /// Trust and model attribution for the last turn.
    pub provider_use: Option<ProviderUse>,
    /// The last turn's failure.
    pub error: Option<String>,
}

/// The AI scan-report read model.
#[derive(Clone, Debug, Default)]
pub struct ReportSnapshot {
    /// Generation is in flight.
    pub generating: bool,
    /// The report body, streamed or served from the cache.
    pub text: Option<String>,
    /// The provider, as a wire id.
    pub provider: Option<String>,
    /// Trust and model attribution.
    pub provider_use: Option<ProviderUse>,
    /// The evidence the report describes.
    pub source_session_id: Option<String>,
    /// Whether the body came from the cache.
    pub cached: bool,
    /// The last failure.
    pub error: Option<String>,
}

/// One task's AI interpretation.
#[derive(Clone, Debug, Default)]
pub struct AnalysisSnapshot {
    /// The interpretation.
    pub interpretation: Option<String>,
    /// Trust and model attribution.
    pub provider_use: Option<ProviderUse>,
    /// Whether it came from the cache.
    pub cached: bool,
    /// Analysis is in flight.
    pub busy: bool,
    /// The last failure.
    pub error: Option<String>,
}

/// The AI issue-prioritisation read model.
#[derive(Clone, Debug, Default)]
pub struct PrioritizationSnapshot {
    /// The model's ranking.
    pub text: Option<String>,
    /// Whether it came from the cache.
    pub cached: bool,
    /// Prioritisation is in flight.
    pub busy: bool,
    /// The last failure.
    pub error: Option<String>,
}

/// The AI fix-plan read model.
#[derive(Clone, Debug, Default)]
pub struct FixPlanSnapshot {
    /// The validated plan.
    pub plan: Option<ValidatedFixPlan>,
    /// Generation is in flight.
    pub busy: bool,
    /// The last failure.
    pub error: Option<String>,
}

/// Every AI-derived projection.
#[derive(Clone, Debug, Default)]
pub struct AiSnapshot {
    /// Agentic chat.
    pub chat: ChatSnapshot,
    /// The scan report.
    pub report: ReportSnapshot,
    /// Per-task interpretations, keyed by task id.
    pub analyses: BTreeMap<String, AnalysisSnapshot>,
    /// Issue prioritisation.
    pub prioritization: PrioritizationSnapshot,
    /// The fix plan.
    pub fix_plan: FixPlanSnapshot,
    /// AI work waiting on a prerequisite.
    pub pending_intent: Option<PendingAiIntent>,
    /// Why the waiting work cannot start.
    pub preparation_error: Option<String>,
}

/// The remediation read model.
#[derive(Clone, Debug, Default)]
pub struct ActionsSnapshot {
    /// A staged preview awaiting first review.
    pub review: Option<ActionProposal>,
    /// A preview awaiting the Repair-specific second confirmation.
    pub repair_confirmation: Option<ActionProposal>,
    /// The run currently executing.
    pub active_run: Option<ActionRunSummary>,
    /// Completed runs, newest first.
    pub history: Vec<ActionRunSummary>,
    /// The last remediation failure.
    pub error: Option<String>,
}

/// The provider-setup read model.
#[derive(Clone, Debug, Default)]
pub struct ProviderSetupSnapshot {
    /// Model catalogs, keyed by provider wire id.
    pub catalogs: BTreeMap<String, CatalogState>,
    /// Subscription CLI accounts, keyed by provider wire id.
    pub accounts: BTreeMap<String, AccountState>,
    /// A confirmation the user has not answered.
    pub install_prompt: Option<InstallPrompt>,
    /// The installer's current stage.
    pub install_progress: Option<SubscriptionInstallProgress>,
    /// The last installation failure.
    pub install_error: Option<String>,
}
