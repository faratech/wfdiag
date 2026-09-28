use crate::OnDeviceModelEngine;

/// Prepare the selected backend before starting the inference deadline.
/// Dropping this future also cancels outstanding native work.
pub async fn prepare_ondevice(
    engine: OnDeviceModelEngine,
    cancel: impl Fn() -> bool + Send + 'static,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let abandoned = Abandoned::default();
        let dropped = abandoned.0.clone();
        tokio::task::spawn_blocking(move || {
            let cancel = || cancel() || dropped.load(std::sync::atomic::Ordering::Relaxed);
            match engine {
                OnDeviceModelEngine::AionInstruct => crate::aion::prepare(&cancel),
                OnDeviceModelEngine::PhiSilica => crate::runtime::prepare_cached_phi(&cancel),
            }
        })
        .await
        .map_err(|e| format!("On-device preparation failed: {e}"))?
    }
    #[cfg(not(windows))]
    {
        let _ = (engine, cancel);
        Err("On-device AI requires Windows".into())
    }
}

/// One-shot entry point used by analysis, prioritization and fix plans.
pub async fn generate_ondevice_response(
    engine: OnDeviceModelEngine,
    prompt: &str,
    cancel: impl Fn() -> bool + Send + Sync + 'static,
) -> Result<String, String> {
    let cancel = std::sync::Arc::new(cancel);
    let preparation_cancel = cancel.clone();
    prepare_ondevice(engine, move || preparation_cancel()).await?;
    generate_prepared(engine, prompt, move || cancel(), None).await
}

#[cfg_attr(not(windows), allow(clippy::unused_async))] // Same async API on unsupported hosts.
pub(crate) async fn generate_prepared(
    engine: OnDeviceModelEngine,
    prompt: &str,
    cancel: impl Fn() -> bool + Send + 'static,
    tx: Option<tokio::sync::mpsc::Sender<String>>,
) -> Result<String, String> {
    #[cfg(windows)]
    {
        if engine == OnDeviceModelEngine::AionInstruct {
            let prompt = prompt.to_string();
            let abandoned = Abandoned::default();
            let dropped = abandoned.0.clone();
            return tokio::task::spawn_blocking(move || {
                crate::aion::generate(
                    &prompt,
                    &|| cancel() || dropped.load(std::sync::atomic::Ordering::Relaxed),
                    tx,
                )
            })
            .await
            .map_err(|e| format!("Aion generation failed: {e}"))?;
        }
        let text = crate::generate_response(prompt, cancel).await?;
        if let Some(tx) = tx {
            tx.send(text.clone())
                .await
                .map_err(|_| "Response consumer closed")?;
        }
        Ok(text)
    }
    #[cfg(not(windows))]
    {
        let _ = (engine, prompt, cancel, tx);
        Err("On-device AI requires Windows".into())
    }
}

#[cfg(windows)]
#[derive(Default)]
struct Abandoned(std::sync::Arc<std::sync::atomic::AtomicBool>);
#[cfg(windows)]
impl Drop for Abandoned {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
