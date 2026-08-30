use std::error::Error;
use wfdiag_native_monitor::{NativeMonitorRuntime, ProcessQuery};

fn main() -> Result<(), Box<dyn Error>> {
    let async_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (monitor, _events) = NativeMonitorRuntime::start(false)?;

    let page = async_runtime.block_on(async {
        monitor
            .request_processes(ProcessQuery::default())?
            .await
            .map_err(std::io::Error::other)
    })?;

    println!(
        "captured {} of {} processes at {}",
        page.items.len(),
        page.total,
        page.captured_at
    );
    Ok(())
}
