//! Routing the export surface's messages.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::dialogs::export::msg::ExportMsg;

impl WfdiagShell {
    /// One export message: a finished picker, or a finished write.
    pub(crate) fn route_export(&mut self, message: ExportMsg) {
        match message {
            ExportMsg::PickerFinished {
                epoch,
                kind,
                outcome,
            } => self.apply_export_picker_reply(epoch, kind, *outcome),
            ExportMsg::FileSaved { epoch, result } => {
                if epoch != self.export.picker_epoch {
                    return;
                }
                self.export.write_task = None;
                self.export.pending = None;
                match *result {
                    Ok(path) => {
                        self.export.error = None;
                        self.shell.status = format!("Results saved to {}", path.display());
                    }
                    Err(error) => {
                        self.export.error = Some(error);
                        self.shell.status =
                            "Failed to save the file. Please try a different location.".to_string();
                    }
                }
            }
            ExportMsg::SupportPackageSaved { epoch, result } => {
                if epoch != self.export.picker_epoch {
                    return;
                }
                self.export.write_task = None;
                self.export.pending = None;
                match *result {
                    Ok(paths) => {
                        self.export.error = None;
                        self.shell.status = format!(
                            "Support package saved · {} · {} · {}",
                            paths.json.display(),
                            paths.text.display(),
                            paths.html.display()
                        );
                    }
                    Err(error) => {
                        self.shell.status = format!(
                            "Support package could not be written completely · {error} · Try exporting individual files"
                        );
                        self.export.error = Some(error);
                    }
                }
            }
        }
    }
}
