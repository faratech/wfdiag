//! The keyboard-shortcut help overlay.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::message::Message;
use crate::app::screen::ShellEnv;
use crate::dialogs::shortcuts_help::state::{SHORTCUT_ROWS, ShortcutHelpDialog, ShortcutHelpMsg};
use windows_reactor::*;

impl ShortcutHelpDialog {
    /// The shortcut list.
    ///
    /// The dialog node stays mounted and toggles its native open property:
    /// inserting a late `ContentDialog` after several empty overlay siblings
    /// can desynchronize the current windows-reactor child index, and a stable
    /// node also preserves clean row geometry across repeated opens.
    pub(crate) fn view(&self, env: &ShellEnv<'_>, vc: &mut ViewContext<WfdiagShell>) -> View {
        let close_shortcuts = vc.message(Message::Shortcuts(ShortcutHelpMsg::Close));
        ContentDialog::new()
            .title("Keyboard Shortcuts")
            .is_open(self.open)
            .close_button_text("Close")
            .on_closed(move |_| {
                let _ = close_shortcuts.call(());
            })
            .content(
                Border::new()
                    .width(400.0)
                    .background(env.palette.card_strong)
                    .padding(Thickness::new(14.0, 10.0, 14.0, 10.0))
                    .content(
                        StackPanel::new().spacing(6.0).keyed_children(
                            SHORTCUT_ROWS
                                .iter()
                                .map(|(keys, description)| {
                                    KeyedView::new(
                                        *keys,
                                        Grid::new()
                                            .min_height(28.0)
                                            .columns([GridLength::Star(1.0), GridLength::Auto])
                                            .column_spacing(16.0)
                                            .children((
                                                TextBlock::new()
                                                    .text((*description).to_string())
                                                    .font_size(13.0)
                                                    .text_wrapping(TextWrapping::Wrap)
                                                    .vertical_alignment(VerticalAlignment::Center),
                                                TextBlock::new()
                                                    .grid_column(1)
                                                    .text((*keys).to_string())
                                                    .font_size(12.0)
                                                    .font_weight(FontWeight::SEMI_BOLD)
                                                    .foreground(env.palette.muted)
                                                    .vertical_alignment(VerticalAlignment::Center),
                                            )),
                                    )
                                })
                                .collect::<Vec<_>>(),
                        ),
                    ),
            )
    }
}
