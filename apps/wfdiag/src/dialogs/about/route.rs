//! Routing the About dialog's messages.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::dialogs::about::state::AboutMsg;
use windows_reactor::*;

impl WfdiagShell {
    /// One About-dialog message.
    pub(crate) fn route_about(&mut self, message: AboutMsg, context: &ComponentContext<Self>) {
        match message {
            AboutMsg::Open => self.open_about(),
            AboutMsg::Closed { epoch } => self.close_about(epoch),
            AboutMsg::ExternalRequested { epoch, action } => {
                self.request_about_external_action(epoch, action, context);
            }
            AboutMsg::ExternalFinished { epoch, result } => {
                if self.about_dialog_is_current(epoch) {
                    self.about.launch_task = None;
                    self.about.action_error = result.err();
                }
            }
            AboutMsg::ExternalRejected { epoch } => {
                if self.about_dialog_is_current(epoch) {
                    self.about.launch_task = None;
                    self.about.action_error =
                        Some("The link could not enter the Reactor background queue".to_string());
                }
            }
        }
    }
}
