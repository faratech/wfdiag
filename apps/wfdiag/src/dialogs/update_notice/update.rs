//! The update notice: showing it, pausing it on hover, and its timer.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::update_notice_remaining_after_elapsed;
use crate::app::tasks::spawn_update_notice_timer;
use std::time::{Duration, Instant};
use wfdiag_native_update::policy::NOTICE_DURATION;
use windows_reactor::*;

impl WfdiagShell {
    /// Raise the notice for a newer release the engine just found.
    pub(crate) fn next_update_notice_timer_generation(&mut self) -> u64 {
        self.update_notice.timer_generation = self.update_notice.timer_generation.wrapping_add(1);
        if self.update_notice.timer_generation == 0 {
            self.update_notice.timer_generation = 1;
        }
        self.update_notice.timer_generation
    }

    pub(crate) fn show_update_notice(&mut self, context: &ComponentContext<Self>) {
        self.update_notice.epoch = self.update_notice.epoch.wrapping_add(1);
        if self.update_notice.epoch == 0 {
            self.update_notice.epoch = 1;
        }
        let epoch = self.update_notice.epoch;
        if let Some(task) = self.update_notice.task.take() {
            task.cancel();
        }
        self.update_notice.visible = true;
        self.update_notice.started_at = Some(Instant::now());
        self.update_notice.remaining = NOTICE_DURATION;
        let timer_generation = self.next_update_notice_timer_generation();
        self.update_notice.task = Some(spawn_update_notice_timer(
            context,
            epoch,
            timer_generation,
            self.update_notice.remaining,
        ));
    }

    pub(crate) fn close_update_notice(&mut self, epoch: u64) {
        if !self.update_notice.visible || self.update_notice.epoch != epoch {
            return;
        }
        if let Some(task) = self.update_notice.task.take() {
            task.cancel();
        }
        self.update_notice.visible = false;
        self.update_notice.started_at = None;
        self.update_notice.remaining = Duration::ZERO;
    }

    pub(crate) fn pause_update_notice(&mut self, epoch: u64) {
        if !self.update_notice.visible || self.update_notice.epoch != epoch {
            return;
        }
        let Some(started_at) = self.update_notice.started_at.take() else {
            return;
        };
        self.update_notice.remaining = update_notice_remaining_after_elapsed(
            self.update_notice.remaining,
            started_at.elapsed(),
        );
        if let Some(task) = self.update_notice.task.take() {
            task.cancel();
        }
        // Invalidate every completion from the cancelled timer immediately;
        // resume may be queued behind that completion on the UI dispatcher.
        self.next_update_notice_timer_generation();
        if self.update_notice.remaining.is_zero() {
            self.update_notice.visible = false;
        }
    }

    pub(crate) fn resume_update_notice(&mut self, epoch: u64, context: &ComponentContext<Self>) {
        if !self.update_notice.visible
            || self.update_notice.epoch != epoch
            || self.update_notice.started_at.is_some()
        {
            return;
        }
        if self.update_notice.remaining.is_zero() {
            self.update_notice.visible = false;
            return;
        }
        self.update_notice.started_at = Some(Instant::now());
        let timer_generation = self.next_update_notice_timer_generation();
        self.update_notice.task = Some(spawn_update_notice_timer(
            context,
            epoch,
            timer_generation,
            self.update_notice.remaining,
        ));
    }
}
