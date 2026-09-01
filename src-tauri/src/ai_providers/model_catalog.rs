//! Tauri command adapter for the shared live model-catalog service.
//!
//! Provider parsing and every discovery path live in
//! `wfdiag-native-ai-provider`; Tauri contributes only its canonical settings
//! service. This keeps draft precedence, DPAPI key lookup, timeouts and model
//! metadata identical to Reactor.

use std::sync::Arc;

use crate::commands::settings::native_settings_service;
use wfdiag_native_ai_chat::ProcessSubscriptionModelCatalogSource;
use wfdiag_native_ai_provider::{
    FoundryCliEndpointSource, ModelCatalogRequest, ModelCatalogService, ProviderKeySource,
    ReqwestOllamaSource, parse_model_catalog_provider,
};
use wfdiag_native_settings::{ProviderKeyId, SettingsService};

pub use wfdiag_native_ai_provider::{ModelCatalog, ModelCatalogEntry, ModelCatalogMetadata};

struct TauriKeySource(SettingsService);

impl ProviderKeySource for TauriKeySource {
    fn load(&self, key: ProviderKeyId) -> Option<String> {
        self.0.load_provider_key(key).ok().flatten()
    }
}

/// List models for a provider. All optional values are unsaved Settings
/// drafts; empty/missing values fall back to stored configuration.
#[tauri::command]
pub async fn ai_list_models(
    provider: String,
    api_key: Option<String>,
    endpoint: Option<String>,
    cli_path: Option<String>,
) -> Result<ModelCatalog, String> {
    let provider = parse_model_catalog_provider(&provider)?;
    let settings = native_settings_service();
    let service = ModelCatalogService::new(
        settings.load_nonsecret_settings().unwrap_or_default(),
        Arc::new(TauriKeySource(settings)),
        Arc::new(FoundryCliEndpointSource::new()),
        Arc::new(ReqwestOllamaSource),
        Arc::new(ProcessSubscriptionModelCatalogSource::new()),
    );
    service
        .list(ModelCatalogRequest {
            provider,
            draft_api_key: api_key,
            draft_endpoint: endpoint,
            draft_cli_path: cli_path,
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shared_structured_catalog_keeps_the_tauri_wire_contract() {
        let catalog = ModelCatalog {
            models: vec![ModelCatalogEntry {
                id: "gpt-next".into(),
                label: Some("GPT Next".into()),
                description: None,
                metadata: None,
            }],
            default_model: Some("gpt-next".into()),
        };
        assert_eq!(
            serde_json::to_value(catalog).unwrap(),
            json!({
                "models": [{"id":"gpt-next","label":"GPT Next"}],
                "defaultModel":"gpt-next"
            })
        );
    }
}
