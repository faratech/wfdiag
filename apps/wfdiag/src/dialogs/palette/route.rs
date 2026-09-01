//! Routing the command palette's messages.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::dialogs::palette::msg::{PaletteFocusAction, PaletteMsg};
use crate::dialogs::palette::view::palette_visible_matches;
use crate::platform::{focus, instance};
use windows_reactor::*;

impl WfdiagShell {
    /// One command-palette message.
    pub(crate) fn route_palette(&mut self, message: PaletteMsg, context: &ComponentContext<Self>) {
        match message {
            PaletteMsg::Toggle => {
                self.set_palette_visibility(!self.palette.open, context);
            }
            PaletteMsg::Close => {
                self.set_palette_visibility(false, context);
            }
            PaletteMsg::FocusReady { epoch, action } => {
                if self.palette.epoch == epoch {
                    self.palette.focus_task = None;
                    match action {
                        PaletteFocusAction::FocusQuery if self.palette.open => {
                            let _ = self.palette.query_reference.request_focus();
                        }
                        PaletteFocusAction::RestorePrevious if !self.palette.open => {
                            // A ContentDialog is a native popup. Reactivate its
                            // owner before restoring the exact XAML element so
                            // the disappearing InputSite cannot retain global
                            // keyboard focus.
                            instance::activate_main_window();
                            if !focus::restore_pre_palette_focus() {
                                let _ = self.palette.button_reference.request_focus();
                            }
                        }
                        _ => {}
                    }
                }
            }
            PaletteMsg::FocusCancelled { epoch } | PaletteMsg::FocusRejected { epoch } => {
                if self.palette.epoch == epoch {
                    self.palette.focus_task = None;
                }
            }
            PaletteMsg::QueryChanged(value) => {
                self.palette.query = value;
                self.palette.active_index = 0;
            }
            PaletteMsg::ActiveChanged(index) => {
                let match_count =
                    palette_visible_matches(self.palette_command_specs(), &self.palette.query)
                        .len();
                self.palette.active_index = index.min(match_count.saturating_sub(1));
            }
            PaletteMsg::Command(tag) => {
                self.set_palette_visibility(false, context);
                self.handle_palette_command(tag, context);
            }
        }
    }
}
