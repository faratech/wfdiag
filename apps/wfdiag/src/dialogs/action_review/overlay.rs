//! The remediation review and repair-confirmation overlays.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::message::Message;
use crate::app::screen::ShellEnv;
use crate::dialogs::action_review::state::{ActionReviewDialog, ActionReviewMsg};
use crate::dialogs::action_review::view::action_review_presentation;
use windows_reactor::*;

impl ActionReviewDialog {
    /// Returns `(review, repair_confirmation)` for the overlay host.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn overlay(
        &self,
        env: &ShellEnv<'_>,
        vc: &mut ViewContext<WfdiagShell>,
    ) -> (View, View) {
        let action_review_dialog = if let Some(proposal) = self.review.as_ref() {
            let presentation = action_review_presentation(proposal, env.is_admin);
            let mut preview = format!(
                "Review the exact, catalog-backed action{}. This approval expires after 10 minutes and can be used only once.\n",
                if presentation.batch { "s" } else { "" }
            );
            for action in &proposal.actions {
                preview.push('\n');
                preview.push_str(&action.remediation.label);
                preview.push_str(" — ");
                preview.push_str(&action.remediation.description);
                for step in &action.steps {
                    preview.push_str("\n  · ");
                    preview.push_str(step);
                }
            }
            if presentation.admin_blocked {
                preview.push_str(
                            "\n\nThis action needs administrator rights. Restart the app as administrator first.",
                        );
            } else if proposal
                .actions
                .iter()
                .any(|action| action.remediation.admin_required)
            {
                preview.push_str("\n\nRuns with administrator rights.");
            }
            if presentation.schedules_restart {
                preview.push_str(
                            "\n\nSave your work first. Windows will restart 60 seconds after approval; run shutdown /a to cancel.",
                        );
            } else if presentation.requires_restart {
                preview.push_str("\n\nA restart is required for this change to take effect.");
            }
            if presentation.long_running {
                preview.push_str(
                    "\n\nThis can take 10–30 minutes. Keep the app open until it finishes.",
                );
                preview.push_str(if presentation.can_stop {
                    " You can stop it safely."
                } else {
                    " It cannot be stopped safely once it starts."
                });
            }
            let proposal_id = proposal.proposal_id.clone();
            let on_closed = vc.callback(move |result| {
                Message::ActionReview(ActionReviewMsg::ReviewClosed {
                    proposal_id: proposal_id.clone(),
                    result,
                })
            });
            ContentDialog::new()
                .title(presentation.title)
                .is_open(true)
                .primary_button_text(presentation.primary_label)
                .secondary_button_text("Cancel")
                .on_closed(on_closed)
                .content(
                    Border::new()
                        .width(438.0)
                        .background(env.palette.card_strong)
                        .padding(Thickness::new(18.0, 14.0, 18.0, 14.0))
                        .content(
                            TextBlock::new()
                                .text(preview)
                                .font_size(12.5)
                                .is_text_selection_enabled(true)
                                .text_wrapping(TextWrapping::Wrap),
                        ),
                )
        } else {
            View::empty()
        };

        let repair_dialog = if let Some(proposal) = self.repair_confirm.as_ref() {
            // This second, explicit gate is reached only after the broker has
            // revalidated the immutable preview and identified a Repair tier.
            let presentation = action_review_presentation(proposal, env.is_admin);
            let mut preview = String::from("Confirm the exact repair steps below:\n");
            for action in &proposal.actions {
                for step in &action.steps {
                    preview.push_str("\n· ");
                    preview.push_str(step);
                }
            }
            if presentation.schedules_restart {
                preview.push_str(
                            "\n\nSave your work first. Windows will restart 60 seconds after approval; run shutdown /a to cancel.",
                        );
            } else if presentation.requires_restart {
                preview.push_str("\n\nA restart is required for this change to take effect.");
            }
            let proposal_id = proposal.proposal_id.clone();
            let on_closed = vc.callback(move |result| {
                Message::ActionReview(ActionReviewMsg::RepairClosed {
                    proposal_id: proposal_id.clone(),
                    result,
                })
            });
            ContentDialog::new()
                .title(presentation.title)
                .is_open(true)
                .primary_button_text(if presentation.schedules_restart {
                    "Schedule restart"
                } else {
                    "Run repair once"
                })
                .secondary_button_text("Cancel")
                .on_closed(on_closed)
                .content(
                    Border::new()
                        .width(412.0)
                        .background(env.palette.card_strong)
                        .padding(Thickness::new(18.0, 14.0, 18.0, 14.0))
                        .content(
                            TextBlock::new()
                                .text(preview)
                                .font_size(12.5)
                                .is_text_selection_enabled(true)
                                .text_wrapping(TextWrapping::Wrap),
                        ),
                )
        } else {
            View::empty()
        };

        (action_review_dialog, repair_dialog)
    }
}
