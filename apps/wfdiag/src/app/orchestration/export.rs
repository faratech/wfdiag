//! Report export and delivery orchestration.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::{
    diagnostic_output_snapshot, export_format_label, resolve_export_picker_selection,
    resolved_export_format,
};
use crate::app::state::{PendingExport, PendingExportAction};
use crate::app::tasks::spawn_export_wait;
use crate::fixtures::visual::LiveTestFixture;
use crate::platform::external::current_export_date_strings;
use crate::platform::save_picker;
use crate::platform::save_picker::SupportPackagePickerOutcome;
use std::sync::Arc;
use wfdiag_native_diagnostics::SharedScanEvidence;
use wfdiag_native_export::{ExportMetadata, ExportRequest, ExportRequestKind};
use wfdiag_native_issues::projection::advance_nonzero_generation;
use windows_reactor::*;

impl WfdiagShell {
    pub(crate) fn resume_export_wait(&mut self, context: &ComponentContext<Self>) {
        if self.export_wait.is_some() || self.export_pending.is_none() {
            return;
        }
        let Some(receiver) = self.export_receiver.as_ref().map(Arc::clone) else {
            self.export_wait = None;
            return;
        };
        self.export_wait = Some(spawn_export_wait(context, receiver));
    }

    pub(crate) fn export_results_snapshot(&self) -> Option<SharedScanEvidence> {
        let current_session = self
            .diagnostic_results
            .first()
            .map(|result| result.session_id.as_str())?;

        if !self.diagnostics_busy()
            && self.issue_source_session_id.as_deref() == Some(current_session)
            && self
                .issue_source_results
                .as_ref()
                .is_some_and(|results| results.len() == self.diagnostic_results.len())
        {
            return self.issue_source_results.as_ref().map(Arc::clone);
        }

        Some(diagnostic_output_snapshot(&self.diagnostic_results))
    }

    pub(crate) fn request_share_to_windowsforum(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · sharing is disabled".to_string();
            return;
        }
        if self.export_pending.is_some() {
            self.status = "A report is already being prepared…".to_string();
            return;
        }
        let Some(results) = self.export_results_snapshot() else {
            self.status = "Run a scan before sharing a report".to_string();
            return;
        };
        let Some(runtime) = self.export_runtime.as_ref() else {
            self.status = self
                .export_error
                .clone()
                .unwrap_or_else(|| "Native report generation is unavailable".to_string());
            return;
        };
        let dates = match current_export_date_strings() {
            Ok(dates) => dates,
            Err(error) => {
                self.export_error = Some(error.to_string());
                self.status = "Failed to prepare share. Please try again.".to_string();
                return;
            }
        };
        let Some(request_id) = advance_nonzero_generation(&mut self.export_request_id) else {
            self.export_error = Some("Native export request identity was exhausted".to_string());
            self.status = "Native report generation is unavailable".to_string();
            return;
        };
        let request = ExportRequest {
            request_id,
            kind: ExportRequestKind::WindowsForumPost {
                metadata: ExportMetadata {
                    generated: dates.generated,
                    local_date: dates.local_date,
                    computer_name: self.system_info.computer_name.clone(),
                    os_version: self.system_info.os_version.clone(),
                    is_admin: self.system_info.is_admin,
                },
            },
            results,
        };
        if let Err(error) = runtime.enqueue(request) {
            self.export_error = Some(error.to_string());
            self.status = "Failed to prepare share. Please try again.".to_string();
            return;
        }
        self.export_pending = Some(PendingExport {
            request_id,
            action: PendingExportAction::ShareToWindowsForum,
        });
        self.export_error = None;
        self.status = "Preparing report for WindowsForum…".to_string();
        self.resume_export_wait(context);
    }

    pub(crate) fn request_email_report(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · email delivery is disabled".to_string();
            return;
        }
        if self.export_pending.is_some() {
            self.status = "A report is already being prepared…".to_string();
            return;
        }
        let Some(results) = self.export_results_snapshot() else {
            self.status = "Run a scan before emailing a report".to_string();
            return;
        };
        let Some(runtime) = self.export_runtime.as_ref() else {
            self.status = self
                .export_error
                .clone()
                .unwrap_or_else(|| "Native report generation is unavailable".to_string());
            return;
        };
        let dates = match current_export_date_strings() {
            Ok(dates) => dates,
            Err(error) => {
                self.export_error = Some(error.to_string());
                self.status =
                    "Failed to prepare email. Please try exporting the report instead.".to_string();
                return;
            }
        };
        let Some(request_id) = advance_nonzero_generation(&mut self.export_request_id) else {
            self.export_error = Some("Native export request identity was exhausted".to_string());
            self.status = "Native report generation is unavailable".to_string();
            return;
        };
        let request = ExportRequest {
            request_id,
            kind: ExportRequestKind::Email {
                metadata: ExportMetadata {
                    generated: dates.generated,
                    local_date: dates.local_date,
                    computer_name: self.system_info.computer_name.clone(),
                    os_version: self.system_info.os_version.clone(),
                    is_admin: self.system_info.is_admin,
                },
            },
            results,
        };
        if let Err(error) = runtime.enqueue(request) {
            self.export_error = Some(error.to_string());
            self.status =
                "Failed to prepare email. Please try exporting the report instead.".to_string();
            return;
        }
        self.export_pending = Some(PendingExport {
            request_id,
            action: PendingExportAction::EmailReport,
        });
        self.export_error = None;
        self.status = "Preparing email report…".to_string();
        self.resume_export_wait(context);
    }

    pub(crate) fn request_copy_diagnostic_report(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · clipboard export is disabled".to_string();
            return;
        }
        if self.export_pending.is_some() {
            self.status = "A report is already being prepared…".to_string();
            return;
        }
        let Some(results) = self.export_results_snapshot() else {
            self.status = "Run a scan before copying a diagnostic report".to_string();
            return;
        };
        let Some(runtime) = self.export_runtime.as_ref() else {
            self.status = self
                .export_error
                .clone()
                .unwrap_or_else(|| "Native report generation is unavailable".to_string());
            return;
        };
        let dates = match current_export_date_strings() {
            Ok(dates) => dates,
            Err(error) => {
                self.export_error = Some(error.to_string());
                self.status =
                    "Failed to prepare the clipboard report. Please try again.".to_string();
                return;
            }
        };
        let Some(request_id) = advance_nonzero_generation(&mut self.export_request_id) else {
            self.export_error = Some("Native export request identity was exhausted".to_string());
            self.status = "Native report generation is unavailable".to_string();
            return;
        };
        let request = ExportRequest {
            request_id,
            kind: ExportRequestKind::ForumClipboard {
                metadata: ExportMetadata {
                    generated: dates.generated,
                    local_date: dates.local_date,
                    computer_name: self.system_info.computer_name.clone(),
                    os_version: self.system_info.os_version.clone(),
                    is_admin: self.system_info.is_admin,
                },
            },
            results,
        };
        if let Err(error) = runtime.enqueue(request) {
            self.export_error = Some(error.to_string());
            self.status = "Failed to prepare the clipboard report. Please try again.".to_string();
            return;
        }
        self.export_pending = Some(PendingExport {
            request_id,
            action: PendingExportAction::CopyDiagnosticReport,
        });
        self.export_error = None;
        self.status = "Preparing diagnostic report for the clipboard…".to_string();
        self.resume_export_wait(context);
    }

    pub(crate) fn request_support_package(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual {
            self.status = "Visual fixture mode · support-package export is disabled".to_string();
            return;
        }
        if self.export_pending.is_some() {
            self.status = "A report is already being prepared…".to_string();
            return;
        }
        let Some(results) = self.export_results_snapshot() else {
            self.status = "Run a scan before generating a support package".to_string();
            return;
        };
        let Some(runtime) = self.export_runtime.as_ref() else {
            self.status = self
                .export_error
                .clone()
                .unwrap_or_else(|| "Native report generation is unavailable".to_string());
            return;
        };
        let paths = match save_picker::show_support_package_save_picker() {
            Ok(SupportPackagePickerOutcome::Cancelled) => return,
            Ok(SupportPackagePickerOutcome::Selected(paths)) => paths,
            Err(error) => {
                self.export_error = Some(error.to_string());
                self.status = format!("Support-package export failed · {error}");
                return;
            }
        };
        let Some(request_id) = advance_nonzero_generation(&mut self.export_request_id) else {
            self.export_error = Some("Native export request identity was exhausted".to_string());
            self.status = "Native report generation is unavailable".to_string();
            return;
        };
        if let Err(error) = runtime.enqueue(ExportRequest {
            request_id,
            kind: ExportRequestKind::SupportPackage { include_raw: true },
            results,
        }) {
            self.export_error = Some(error.to_string());
            self.status = "Failed to prepare the support package. Please try again.".to_string();
            return;
        }
        self.export_pending = Some(PendingExport {
            request_id,
            action: PendingExportAction::SupportPackage { paths },
        });
        self.export_error = None;
        self.status = "Preparing JSON, TXT, and HTML support reports…".to_string();
        self.resume_export_wait(context);
    }

    /// Export the latest completed scan to a user-chosen file, mirroring the
    /// Store 2.5.8 flow: the native save dialog runs synchronously on this
    /// UI thread (owner-validated by `save_picker`), while rendering and
    /// file I/O stay on workers. A dialog cancellation is a silent no-op,
    /// exactly like the shipping `save()` dialog path.
    pub(crate) fn request_export_to_file(&mut self, context: &ComponentContext<Self>) {
        if self.deterministic_visual
            && self.live_test_fixture != Some(LiveTestFixture::ExportFallback)
        {
            self.status = "Visual fixture mode · file export is disabled".to_string();
            return;
        }
        if self.export_pending.is_some() {
            self.status = "A report is already being prepared…".to_string();
            return;
        }
        let Some(results) = self.export_results_snapshot() else {
            self.status = "Run a scan before exporting a report".to_string();
            return;
        };
        let Some(runtime) = self.export_runtime.as_ref() else {
            self.status = self
                .export_error
                .clone()
                .unwrap_or_else(|| "Native report generation is unavailable".to_string());
            return;
        };
        let format = resolved_export_format(&self.settings_snapshot.export_format);
        let path =
            match resolve_export_picker_selection(save_picker::show_export_save_picker(format)) {
                Ok(Some(path)) => path,
                Ok(None) => return,
                Err(error) => {
                    self.export_error = Some(error.clone());
                    self.status = format!("Export failed · {error}");
                    return;
                }
            };
        let dates = match current_export_date_strings() {
            Ok(dates) => dates,
            Err(error) => {
                self.export_error = Some(error.to_string());
                self.status = "Failed to prepare export. Please try again.".to_string();
                return;
            }
        };
        let Some(request_id) = advance_nonzero_generation(&mut self.export_request_id) else {
            self.export_error = Some("Native export request identity was exhausted".to_string());
            self.status = "Native report generation is unavailable".to_string();
            return;
        };
        let request = ExportRequest {
            request_id,
            kind: ExportRequestKind::SavedReport {
                format,
                include_raw: true,
                metadata: ExportMetadata {
                    generated: dates.generated,
                    local_date: dates.local_date,
                    computer_name: self.system_info.computer_name.clone(),
                    os_version: self.system_info.os_version.clone(),
                    is_admin: self.system_info.is_admin,
                },
            },
            results,
        };
        if let Err(error) = runtime.enqueue(request) {
            self.export_error = Some(error.to_string());
            self.status = "Failed to prepare export. Please try again.".to_string();
            return;
        }
        self.export_pending = Some(PendingExport {
            request_id,
            action: PendingExportAction::SaveToFile { path },
        });
        self.export_error = None;
        self.status = format!("Preparing {} export…", export_format_label(format));
        self.resume_export_wait(context);
    }
}
