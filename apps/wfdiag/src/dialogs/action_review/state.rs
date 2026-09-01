//! The remediation review dialogs' own state and message alphabet.
//!
//! Both proposals are projected from [`wfdiag_app::AppSnapshot`]; the dialog
//! only decides what the two confirm surfaces show and what the user's answer
//! means.

#![deny(unsafe_code)]

use wfdiag_native_remediation::broker::ActionProposal;
use windows_reactor::*;

/// The two staged-remediation confirm surfaces.
#[derive(Default)]
pub(crate) struct ActionReviewDialog {
    /// The staged preview awaiting the user's review.
    pub(crate) review: Option<ActionProposal>,
    /// The second, explicit Repair-tier gate, reached only after the broker
    /// revalidated the immutable preview.
    pub(crate) repair_confirm: Option<ActionProposal>,
}

impl ActionReviewDialog {
    /// Whether either confirm surface is up.
    pub(crate) const fn open(&self) -> bool {
        self.review.is_some() || self.repair_confirm.is_some()
    }
}

/// Everything the review dialogs can report.
#[derive(Clone)]
pub(crate) enum ActionReviewMsg {
    ReviewClosed {
        proposal_id: String,
        result: ContentDialogResult,
    },
    RepairClosed {
        proposal_id: String,
        result: ContentDialogResult,
    },
}
