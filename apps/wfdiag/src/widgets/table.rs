//! Shared table primitives.

#![deny(unsafe_code)]

use windows_reactor::*;

pub(crate) fn table_header(label: &str, column: i32) -> TextBlock {
    TextBlock::new()
        .text(label)
        .grid_column(column)
        .margin(Thickness::new(8.0, 8.0, 8.0, 8.0))
        .font_size(10.0)
        .font_weight(FontWeight::BOLD)
}

pub(crate) fn table_cell(text: impl Into<String>, column: i32) -> TextBlock {
    TextBlock::new()
        .text(text)
        .grid_column(column)
        .margin(Thickness::xy(7.0, 0.0))
        .font_size(10.5)
        .vertical_alignment(VerticalAlignment::Center)
}
