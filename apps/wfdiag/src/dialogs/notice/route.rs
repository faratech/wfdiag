//! Routing the outcome notice's messages.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::update_notice_timer_callback_is_current;
use crate::dialogs::notice::state::NoticeMsg;

impl WfdiagShell {
    /// One notice message.
    pub(crate) fn route_notice(&mut self, message: NoticeMsg) {
        match message {
            NoticeMsg::Closed { epoch } => self.close_notice(epoch),
            NoticeMsg::Expired {
                epoch,
                timer_generation,
            }
            | NoticeMsg::TimerRejected {
                epoch,
                timer_generation,
            } => {
                if update_notice_timer_callback_is_current(
                    self.notice.visible,
                    self.notice.epoch,
                    self.notice.timer_generation,
                    epoch,
                    timer_generation,
                ) {
                    self.notice.task = None;
                    self.notice.visible = false;
                }
            }
            NoticeMsg::TimerCancelled {
                epoch,
                timer_generation,
            } => {
                if update_notice_timer_callback_is_current(
                    self.notice.visible,
                    self.notice.epoch,
                    self.notice.timer_generation,
                    epoch,
                    timer_generation,
                ) {
                    self.notice.task = None;
                }
            }
        }
    }
}
