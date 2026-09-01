//! About dialog and update-check orchestration.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::consts::APP_VERSION;
use crate::app::policy::{about_dialog_callback_is_current, update_notice_remaining_after_elapsed};
use crate::app::tasks::{
    spawn_about_external_action, spawn_update_notice_timer, spawn_update_wait,
};
use std::time::{Duration, Instant};
use wfdiag_native_update::policy::{
    AboutExternalAction, NOTICE_DURATION, UpdateThrottle, trusted_release_url,
};
use wfdiag_native_update::{NativeUpdateRuntime, UpdateInfo, UpdateService};
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

    pub(crate) fn begin_update_check(
        &mut self,
        throttle: Option<UpdateThrottle>,
        context: &ComponentContext<Self>,
    ) {
        self.update_delay_task = None;
        if self.deterministic_visual
            || self.update_runtime.is_some()
            || self.update_check_task.is_some()
        {
            return;
        }

        let Ok(service) = UpdateService::shipping_from_str(APP_VERSION, cfg!(debug_assertions))
        else {
            return;
        };
        let Ok(runtime) = NativeUpdateRuntime::start(service) else {
            return;
        };
        let Ok(reply) = runtime.request_check() else {
            return;
        };

        self.update_check_task = Some(spawn_update_wait(context, reply, throttle));
        self.update_runtime = Some(runtime);
    }

    pub(crate) fn apply_update_check_result(
        &mut self,
        result: Result<Option<UpdateInfo>, String>,
        context: &ComponentContext<Self>,
    ) {
        self.update_check_task = None;
        self.update_runtime = None;

        let Ok(Some(update)) = result else {
            // Store builds, debug builds, offline hosts, malformed responses,
            // and runtime failures all intentionally remain invisible.
            return;
        };
        if trusted_release_url(&update).is_none() {
            return;
        }

        self.update_info = Some(update);
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
