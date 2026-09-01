//! Opening, closing, and launching links from the About dialog.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::policy::about_dialog_callback_is_current;
use crate::app::tasks::spawn_about_external_action;
use wfdiag_native_update::policy::AboutExternalAction;
use windows_reactor::*;

impl WfdiagShell {
    pub(crate) fn next_about_dialog_epoch(&mut self) -> u64 {
        self.about.epoch = self.about.epoch.wrapping_add(1);
        if self.about.epoch == 0 {
            self.about.epoch = 1;
        }
        self.about.epoch
    }

    pub(crate) fn about_dialog_is_current(&self, epoch: u64) -> bool {
        about_dialog_callback_is_current(self.about.open, self.about.epoch, epoch)
    }

    pub(crate) fn open_about(&mut self) {
        if self.about.open || self.settings.open || self.settings.saving {
            return;
        }
        self.next_about_dialog_epoch();
        self.about.action_error = None;
        self.about.open = true;
    }

    pub(crate) fn close_about(&mut self, epoch: u64) {
        if !self.about_dialog_is_current(epoch) {
            return;
        }
        if let Some(task) = self.about.launch_task.take() {
            task.cancel();
        }
        self.about.action_error = None;
        self.about.open = false;
    }

    pub(crate) fn request_about_external_action(
        &mut self,
        epoch: u64,
        action: AboutExternalAction,
        context: &ComponentContext<Self>,
    ) {
        if !self.about_dialog_is_current(epoch) || self.about.launch_task.is_some() {
            return;
        }
        self.about.action_error = None;
        self.about.launch_task = Some(spawn_about_external_action(
            context,
            epoch,
            action,
            self.update_notice.info.clone(),
        ));
    }
}
