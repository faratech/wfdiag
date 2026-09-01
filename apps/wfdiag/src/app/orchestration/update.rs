//! The About dialog and the update notice.
//!
//! The check itself belongs to the engine — [`AppCommand::CheckForUpdates`]
//! and its once-a-day throttle — so what is left here is presentation: the
//! dialog's epoch, the notice's dismiss timer, and its hover pause.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::{about_dialog_callback_is_current, update_notice_remaining_after_elapsed};
use crate::app::tasks::{spawn_about_external_action, spawn_update_notice_timer};
use std::time::{Duration, Instant};
use wfdiag_native_update::policy::{AboutExternalAction, NOTICE_DURATION};
use windows_reactor::*;

impl WfdiagShell {
    pub(crate) fn next_update_notice_timer_generation(&mut self) -> u64 {
        self.update_notice_timer_generation = self.update_notice_timer_generation.wrapping_add(1);
        if self.update_notice_timer_generation == 0 {
            self.update_notice_timer_generation = 1;
        }
        self.update_notice_timer_generation
    }

    pub(crate) fn next_about_dialog_epoch(&mut self) -> u64 {
        self.about_dialog_epoch = self.about_dialog_epoch.wrapping_add(1);
        if self.about_dialog_epoch == 0 {
            self.about_dialog_epoch = 1;
        }
        self.about_dialog_epoch
    }

    pub(crate) fn about_dialog_is_current(&self, epoch: u64) -> bool {
        about_dialog_callback_is_current(self.about_open, self.about_dialog_epoch, epoch)
    }

    pub(crate) fn open_about(&mut self) {
        if self.about_open || self.settings_open || self.settings_saving {
            return;
        }
        self.next_about_dialog_epoch();
        self.about_action_error = None;
        self.about_open = true;
    }

    pub(crate) fn close_about(&mut self, epoch: u64) {
        if !self.about_dialog_is_current(epoch) {
            return;
        }
        if let Some(task) = self.about_launch_task.take() {
            task.cancel();
        }
        self.about_action_error = None;
        self.about_open = false;
    }

    pub(crate) fn request_about_external_action(
        &mut self,
        epoch: u64,
        action: AboutExternalAction,
        context: &ComponentContext<Self>,
    ) {
        if !self.about_dialog_is_current(epoch) || self.about_launch_task.is_some() {
            return;
        }
        self.about_action_error = None;
        self.about_launch_task = Some(spawn_about_external_action(
            context,
            epoch,
            action,
            self.update_info.clone(),
        ));
    }

    /// Raise the notice for a newer release the engine just found.
    pub(crate) fn show_update_notice(&mut self, context: &ComponentContext<Self>) {
        self.update_notice_epoch = self.update_notice_epoch.wrapping_add(1);
        if self.update_notice_epoch == 0 {
            self.update_notice_epoch = 1;
        }
        let epoch = self.update_notice_epoch;
        if let Some(task) = self.update_notice_task.take() {
            task.cancel();
        }
        self.update_notice_visible = true;
        self.update_notice_started_at = Some(Instant::now());
        self.update_notice_remaining = NOTICE_DURATION;
        let timer_generation = self.next_update_notice_timer_generation();
        self.update_notice_task = Some(spawn_update_notice_timer(
            context,
            epoch,
            timer_generation,
            self.update_notice_remaining,
        ));
    }

    pub(crate) fn close_update_notice(&mut self, epoch: u64) {
        if !self.update_notice_visible || self.update_notice_epoch != epoch {
            return;
        }
        if let Some(task) = self.update_notice_task.take() {
            task.cancel();
        }
        self.update_notice_visible = false;
        self.update_notice_started_at = None;
        self.update_notice_remaining = Duration::ZERO;
    }

    pub(crate) fn pause_update_notice(&mut self, epoch: u64) {
        if !self.update_notice_visible || self.update_notice_epoch != epoch {
            return;
        }
        let Some(started_at) = self.update_notice_started_at.take() else {
            return;
        };
        self.update_notice_remaining = update_notice_remaining_after_elapsed(
            self.update_notice_remaining,
            started_at.elapsed(),
        );
        if let Some(task) = self.update_notice_task.take() {
            task.cancel();
        }
        // Invalidate every completion from the cancelled timer immediately;
        // resume may be queued behind that completion on the UI dispatcher.
        self.next_update_notice_timer_generation();
        if self.update_notice_remaining.is_zero() {
            self.update_notice_visible = false;
        }
    }

    pub(crate) fn resume_update_notice(&mut self, epoch: u64, context: &ComponentContext<Self>) {
        if !self.update_notice_visible
            || self.update_notice_epoch != epoch
            || self.update_notice_started_at.is_some()
        {
            return;
        }
        if self.update_notice_remaining.is_zero() {
            self.update_notice_visible = false;
            return;
        }
        self.update_notice_started_at = Some(Instant::now());
        let timer_generation = self.next_update_notice_timer_generation();
        self.update_notice_task = Some(spawn_update_notice_timer(
            context,
            epoch,
            timer_generation,
            self.update_notice_remaining,
        ));
    }
}
