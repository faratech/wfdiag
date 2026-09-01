//! Routing the update notice's messages.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::update_notice_timer_callback_is_current;
use crate::dialogs::update_notice::state::UpdateNoticeMsg;
use std::time::Duration;
use windows_reactor::*;

impl WfdiagShell {
    /// One update-notice message.
    pub(crate) fn route_update_notice(
        &mut self,
        message: UpdateNoticeMsg,
        context: &ComponentContext<Self>,
    ) {
        match message {
            UpdateNoticeMsg::Closed { epoch } => self.close_update_notice(epoch),
            UpdateNoticeMsg::Expired {
                epoch,
                timer_generation,
            }
            | UpdateNoticeMsg::TimerRejected {
                epoch,
                timer_generation,
            } => {
                if update_notice_timer_callback_is_current(
                    self.update_notice.visible,
                    self.update_notice.epoch,
                    self.update_notice.timer_generation,
                    epoch,
                    timer_generation,
                ) {
                    self.update_notice.task = None;
                    self.update_notice.visible = false;
                    self.update_notice.started_at = None;
                    self.update_notice.remaining = Duration::ZERO;
                }
            }
            UpdateNoticeMsg::PointerEntered { epoch } => self.pause_update_notice(epoch),
            UpdateNoticeMsg::PointerExited { epoch } => {
                self.resume_update_notice(epoch, context);
            }
            UpdateNoticeMsg::TimerCancelled {
                epoch,
                timer_generation,
            } => {
                if update_notice_timer_callback_is_current(
                    self.update_notice.visible,
                    self.update_notice.epoch,
                    self.update_notice.timer_generation,
                    epoch,
                    timer_generation,
                ) {
                    self.update_notice.task = None;
                }
            }
        }
    }
}
