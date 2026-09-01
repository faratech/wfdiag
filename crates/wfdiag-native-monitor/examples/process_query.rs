#[cfg(windows)]
use std::error::Error;
#[cfg(windows)]
use wfdiag_native_monitor::{NativeMonitorRuntime, ProcessQuery, ProcessQueryOutcome};

#[cfg(windows)]
fn main() -> Result<(), Box<dyn Error>> {
    let async_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (monitor, _events) = NativeMonitorRuntime::start(false)?;

    let page = async_runtime.block_on(async {
        match monitor.request_processes(ProcessQuery::default())?.await {
            Ok(ProcessQueryOutcome::Page(page)) => Ok(page),
            Ok(ProcessQueryOutcome::Superseded) => {
                Err(std::io::Error::other("query superseded before it ran"))
            }
            Err(error) => Err(std::io::Error::other(error)),
        }
    })?;

    println!(
        "captured {} of {} processes at {}",
        page.items.len(),
        page.total,
        page.captured_at
    );
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("process_query is available only on Windows");
}
