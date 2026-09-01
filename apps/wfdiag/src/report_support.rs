//! Reactor's platform wiring for the shared native AI report runtime.
//!
//! [`wfdiag_native_ai_report::NativeReportRuntime`] owns the worker thread,
//! cancellation, duplicate suppression, and the event projection. What stays
//! here is the concrete provider resolution for one generation: live settings,
//! DPAPI-backed keys, local endpoint discovery, and on-device Phi Silica.

#![deny(unsafe_code)]

use std::sync::Arc;
use std::sync::mpsc as std_mpsc;

use wfdiag_native_ai_chat::{ChatProvider, CompatChatProvider};
use wfdiag_native_ai_provider::{
    AIProvider, AIProviderPreference, CompatConfigPorts, FoundryEndpointSource, OllamaSource,
    ProviderAvailability, SharedAiCache, SubscriptionConfigPorts, next_auto_local_route,
    parse_provider_preference, provider_config_fingerprint, resolve_compat_config,
    resolve_subscription_config,
};
use wfdiag_native_ai_report::{
    NativeReportRuntime, ReportFuture, ReportProviderResolver, ReportResolverFactory,
    ReportWorkerEvent, ResolvedReportProvider,
};
use wfdiag_native_phi::PhiChatProvider;
use wfdiag_native_settings::SettingsService;

use crate::chat_support::ShellChatSource;
use crate::ui_wake_support;

/// One report's routing inputs. The active provider and complete availability
/// snapshot come from the same provider-status response, so the report core's
/// Phi-to-local policy cannot race a second, independently ordered probe.
struct TurnReportResolver {
    ports: Arc<CompatConfigPorts>,
    subscription_ports: Arc<SubscriptionConfigPorts>,
    provider: AIProvider,
    availability: ProviderAvailability,
}

impl ReportProviderResolver for TurnReportResolver {
    fn preference(&self) -> AIProviderPreference {
        parse_provider_preference(&self.ports.settings.preferred_ai_provider)
    }

    fn determine_active(&self, _preference: AIProviderPreference) -> ReportFuture<'_, AIProvider> {
        Box::pin(async move { self.provider })
    }

    fn next_auto_local(
        &self,
        preference: AIProviderPreference,
        tried: &[AIProvider],
    ) -> ReportFuture<'_, Option<AIProvider>> {
        let next = next_auto_local_route(preference, tried, self.availability);
        Box::pin(async move { next })
    }

    fn resolve(
        &self,
        provider: AIProvider,
    ) -> ReportFuture<'_, Result<ResolvedReportProvider, String>> {
        Box::pin(async move {
            if provider == AIProvider::PhiSilica {
                return Ok(ResolvedReportProvider {
                    // No per-report cancellation token is threaded to this
                    // resolver yet; Default never cancels early, same
                    // behavior as before this adapter took a cancel field.
                    chat: Arc::new(PhiChatProvider::default()),
                    config_fingerprint: "provider=phi_silica;runtime=windows_ai".to_string(),
                    requested_model: None,
                });
            }
            let cfg = match provider {
                AIProvider::CodexCli | AIProvider::ClaudeCode => {
                    resolve_subscription_config(provider, &self.subscription_ports).await?
                }
                _ => resolve_compat_config(provider, &self.ports).await?,
            };
            let requested_model = cfg.model.clone();
            let config_fingerprint = provider_config_fingerprint(provider, &cfg);
            let chat: Arc<dyn ChatProvider> = Arc::new(CompatChatProvider { provider, cfg });
            Ok(ResolvedReportProvider {
                chat,
                config_fingerprint,
                requested_model,
            })
        })
    }
}

/// Builds one turn's resolver from live settings, so a settings edit or a
/// newly saved key applies to the very next report.
struct ShellReportResolvers {
    source: ShellChatSource,
}

impl ReportResolverFactory for ShellReportResolvers {
    fn resolver(
        &self,
        provider: AIProvider,
        availability: ProviderAvailability,
    ) -> Arc<dyn ReportProviderResolver> {
        Arc::new(TurnReportResolver {
            ports: Arc::new(self.source.ports()),
            subscription_ports: Arc::new(self.source.subscription_ports()),
            provider,
            availability,
        })
    }
}

/// Start the report worker with this shell's provider wiring.
///
/// # Errors
/// When the worker thread or its Tokio runtime cannot be created.
pub fn start_report_runtime(
    settings: SettingsService,
    foundry: Arc<dyn FoundryEndpointSource>,
    ollama: Arc<dyn OllamaSource>,
    cache: SharedAiCache,
) -> std::io::Result<(NativeReportRuntime, std_mpsc::Receiver<ReportWorkerEvent>)> {
    NativeReportRuntime::start(
        Box::new(ShellReportResolvers {
            source: ShellChatSource::new(settings, foundry, ollama),
        }),
        cache,
        Arc::new(ui_wake_support::notify),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wfdiag_native_ai_provider::{BackendFuture, ProviderKeySource};
    use wfdiag_native_settings::{AppSettings, ProviderKeyId};

    struct NoKeys;

    impl ProviderKeySource for NoKeys {
        fn load(&self, _key: ProviderKeyId) -> Option<String> {
            None
        }
    }

    struct Unreachable;

    impl FoundryEndpointSource for Unreachable {
        fn probe(&self, _configured: Option<String>) -> BackendFuture<'_, Option<String>> {
            Box::pin(async { None })
        }
    }

    impl OllamaSource for Unreachable {
        fn discover(&self, _configured: Option<String>) -> BackendFuture<'_, Option<String>> {
            Box::pin(async { None })
        }

        fn list_models(&self, _endpoint: String) -> BackendFuture<'_, Result<Vec<String>, String>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    fn resolver(provider: AIProvider, availability: ProviderAvailability) -> TurnReportResolver {
        TurnReportResolver {
            ports: Arc::new(CompatConfigPorts {
                settings: AppSettings {
                    preferred_ai_provider: "auto".to_string(),
                    ..AppSettings::default()
                },
                keys: Arc::new(NoKeys),
                foundry: Arc::new(Unreachable),
                ollama: Arc::new(Unreachable),
            }),
            subscription_ports: Arc::new(SubscriptionConfigPorts {
                settings: AppSettings::default(),
                status: Arc::new(
                    wfdiag_native_ai_provider::ProcessSubscriptionCliStatusSource::new(),
                ),
            }),
            provider,
            availability,
        }
    }

    #[test]
    fn the_active_provider_is_pinned_and_auto_reroute_stays_local() {
        let availability = ProviderAvailability {
            phi: true,
            foundry: true,
            openai: true,
            ..ProviderAvailability::default()
        };
        let resolver = resolver(AIProvider::PhiSilica, availability);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        assert_eq!(resolver.preference(), AIProviderPreference::Auto);
        // The status reply already chose the provider: resolving must not
        // re-probe and must not disagree with the UI's attribution.
        assert_eq!(
            runtime.block_on(resolver.determine_active(AIProviderPreference::OpenAI)),
            AIProvider::PhiSilica
        );
        assert_eq!(
            runtime.block_on(
                resolver.next_auto_local(AIProviderPreference::Auto, &[AIProvider::PhiSilica])
            ),
            Some(AIProvider::FoundryLocal),
            "a wide report may move to another local provider, never to cloud"
        );
    }
}
