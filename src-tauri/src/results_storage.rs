//! Compatibility shim: scan history storage now lives in
//! `wfdiag-native-history` so both shells compile one implementation.

pub use wfdiag_native_history::storage::*;

/// Open the shipping Tauri scan store.
///
/// This shell must inject the user's retention setting and the live
/// diagnostic catalog instead of compatibility defaults, exactly as the
/// backend did before the store moved into its own crate.
pub fn open_default_storage() -> Result<ScanStorage, String> {
    ScanStorage::new_in(
        ScanStorage::default_storage_directory()?,
        crate::commands::settings::history_retention,
        || {
            crate::diagnostics::get_all_tasks()
                .into_iter()
                .map(|task| wfdiag_native_history::DiagnosticTask {
                    id: task.id,
                    name: task.name,
                    description: task.description,
                    category: task.category,
                    admin_required: task.admin_required,
                })
                .collect()
        },
    )
}
