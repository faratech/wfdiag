//! Tauri adapter over the canonical grounding implementation.
//!
//! The sanitizer, demand gate, MCP client, and trace types live in
//! `wfdiag_native_ai_chat::grounding` — the single copy shared by both shells.
//! This module only supplies the two host-owned policies that stay in the
//! Tauri backend: which `ContextType`s may use live facts, and the
//! settings/env kill switch.

use crate::ai_service::ContextType;

pub use wfdiag_native_ai_chat::{
    AnalysisGrounding, GroundingTrace, chat_grounding_query, needs_live_grounding,
    search_windows_knowledge,
};

/// Live RAG context for one-shot AI analysis.
///
/// This deliberately does not use baked-in Windows release tables. Current
/// facts come through the WindowsForum MCP RAG endpoint on each request.
pub async fn analysis_grounding(
    context_type: ContextType,
    label: Option<&str>,
    data: &str,
    max_chars: usize,
) -> Option<AnalysisGrounding> {
    wfdiag_native_ai_chat::analysis_grounding(
        grounding_supported_for(context_type),
        crate::commands::settings::network_grounding_enabled(),
        label,
        data,
        max_chars,
    )
    .await
}

/// Health-score explanations are scored from local evidence only, so they
/// never justify a network request.
const fn grounding_supported_for(context_type: ContextType) -> bool {
    matches!(
        context_type,
        ContextType::DiagnosticInterpretation
            | ContextType::SectionSummary
            | ContextType::IssuePrioritization
            | ContextType::GeneralAnalysis
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_score_explanations_never_reach_the_network() {
        assert!(!grounding_supported_for(
            ContextType::HealthScoreExplanation
        ));
        for context_type in [
            ContextType::DiagnosticInterpretation,
            ContextType::SectionSummary,
            ContextType::IssuePrioritization,
            ContextType::GeneralAnalysis,
        ] {
            assert!(grounding_supported_for(context_type));
        }
    }

    #[tokio::test]
    async fn unsupported_context_is_rejected_before_the_demand_gate() {
        assert!(
            analysis_grounding(
                ContextType::HealthScoreExplanation,
                Some("Operating System"),
                "Is Windows build 26200 still supported?",
                1_200,
            )
            .await
            .is_none()
        );
    }
}
