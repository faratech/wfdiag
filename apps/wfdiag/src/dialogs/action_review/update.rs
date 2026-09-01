//! How the remediation review dialogs answer the user.

#![deny(unsafe_code)]

use crate::app::screen::{Effect, ScreenCx};
use crate::dialogs::action_review::state::{ActionReviewDialog, ActionReviewMsg};
use wfdiag_app::AppCommand;
use windows_reactor::*;

impl ActionReviewDialog {
    pub(crate) fn update(&mut self, message: ActionReviewMsg, cx: &mut ScreenCx<'_>) {
        match message {
            ActionReviewMsg::ReviewClosed {
                proposal_id,
                result,
            } => self.close_review(&proposal_id, result, cx),
            ActionReviewMsg::RepairClosed {
                proposal_id,
                result,
            } => self.close_repair(&proposal_id, result, cx),
        }
    }

    fn close_review(
        &mut self,
        proposal_id: &str,
        result: ContentDialogResult,
        cx: &mut ScreenCx<'_>,
    ) {
        let Some(proposal) = self.review.clone() else {
            return;
        };
        if proposal.proposal_id != proposal_id {
            return;
        }
        let admin_blocked = !cx.shell.is_admin
            && proposal
                .actions
                .iter()
                .any(|action| action.remediation.admin_required);
        if result == ContentDialogResult::Primary && admin_blocked {
            cx.effect(Effect::RestartAsAdmin);
            return;
        }
        if result == ContentDialogResult::Primary {
            // The Repair gate lives in the broker. Passing the review's
            // confirmation here is not the second approval: a Repair-tier
            // preview still comes back as `RepairConfirmationRequired`.
            let confirm_repair = crate::app::policy::action_proposal_contains_repair(&proposal);
            let outcome = cx.dispatch(AppCommand::ApproveAction {
                proposal_id: proposal.proposal_id,
                confirm_repair,
            });
            if outcome.is_accepted() {
                cx.status("Revalidating the reviewed remediation…");
            } else {
                cx.report_rejection(&outcome);
            }
        } else {
            let outcome = cx.dispatch(AppCommand::DiscardProposal {
                proposal_id: proposal.proposal_id,
            });
            cx.report_rejection(&outcome);
        }
    }

    fn close_repair(
        &mut self,
        proposal_id: &str,
        result: ContentDialogResult,
        cx: &mut ScreenCx<'_>,
    ) {
        let Some(proposal) = self.repair_confirm.clone() else {
            return;
        };
        if proposal.proposal_id != proposal_id {
            return;
        }
        let outcome = if result == ContentDialogResult::Primary {
            cx.dispatch(AppCommand::ApproveAction {
                proposal_id: proposal.proposal_id,
                confirm_repair: true,
            })
        } else {
            cx.dispatch(AppCommand::DiscardProposal {
                proposal_id: proposal.proposal_id,
            })
        };
        if outcome.is_accepted() && result == ContentDialogResult::Primary {
            cx.status("Revalidating the reviewed remediation…");
        } else {
            cx.report_rejection(&outcome);
        }
    }
}
