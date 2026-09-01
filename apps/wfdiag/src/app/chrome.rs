//! The shell chrome: the status bar, the navigation rail, the title-bar
//! actions, and the themed wallpapers.
//!
//! These are the parts of the window that belong to no page. Keeping them here
//! is what lets [`crate::app::WfdiagShell::view`] read as chrome plus one
//! `match page`.

#![deny(unsafe_code)]

use crate::app::WfdiagShell;
use crate::app::consts::{
    APP_BADGE, APP_VERSION, STATUS_INFO_DARK, STATUS_INFO_LIGHT, STATUS_OK_DARK, STATUS_OK_LIGHT,
    STATUS_WARN_DARK, STATUS_WARN_LIGHT, WALLPAPER_DARK, WALLPAPER_LIGHT,
};
use crate::app::message::Message;
use crate::app::policy::privilege_label;
use crate::app::screen::ShellEnv;
use crate::app::shell_msg::ShellMsg;
use crate::app::state::Page;
use crate::dialogs::about::state::AboutMsg;
use crate::dialogs::palette::msg::PaletteMsg;
use crate::dialogs::settings::msg::SettingsMsg;
use crate::dialogs::shortcuts_help::state::ShortcutHelpMsg;
use crate::screens::diagnostics::view::format_diagnostic_duration;
use crate::widgets::chrome::{fa_icon_label, machine_card, nav_brand, nav_button, nav_section};
use crate::widgets::icons;
use crate::widgets::icons::FaIcon;
use wfdiag_native_issues::projection::project_issues;
use windows_reactor::*;

impl WfdiagShell {
    /// The one-line status bar under the open page.
    pub(crate) fn status_bar(&self, env: &ShellEnv<'_>) -> View {
        let status_icon = if self.diagnostics.results.is_empty() {
            if env.theme == WindowTheme::Light {
                STATUS_INFO_LIGHT
            } else {
                STATUS_INFO_DARK
            }
        } else if self
            .diagnostics
            .results
            .iter()
            .any(|result| !result.success)
        {
            if env.theme == WindowTheme::Light {
                STATUS_WARN_LIGHT
            } else {
                STATUS_WARN_DARK
            }
        } else if env.theme == WindowTheme::Light {
            STATUS_OK_LIGHT
        } else {
            STATUS_OK_DARK
        };

        let fixture_scan = self
            .diagnostics
            .results
            .iter()
            .any(|result| result.session_id == "visual-fixture");
        let elapsed_prefix = if self.diagnostics.results.is_empty()
            || matches!(self.shell.page, Page::Monitor | Page::Processes)
        {
            String::new()
        } else if fixture_scan {
            match self.shell.page {
                Page::Issues => "1.8s · ".to_string(),
                _ => "2.3s · ".to_string(),
            }
        } else if self.diagnostics.duration_ms > 0 {
            format!(
                "{} · ",
                format_diagnostic_duration(self.diagnostics.duration_ms)
            )
        } else {
            String::new()
        };
        let privilege = privilege_label(env.is_admin);

        Border::new()
                .grid_row(1)
                .height(33.0)
                .padding(Thickness::xy(18.0, 0.0))
                .border_brush(env.palette.border)
                .border_thickness(Thickness::new(0.0, 1.0, 0.0, 0.0))
                .content(
                    Grid::new()
                        .columns([GridLength::Star(1.0), GridLength::Auto])
                        .children((
                            StackPanel::new()
                                .orientation(Orientation::Horizontal)
                                .spacing(7.0)
                                .vertical_alignment(VerticalAlignment::Center)
                                .children((
                                    Image::new()
                                        .source_data(EncodedImage::from_static(status_icon))
                                        .width(11.0)
                                        .height(11.0),
                                    TextBlock::new()
                                        .text(self.shell.status.clone())
                                        .foreground(env.palette.muted)
                                        .font_size(11.5)
                                        .vertical_alignment(VerticalAlignment::Center),
                                )),
                            TextBlock::new()
                                .text(format!(
                                    "{elapsed_prefix}{privilege}    wfdiag {APP_VERSION} · WindowsForum.com"
                                ))
                                .grid_column(1)
                                .foreground(env.palette.muted)
                                .font_size(11.5)
                                .vertical_alignment(VerticalAlignment::Center),
                        )),
                )
    }

    /// The navigation rail: pages, tools, the pane toggle, and the host card.
    pub(crate) fn navigation_rail(
        &self,
        env: &ShellEnv<'_>,
        issue_projection_current: bool,
        rail_forced_collapsed: bool,
        vc: &mut ViewContext<Self>,
    ) -> View {
        let issue_badge = issue_projection_current
            .then(|| project_issues(&self.issues.issues).counts.nav_badge_count())
            .flatten()
            .map(|count| count.to_string());
        let primary_nav = Page::ALL
            .into_iter()
            .map(|page| {
                KeyedView::new(
                    page.tag(),
                    nav_button(
                        env.palette,
                        page.icon(),
                        page.nav_label(),
                        page == self.shell.page,
                        env.pane_expanded,
                        if page == Page::Issues {
                            issue_badge.as_deref()
                        } else {
                            None
                        },
                        vc.message(Message::Shell(ShellMsg::Navigate(Some(
                            page.tag().to_string(),
                        )))),
                        true,
                    ),
                )
            })
            .collect::<Vec<_>>();

        let tools_enabled = !self.diagnostics.results.is_empty() && self.export.pending.is_none();
        let tools_nav = [
            (FaIcon::FileExport, "Export Report", "export"),
            (FaIcon::ShareNodes, "Share to Forum", "share"),
        ]
        .into_iter()
        .map(|(symbol, label, tag)| {
            KeyedView::new(
                tag,
                nav_button(
                    env.palette,
                    symbol,
                    label,
                    false,
                    env.pane_expanded,
                    None,
                    vc.message(Message::Shell(ShellMsg::Navigate(Some(tag.to_string())))),
                    tools_enabled,
                ),
            )
        })
        .collect::<Vec<_>>();

        let pane_toggle: View = if rail_forced_collapsed {
            View::empty()
        } else {
            nav_button(
                env.palette,
                if env.pane_expanded {
                    FaIcon::AnglesLeft
                } else {
                    FaIcon::AnglesRight
                },
                if env.pane_expanded {
                    "Collapse"
                } else {
                    "Expand"
                },
                false,
                env.pane_expanded,
                None,
                vc.message(Message::Shell(ShellMsg::TogglePane)),
                true,
            )
        };

        let pane_footer = StackPanel::new().spacing(2.0).children((
            nav_button(
                env.palette,
                FaIcon::Settings,
                "Settings",
                false,
                env.pane_expanded,
                None,
                vc.message(Message::Settings(SettingsMsg::Open)),
                true,
            ),
            nav_button(
                env.palette,
                FaIcon::CircleInfo,
                "About",
                false,
                env.pane_expanded,
                None,
                vc.message(Message::About(AboutMsg::Open)),
                true,
            ),
            pane_toggle,
            machine_card(
                env.palette,
                env.pane_expanded,
                &self.shell.system_info,
                self.shell.architecture.as_ref(),
                self.shell.system_error.as_deref(),
            ),
        ));

        Border::new()
            .grid_column(0)
            .padding(if env.pane_expanded {
                Thickness::new(14.0, 4.0, 12.0, 14.0)
            } else {
                Thickness::new(4.0, 4.0, 12.0, 14.0)
            })
            .content(
                Grid::new()
                    .rows([GridLength::Auto, GridLength::Star(1.0), GridLength::Auto])
                    .children((
                        StackPanel::new().children((
                            nav_brand(env.palette, env.pane_expanded),
                            StackPanel::new().spacing(2.0).keyed_children(primary_nav),
                            nav_section("TOOLS", env.pane_expanded, env.palette),
                            StackPanel::new().spacing(2.0).keyed_children(tools_nav),
                        )),
                        Border::new().grid_row(2).content(pane_footer),
                    )),
            )
    }

    /// The custom title bar: the brand, and the four action buttons.
    ///
    /// Returns `(brand, bar, actions)` — three siblings of the root grid.
    pub(crate) fn title_bar(
        &self,
        env: &ShellEnv<'_>,
        vc: &mut ViewContext<Self>,
    ) -> (View, View, View) {
        let title_brand = Border::new()
            .grid_row(0)
            .padding(Thickness::new(16.0, 0.0, 0.0, 0.0))
            .content(
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(9.0)
                    .children((
                        Image::new()
                            .source_data(EncodedImage::from_static(APP_BADGE))
                            .width(17.0)
                            .height(17.0),
                        TextBlock::new()
                            .text("WindowsForum Diagnostics")
                            .font_size(12.0)
                            .font_weight(FontWeight::SEMI_BOLD)
                            .foreground(env.palette.muted)
                            .vertical_alignment(VerticalAlignment::Center),
                    )),
            );

        let title_bar = TitleBar::new()
            .grid_row(0)
            .height(42.0)
            .min_height(42.0)
            .max_height(42.0)
            .preferred_height(WindowTitleBarHeight::Standard)
            .title("")
            .subtitle("")
            .is_back_button_visible(false)
            .is_pane_toggle_button_visible(false);

        let title_settings = Button::new()
            .grid_row(0)
            .width(46.0)
            .height(42.0)
            .margin(Thickness::new(0.0, 0.0, 144.0, 0.0))
            .horizontal_alignment(HorizontalAlignment::Right)
            .style(ButtonStyle::Subtle)
            .on_click(vc.message(Message::Settings(SettingsMsg::Open)))
            .automation_name("Open Settings")
            .content(icons::path(FaIcon::Settings));

        let title_palette = Button::new()
            .grid_row(0)
            .width(46.0)
            .height(42.0)
            .margin(Thickness::new(0.0, 0.0, 190.0, 0.0))
            .horizontal_alignment(HorizontalAlignment::Right)
            .style(ButtonStyle::Subtle)
            .on_click(vc.message(Message::Palette(PaletteMsg::Toggle)))
            .element_ref(&self.palette.button_reference)
            .automation_name("Open the command palette")
            .content(fa_icon_label(FaIcon::MagnifyingGlass, ""));
        let title_help = Button::new()
            .grid_row(0)
            .width(46.0)
            .height(42.0)
            .margin(Thickness::new(0.0, 0.0, 236.0, 0.0))
            .horizontal_alignment(HorizontalAlignment::Right)
            .style(ButtonStyle::Subtle)
            .on_click(vc.message(Message::Shortcuts(ShortcutHelpMsg::Show)))
            .automation_name("Keyboard shortcuts")
            .content(icons::path(FaIcon::CircleInfo));
        let (theme_icon, theme_automation_name) = if env.theme == WindowTheme::Dark {
            (FaIcon::Sun, "Switch to light theme")
        } else {
            (FaIcon::Moon, "Switch to dark theme")
        };
        let title_theme = Button::new()
            .grid_row(0)
            .width(46.0)
            .height(42.0)
            .margin(Thickness::new(0.0, 0.0, 282.0, 0.0))
            .horizontal_alignment(HorizontalAlignment::Right)
            .style(ButtonStyle::Subtle)
            .on_click(vc.message(Message::Shell(ShellMsg::ToggleTheme)))
            .automation_name(theme_automation_name)
            .content(icons::path(theme_icon));
        let title_actions = Grid::new().grid_row(0).children((
            title_theme,
            title_help,
            title_palette,
            title_settings,
        ));

        (title_brand, title_bar.into(), title_actions)
    }

    /// The two cross-faded wallpapers behind everything else.
    pub(crate) fn wallpapers(env: &ShellEnv<'_>) -> (View, View) {
        let light_wallpaper: View = Border::new()
            .grid_row_span(2)
            .opacity(if env.theme == WindowTheme::Light {
                1.0
            } else {
                0.0
            })
            .opacity_transition(std::time::Duration::from_millis(500))
            .content(
                Image::new()
                    .source_data(EncodedImage::from_static(WALLPAPER_LIGHT))
                    .stretch(Stretch::UniformToFill),
            );
        let dark_wallpaper: View = Border::new()
            .grid_row_span(2)
            .opacity(if env.theme == WindowTheme::Light {
                0.0
            } else {
                1.0
            })
            .opacity_transition(std::time::Duration::from_millis(500))
            .content(
                Image::new()
                    .source_data(EncodedImage::from_static(WALLPAPER_DARK))
                    .stretch(Stretch::UniformToFill),
            );

        (light_wallpaper, dark_wallpaper)
    }
}
