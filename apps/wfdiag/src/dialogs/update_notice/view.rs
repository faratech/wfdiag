//! The update-available notice.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::message::Message;
use crate::dialogs::update_notice::state::{UpdateNoticeDialog, UpdateNoticeMsg};
use windows_reactor::*;

impl UpdateNoticeDialog {
    /// The transient info bar, or nothing when it is not showing.
    pub(crate) fn view(&self, vc: &mut ViewContext<WfdiagShell>) -> View {
        if self.visible {
            self.info.as_ref().map_or_else(View::empty, |update| {
                let epoch = self.epoch;
                Border::new()
                    .grid_row_span(2)
                    .width(430.0)
                    .margin(Thickness::new(0.0, 0.0, 0.0, 28.0))
                    .horizontal_alignment(HorizontalAlignment::Center)
                    .vertical_alignment(VerticalAlignment::Bottom)
                    .on_pointer_entered(vc.callback(move |_| {
                        Message::UpdateNotice(UpdateNoticeMsg::PointerEntered { epoch })
                    }))
                    .on_pointer_exited(vc.callback(move |_| {
                        Message::UpdateNotice(UpdateNoticeMsg::PointerExited { epoch })
                    }))
                    .content(
                        InfoBar::new()
                            .title(format!("Update available: v{}", update.version))
                            .message("Open About to download the new version")
                            .severity(InfoBarSeverity::Informational)
                            .is_open(true)
                            .is_closable(true)
                            .on_closed(
                                vc.message(Message::UpdateNotice(UpdateNoticeMsg::Closed {
                                    epoch,
                                })),
                            ),
                    )
            })
        } else {
            View::empty()
        }
    }
}
