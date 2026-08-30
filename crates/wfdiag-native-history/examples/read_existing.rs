use wfdiag_native_history::{EncryptedStorage, ScanRecord, ScanStorage};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = ScanStorage::default_storage_directory().map_err(std::io::Error::other)?;
    let encrypted = EncryptedStorage::new(directory.clone())?;
    let ids = encrypted.list_files()?;
    let scan_ids: Vec<&str> = ids
        .iter()
        .map(String::as_str)
        .filter(|id| *id != "_scan_summary_index")
        .collect();
    let Some(scan_id) = scan_ids.first() else {
        println!("existing history probe: 0 scans at {}", directory.display());
        return Ok(());
    };
    let scan: ScanRecord = encrypted.load(scan_id)?;
    println!(
        "existing history probe: {} scans; loaded {} with {} task results",
        scan_ids.len(),
        scan.id,
        scan.results.len()
    );
    Ok(())
}
