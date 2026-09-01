//! Report export and delivery.
//!
//! # The picker runs first, and never on the UI thread (#140, #196)
//!
//! The old order was: render, then open a modal `IFileSaveDialog::Show`
//! *inside* `Component::update`, which froze the shell for as long as the
//! dialog was open. The order is now:
//!
//! 1. ask [`SavePickerHost`] for a destination — it runs the dialog on its own
//!    STA thread, owned by the registered Reactor window, and posts the answer
//!    back as [`Message::Export(ExportMsg::PickerFinished)`] with an epoch guard;
//! 2. cancellation is a silent no-op, a typed failure keeps its status text;
//! 3. only then dispatch [`AppCommand::ExportResults`], which renders the
//!    committed evidence on the export worker;
//! 4. write the payload on a background task.
//!
//! The clipboard and external-launch deliveries need no destination, so they
//! go straight to step 3.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::{export_format_label, rejection_text, resolved_export_format};
use crate::app::state::PendingExportAction;
use crate::app::tasks::{spawn_export_file_write, spawn_support_package_write};
use crate::dialogs::export::msg::ExportPickerKind;
use crate::fixtures::visual::LiveTestFixture;
use crate::platform::external::{
    current_export_date_strings, launch_email_compose_draft, launch_export_external_action,
    write_text_to_clipboard,
};
use crate::platform::save_picker::{SavePickerHost, SavePickerReply, SavePickerRequest};
use crate::platform::ui_wake;
use wfdiag_app::{AppCommand, DispatchOutcome};
use wfdiag_native_export::{
    ExportExternalAction, ExportMetadata, ExportPayload, ExportRequestKind,
};
use windows_reactor::*;

impl WfdiagShell {
    /// The machine metadata every decorated export carries.
    fn export_metadata(&mut self) -> Option<ExportMetadata> {
        match current_export_date_strings() {
            Ok(dates) => Some(ExportMetadata {
                generated: dates.generated,
                local_date: dates.local_date,
                computer_name: self.shell.system_info.computer_name.clone(),
                os_version: self.shell.system_info.os_version.clone(),
                is_admin: self.shell.system_info.is_admin,
            }),
            Err(error) => {
                self.export.error = Some(error.to_string());
                None
            }
        }
    }

    /// Common admission for every export entry point.
    fn export_admitted(&mut self, blocked_status: &str, empty_status: &str) -> bool {
        if self.shell.deterministic_visual {
            self.shell.status = blocked_status.to_string();
            return false;
        }
        if self.export.pending.is_some() || self.export.picker_busy {
            self.shell.status = "A report is already being prepared…".to_string();
            return false;
        }
        if self.diagnostics.results.is_empty() {
            self.shell.status = empty_status.to_string();
            return false;
        }
        true
    }

    /// Render one payload and remember what to do with it.
    fn begin_export(&mut self, action: PendingExportAction, kind: ExportRequestKind) -> bool {
        match self.dispatch(AppCommand::ExportResults {
            kind: Box::new(kind),
        }) {
            DispatchOutcome::Accepted { .. } => {
                self.export.pending = Some(action);
                self.export.error = None;
                true
            }
            outcome => {
                if let Some(reason) = outcome.rejection() {
                    self.export.error = Some(rejection_text(reason));
                }
                false
            }
        }
    }

    pub(crate) fn request_share_to_windowsforum(&mut self) {
        if !self.export_admitted(
            "Visual fixture mode · sharing is disabled",
            "Run a scan before sharing a report",
        ) {
            return;
        }
        let Some(metadata) = self.export_metadata() else {
            self.shell.status = "Failed to prepare share. Please try again.".to_string();
            return;
        };
        if self.begin_export(
            PendingExportAction::ShareToWindowsForum,
            ExportRequestKind::WindowsForumPost { metadata },
        ) {
            self.shell.status = "Preparing report for WindowsForum…".to_string();
        } else {
            self.shell.status = "Failed to prepare share. Please try again.".to_string();
        }
    }

    pub(crate) fn request_email_report(&mut self) {
        if !self.export_admitted(
            "Visual fixture mode · email delivery is disabled",
            "Run a scan before emailing a report",
        ) {
            return;
        }
        let Some(metadata) = self.export_metadata() else {
            self.shell.status =
                "Failed to prepare email. Please try exporting the report instead.".to_string();
            return;
        };
        if self.begin_export(
            PendingExportAction::EmailReport,
            ExportRequestKind::Email { metadata },
        ) {
            self.shell.status = "Preparing email report…".to_string();
        } else {
            self.shell.status =
                "Failed to prepare email. Please try exporting the report instead.".to_string();
        }
    }

    pub(crate) fn request_copy_diagnostic_report(&mut self) {
        if !self.export_admitted(
            "Visual fixture mode · clipboard export is disabled",
            "Run a scan before copying a diagnostic report",
        ) {
            return;
        }
        let Some(metadata) = self.export_metadata() else {
            self.shell.status =
                "Failed to prepare the clipboard report. Please try again.".to_string();
            return;
        };
        if self.begin_export(
            PendingExportAction::CopyDiagnosticReport,
            ExportRequestKind::ForumClipboard { metadata },
        ) {
            self.shell.status = "Preparing diagnostic report for the clipboard…".to_string();
        } else {
            self.shell.status =
                "Failed to prepare the clipboard report. Please try again.".to_string();
        }
    }

    pub(crate) fn request_support_package(&mut self) {
        if !self.export_admitted(
            "Visual fixture mode · support-package export is disabled",
            "Run a scan before generating a support package",
        ) {
            return;
        }
        self.open_save_picker(SavePickerRequest::SupportPackage);
    }

    /// Export the latest completed scan to a user-chosen file.
    pub(crate) fn request_export_to_file(&mut self) {
        if self.shell.deterministic_visual
            && self.shell.live_test_fixture != Some(LiveTestFixture::ExportFallback)
        {
            self.shell.status = "Visual fixture mode · file export is disabled".to_string();
            return;
        }
        if self.export.pending.is_some() || self.export.picker_busy {
            self.shell.status = "A report is already being prepared…".to_string();
            return;
        }
        if self.diagnostics.results.is_empty() {
            self.shell.status = "Run a scan before exporting a report".to_string();
            return;
        }
        let format = resolved_export_format(&self.shell.settings.export_format);
        self.open_save_picker(SavePickerRequest::Export(format));
    }

    /// Start one picker on its own STA thread and guard its answer.
    ///
    /// The completion carries the request back, so the shell does not have to
    /// remember which picker is open — only which generation it is.
    fn open_save_picker(&mut self, request: SavePickerRequest) {
        self.export.picker_epoch = self.export.picker_epoch.wrapping_add(1);
        match SavePickerHost::request(request, self.export.picker_epoch, ui_wake::notify) {
            Ok(()) => self.export.picker_busy = true,
            Err(error) => {
                self.export.error = Some(error.clone());
                self.shell.status = format!("Export failed · {error}");
            }
        }
    }

    /// Apply one picker answer.
    pub(crate) fn apply_export_picker_reply(
        &mut self,
        epoch: u64,
        kind: ExportPickerKind,
        reply: SavePickerReply,
    ) {
        if epoch != self.export.picker_epoch {
            return;
        }
        self.export.picker_busy = false;
        match (kind, reply) {
            // Cancellation stays silent, exactly like the shipping `save()`
            // dialog path.
            (_, SavePickerReply::Cancelled) => {}
            (_, SavePickerReply::Failed(error)) => {
                self.export.error = Some(error.clone());
                self.shell.status = if kind == ExportPickerKind::SupportPackage {
                    format!("Support-package export failed · {error}")
                } else {
                    format!("Export failed · {error}")
                };
            }
            (ExportPickerKind::File, SavePickerReply::Export(path)) => {
                let format = path.format();
                let Some(metadata) = self.export_metadata() else {
                    self.shell.status = "Failed to prepare export. Please try again.".to_string();
                    return;
                };
                if self.begin_export(
                    PendingExportAction::SaveToFile { path },
                    ExportRequestKind::SavedReport {
                        format,
                        include_raw: true,
                        metadata,
                    },
                ) {
                    self.shell.status =
                        format!("Preparing {} export…", export_format_label(format));
                } else {
                    self.shell.status = "Failed to prepare export. Please try again.".to_string();
                }
            }
            (ExportPickerKind::SupportPackage, SavePickerReply::SupportPackage(paths)) => {
                if self.begin_export(
                    PendingExportAction::SupportPackage { paths },
                    ExportRequestKind::SupportPackage { include_raw: true },
                ) {
                    self.shell.status =
                        "Preparing JSON, TXT, and HTML support reports…".to_string();
                } else {
                    self.shell.status =
                        "Failed to prepare the support package. Please try again.".to_string();
                }
            }
            // A picker answers only the request it was given; a mismatched
            // pair cannot be produced by `SavePickerHost`.
            _ => {}
        }
    }

    /// Deliver a rendered payload to the destination chosen before it ran.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn deliver_export_payload(
        &mut self,
        payload: ExportPayload,
        context: &ComponentContext<Self>,
    ) {
        let Some(action) = self.export.pending.take() else {
            return;
        };
        match (action, payload) {
            (PendingExportAction::ShareToWindowsForum, ExportPayload::WindowsForumPost(post)) => {
                match write_text_to_clipboard(&post) {
                    Ok(()) => {
                        match launch_export_external_action(
                            ExportExternalAction::WindowsForumNewThread,
                        ) {
                            Ok(()) => {
                                self.export.error = None;
                                self.shell.status =
                                "Report ready to share · copied to clipboard · paste with Ctrl+V"
                                    .to_string();
                            }
                            Err(error) => {
                                self.export.error = Some(error.to_string());
                                self.shell.status =
                                "Report copied to clipboard, but Windows could not open the forum"
                                    .to_string();
                            }
                        }
                    }
                    Err(error) => {
                        self.export.error = Some(error.to_string());
                        self.shell.status =
                            "Failed to prepare share. Please try again.".to_string();
                    }
                }
            }
            (PendingExportAction::EmailReport, ExportPayload::Email(email)) => {
                match write_text_to_clipboard(&email.clipboard_body) {
                    Ok(()) => match launch_email_compose_draft(&email) {
                        Ok(()) => {
                            self.export.error = None;
                            self.shell.status =
                                "Email ready · report copied to clipboard · paste with Ctrl+V"
                                    .to_string();
                        }
                        Err(error) => {
                            self.export.error = Some(error.to_string());
                            self.shell.status = "Report copied to clipboard, but Windows could not open a new email"
                                .to_string();
                        }
                    },
                    Err(error) => {
                        self.export.error = Some(error.to_string());
                        self.shell.status =
                            "Failed to prepare email. Please try exporting the report instead."
                                .to_string();
                    }
                }
            }
            (PendingExportAction::CopyDiagnosticReport, ExportPayload::ForumClipboard(report)) => {
                match write_text_to_clipboard(&report) {
                    Ok(()) => {
                        self.export.error = None;
                        self.shell.status = "Diagnostic report copied to the clipboard".to_string();
                    }
                    Err(error) => {
                        self.export.error = Some(error.to_string());
                        self.shell.status =
                            "Failed to copy the diagnostic report. Please try again.".to_string();
                    }
                }
            }
            (
                PendingExportAction::SupportPackage { paths },
                ExportPayload::SupportPackage(package),
            ) => {
                self.export.error = None;
                self.shell.status = "Writing JSON, TXT, and HTML support reports…".to_string();
                self.export.pending = Some(PendingExportAction::SupportPackage {
                    paths: paths.clone(),
                });
                self.export.write_task = Some(spawn_support_package_write(
                    context,
                    self.export.picker_epoch,
                    paths,
                    package,
                ));
            }
            (PendingExportAction::SaveToFile { path }, ExportPayload::Report(content)) => {
                self.export.error = None;
                self.shell.status =
                    format!("Writing {} report…", export_format_label(path.format()));
                self.export.pending = Some(PendingExportAction::SaveToFile { path: path.clone() });
                self.export.write_task = Some(spawn_export_file_write(
                    context,
                    self.export.picker_epoch,
                    path,
                    content,
                ));
            }
            _ => {
                self.export.error =
                    Some("Native export worker returned an unexpected payload".to_string());
                self.shell.status = "Failed to prepare share. Please try again.".to_string();
            }
        }
    }
}
