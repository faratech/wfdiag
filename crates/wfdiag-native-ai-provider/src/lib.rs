//! UI-framework-neutral AI provider status, routing, and management.
//!
//! This crate owns the wire contracts and composition service used by the
//! shipping Tauri shell and prepared for the native `WinUI` shell. It includes
//! concrete Foundry, subscription CLI, Ollama, and custom-endpoint probes;
//! settings, identity, and Phi inputs remain explicit interfaces. Neither
//! Reactor nor this crate depends on Tauri, `WebView2`, or Phi activation.

#![allow(clippy::missing_errors_doc)]

use serde::{Deserialize, Serialize};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::thread::JoinHandle;
use tokio::sync::{mpsc, oneshot};

mod cache;
mod composition;
mod local_probes;
mod network;

pub use cache::{ProviderCacheControl, SharedAiCache};
pub use composition::{
    CustomEndpointSource, FoundryEndpointSource, OllamaSource, PackageIdentitySource,
    PhiStatusSnapshot, PhiStatusSource, ProviderConfigurationSnapshot, ProviderConfigurationSource,
    ProviderManagementService, ProviderPreferenceSettingsValidator, ProviderProbeBundle,
    ProviderSelectionState, SettingsServiceProviderConfigurationSource, SubscriptionCli,
    SubscriptionCliStatusSource,
};
pub use local_probes::{
    FoundryCliEndpointSource, ProcessSubscriptionCliStatusSource, SUBSCRIPTION_OVERRIDE_ENV_VARS,
    extract_http_base, foundry_service_is_healthy, valid_foundry_status_body,
};
pub use network::{
    OLLAMA_DEFAULT_ENDPOINT, ReqwestOllamaSource, TcpCustomEndpointSource,
    discover_ollama_endpoint, list_ollama_models, normalize_base_url, ollama_model_supports_tools,
    parse_ollama_capabilities, parse_ollama_tags, probe_http_endpoint, probe_http_endpoint_async,
    resolve_ollama_model,
};

/// Exact provider identifiers exposed to every UI shell.
///
/// Wire strings are pinned explicitly. Serde's snake-case conversion turns
/// `OpenAI` into `open_a_i`, which is not the established frontend contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AIProvider {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "openai", alias = "open_a_i")]
    OpenAI,
    #[serde(rename = "phi_silica")]
    PhiSilica,
    #[serde(rename = "foundry_local")]
    FoundryLocal,
    #[serde(rename = "ollama")]
    Ollama,
    #[serde(rename = "custom_openai")]
    CustomOpenAI,
    #[serde(rename = "codex_cli")]
    CodexCli,
    #[serde(rename = "claude_code")]
    ClaudeCode,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "deepseek")]
    DeepSeek,
}

impl fmt::Display for AIProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::None => "none",
            Self::OpenAI => "openai",
            Self::PhiSilica => "phi_silica",
            Self::FoundryLocal => "foundry_local",
            Self::Ollama => "ollama",
            Self::CustomOpenAI => "custom_openai",
            Self::CodexCli => "codex_cli",
            Self::ClaudeCode => "claude_code",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::DeepSeek => "deepseek",
        })
    }
}

/// User-selected provider routing policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AIProviderPreference {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "openai", alias = "open_a_i")]
    OpenAI,
    #[serde(rename = "phi_silica")]
    PhiSilica,
    #[serde(rename = "foundry_local")]
    FoundryLocal,
    #[serde(rename = "ollama")]
    Ollama,
    #[serde(rename = "custom_openai")]
    CustomOpenAI,
    #[serde(rename = "codex_cli")]
    CodexCli,
    #[serde(rename = "claude_code")]
    ClaudeCode,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "deepseek")]
    DeepSeek,
}

/// Explicit Phi selection is valid only for an identified Store process.
pub const PHI_SILICA_STORE_REQUIRED: &str = "Phi Silica requires the Microsoft Store version of this app (registered package identity with the systemAIModels capability). Select Auto or another available provider in this build.";

/// Parse the historical provider aliases accepted by Settings and IPC.
/// Unknown values intentionally retain the established Auto fallback.
#[must_use]
pub fn parse_provider_preference(preference: &str) -> AIProviderPreference {
    match preference.trim().to_ascii_lowercase().as_str() {
        "openai" => AIProviderPreference::OpenAI,
        "phi_silica" | "phisilica" => AIProviderPreference::PhiSilica,
        "foundry_local" | "foundrylocal" => AIProviderPreference::FoundryLocal,
        "ollama" => AIProviderPreference::Ollama,
        "custom_openai" | "custom" => AIProviderPreference::CustomOpenAI,
        "codex_cli" | "codexcli" | "codex" => AIProviderPreference::CodexCli,
        "claude_code" | "claudecode" | "claude" => AIProviderPreference::ClaudeCode,
        "anthropic" => AIProviderPreference::Anthropic,
        "gemini" => AIProviderPreference::Gemini,
        "deepseek" => AIProviderPreference::DeepSeek,
        _ => AIProviderPreference::Auto,
    }
}

/// Reject an explicit Phi preference for an unpackaged process.
pub fn validate_provider_preference(
    preference: AIProviderPreference,
    has_package_identity: bool,
) -> Result<AIProviderPreference, String> {
    if preference == AIProviderPreference::PhiSilica && !has_package_identity {
        return Err(PHI_SILICA_STORE_REQUIRED.to_string());
    }
    Ok(preference)
}

/// Parse and validate one provider preference.
pub fn parse_and_validate_provider_preference(
    preference: &str,
    has_package_identity: bool,
) -> Result<AIProviderPreference, String> {
    validate_provider_preference(parse_provider_preference(preference), has_package_identity)
}

/// Normalize a stale explicit Phi setting to Auto for an unpackaged runtime.
#[must_use]
pub fn provider_preference_for_runtime(
    preference: &str,
    has_package_identity: bool,
) -> AIProviderPreference {
    parse_and_validate_provider_preference(preference, has_package_identity)
        .unwrap_or(AIProviderPreference::Auto)
}

/// Provider capabilities and context budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProviderCaps {
    pub supports_tools: bool,
    pub supports_streaming: bool,
    pub context_budget_chars: usize,
}

/// Single source of truth for provider capabilities.
#[must_use]
pub const fn capabilities(provider: AIProvider) -> ProviderCaps {
    match provider {
        AIProvider::None => ProviderCaps {
            supports_tools: false,
            supports_streaming: false,
            context_budget_chars: 0,
        },
        AIProvider::PhiSilica => ProviderCaps {
            supports_tools: false,
            supports_streaming: false,
            context_budget_chars: 2_500,
        },
        AIProvider::FoundryLocal => ProviderCaps {
            supports_tools: false,
            supports_streaming: true,
            context_budget_chars: 12_000,
        },
        AIProvider::Ollama => ProviderCaps {
            supports_tools: true,
            supports_streaming: true,
            context_budget_chars: 12_000,
        },
        AIProvider::CustomOpenAI => ProviderCaps {
            supports_tools: true,
            supports_streaming: true,
            context_budget_chars: 24_000,
        },
        AIProvider::CodexCli => ProviderCaps {
            supports_tools: false,
            supports_streaming: false,
            context_budget_chars: 24_000,
        },
        AIProvider::ClaudeCode => ProviderCaps {
            supports_tools: false,
            supports_streaming: true,
            context_budget_chars: 24_000,
        },
        AIProvider::OpenAI | AIProvider::Anthropic | AIProvider::Gemini | AIProvider::DeepSeek => {
            ProviderCaps {
                supports_tools: true,
                supports_streaming: true,
                context_budget_chars: 48_000,
            }
        }
    }
}

/// Snapshot of which providers can serve a request right now.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderAvailability {
    pub phi: bool,
    pub foundry: bool,
    pub ollama: bool,
    pub custom: bool,
    pub codex: bool,
    pub claude: bool,
    pub openai: bool,
    pub anthropic: bool,
    pub gemini: bool,
    pub deepseek: bool,
}

/// Pure local-first routing decision used by every shell and request path.
#[must_use]
pub const fn route_provider(
    preference: AIProviderPreference,
    availability: ProviderAvailability,
) -> AIProvider {
    match preference {
        AIProviderPreference::Auto => {
            if availability.phi {
                AIProvider::PhiSilica
            } else if availability.foundry {
                AIProvider::FoundryLocal
            } else if availability.ollama {
                AIProvider::Ollama
            } else if availability.custom {
                AIProvider::CustomOpenAI
            } else if availability.codex {
                AIProvider::CodexCli
            } else if availability.claude {
                AIProvider::ClaudeCode
            } else if availability.openai {
                AIProvider::OpenAI
            } else if availability.anthropic {
                AIProvider::Anthropic
            } else if availability.gemini {
                AIProvider::Gemini
            } else if availability.deepseek {
                AIProvider::DeepSeek
            } else {
                AIProvider::None
            }
        }
        AIProviderPreference::OpenAI if availability.openai => AIProvider::OpenAI,
        AIProviderPreference::PhiSilica if availability.phi => AIProvider::PhiSilica,
        AIProviderPreference::FoundryLocal if availability.foundry => AIProvider::FoundryLocal,
        AIProviderPreference::Ollama if availability.ollama => AIProvider::Ollama,
        AIProviderPreference::CustomOpenAI if availability.custom => AIProvider::CustomOpenAI,
        AIProviderPreference::CodexCli if availability.codex => AIProvider::CodexCli,
        AIProviderPreference::ClaudeCode if availability.claude => AIProvider::ClaudeCode,
        AIProviderPreference::Anthropic if availability.anthropic => AIProvider::Anthropic,
        AIProviderPreference::Gemini if availability.gemini => AIProvider::Gemini,
        AIProviderPreference::DeepSeek if availability.deepseek => AIProvider::DeepSeek,
        _ => AIProvider::None,
    }
}

/// Per-provider row used by Settings badges and provider pickers.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: AIProvider,
    pub available: bool,
    pub configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub supports_tools: bool,
    pub supports_streaming: bool,
}

/// Exact `ai_get_status` wire contract.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AIProviderStatus {
    pub preferred_provider: AIProvider,
    pub openai_available: bool,
    pub openai_api_key_set: bool,
    pub phi_silica_available: bool,
    pub phi_silica_ready: bool,
    pub phi_silica_message: Option<String>,
    #[serde(default)]
    pub foundry_local_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foundry_local_endpoint: Option<String>,
    pub active_provider: AIProvider,
    #[serde(default)]
    pub providers: Vec<ProviderInfo>,
}

/// Minimal CLI-probe projection needed by provider status.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliProbeSnapshot {
    pub usable: bool,
    pub installed: bool,
    pub path: Option<String>,
}

/// Results of all provider probes used by a status refresh.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderProbeSnapshot {
    pub openai_available: bool,
    pub phi_silica_available: bool,
    pub phi_silica_ready: bool,
    pub phi_silica_message: Option<String>,
    pub foundry_endpoint: Option<String>,
    pub ollama_endpoint: Option<String>,
    pub custom_endpoint: Option<String>,
    pub codex: CliProbeSnapshot,
    pub claude: CliProbeSnapshot,
    pub anthropic_available: bool,
    pub gemini_available: bool,
    pub deepseek_available: bool,
}

/// Model defaults remain injected from the provider implementations. This
/// prevents the status seam from becoming a second source of model ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderModelDefaults {
    pub foundry: String,
    pub openai: String,
    pub anthropic: String,
    pub gemini: String,
    pub deepseek: String,
}

/// Non-secret settings needed to project provider status.
///
/// This deliberately excludes every credential field from the canonical
/// application settings schema. Secure storage remains a separate seam.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderSettingsSnapshot {
    pub local_ai_model: Option<String>,
    pub ollama_model: Option<String>,
    pub custom_endpoint: Option<String>,
    pub custom_model: Option<String>,
    pub codex_model: Option<String>,
    pub claude_model: Option<String>,
    pub open_ai_model: Option<String>,
    pub anthropic_model: Option<String>,
    pub gemini_model: Option<String>,
    pub deepseek_model: Option<String>,
}

/// Complete pure input for one provider-status projection.
#[derive(Debug, Clone)]
pub struct ProviderStatusInput {
    pub preference: AIProviderPreference,
    pub settings: ProviderSettingsSnapshot,
    pub probes: ProviderProbeSnapshot,
    pub defaults: ProviderModelDefaults,
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|item| !item.trim().is_empty())
}

fn provider_info(
    id: AIProvider,
    available: bool,
    configured: bool,
    model: Option<String>,
    endpoint: Option<String>,
) -> ProviderInfo {
    let caps = capabilities(id);
    ProviderInfo {
        id,
        available,
        configured,
        model: nonempty(model),
        endpoint,
        supports_tools: caps.supports_tools,
        supports_streaming: caps.supports_streaming,
    }
}

/// Build the exact shipping status response from injected settings and probe
/// results. Provider order is the Auto routing order.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn project_provider_status(input: ProviderStatusInput) -> AIProviderStatus {
    let ProviderStatusInput {
        preference,
        settings,
        probes,
        defaults,
    } = input;
    let availability = ProviderAvailability {
        phi: probes.phi_silica_ready,
        foundry: probes.foundry_endpoint.is_some(),
        ollama: probes.ollama_endpoint.is_some(),
        custom: probes.custom_endpoint.is_some(),
        codex: probes.codex.usable,
        claude: probes.claude.usable,
        openai: probes.openai_available,
        anthropic: probes.anthropic_available,
        gemini: probes.gemini_available,
        deepseek: probes.deepseek_available,
    };
    let active = route_provider(preference, availability);
    let custom_configured = settings
        .custom_endpoint
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        && settings
            .custom_model
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
    let providers = vec![
        provider_info(
            AIProvider::PhiSilica,
            probes.phi_silica_ready,
            probes.phi_silica_available,
            None,
            None,
        ),
        provider_info(
            AIProvider::FoundryLocal,
            probes.foundry_endpoint.is_some(),
            probes.foundry_endpoint.is_some(),
            Some(nonempty(settings.local_ai_model).unwrap_or(defaults.foundry)),
            probes.foundry_endpoint.clone(),
        ),
        provider_info(
            AIProvider::Ollama,
            probes.ollama_endpoint.is_some(),
            true,
            settings.ollama_model,
            probes.ollama_endpoint,
        ),
        provider_info(
            AIProvider::CustomOpenAI,
            probes.custom_endpoint.is_some(),
            custom_configured,
            settings.custom_model,
            probes.custom_endpoint.or(settings.custom_endpoint),
        ),
        provider_info(
            AIProvider::CodexCli,
            probes.codex.usable,
            probes.codex.installed,
            settings.codex_model,
            probes.codex.path,
        ),
        provider_info(
            AIProvider::ClaudeCode,
            probes.claude.usable,
            probes.claude.installed,
            settings.claude_model,
            probes.claude.path,
        ),
        provider_info(
            AIProvider::OpenAI,
            probes.openai_available,
            probes.openai_available,
            Some(nonempty(settings.open_ai_model).unwrap_or(defaults.openai)),
            None,
        ),
        provider_info(
            AIProvider::Anthropic,
            probes.anthropic_available,
            probes.anthropic_available,
            Some(nonempty(settings.anthropic_model).unwrap_or(defaults.anthropic)),
            None,
        ),
        provider_info(
            AIProvider::Gemini,
            probes.gemini_available,
            probes.gemini_available,
            Some(nonempty(settings.gemini_model).unwrap_or(defaults.gemini)),
            None,
        ),
        provider_info(
            AIProvider::DeepSeek,
            probes.deepseek_available,
            probes.deepseek_available,
            Some(nonempty(settings.deepseek_model).unwrap_or(defaults.deepseek)),
            None,
        ),
    ];

    AIProviderStatus {
        // Historical contract: this field reports the resolved provider, not
        // the raw `auto` preference. Keep it byte-compatible with 2.5.8.
        preferred_provider: active,
        openai_available: probes.openai_available,
        openai_api_key_set: probes.openai_available,
        phi_silica_available: probes.phi_silica_available,
        phi_silica_ready: probes.phi_silica_ready,
        phi_silica_message: probes.phi_silica_message,
        foundry_local_available: probes.foundry_endpoint.is_some(),
        foundry_local_endpoint: probes.foundry_endpoint,
        active_provider: active,
        providers,
    }
}

/// Boxed backend future keeps the service object-safe without a macro/runtime
/// dependency leaking into the Tauri or Reactor adapter.
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Provider-specific operations injected by the application backend.
///
/// Implementations may perform network, CLI, secure-storage, or `WinRT` work.
/// They always execute on [`NativeAiProviderRuntime`]'s worker, never on the
/// caller's UI thread.
pub trait ProviderManagementBackend: Send + Sync + 'static {
    fn status_input(&self) -> BackendFuture<'_, ProviderStatusInput>;
    fn has_package_identity(&self) -> bool;
    fn set_preference(&self, preference: AIProviderPreference);
    fn clear_cache(&self, session_id: Option<&str>);
    fn list_ollama_models(&self) -> BackendFuture<'_, Result<Vec<String>, String>>;
}

enum ProviderCommand {
    GetStatus {
        reply: oneshot::Sender<AIProviderStatus>,
    },
    SetPreference {
        preference: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SetPreferenceAndGetStatus {
        preference: String,
        reply: oneshot::Sender<Result<AIProviderStatus, String>>,
    },
    ClearCache {
        session_id: Option<String>,
        reply: oneshot::Sender<()>,
    },
    ListOllamaModels {
        reply: oneshot::Sender<Result<Vec<String>, String>>,
    },
    Shutdown,
}

pub type ProviderStatusReply = oneshot::Receiver<AIProviderStatus>;
pub type ProviderMutationReply = oneshot::Receiver<Result<(), String>>;
pub type ProviderPreferenceStatusReply = oneshot::Receiver<Result<AIProviderStatus, String>>;
pub type ProviderCacheReply = oneshot::Receiver<()>;
pub type OllamaModelsReply = oneshot::Receiver<Result<Vec<String>, String>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRuntimeError {
    SpawnFailed,
    WorkerStopped,
}

impl fmt::Display for ProviderRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpawnFailed => formatter.write_str("failed to start native AI provider worker"),
            Self::WorkerStopped => formatter.write_str("native AI provider worker stopped"),
        }
    }
}

impl std::error::Error for ProviderRuntimeError {}

fn reap_worker(worker: JoinHandle<()>) {
    let _ = std::thread::Builder::new()
        .name("wfdiag-ai-provider-reaper".to_string())
        .spawn(move || {
            let _ = worker.join();
        });
}

/// Dedicated provider-management worker for native UI shells.
///
/// Request methods enqueue typed commands and return immediately. The worker
/// owns a Tokio runtime for existing asynchronous provider probes.
pub struct NativeAiProviderRuntime {
    commands: mpsc::UnboundedSender<ProviderCommand>,
    worker: Option<JoinHandle<()>>,
}

impl NativeAiProviderRuntime {
    pub fn start(
        backend: Arc<dyn ProviderManagementBackend>,
    ) -> Result<Self, ProviderRuntimeError> {
        let (commands, mut receiver) = mpsc::unbounded_channel();
        let worker = std::thread::Builder::new()
            .name("wfdiag-native-ai-provider".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let Ok(runtime) = runtime else {
                    return;
                };
                while let Some(command) = receiver.blocking_recv() {
                    match command {
                        ProviderCommand::GetStatus { mut reply } => {
                            let input = runtime.block_on(async {
                                tokio::select! {
                                    biased;
                                    () = reply.closed() => None,
                                    input = backend.status_input() => Some(input),
                                }
                            });
                            if let Some(input) = input {
                                let _ = reply.send(project_provider_status(input));
                            }
                        }
                        ProviderCommand::SetPreference { preference, reply } => {
                            let result = parse_and_validate_provider_preference(
                                &preference,
                                backend.has_package_identity(),
                            )
                            .map(|preference| backend.set_preference(preference));
                            let _ = reply.send(result);
                        }
                        ProviderCommand::SetPreferenceAndGetStatus {
                            preference,
                            mut reply,
                        } => {
                            let result = parse_and_validate_provider_preference(
                                &preference,
                                backend.has_package_identity(),
                            );
                            match result {
                                Ok(preference) => {
                                    backend.set_preference(preference);
                                    let input = runtime.block_on(async {
                                        tokio::select! {
                                            biased;
                                            () = reply.closed() => None,
                                            input = backend.status_input() => Some(input),
                                        }
                                    });
                                    if let Some(input) = input {
                                        let _ = reply.send(Ok(project_provider_status(input)));
                                    }
                                }
                                Err(error) => {
                                    let _ = reply.send(Err(error));
                                }
                            }
                        }
                        ProviderCommand::ClearCache { session_id, reply } => {
                            backend.clear_cache(session_id.as_deref());
                            let _ = reply.send(());
                        }
                        ProviderCommand::ListOllamaModels { reply } => {
                            let result = runtime.block_on(backend.list_ollama_models());
                            let _ = reply.send(result);
                        }
                        ProviderCommand::Shutdown => break,
                    }
                }
            })
            .map_err(|_| ProviderRuntimeError::SpawnFailed)?;
        Ok(Self {
            commands,
            worker: Some(worker),
        })
    }

    pub fn request_status(&self) -> Result<ProviderStatusReply, ProviderRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.commands
            .send(ProviderCommand::GetStatus { reply })
            .map_err(|_| ProviderRuntimeError::WorkerStopped)?;
        Ok(receiver)
    }

    pub fn request_set_preference(
        &self,
        preference: String,
    ) -> Result<ProviderMutationReply, ProviderRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.commands
            .send(ProviderCommand::SetPreference { preference, reply })
            .map_err(|_| ProviderRuntimeError::WorkerStopped)?;
        Ok(receiver)
    }

    /// Atomically apply a validated preference and return the status projected
    /// after that mutation. FIFO worker ordering prevents a stale selection
    /// from being rendered between a committed Settings save and its refresh.
    pub fn request_set_preference_and_status(
        &self,
        preference: String,
    ) -> Result<ProviderPreferenceStatusReply, ProviderRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.commands
            .send(ProviderCommand::SetPreferenceAndGetStatus { preference, reply })
            .map_err(|_| ProviderRuntimeError::WorkerStopped)?;
        Ok(receiver)
    }

    pub fn request_clear_cache(
        &self,
        session_id: Option<String>,
    ) -> Result<ProviderCacheReply, ProviderRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.commands
            .send(ProviderCommand::ClearCache { session_id, reply })
            .map_err(|_| ProviderRuntimeError::WorkerStopped)?;
        Ok(receiver)
    }

    pub fn request_ollama_models(&self) -> Result<OllamaModelsReply, ProviderRuntimeError> {
        let (reply, receiver) = oneshot::channel();
        self.commands
            .send(ProviderCommand::ListOllamaModels { reply })
            .map_err(|_| ProviderRuntimeError::WorkerStopped)?;
        Ok(receiver)
    }
}

impl Drop for NativeAiProviderRuntime {
    fn drop(&mut self) {
        let _ = self.commands.send(ProviderCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            reap_worker(worker);
        }
    }
}

#[cfg(test)]
mod tests;
