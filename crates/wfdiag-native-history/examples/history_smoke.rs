use std::collections::HashMap;
use std::sync::Arc;
use wfdiag_native_history::{
    DiagnosticTask, HistoryRuntimeConfig, NativeHistoryRuntime, ScanRecord, TaskResult, Timestamp,
};

fn scan(id: &str, timestamp: &str, success: bool) -> ScanRecord {
    ScanRecord {
        id: id.to_string(),
        timestamp: Timestamp::from_iso_string(timestamp).expect("valid smoke timestamp"),
        computer_name: "HISTORY-SMOKE".to_string(),
        os_version: "Windows".to_string(),
        is_admin: false,
        results: HashMap::from([(
            "os_info".to_string(),
            Arc::new(TaskResult {
                success,
                output: if success { "ok" } else { "failed" }.to_string(),
                error: None,
                duration_ms: 1,
            }),
        )]),
        task_count: 1,
        success_count: usize::from(success),
        failure_count: usize::from(!success),
        duration_ms: 1,
        label: None,
        tags: Vec::new(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!(
        "wfdiag-native-history-smoke-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&directory).ok();
    let config = HistoryRuntimeConfig::new(
        directory.clone(),
        || (true, 30),
        || {
            vec![DiagnosticTask {
                id: "os_info".to_string(),
                name: "Operating System".to_string(),
                description: "Operating system details".to_string(),
                category: "System".to_string(),
                admin_required: false,
            }]
        },
    );
    let history = NativeHistoryRuntime::start(config)?;
    let async_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    async_runtime.block_on(async {
        history
            .request_save(scan("before", "2026-08-30T10:00:00Z", true))?
            .await?
            .map_err(std::io::Error::other)?;
        history
            .request_save(scan("after", "2026-08-30T11:00:00Z", false))?
            .await?
            .map_err(std::io::Error::other)?;
        let listed = history
            .request_list()?
            .await?
            .map_err(std::io::Error::other)?;
        let comparison = history
            .request_compare("after", "before")?
            .await?
            .map_err(std::io::Error::other)?;
        println!(
            "native history smoke: {} scans, {} regression, task={}",
            listed.len(),
            comparison.new_failures.len(),
            comparison
                .new_failures
                .first()
                .map_or("none", |change| change.task_name.as_str())
        );
        history
            .request_clear()?
            .await?
            .map_err(std::io::Error::other)?;
        Ok::<(), Box<dyn std::error::Error>>(())
    })?;
    drop(history);
    std::fs::remove_dir_all(directory).ok();
    Ok(())
}
