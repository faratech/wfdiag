//! AI scan report: one click turns the current scan into a structured health
//! report (health summary, top issues, changes since the last scan,
//! fix-first actions).
//!
//! Unlike the chat there is NO tool loop — the context is assembled
//! deterministically from data the app already has (scan results, the
//! rule-based issue detector, and `compute_comparison` against a stored
//! scan), then streamed through the shared turn machinery via
//! `ai-report://delta|done|error` events. Reports are cached by scan-content
//! hash in the shared AI cache.
//!
//! Provider policy, evidence assembly, cache identity, duplicate
//! suppression, and the generation lifecycle live in
//! `wfdiag_native_ai_report`. This module keeps only the Tauri adapters:
//! the app-state scan/comparison plumbing, the provider resolver, and the
//! event emitter.

use crate::ai_chat::RealChatProvider;
use crate::ai_service::{
    AIProvider, AIProviderPreference, determine_active_provider, get_user_preference,
    next_auto_local_provider, provider_config_fingerprint,
};
use crate::results_storage::{ComparisonResult, ScanRecord, ScanStorage};
use crate::state::AppState;
use std::sync::{Arc, OnceLock};
use tauri::{Emitter, State};
use wfdiag_native_ai_report::{
    ReportAck, ReportDeltaPayload, ReportDonePayload, ReportEmitter, ReportErrorPayload,
    ReportProviderResolver, ReportRequest, ReportScan, ReportService, ResolvedReportProvider,
};

fn report_service() -> &'static ReportService {
    static SERVICE: OnceLock<ReportService> = OnceLock::new();
    SERVICE.get_or_init(|| ReportService::new(crate::ai_service::get_cache().clone()))
}

/// Maps the shared report service's events onto `ai-report://*`. The payload
/// types serialize to exactly the historical wire shape (pinned by tests).
struct TauriReportEmitter(tauri::AppHandle);

impl ReportEmitter for TauriReportEmitter {
    fn delta(&self, payload: &ReportDeltaPayload) {
        let _ = self.0.emit("ai-report://delta", payload);
    }

    fn done(&self, payload: &ReportDonePayload) {
        let _ = self.0.emit("ai-report://done", payload);
    }

    fn error(&self, payload: &ReportErrorPayload) {
        let _ = self.0.emit("ai-report://error", payload);
    }
}

/// Resolves concrete provider calls from secure settings. The report core
/// owns the routing policy; this adapter only answers its questions.
struct TauriReportResolver;

impl ReportProviderResolver for TauriReportResolver {
    fn preference(&self) -> AIProviderPreference {
        get_user_preference()
    }

    fn determine_active(
        &self,
        preference: AIProviderPreference,
    ) -> wfdiag_native_ai_report::ReportFuture<'_, AIProvider> {
        Box::pin(determine_active_provider(preference))
    }

    fn next_auto_local(
        &self,
        preference: AIProviderPreference,
        tried: &[AIProvider],
    ) -> wfdiag_native_ai_report::ReportFuture<'_, Option<AIProvider>> {
        // The routed future borrows `tried`; the trait ties the return to
        // `&self`, so own the slice across the await instead.
        let tried = tried.to_vec();
        Box::pin(async move { next_auto_local_provider(preference, &tried).await })
    }

    fn resolve<'a>(
        &'a self,
        provider: AIProvider,
    ) -> wfdiag_native_ai_report::ReportFuture<'a, Result<ResolvedReportProvider, String>> {
        Box::pin(async move {
            let cfg = crate::ai_providers::resolve_config(provider).await?;
            let config_fingerprint = provider_config_fingerprint(provider, &cfg);
            let requested_model = cfg.model.clone();
            Ok(ResolvedReportProvider {
                // No per-report cancellation token is threaded to this
                // resolver yet; `|| false` never cancels early, same
                // behavior as before this field existed.
                chat: Arc::new(RealChatProvider {
                    provider,
                    cfg,
                    is_cancelled: Arc::new(|| false),
                }),
                config_fingerprint,
                requested_model,
            })
        })
    }
}

/// The history crate compiles the same `results_storage` schema this crate
/// compiles, and the report contract accepts the portable copy, so convert
/// once at the boundary. The schemas are identical by construction, so the
/// serde round-trip cannot drop data.
fn portable_comparison(
    comparison: &ComparisonResult,
) -> Result<wfdiag_native_history::ComparisonResult, String> {
    serde_json::to_value(comparison)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or_else(|| {
            "Could not convert the comparison snapshot for report generation.".to_string()
        })
}

/// Generate an AI health report for the current scan. Returns immediately;
/// the report streams via `ai-report://delta|done|error` — unless it was
/// cached, in which case `report` is set and no events follow.
#[tauri::command]
pub async fn ai_generate_report(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    previous_scan_id: Option<String>,
    force_refresh: Option<bool>,
) -> Result<ReportAck, String> {
    // Snapshot the current scan
    let (results, session_id) = {
        let session = state.current_session.lock().await;
        match session.as_ref() {
            Some(session) if !session.results.is_empty() => {
                (session.results.clone(), session.session_id.clone())
            }
            _ => {
                return Err(
                    "No scan data is available for this report. The application should collect a Quick Scan and retry automatically."
                        .to_string(),
                );
            }
        }
    };

    // Comparison baseline: explicit id, else the newest stored scan that is
    // not this session (auto-save may have stored the current scan already).
    let explicit_previous_id =
        wfdiag_native_ai_report::explicit_previous_scan_id(previous_scan_id.as_deref());
    let comparison_info: Result<Option<(ComparisonResult, String)>, String> = {
        let storage = state.scan_storage.lock().await;
        match storage.as_ref() {
            Some(storage) => {
                let previous_id = match explicit_previous_id {
                    Some(id) => Some(id.to_string()),
                    None => storage.list_scans().ok().and_then(|scans| {
                        scans.into_iter().map(|s| s.id).find(|id| *id != session_id)
                    }),
                };
                match previous_id {
                    Some(previous_id) => {
                        let load_result = storage.load_scan(&previous_id);
                        match wfdiag_native_ai_report::resolve_loaded_report_baseline(
                            explicit_previous_id,
                            previous_id,
                            load_result,
                        )? {
                            Some((previous, previous_id)) => {
                                let success_count = results.values().filter(|r| r.success).count();
                                let current = ScanRecord {
                                    id: session_id.clone(),
                                    timestamp: crate::timestamp::Timestamp::now(),
                                    computer_name: String::new(),
                                    os_version: String::new(),
                                    is_admin: false,
                                    task_count: results.len(),
                                    success_count,
                                    failure_count: results.len() - success_count,
                                    duration_ms: 0,
                                    label: None,
                                    tags: Vec::new(),
                                    results: results.clone(),
                                };
                                Ok(Some((
                                    ScanStorage::compute_comparison(current, previous),
                                    previous_id,
                                )))
                            }
                            None => Ok(None),
                        }
                    }
                    None => Ok(None),
                }
            }
            None if explicit_previous_id.is_some() => Err(
                "Selected comparison scan could not be loaded because scan history is unavailable."
                    .to_string(),
            ),
            None => Ok(None),
        }
    };
    let (comparison, _resolved_previous_scan_id) = match comparison_info? {
        Some((comparison, previous_id)) => (Some(comparison), Some(previous_id)),
        None => (None, None),
    };
    let portable_comparison = match &comparison {
        Some(comparison) => Some(portable_comparison(comparison)?),
        None => None,
    };

    let request = ReportRequest {
        scan: ReportScan {
            session_id,
            results: Arc::new(
                results
                    .into_iter()
                    .map(|(task_id, result)| (task_id, Arc::new(result)))
                    .collect(),
            ),
        },
        comparison: portable_comparison,
        force_refresh: force_refresh.unwrap_or(false),
        // The portable detector clock keeps issue detection deterministic;
        // no detector reads the wall clock itself.
        detection_now: wfdiag_native_issues::Timestamp::from_secs(
            crate::timestamp::Timestamp::now().secs,
        ),
    };

    let emitter: Arc<dyn ReportEmitter> = Arc::new(TauriReportEmitter(app));
    report_service()
        .generate(request, Arc::new(TauriReportResolver), emitter)
        .await
}

/// Cancel an in-flight report. Partial or cancelled reports are never cached.
#[tauri::command]
pub async fn ai_report_cancel(report_id: String) -> Result<(), String> {
    report_service().cancel(&report_id).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn emitter_payloads_keep_the_historical_wire_shape() {
        let delta = serde_json::to_value(&ReportDeltaPayload {
            report_id: "report_1".into(),
            text: "hello".into(),
        })
        .unwrap();
        assert_eq!(
            delta,
            serde_json::json!({"reportId": "report_1", "text": "hello"})
        );

        let done = serde_json::to_value(&ReportDonePayload {
            report_id: "report_1".into(),
            finish_reason: "stop".into(),
            provider: "openai".into(),
            provider_use: wfdiag_native_ai_chat::ProviderUse::for_provider(
                wfdiag_native_ai_provider::AIProvider::OpenAI,
                None,
            ),
        })
        .unwrap();
        assert_eq!(done["reportId"], "report_1");
        assert_eq!(done["finishReason"], "stop");
        assert_eq!(done["provider"], "openai");
        assert_eq!(done["providerUse"]["providerId"], "openai");

        let error = serde_json::to_value(&ReportErrorPayload {
            report_id: "report_1".into(),
            message: "boom".into(),
        })
        .unwrap();
        assert_eq!(
            error,
            serde_json::json!({"reportId": "report_1", "message": "boom"})
        );
    }

    #[test]
    fn comparison_snapshot_converts_without_data_loss() {
        let comparison = ComparisonResult {
            current_scan: summary("scan_a", "ANDROMEDA"),
            previous_scan: summary("scan_b", "ANDROMEDA"),
            total_changes: 3,
            new_failures: vec![change("chkdsk", true, false)],
            new_successes: vec![change("sfc", false, true)],
            status_unchanged: vec![change("os_info", true, true)],
        };
        let portable = portable_comparison(&comparison).unwrap();
        let round_tripped: ComparisonResult =
            serde_json::from_slice(&serde_json::to_vec(&portable).unwrap()).unwrap();
        let original = serde_json::to_value(&comparison).unwrap();
        assert_eq!(original, serde_json::to_value(&round_tripped).unwrap());
    }

    fn summary(id: &str, computer_name: &str) -> crate::results_storage::ScanSummary {
        crate::results_storage::ScanSummary {
            id: id.to_string(),
            timestamp: crate::timestamp::Timestamp::now(),
            computer_name: computer_name.to_string(),
            task_count: 17,
            success_count: 16,
            failure_count: 1,
            duration_ms: 2_300,
            label: None,
            tags: Vec::new(),
        }
    }

    fn change(task_id: &str, current: bool, previous: bool) -> crate::results_storage::TaskChange {
        crate::results_storage::TaskChange {
            task_id: task_id.to_string(),
            task_name: format!("{task_id} name"),
            category: "Storage".to_string(),
            current_success: current,
            previous_success: previous,
            current_output: "current".to_string(),
            previous_output: "previous".to_string(),
            output_changed: true,
        }
    }

    /// The comparison conversion is intentionally total: any future schema
    /// drift must fail loudly here instead of silently dropping a baseline.
    #[test]
    fn comparison_conversion_rejects_unknown_fields() {
        let mut value: Value = serde_json::json!({
            "currentScan": {"id": "a"},
            "previousScan": {"id": "b"},
            "totalChanges": 0
        });
        value["unexpected"] = Value::Null;
        assert!(serde_json::from_value::<wfdiag_native_history::ComparisonResult>(value).is_err());
    }
}
