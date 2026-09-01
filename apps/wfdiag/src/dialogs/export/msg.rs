//! The export surface's message alphabet.

#![deny(unsafe_code)]

use crate::platform::save_picker::{SavePickerReply, ValidatedSupportPackagePaths};

/// Which export payload a finished picker or write belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExportPickerKind {
    /// The single-file report export.
    File,
    /// The three-file support package.
    SupportPackage,
}

/// Everything the export surface can report.
#[derive(Clone)]
pub(crate) enum ExportMsg {
    /// The off-UI-thread save picker answered (#140, #196). The epoch rejects
    /// an answer from a request the user has already superseded.
    PickerFinished {
        epoch: u64,
        kind: ExportPickerKind,
        outcome: Box<SavePickerReply>,
    },
    FileSaved {
        epoch: u64,
        result: Box<Result<std::path::PathBuf, String>>,
    },
    SupportPackageSaved {
        epoch: u64,
        result: Box<Result<ValidatedSupportPackagePaths, String>>,
    },
}
