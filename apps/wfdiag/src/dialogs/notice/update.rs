//! Raising, arming and closing the outcome notice.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::tasks::spawn_notice_timer;
use crate::dialogs::notice::state::NoticeRequest;
use windows_reactor::*;

impl WfdiagShell {
    fn next_notice_timer_generation(&mut self) -> u64 {
        self.notice.timer_generation = self.notice.timer_generation.wrapping_add(1);
        if self.notice.timer_generation == 0 {
            self.notice.timer_generation = 1;
        }
        self.notice.timer_generation
    }

    /// Raise a notice, replacing whichever one is showing. The timer is armed
    /// by [`Self::arm_pending_notice`] at the end of the current update, so
    /// callers deep inside event handling need no Reactor context.
    pub(crate) fn show_notice(&mut self, request: NoticeRequest) {
        self.notice.epoch = self.notice.epoch.wrapping_add(1);
        if self.notice.epoch == 0 {
            self.notice.epoch = 1;
        }
        if let Some(task) = self.notice.task.take() {
            task.cancel();
        }
        // Invalidate the cancelled timer's completion before it can land.
        self.next_notice_timer_generation();
        self.notice.visible = true;
        self.notice.kind = request.kind;
        self.notice.title = request.title;
        self.notice.message = request.message;
        self.notice.arm_pending = true;
    }

    /// Arm the dismissal timer for a notice raised during this update.
    pub(crate) fn arm_pending_notice(&mut self, context: &ComponentContext<Self>) {
        if !self.notice.arm_pending {
            return;
        }
        self.notice.arm_pending = false;
        if !self.notice.visible {
            return;
        }
        let epoch = self.notice.epoch;
        let timer_generation = self.next_notice_timer_generation();
        self.notice.task = Some(spawn_notice_timer(
            context,
            epoch,
            timer_generation,
            self.notice.kind.duration(),
        ));
    }

    pub(crate) fn close_notice(&mut self, epoch: u64) {
        if !self.notice.visible || self.notice.epoch != epoch {
            return;
        }
        if let Some(task) = self.notice.task.take() {
            task.cancel();
        }
        self.notice.visible = false;
        self.notice.arm_pending = false;
    }
}
