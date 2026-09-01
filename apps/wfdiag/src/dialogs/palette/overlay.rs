//! The command palette overlay.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::message::Message;
use crate::app::screen::ShellEnv;
use crate::dialogs::palette::msg::PaletteMsg;
use crate::dialogs::palette::state::PaletteDialog;
use crate::dialogs::palette::view::{
    PaletteCommandSpec, command_palette_footer, command_palette_highlighted_label,
    command_palette_key_chip, palette_visible_matches,
};
use crate::widgets::icons;
use crate::widgets::icons::FaIcon;
use windows_reactor::*;

impl PaletteDialog {
    /// The palette overlay: its rows, its query box, and its footer.
    ///
    /// Kept in a permanently mounted `ContentDialog`. Reactor's dialog
    /// lifecycle is designed for live open/close transitions; a conditional
    /// `Grid` subtree can fail a native `Children` insertion. Expensive result
    /// rows are still created only while it is open.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn view(
        &self,
        env: &ShellEnv<'_>,
        specs: Vec<PaletteCommandSpec>,
        vc: &mut ViewContext<WfdiagShell>,
    ) -> View {
        let palette_rows = if self.open {
            let palette_matches = palette_visible_matches(specs.clone(), &self.query);
            let palette_match_count = palette_matches.len();
            let active_palette_index = self.active_index.min(palette_match_count.saturating_sub(1));
            let mut palette_rows = Vec::new();
            let mut previous_section = None;
            for (index, matched) in palette_matches.into_iter().enumerate() {
                let match_indices = matched.indices;
                let command = matched.command;
                if previous_section != Some(command.section) {
                    previous_section = Some(command.section);
                    palette_rows.push(KeyedView::new(
                        format!("palette-section:{}", command.section),
                        TextBlock::new()
                            .text(command.section.to_ascii_uppercase())
                            .margin(Thickness::new(12.0, 10.0, 12.0, 4.0))
                            .font_size(10.0)
                            .font_weight(FontWeight::BOLD)
                            .foreground(env.palette.muted)
                            .automation_heading_level(AutomationHeadingLevel::Level3),
                    ));
                }
                let tag = command.tag.into_owned();
                let execute = vc.message(Message::Palette(PaletteMsg::Command(tag.clone())));
                let activate =
                    vc.callback(move |_| Message::Palette(PaletteMsg::ActiveChanged(index)));
                let active = index == active_palette_index;
                let icon = command.icon;
                let label = command.label.into_owned();
                let automation_label = label.clone();
                let automation_id = format!("palette-item-{}", tag.replace(':', "-"));
                let enabled = command.enabled;
                let label_view =
                    command_palette_highlighted_label(env.palette, label, &match_indices, enabled);
                let shortcut: View = command.shortcut.map_or_else(View::empty, |shortcut| {
                    command_palette_key_chip(env.palette, shortcut.into_owned())
                });
                palette_rows.push(KeyedView::new(
                    tag,
                    Border::new()
                        .background(if active {
                            env.palette.active
                        } else {
                            Color::transparent()
                        })
                        .corner_radius(6.0)
                        .on_pointer_entered(activate)
                        .content(
                            Button::new()
                                .height(36.0)
                                .style(ButtonStyle::Subtle)
                                .horizontal_alignment(HorizontalAlignment::Stretch)
                                .horizontal_content_alignment(HorizontalAlignment::Stretch)
                                .vertical_content_alignment(VerticalAlignment::Center)
                                // Popup realization in WinUI 3 cannot repackage
                                // boxed Thickness/CornerRadius values from a local
                                // resource dictionary. Keep only brush overrides;
                                // the Subtle style supplies native geometry.
                                .resource_overrides(
                                    ResourceOverrides::new()
                                        .set("ButtonBackground", Color::transparent())
                                        .set("ButtonBackgroundPointerOver", env.palette.active)
                                        .set("ButtonBackgroundPressed", env.palette.active)
                                        .set(
                                            "ButtonForeground",
                                            if active {
                                                env.palette.accent
                                            } else {
                                                env.palette.muted
                                            },
                                        )
                                        .set("ButtonForegroundDisabled", env.palette.muted),
                                )
                                .is_enabled(enabled)
                                .automation_name(automation_label)
                                .automation_id(automation_id)
                                .element_ref(&self.result_references[index])
                                .on_click(execute)
                                .content(
                                    Grid::new()
                                        .columns([
                                            GridLength::Pixel(22.0),
                                            GridLength::Star(1.0),
                                            GridLength::Pixel(88.0),
                                        ])
                                        .column_spacing(8.0)
                                        .children((
                                            icons::path(icon)
                                                .width(14.0)
                                                .height(14.0)
                                                .vertical_alignment(VerticalAlignment::Center),
                                            label_view,
                                            Border::new()
                                                .grid_column(2)
                                                .horizontal_alignment(HorizontalAlignment::Right)
                                                .vertical_alignment(VerticalAlignment::Center)
                                                .content(shortcut),
                                        )),
                                ),
                        ),
                ));
            }
            if palette_rows.is_empty() {
                palette_rows.push(KeyedView::new(
                    "palette-empty",
                    TextBlock::new()
                        .text("No matching commands")
                        .margin(Thickness::new(16.0, 30.0, 16.0, 30.0))
                        .font_size(13.0)
                        .foreground(env.palette.muted)
                        .horizontal_alignment(HorizontalAlignment::Center),
                ));
            }
            palette_rows
        } else {
            Vec::new()
        };
        let palette_open = self.open;
        let palette_epoch = self.epoch;
        let palette_query_reference = self.query_reference.clone();
        vc.use_effect(
            "command-palette-focus",
            (palette_open, palette_epoch),
            move || {
                if palette_open {
                    let _ = palette_query_reference.request_focus();
                }
                None
            },
        );
        // Keep the palette in a permanently mounted ContentDialog. Reactor's
        // dialog lifecycle is designed for live open/close transitions; a
        // conditional Grid subtree can fail a native Children insertion.
        // Expensive result rows are still created only while it is open.
        let query_changed = vc.callback(|value| Message::Palette(PaletteMsg::QueryChanged(value)));
        let palette_width = (env.window_size.width - 48.0).clamp(360.0, 560.0);
        let palette_list_height = (env.window_size.height * 0.60 - 94.0).clamp(220.0, 430.0);
        let close_palette = vc.message(Message::Palette(PaletteMsg::Close));
        ContentDialog::new()
            .is_open(self.open)
            .on_closed(move |_| {
                let _ = close_palette.call(());
            })
            .content(
                Border::new().width(palette_width).content(
                    Grid::new()
                        .automation_name("Command palette")
                        .rows([
                            GridLength::Pixel(54.0),
                            GridLength::Auto,
                            GridLength::Pixel(39.0),
                        ])
                        .children((
                            Border::new()
                                .padding(Thickness::new(16.0, 7.0, 13.0, 7.0))
                                .border_brush(env.palette.border)
                                .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                                .content(
                                    Grid::new()
                                        .columns([
                                            GridLength::Pixel(19.0),
                                            GridLength::Star(1.0),
                                            GridLength::Pixel(42.0),
                                        ])
                                        .column_spacing(9.0)
                                        .children((
                                            icons::path(FaIcon::MagnifyingGlass)
                                                .width(14.0)
                                                .height(14.0)
                                                .vertical_alignment(VerticalAlignment::Center),
                                            TextBox::new()
                                                .grid_column(1)
                                                .height(40.0)
                                                .text(self.query.clone())
                                                .automation_name("Search commands")
                                                .placeholder_text(
                                                    "Search commands, screens and diagnostics…",
                                                )
                                                .background(Color::transparent())
                                                .border_thickness(Thickness::uniform(0.0))
                                                .on_text_changed(query_changed)
                                                .element_ref(&self.query_reference),
                                            Border::new()
                                                .grid_column(2)
                                                .horizontal_alignment(HorizontalAlignment::Right)
                                                .vertical_alignment(VerticalAlignment::Center)
                                                .content(command_palette_key_chip(
                                                    env.palette,
                                                    "Esc",
                                                )),
                                        )),
                                ),
                            ScrollViewer::new()
                                .grid_row(1)
                                .max_height(palette_list_height)
                                .margin(Thickness::uniform(6.0))
                                .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
                                .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                                .content(
                                    StackPanel::new().spacing(2.0).keyed_children(palette_rows),
                                ),
                            Border::new()
                                .grid_row(2)
                                .padding(Thickness::xy(14.0, 0.0))
                                .border_brush(env.palette.border)
                                .border_thickness(Thickness::new(0.0, 1.0, 0.0, 0.0))
                                .content(command_palette_footer(
                                    env.palette,
                                    palette_width >= 450.0,
                                )),
                        )),
                ),
            )
    }
}
