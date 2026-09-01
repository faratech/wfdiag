//! The About overlay: its scrim and the dialog itself.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::message::Message;
use crate::app::screen::ShellEnv;
use crate::dialogs::about::state::{AboutDialog, AboutMsg};
use crate::dialogs::about::view::about_dialog;
use wfdiag_native_update::UpdateInfo;
use wfdiag_native_update::policy::AboutExternalAction;
use windows_reactor::*;

impl AboutDialog {
    /// Returns `(scrim, dialog)`: the shell hosts both in the root grid.
    pub(crate) fn overlay(
        &self,
        env: &ShellEnv<'_>,
        update_info: Option<&UpdateInfo>,
        vc: &mut ViewContext<WfdiagShell>,
    ) -> (View, View) {
        let about_epoch = self.epoch;
        let about_is_open = self.open;
        let about_close_reference = self.close_reference.clone();
        vc.use_effect(
            "about-header-close-focus",
            (about_is_open, about_epoch),
            move || {
                if about_is_open {
                    let _ = about_close_reference.request_focus();
                }
                None
            },
        );
        let about_scrim: View = if self.open {
            // ContentDialog supplies the modal surface and its own light-dismiss
            // layer. The Store 2.5.8 React modal is about 21% darker at matched
            // wallpaper samples, so add the measured residual opacity behind
            // the native popup. Reactor has no public per-element backdrop-blur
            // projection at the pinned revision.
            Border::new()
                .grid_row_span(2)
                .background(Color::argb(52, 0, 0, 0))
                .into()
        } else {
            View::empty()
        };
        let dialog = about_dialog(
            env.palette,
            self.open,
            &self.close_reference,
            update_info,
            self.action_error.as_deref(),
            self.launch_task.is_none(),
            vc.callback(move |_| Message::About(AboutMsg::Closed { epoch: about_epoch })),
            vc.message(Message::About(AboutMsg::ExternalRequested {
                epoch: about_epoch,
                action: AboutExternalAction::DownloadUpdate,
            })),
            vc.message(Message::About(AboutMsg::ExternalRequested {
                epoch: about_epoch,
                action: AboutExternalAction::WindowsForum,
            })),
            vc.message(Message::About(AboutMsg::ExternalRequested {
                epoch: about_epoch,
                action: AboutExternalAction::GithubRepository,
            })),
            vc.message(Message::About(AboutMsg::Closed { epoch: about_epoch })),
        );

        (about_scrim, dialog)
    }
}
