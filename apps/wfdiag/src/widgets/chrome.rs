//! Shell chrome: navigation rail, page header, and machine identity card.

#![deny(unsafe_code)]

use crate::app::consts::{APP_BADGE, APP_VERSION};
use crate::app::policy::{machine_card_accessibility_name, privilege_label};
use crate::app::state::Page;
use crate::widgets::icons;
use crate::widgets::icons::FaIcon;
use crate::widgets::palette_colors::Palette;
use wfdiag_native_system::{ArchitectureSnapshot, SystemInfo};
use windows_reactor::*;

pub(crate) fn nav_brand(palette: Palette, expanded: bool) -> View {
    let content: View = if expanded {
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(11.0)
            .children((
                Image::new()
                    .source_data(EncodedImage::from_static(APP_BADGE))
                    .width(32.0)
                    .height(32.0),
                StackPanel::new().spacing(1.0).children((
                    TextBlock::new()
                        .text("WindowsForum")
                        .font_size(13.5)
                        .font_weight(FontWeight::BOLD),
                    TextBlock::new()
                        .text(format!("Diagnostics · {APP_VERSION}"))
                        .font_size(11.0)
                        .foreground(palette.muted),
                )),
            ))
    } else {
        Image::new()
            .source_data(EncodedImage::from_static(APP_BADGE))
            .width(32.0)
            .height(32.0)
            .horizontal_alignment(HorizontalAlignment::Center)
            .into()
    };

    Border::new()
        .padding(if expanded {
            Thickness::new(10.0, 10.0, 10.0, 18.0)
        } else {
            Thickness::new(0.0, 10.0, 0.0, 18.0)
        })
        .content(content)
}

pub(crate) fn nav_section(label: &'static str, expanded: bool, palette: Palette) -> View {
    if expanded {
        Border::new()
            .margin(Thickness::new(11.0, 20.0, 11.0, 7.0))
            .content(
                TextBlock::new()
                    .text(label)
                    .font_size(10.5)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .foreground(palette.muted),
            )
    } else {
        Border::new().height(12.0).into()
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn nav_button(
    palette: Palette,
    icon: FaIcon,
    label: &'static str,
    selected: bool,
    expanded: bool,
    badge: Option<&str>,
    action: Callback<()>,
    enabled: bool,
) -> View {
    let content: View = if expanded {
        let badge: View = if let Some(value) = badge {
            Border::new()
                .grid_column(2)
                .min_width(18.0)
                .height(18.0)
                .padding(Thickness::xy(5.0, 0.0))
                .background(palette.warn_bg)
                .corner_radius(999.0)
                .vertical_alignment(VerticalAlignment::Center)
                .content(
                    TextBlock::new()
                        .text(value)
                        .font_size(10.5)
                        .font_weight(FontWeight::BOLD)
                        .foreground(palette.warn)
                        .horizontal_alignment(HorizontalAlignment::Center),
                )
        } else {
            View::empty()
        };
        Grid::new()
            .columns([
                GridLength::Pixel(17.0),
                GridLength::Star(1.0),
                GridLength::Auto,
            ])
            .column_spacing(11.0)
            .children((
                icons::path(icon),
                TextBlock::new()
                    .text(label)
                    .grid_column(1)
                    .font_size(13.0)
                    .font_weight(if selected {
                        FontWeight::SEMI_BOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .vertical_alignment(VerticalAlignment::Center),
                badge,
            ))
    } else {
        icons::path(icon).into()
    };

    let button = Button::new()
        .height(37.0)
        .horizontal_alignment(HorizontalAlignment::Stretch)
        .horizontal_content_alignment(if expanded {
            HorizontalAlignment::Left
        } else {
            HorizontalAlignment::Center
        })
        .resource_overrides(
            ResourceOverrides::new()
                .set(
                    "ButtonBackground",
                    if selected {
                        palette.active
                    } else {
                        Color::transparent()
                    },
                )
                .set("ButtonBackgroundDisabled", Color::transparent())
                .set("ButtonBackgroundPointerOver", palette.card)
                .set("ButtonBackgroundPressed", palette.active)
                .set("ButtonForeground", palette.text)
                .set("ButtonForegroundDisabled", palette.muted)
                .set("ButtonBorderBrushDisabled", Color::transparent())
                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                .set(
                    "ButtonPadding",
                    if expanded {
                        Thickness::xy(11.0, 0.0)
                    } else {
                        Thickness::uniform(0.0)
                    },
                )
                .set("ControlCornerRadius", CornerRadius::uniform(6.0)),
        )
        .is_enabled(enabled)
        .on_click(action)
        .automation_name(label)
        .content(content);

    if selected {
        Grid::new().children((
            button,
            Border::new()
                .width(3.0)
                .height(17.0)
                .margin(Thickness::new(0.0, 0.0, 0.0, 0.0))
                .background(palette.accent)
                .corner_radius(999.0)
                .horizontal_alignment(HorizontalAlignment::Left)
                .vertical_alignment(VerticalAlignment::Center),
        ))
    } else {
        button
    }
}

pub(crate) fn machine_card(
    palette: Palette,
    expanded: bool,
    system_info: &SystemInfo,
    architecture: Option<&ArchitectureSnapshot>,
    system_error: Option<&str>,
) -> View {
    if !expanded {
        return View::empty();
    }

    Border::new()
        .margin(Thickness::new(0.0, 8.0, 0.0, 0.0))
        .padding(Thickness::new(12.0, 11.0, 12.0, 11.0))
        .background(palette.card)
        .corner_radius(8.0)
        .automation_name(machine_card_accessibility_name(
            system_info,
            architecture,
            system_error,
        ))
        .content(StackPanel::new().spacing(7.0).children((
            machine_icon_label(FaIcon::Desktop, system_info.computer_name.clone()),
            machine_icon_label(FaIcon::Windows, system_info.os_version.clone()),
            machine_icon_label(FaIcon::UserShield, privilege_label(system_info.is_admin)),
        )))
}

pub(crate) fn machine_icon_label(icon: FaIcon, label: impl Into<String>) -> View {
    Grid::new()
        .columns([GridLength::Pixel(17.0), GridLength::Star(1.0)])
        .column_spacing(8.0)
        .children((
            icons::path(icon),
            TextBlock::new()
                .text(label)
                .grid_column(1)
                .text_trimming(TextTrimming::CharacterEllipsis),
        ))
}

pub(crate) fn fa_icon_label(icon: FaIcon, label: impl Into<String>) -> View {
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(8.0)
        .children((icons::path(icon), label.into()))
}

pub(crate) fn page_header(palette: Palette, page: Page, trailing: impl Into<View>) -> View {
    Grid::new()
        .columns([GridLength::Star(1.0), GridLength::Auto])
        .children((
            StackPanel::new().spacing(3.0).children((
                TextBlock::new()
                    .text(page.title())
                    .font_size(21.0)
                    .font_weight(FontWeight::BOLD)
                    .automation_heading_level(AutomationHeadingLevel::Level1),
                TextBlock::new()
                    .text(page.subtitle())
                    .font_size(12.5)
                    .foreground(palette.muted),
            )),
            Border::new().grid_column(1).content(trailing),
        ))
}

pub(crate) fn placed(view: impl Into<View>, column: i32, row: i32) -> View {
    Border::new()
        .grid_column(column)
        .grid_row(row)
        .content(view)
}
