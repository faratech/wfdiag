//! AI-proposed fix plans.
//!
//! The model sees the DETECTED issues and the vetted remediation catalog
//! (ids + labels + descriptions + tiers) and proposes which remediations to
//! run, in order. SAFETY CHAIN: the model emits only catalog IDs →
//! [`parse_fix_plan`] drops anything not in the catalog or not among the
//! detected issues → the user reviews a broker proposal → one-use
//! authorization executes the immutable catalog entry. The model never executes anything and its
//! strings never reach an argv.

use crate::ai_service::AIProvider;
use crate::issue_catalog::Issue;
use crate::state::AppState;
use serde::Serialize;
use tauri::State;
use wfdiag_native_ai_analysis::{PLAN_SYSTEM, one_shot_data_budget};
use wfdiag_native_issues::{build_fix_plan_prompt, parse_fix_plan, remediation_catalog};

pub use wfdiag_native_issues::FixPlanEntry;

#[derive(Debug, Clone, Serialize)]
pub struct FixPlan {
    pub entries: Vec<FixPlanEntry>,
    pub notes: String,
    pub provider_used: AIProvider,
    /// Evidence/catalog versions the action broker must still observe before
    /// it will prepare any entry from this plan.
    pub scan_fingerprint: String,
    pub catalog_fingerprint: String,
}

/// Propose a fix plan for the current scan's detected issues.
#[tauri::command]
pub async fn ai_propose_fix_plan(state: State<'_, AppState>) -> Result<FixPlan, String> {
    let pref = crate::ai_service::get_user_preference();
    let initial_provider = crate::ai_service::determine_active_provider(pref).await;
    // Planning across every detected issue and catalog entry is a wide,
    // structured-output workload. In Auto, use the next available private
    // local model instead of squeezing it through Phi's small context. An
    // explicit Phi preference is still honored, and this surface never
    // crosses into cloud fallback without a consent flow.
    let provider = if pref == crate::ai_service::AIProviderPreference::Auto
        && initial_provider == AIProvider::PhiSilica
    {
        crate::ai_service::next_auto_local_provider(pref, &[initial_provider])
            .await
            .unwrap_or(initial_provider)
    } else {
        initial_provider
    };
    if provider == AIProvider::None {
        return Err(
            "No AI provider available. Add an API key in Settings, sign in with a ChatGPT or \
             Claude subscription, or install Foundry Local or Ollama for local AI."
                .to_string(),
        );
    }
    let cfg = crate::ai_providers::resolve_config(provider).await?;

    // Detect from the current session
    let (issues, scan_fingerprint) = {
        let session = state.current_session.lock().await;
        let Some(session) = session.as_ref() else {
            return Err(
                "No scan data is available for a fix plan. The application should collect a Quick Scan and retry automatically."
                    .to_string(),
            );
        };
        // Match the UI's detect_issues so the same issues (incl. temp_files)
        // are in scope — otherwise a planned clear_temp_files entry would be
        // dropped as "not a detected issue".
        let temp_file_count = std::fs::read_dir(std::env::temp_dir())
            .ok()
            .map(|entries| entries.count());
        let ctx = crate::issue_catalog::DetectCtx {
            results: &session.results,
            now: crate::timestamp::Timestamp::now(),
            temp_file_count,
        };
        (
            crate::issue_catalog::detect_all(&ctx),
            crate::action_broker::scan_fingerprint(Some(session)),
        )
    };
    let detected: Vec<Issue> = issues.into_iter().filter(|i| i.detected).collect();
    if detected.is_empty() {
        return Ok(FixPlan {
            entries: Vec::new(),
            notes: "No issues detected — nothing to plan.".to_string(),
            provider_used: provider,
            scan_fingerprint,
            catalog_fingerprint: crate::action_broker::catalog_fingerprint(),
        });
    }

    // Prompt budget and system prompt are the shared one-shot policy; the
    // native shell plans with the exact same text.
    let budget = one_shot_data_budget(provider);
    let prompt = build_fix_plan_prompt(&detected, remediation_catalog(), budget);
    let text = crate::ai_providers::one_shot(provider, &cfg, PLAN_SYSTEM, &prompt).await?;

    let parsed = parse_fix_plan(&text, &detected, remediation_catalog());
    Ok(FixPlan {
        entries: parsed.entries,
        notes: parsed.notes,
        provider_used: provider,
        scan_fingerprint,
        catalog_fingerprint: crate::action_broker::catalog_fingerprint(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue_catalog::{IssueSeverity, IssueStatus};
    use crate::remediation::RemediationTier;

    fn detected_issue(id: &str) -> Issue {
        Issue {
            id: id.to_string(),
            category: "Test".into(),
            severity: IssueSeverity::Warning,
            status: IssueStatus::Detected,
            title: id.to_string(),
            description: format!("{} description", id),
            recommendation: "fix it".into(),
            detected: true,
            source_tasks: None,
            remediation: None,
        }
    }

    fn detected_issue_with_remediation(id: &str, remediation_id: &str) -> Issue {
        Issue {
            remediation: crate::remediation::summary(remediation_id),
            ..detected_issue(id)
        }
    }

    #[test]
    fn prompt_contains_only_detected_issues_and_all_ids() {
        let mut ok_issue = detected_issue("low_disk_space");
        ok_issue.detected = false;
        let issues = vec![detected_issue("dism_corruption"), ok_issue];
        let prompt = build_fix_plan_prompt(&issues, remediation_catalog(), 10_000);
        assert!(prompt.contains("dism_corruption"));
        assert!(!prompt.contains("low_disk_space"));
        assert!(prompt.contains("dism_restorehealth"));
        assert!(prompt.contains("Respond with ONLY this JSON"));
    }

    #[test]
    fn parse_happy_path_and_fences() {
        let detected = vec![detected_issue_with_remediation(
            "dism_corruption",
            "dism_restorehealth",
        )];
        let text = r#"```json
{"entries": [{"issue_id": "dism_corruption", "remediation_id": "dism_restorehealth", "rationale": "Repairs the store."}], "notes": "One repair."}
```"#;
        let parsed = parse_fix_plan(text, &detected, remediation_catalog());
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].remediation_id, "dism_restorehealth");
        assert_eq!(parsed.entries[0].tier, RemediationTier::Repair);
        assert_eq!(parsed.notes, "One repair.");
    }

    #[test]
    fn parse_drops_unknown_and_undetected_and_dedups() {
        let detected = vec![
            detected_issue_with_remediation("temp_files", "clear_temp_files"),
            detected_issue("dns_misconfigured"),
        ];
        let text = r#"{"entries": [
            {"issue_id": "temp_files", "remediation_id": "clear_temp_files", "rationale": "a"},
            {"issue_id": "temp_files", "remediation_id": "clear_temp_files", "rationale": "dup"},
            {"issue_id": "temp_files", "remediation_id": "format_c_drive", "rationale": "invented"},
            {"issue_id": "temp_files", "remediation_id": "flush_dns", "rationale": "unrelated"},
            {"issue_id": "dns_misconfigured", "remediation_id": "flush_dns", "rationale": "no mapped remediation"},
            {"issue_id": "not_detected_issue", "remediation_id": "clear_temp_files", "rationale": "wrong issue"}
        ], "notes": "n"}"#;
        let parsed = parse_fix_plan(text, &detected, remediation_catalog());
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].remediation_id, "clear_temp_files");
    }

    #[test]
    fn parse_tolerates_garbage() {
        let detected = vec![detected_issue("temp_files")];
        let parsed = parse_fix_plan("I cannot help with that.", &detected, remediation_catalog());
        assert!(parsed.entries.is_empty());
        assert!(!parsed.notes.is_empty());
        let parsed = parse_fix_plan("{broken json", &detected, remediation_catalog());
        assert!(parsed.entries.is_empty());
    }

    #[test]
    fn parse_caps_entries_and_rationale() {
        let detected: Vec<Issue> = (0..12)
            .map(|i| detected_issue_with_remediation(&format!("issue{}", i), "flush_dns"))
            .collect();
        // Build 12 entries all pointing at a real remediation, with fake issue
        // ids that ARE detected (issue0..)
        let entries_json: Vec<String> = (0..12)
            .map(|i| {
                format!(
                    r#"{{"issue_id": "issue{}", "remediation_id": "flush_dns", "rationale": "{}"}}"#,
                    i,
                    "r".repeat(500)
                )
            })
            .collect();
        let text = format!(
            r#"{{"entries": [{}], "notes": "{}"}}"#,
            entries_json.join(","),
            "n".repeat(2_000),
        );
        let parsed = parse_fix_plan(&text, &detected, remediation_catalog());
        assert_eq!(
            parsed.entries.len(),
            wfdiag_native_issues::MAX_FIX_PLAN_ENTRIES
        );
        assert!(
            parsed.entries[0].rationale.chars().count()
                <= wfdiag_native_issues::MAX_FIX_PLAN_RATIONALE_CHARS
        );
        assert_eq!(
            parsed.notes.chars().count(),
            wfdiag_native_issues::MAX_FIX_PLAN_NOTES_CHARS
        );
    }
}
