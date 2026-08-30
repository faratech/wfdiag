//! UI-framework-neutral Windows AI/Phi Silica runtime.
//!
//! This crate is the single owner of WFDiag's package-identity gate, Limited
//! Access Feature handling, Windows AI activation fallback, model cache, and
//! generation path. UI shells consume the typed provider-status adapter or
//! the explicit runtime functions; neither path depends on Tauri or WebView2.

#![allow(clippy::missing_errors_doc)]

use serde::Serialize;
#[cfg(windows)]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
#[rustfmt::skip]
mod windows_ai_bindings;

mod runtime;

pub use runtime::{
    PhiPromptFit, PhiSilicaAnalysisResponse, PhiSilicaStatus, ensure_phi_silica_ready,
    generate_response, is_phi_silica_available, measure_prompt_fit,
};

use wfdiag_native_ai_provider::{BackendFuture, PhiStatusSnapshot, PhiStatusSource};

/// Real package-identity/LAF-aware status source shared by Tauri and Reactor.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsPhiStatusSource;

impl PhiStatusSource for WindowsPhiStatusSource {
    fn probe(&self) -> BackendFuture<'_, PhiStatusSnapshot> {
        Box::pin(async {
            match probe_phi_silica_status().await {
                Ok(status) => PhiStatusSnapshot {
                    available: status.available,
                    ready: status.ready_state.as_deref() == Some("Ready"),
                    message: Some(status.message),
                },
                Err(error) => PhiStatusSnapshot {
                    available: false,
                    ready: false,
                    message: Some(error),
                },
            }
        })
    }
}

/// Run the blocking WinRT availability probe off the async runtime worker.
pub async fn probe_phi_silica_status() -> Result<PhiSilicaStatus, String> {
    tokio::task::spawn_blocking(is_phi_silica_available)
        .await
        .map_err(|error| format!("Phi Silica availability check task panicked: {error}"))
}

/// True when the current process has registered package identity.
///
/// The runtime checks this before initializing WinRT, loading Windows App SDK
/// DLLs, or attempting LAF unlock. A loose executable therefore never reaches
/// the Windows AI activation path.
#[cfg(windows)]
#[must_use]
pub fn has_package_identity() -> bool {
    use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;

    let mut length = 0;
    let result = unsafe { GetCurrentPackageFullName(&raw mut length, None) };
    result == ERROR_INSUFFICIENT_BUFFER
}

#[cfg(not(windows))]
#[must_use]
pub const fn has_package_identity() -> bool {
    false
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "details")]
enum PhiError {
    #[cfg(windows)]
    AiUnavailable { provider: String, reason: String },
    #[cfg(not(windows))]
    PlatformNotSupported { operation: String },
}

impl PhiError {
    #[cfg(windows)]
    fn ai_unavailable(provider: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::AiUnavailable {
            provider: provider.into(),
            reason: reason.into(),
        }
    }
}

impl From<PhiError> for String {
    fn from(error: PhiError) -> Self {
        serde_json::to_string(&error).unwrap_or_else(|_| "Phi Silica operation failed".to_string())
    }
}

#[cfg(windows)]
fn format_log_time(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        (seconds / 3600) % 24,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_windows_identity_is_false() {
        #[cfg(not(windows))]
        assert!(!has_package_identity());
    }

    #[cfg(not(windows))]
    #[test]
    fn provider_adapter_projects_the_platform_status() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime");
        let status = runtime.block_on(WindowsPhiStatusSource.probe());

        assert!(!status.available);
        assert!(!status.ready);
        assert_eq!(
            status.message.as_deref(),
            Some("Phi Silica is only available on Windows")
        );
    }

    #[cfg(windows)]
    #[test]
    fn error_wire_shape_remains_compatible() {
        let encoded: String = PhiError::ai_unavailable("phi_silica", "not ready").into();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&encoded).unwrap(),
            serde_json::json!({
                "type": "AiUnavailable",
                "details": { "provider": "phi_silica", "reason": "not ready" }
            })
        );
    }
}
