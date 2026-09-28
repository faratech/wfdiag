//! Aion preview backend. Its ABI is independent of retail Windows AI.
use crate::aion_bindings::{LanguageModel, LanguageModelResponseStatus};
use crate::runtime::{attach_framework, enter_winrt_apartment};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};
use windows_core::Interface;
use windows_future::{AsyncOperationProgressHandler, AsyncStatus, IAsyncInfo};

const LOAD_TIMEOUT: Duration = Duration::from_mins(10);
static MODEL: Mutex<Option<LanguageModel>> = Mutex::new(None);
const SETUP: &str = "Aion Preview requires a supported ARM64 Snapdragon Copilot+ PC, the Aion Preview framework, Windows App Runtime 1.8 and the QNN 1.8 execution provider. Install the prerequisites from Microsoft's Aion Instruct Preview sample and restart WFDiag.";

pub(crate) fn prerequisites() -> Result<(), String> {
    if !crate::has_package_identity() {
        return Err("Aion Instruct in WFDiag requires the Microsoft Store version".into());
    }
    if !cfg!(target_arch = "aarch64") {
        return Err("This Aion Preview SDK supports ARM64 Snapdragon devices only".into());
    }
    // Optional process dependencies preserve the production Runtime 2 pin.
    // Version fields are packed as major/minor/build/revision (16 bits each).
    attach_framework(
        "Microsoft.AionInstructPreview.Framework.1.0_8wekyb3d8bbwe",
        1_u64 << 48,
    )
    .map_err(|error| format!("{error}. {SETUP}"))?;
    attach_framework(
        "Microsoft.WindowsAppRuntime.1.8_8wekyb3d8bbwe",
        (8000_u64 << 48) | (836_u64 << 32) | (2153_u64 << 16),
    )
    .map_err(|error| format!("{error}. {SETUP}"))?;
    Ok(())
}

pub(crate) fn probe() -> Result<(), String> {
    prerequisites()?;
    let _apartment = enter_winrt_apartment();
    crate::runtime::get_activation_factory_by_name::<crate::aion_bindings::ILanguageModelStatics>(
        "AionInstructPreview.Text.LanguageModel",
    )
    .map(|_| ())
    .map_err(|error| format!("Aion Preview activation failed: {error}. {SETUP}"))
}

fn model_lock(
    cancel: &dyn Fn() -> bool,
) -> Result<MutexGuard<'static, Option<LanguageModel>>, String> {
    let deadline = Instant::now() + LOAD_TIMEOUT;
    loop {
        if cancel() {
            return Err("Aion operation cancelled".into());
        }
        match MODEL.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(_)) => {
                return Err("Aion model lock poisoned; restart WFDiag".into());
            }
            Err(TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err("Aion is busy; try again shortly".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

pub(crate) fn prepare(cancel: &dyn Fn() -> bool) -> Result<(), String> {
    let _apartment = enter_winrt_apartment();
    let mut cached = model_lock(cancel)?;
    if cached.is_some() {
        return Ok(());
    }
    prerequisites()?;
    let op = LanguageModel::CreateAsync().map_err(|e| format!("Cannot load Aion: {e}. {SETUP}"))?;
    let info: IAsyncInfo = op.cast().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + LOAD_TIMEOUT;
    loop {
        if cancel() || Instant::now() >= deadline {
            let _ = info.Cancel();
            return Err(if cancel() {
                "Aion model preparation cancelled"
            } else {
                "Aion model preparation timed out after 10 minutes"
            }
            .into());
        }
        match info.Status().map_err(|e| e.to_string())? {
            AsyncStatus::Completed => {
                *cached = Some(
                    op.GetResults()
                        .map_err(|e| format!("Aion model preparation failed: {e}. {SETUP}"))?,
                );
                return Ok(());
            }
            AsyncStatus::Started => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                return Err(format!(
                    "Aion model preparation failed: {:?}. {SETUP}",
                    info.ErrorCode()
                ));
            }
        }
    }
}

pub(crate) fn generate(
    prompt: &str,
    cancel: &dyn Fn() -> bool,
    tx: Option<tokio::sync::mpsc::Sender<String>>,
) -> Result<String, String> {
    let _apartment = enter_winrt_apartment();
    let mut cached = model_lock(cancel)?;
    let model = cached
        .as_ref()
        .ok_or("Aion needs preparation before inference")?;
    let result = generate_with_model(model, prompt, cancel, tx);
    // Keep request failures (including context overflow) isolated from future
    // requests: prompt-only generation never retains native conversation state.
    if result.is_err()
        && let Some(model) = cached.take()
    {
        let _ = model.Close();
    }
    result
}

fn generate_with_model(
    model: &LanguageModel,
    prompt: &str,
    cancel: &dyn Fn() -> bool,
    tx: Option<tokio::sync::mpsc::Sender<String>>,
) -> Result<String, String> {
    let op = model
        .GenerateResponseAsync(&prompt.into())
        .map_err(|e| e.to_string())?;
    let emitted = Arc::new(Mutex::new(crate::stream::StreamReconciler::default()));
    let progress = Arc::clone(&emitted);
    let progress_tx = tx.clone();
    op.SetProgress(&AsyncOperationProgressHandler::new(
        move |_, delta: windows_core::InRef<'_, windows_core::HSTRING>| {
            let mut state = progress.lock().map_err(|_| windows_core::Error::empty())?;
            // Never block a runtime callback on a full consumer queue. On failure,
            // remember the error and cancel from the polling thread.
            if let Some(tx) = &progress_tx {
                state.forward(&delta.to_string_lossy(), |text| send_delta(tx, text));
            }
            Ok(())
        },
    ))
    .map_err(|e| e.to_string())?;
    let response = crate::runtime::wait_for_async_with_progress_blocking_timeout(
        op,
        Duration::from_secs(150),
        "Aion generation",
        &|| cancel() || emitted.lock().map_or(true, |s| s.failed()),
    )?;
    let status = response.Status().map_err(|e| e.to_string())?;
    if status != LanguageModelResponseStatus::Complete {
        return Err(match status {
            LanguageModelResponseStatus::PromptLargerThanContext => {
                "Aion prompt exceeds its context; reduce the evidence or start a new chat".into()
            }
            LanguageModelResponseStatus::Error => "Aion generation failed".into(),
            _ => format!("Aion returned unexpected terminal status {}", status.0),
        });
    }
    let text = response
        .Text()
        .map_err(|e| e.to_string())?
        .to_string_lossy();
    if let Some(tx) = tx {
        let mut state = emitted.lock().map_err(|_| "Aion stream state poisoned")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        state.finish(&text, |text| {
            loop {
                if cancel() || Instant::now() >= deadline {
                    return Err("Aion stream cancelled or stalled".into());
                }
                if send_delta(&tx, text.clone())? {
                    return Ok(true);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })?;
    }
    Ok(text)
}

fn send_delta(tx: &tokio::sync::mpsc::Sender<String>, text: String) -> Result<bool, String> {
    match tx.try_send(text) {
        Ok(()) => Ok(true),
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => Ok(false),
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            Err("Aion response consumer closed".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aion_bindings::{ILanguageModel, ILanguageModelStatics, ILanguageModelStatics_Vtbl};

    #[test]
    fn preview_contract_cannot_be_reinterpreted_as_retail() {
        assert_eq!(
            ILanguageModelStatics::IID,
            windows_core::GUID::from_u128(0xf37c8314_9118_5036_8f18_4a071bf9103d)
        );
        assert_eq!(
            ILanguageModel::IID,
            windows_core::GUID::from_u128(0x01216f3c_4cee_5f00_aedc_c705ed94c10e)
        );
        assert_ne!(
            ILanguageModelStatics::IID,
            crate::windows_ai_bindings::ILanguageModelStatics::IID
        );
        assert_eq!(
            size_of::<ILanguageModelStatics_Vtbl>(),
            size_of::<windows_core::IInspectable_Vtbl>() + size_of::<usize>()
        );
        assert_eq!(LanguageModelResponseStatus::Error.0, 2);
        assert_eq!(LanguageModelResponseStatus::PromptLargerThanContext.0, 3);
    }
}
