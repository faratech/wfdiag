//! The About dialog.

#![deny(unsafe_code)]

use crate::app::consts::{ABOUT_DESCRIPTION, APP_BADGE, APP_VERSION};
use crate::screens::ai::view::primary_button_resources;
use crate::widgets::icons;
use crate::widgets::icons::FaIcon;
use crate::widgets::palette_colors::Palette;
use wfdiag_native_update::UpdateInfo;
use windows_reactor::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn about_dialog(
    palette: Palette,
    open: bool,
    close_reference: &ElementRef<Button>,
    update: Option<&UpdateInfo>,
    action_error: Option<&str>,
    actions_enabled: bool,
    on_closed: Callback<ContentDialogResult>,
    on_download: Callback<()>,
    on_windowsforum: Callback<()>,
    on_github: Callback<()>,
    on_close: Callback<()>,
) -> View {
    let update_status: View = update.map_or_else(View::empty, |update| {
        Border::new()
            .margin(Thickness::new(0.0, 14.0, 0.0, 0.0))
            .content(
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .horizontal_alignment(HorizontalAlignment::Center)
                    .spacing(6.0)
                    .children((
                        icons::path(FaIcon::CircleUp),
                        TextBlock::new()
                            .text(format!("Version {} is available.", update.version))
                            .font_size(13.0)
                            .foreground(palette.muted),
                    )),
            )
    });

    let action_error: View = action_error.map_or_else(View::empty, |error| {
        TextBlock::new()
            .text(error)
            .font_size(12.0)
            .foreground(palette.err)
            .text_wrapping(TextWrapping::Wrap)
            .horizontal_alignment(HorizontalAlignment::Center)
            .margin(Thickness::new(0.0, 10.0, 0.0, 0.0))
            .into()
    });

    let header_close = on_close.clone();
    let mut action_buttons = Vec::new();
    if let Some(update) = update {
        action_buttons.push(KeyedView::new(
            "download-update",
            Button::new()
                .resource_overrides(primary_button_resources())
                .is_enabled(actions_enabled)
                .on_click(on_download)
                .content(format!("Download v{}", update.version)),
        ));
    }
    let close_button = Button::new()
        .width(60.667)
        .height(31.333)
        .automation_name("Close");
    let close_button = if update.is_some() {
        close_button.style(ButtonStyle::Default)
    } else {
        close_button.resource_overrides(primary_button_resources())
    }
    .on_click(on_close)
    .content("Close");
    action_buttons.extend([
        KeyedView::new(
            "windowsforum",
            Button::new()
                .width(141.333)
                .height(31.333)
                .automation_name("WindowsForum")
                .is_enabled(actions_enabled)
                .on_click(on_windowsforum)
                .content(about_icon_label(FaIcon::Globe, "WindowsForum")),
        ),
        KeyedView::new(
            "github",
            Button::new()
                .width(92.667)
                .height(31.333)
                .automation_name("GitHub")
                .is_enabled(actions_enabled)
                .on_click(on_github)
                .content(about_icon_label(FaIcon::Github, "GitHub")),
        ),
        KeyedView::new("close", close_button),
    ]);

    // Reactor's pinned ContentDialog surface accepts only a string title. Keep
    // that real native title for modal/UIA semantics, then cover its visual row
    // with the Store 2.5.8 header chrome from inside the dialog content. The
    // negative margins extend only into ContentDialog's own measured padding;
    // focus confinement, Escape handling, and native Hide/Closed behavior stay
    // owned by WinUI.
    let header_overlay = Border::new()
        .height(59.0)
        .margin(Thickness::new(-22.0, -64.0, -22.0, 0.0))
        .vertical_alignment(VerticalAlignment::Top)
        .background(palette.card_strong)
        .border_brush(Color::rgb(53, 54, 56))
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .padding(Thickness::new(17.0, 0.0, 22.0, 0.0))
        .content(
            Grid::new()
                .columns([
                    GridLength::Pixel(3.0),
                    GridLength::Star(1.0),
                    GridLength::Pixel(29.333),
                ])
                .column_spacing(10.0)
                .children((
                    Border::new()
                        .width(3.0)
                        .height(15.0)
                        .background(palette.accent)
                        .corner_radius(999.0)
                        .vertical_alignment(VerticalAlignment::Center),
                    TextBlock::new()
                        .grid_column(1)
                        .text("About")
                        .font_size(13.0)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .vertical_alignment(VerticalAlignment::Center),
                    Button::new()
                        .grid_column(2)
                        .width(29.333)
                        .height(29.333)
                        .element_ref(close_reference)
                        .vertical_alignment(VerticalAlignment::Center)
                        .resource_overrides(
                            ResourceOverrides::new()
                                .set("ButtonBackground", Color::transparent())
                                .set("ButtonBackgroundPointerOver", palette.active)
                                .set("ButtonBackgroundPressed", palette.active)
                                .set("ButtonForeground", palette.muted)
                                .set("ButtonBorderThemeThickness", Thickness::uniform(0.0))
                                .set("ButtonPadding", Thickness::uniform(7.0))
                                .set("ControlCornerRadius", CornerRadius::uniform(6.0)),
                        )
                        .horizontal_content_alignment(HorizontalAlignment::Center)
                        .vertical_content_alignment(VerticalAlignment::Center)
                        .automation_name("Close")
                        .automation_id("about-close")
                        .on_click(header_close)
                        .content(
                            Viewbox::new()
                                .width(12.0)
                                .height(12.0)
                                .stretch(Stretch::Uniform)
                                .slot(
                                    ViewboxSlot::Child,
                                    FontIcon::new()
                                        // Segoe Fluent Icons: Cancel. The
                                        // Viewbox prevents the font baseline
                                        // from clipping this glyph to one half.
                                        .glyph("\u{E711}"),
                                ),
                        ),
                )),
        );

    ContentDialog::new()
        .title("About")
        .is_open(open)
        .on_closed(on_closed)
        .content(
            Grid::new()
                // Store 2.5.8's card is ~457 DIPs wide while its description
                // line box is ~385 DIPs. ContentDialog contributes ~45 DIPs of
                // native padding, so a 412-DIP content slot reproduces the card
                // width and the body remains centered at its measured width.
                .width(412.0)
                .background(palette.card_strong)
                .children((
                    StackPanel::new()
                        .width(385.0)
                        .horizontal_alignment(HorizontalAlignment::Center)
                        .spacing(0.0)
                        .children((
                            Border::new()
                                .width(56.0)
                                .height(56.0)
                                .margin(Thickness::new(0.0, 8.0, 0.0, 31.5))
                                .horizontal_alignment(HorizontalAlignment::Center)
                                .content(
                                    Image::new()
                                        .source_data(EncodedImage::from_static(APP_BADGE))
                                        .width(36.0)
                                        .height(36.0),
                                ),
                            TextBlock::new()
                                .text("WindowsForum Diagnostics")
                                .font_size(20.0)
                                .font_weight(FontWeight::BOLD)
                                .horizontal_alignment(HorizontalAlignment::Center),
                            TextBlock::new()
                                .text(format!("Version {APP_VERSION}"))
                                .font_size(13.0)
                                .foreground(palette.muted)
                                .horizontal_alignment(HorizontalAlignment::Center)
                                .margin(Thickness::new(0.0, 4.0, 0.0, 16.0)),
                            TextBlock::new()
                                .text(ABOUT_DESCRIPTION)
                                .automation_name(ABOUT_DESCRIPTION)
                                .font_size(13.0)
                                .foreground(palette.muted)
                                .text_wrapping(TextWrapping::Wrap)
                                .margin(Thickness::new(0.0, 4.0, 0.0, 0.0)),
                            update_status,
                            action_error,
                            StackPanel::new()
                                .orientation(Orientation::Horizontal)
                                .horizontal_alignment(HorizontalAlignment::Center)
                                .spacing(7.0)
                                .margin(Thickness::new(0.0, 23.5, 0.0, 2.0))
                                .keyed_children(action_buttons),
                        )),
                    header_overlay,
                )),
        )
}

pub(crate) fn about_icon_label(icon: FaIcon, label: &'static str) -> View {
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(6.0)
        .children((icons::path(icon), TextBlock::new().text(label)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_update_icons_use_the_pinned_font_awesome_sources() {
        assert_eq!(FaIcon::CircleUp.source_name(), "circle-up");
        assert_eq!(FaIcon::Globe.source_name(), "globe");
        assert_eq!(FaIcon::Github.source_name(), "github");
        assert_eq!(FaIcon::CircleUp.data(), icons::CIRCLE_UP);
        assert_eq!(FaIcon::Globe.data(), icons::GLOBE);
        assert_eq!(FaIcon::Github.data(), icons::GITHUB);
    }
}
