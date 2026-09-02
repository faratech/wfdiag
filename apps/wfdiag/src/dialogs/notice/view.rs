//! The outcome notice's info bar.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::message::Message;
use crate::dialogs::notice::state::{NoticeDialog, NoticeKind, NoticeMsg};
use windows_reactor::*;

/// Bottom margin when the notice has the bottom edge to itself.
const NOTICE_BOTTOM_MARGIN: f64 = 28.0;

/// Extra clearance when the update-available bar is also showing, so the
/// two never overlap.
const UPDATE_NOTICE_CLEARANCE: f64 = 84.0;

impl NoticeDialog {
    /// The transient info bar, or nothing when it is not showing.
    pub(crate) fn view(
        &self,
        update_notice_visible: bool,
        vc: &mut ViewContext<WfdiagShell>,
    ) -> View {
        if !self.visible {
            return View::empty();
        }
        let epoch = self.epoch;
        let severity = match self.kind {
            NoticeKind::Success => InfoBarSeverity::Success,
            NoticeKind::Info => InfoBarSeverity::Informational,
            NoticeKind::Warning => InfoBarSeverity::Warning,
            NoticeKind::Error => InfoBarSeverity::Error,
        };
        let bottom = if update_notice_visible {
            NOTICE_BOTTOM_MARGIN + UPDATE_NOTICE_CLEARANCE
        } else {
            NOTICE_BOTTOM_MARGIN
        };
        Border::new()
            .grid_row_span(2)
            .width(430.0)
            .margin(Thickness::new(0.0, 0.0, 0.0, bottom))
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Bottom)
            .automation_name(self.title.clone())
            .content(
                InfoBar::new()
                    .title(self.title.clone())
                    .message(self.message.clone())
                    .severity(severity)
                    .is_open(true)
                    .is_closable(true)
                    .on_closed(vc.message(Message::Notice(NoticeMsg::Closed { epoch }))),
            )
    }
}
