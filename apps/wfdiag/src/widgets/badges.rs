//! Shared status pills.

#![deny(unsafe_code)]

use windows_reactor::*;

pub(crate) fn status_pill(label: impl Into<String>, foreground: Color, background: Color) -> View {
    Border::new()
        .height(24.0)
        .background(background)
        .corner_radius(999.0)
        .padding(Thickness::xy(12.0, 0.0))
        .vertical_alignment(VerticalAlignment::Top)
        .content(
            TextBlock::new()
                .text(label)
                .foreground(foreground)
                .font_size(11.5)
                .font_weight(FontWeight::SEMI_BOLD)
                .vertical_alignment(VerticalAlignment::Center),
        )
}

pub(crate) fn icon_status_pill(
    label: impl Into<String>,
    icon: &'static [u8],
    foreground: Color,
    background: Color,
) -> View {
    Border::new()
        .height(24.0)
        .background(background)
        .corner_radius(999.0)
        .padding(Thickness::xy(12.0, 0.0))
        .vertical_alignment(VerticalAlignment::Top)
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(6.0)
                .children((
                    Image::new()
                        .source_data(EncodedImage::from_static(icon))
                        .width(11.0)
                        .height(11.0)
                        .vertical_alignment(VerticalAlignment::Center),
                    TextBlock::new()
                        .text(label)
                        .foreground(foreground)
                        .font_size(11.5)
                        .font_weight(FontWeight::SEMI_BOLD)
                        .vertical_alignment(VerticalAlignment::Center),
                )),
        )
}
