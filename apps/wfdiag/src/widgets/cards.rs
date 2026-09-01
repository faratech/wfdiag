//! Shared card and statistic surfaces.

#![deny(unsafe_code)]

use crate::screens::monitor::view::{monitor_axis, monitor_graph};
use crate::widgets::palette_colors::Palette;
use windows_reactor::*;

pub(crate) fn metric_card(
    palette: Palette,
    name: &str,
    hint: &str,
    value: &str,
    unit: &'static str,
    series: &[f64],
    max: f64,
) -> View {
    Border::new()
        .height(156.0)
        .background(palette.card)
        .border_brush(palette.border)
        .border_thickness(1.0)
        .corner_radius(9.0)
        .padding(Thickness::new(17.0, 15.0, 17.0, 10.0))
        .content(
            StackPanel::new().children((
                Grid::new()
                    .columns([GridLength::Star(1.0), GridLength::Auto])
                    .children((
                        StackPanel::new().spacing(4.0).children((
                            TextBlock::new()
                                .text(name)
                                .font_size(10.5)
                                .font_weight(FontWeight::SEMI_BOLD)
                                .foreground(palette.muted),
                            TextBlock::new()
                                .text(hint)
                                .font_size(11.5)
                                .foreground(palette.muted)
                                .text_wrapping(TextWrapping::NoWrap)
                                .text_trimming(TextTrimming::CharacterEllipsis),
                        )),
                        StackPanel::new()
                            .grid_column(1)
                            .orientation(Orientation::Horizontal)
                            .spacing(3.0)
                            .margin(Thickness::new(0.0, -6.0, 0.0, 6.0))
                            .vertical_alignment(VerticalAlignment::Top)
                            .children((
                                TextBlock::new()
                                    .text(value)
                                    .font_size(26.0)
                                    .font_weight(FontWeight::LIGHT),
                                TextBlock::new()
                                    .text(unit)
                                    .margin(Thickness::new(0.0, 13.0, 0.0, 0.0))
                                    .font_size(12.0)
                                    .font_weight(FontWeight::NORMAL)
                                    .foreground(palette.muted),
                            )),
                    )),
                monitor_graph(palette, series, max),
                monitor_axis(palette),
            )),
        )
}

pub(crate) fn statistic(label: &str, value: &str, color: Color) -> View {
    StackPanel::new().spacing(1.0).children((
        TextBlock::new()
            .text(label)
            .font_size(10.0)
            .font_weight(FontWeight::BOLD),
        TextBlock::new()
            .text(value)
            .font_size(22.0)
            .foreground(color),
    ))
}
