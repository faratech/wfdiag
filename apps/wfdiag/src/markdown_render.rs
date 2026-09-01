//! Native WinUI rendering for the Markdown-lite documents produced by
//! [`wfdiag_native_projection::markdown`].
//!
//! Parsing, the link policy, and their tests live in the projection crate.
//! Rendering uses Reactor's structured [`RichText`] model and ordinary WinUI
//! controls; no browser or HTML parser is involved.

#![deny(unsafe_code)]

use wfdiag_native_projection::markdown::{
    MarkdownAlignment, MarkdownBlock, MarkdownInline, parse_markdown_lite,
};
use windows_reactor::*;

/// surfaces that do not inherit it (code and table chrome).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkdownStyle {
    pub body_font_size: f64,
    pub heading_font_size: f64,
    pub code_font_size: f64,
    pub block_spacing: f64,
    pub list_indent: f64,
    pub code_padding: f64,
    pub code_corner_radius: f64,
    pub table_cell_padding: f64,
    pub code_foreground: Brush,
    pub code_background: Brush,
    pub border: Brush,
}

impl MarkdownStyle {
    /// Apply the caller's current palette without coupling this module to the
    /// shell-private `Palette` type.
    #[must_use]
    pub fn with_palette(text: Color, code_background: Color, border: Color) -> Self {
        Self {
            code_foreground: text.into(),
            code_background: code_background.into(),
            border: border.into(),
            ..Self::default()
        }
    }
}

impl Default for MarkdownStyle {
    fn default() -> Self {
        Self {
            body_font_size: 13.0,
            heading_font_size: 15.0,
            code_font_size: 12.0,
            block_spacing: 8.0,
            list_indent: 14.0,
            code_padding: 10.0,
            code_corner_radius: 6.0,
            table_cell_padding: 7.0,
            code_foreground: ThemeBrush::PrimaryText.into(),
            code_background: ThemeBrush::CardBackground.into(),
            border: ThemeBrush::CardStroke.into(),
        }
    }
}

/// Render Markdown-lite directly to native Reactor/WinUI views. All text is
/// selectable, code blocks scroll horizontally, and only allowlisted absolute
/// URLs are emitted as native hyperlinks.
#[must_use]
pub fn render_markdown_lite(text: &str, style: MarkdownStyle) -> View {
    let document = parse_markdown_lite(text);
    let children = document
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| KeyedView::new(index, render_block(block, style)))
        .collect::<Vec<_>>();
    StackPanel::new()
        .orientation(Orientation::Vertical)
        .spacing(style.block_spacing)
        .keyed_children(children)
}

fn render_block(block: &MarkdownBlock, style: MarkdownStyle) -> View {
    match block {
        MarkdownBlock::Heading { content, .. } => rich_text_block(content, style, true)
            .font_size(style.heading_font_size)
            .into(),
        MarkdownBlock::Paragraph(content) => rich_text_block(content, style, false).into(),
        MarkdownBlock::List { ordered, items } => render_list(*ordered, items, style),
        MarkdownBlock::CodeBlock(code) => render_code_block(code, style),
        MarkdownBlock::Table {
            headers,
            alignments,
            rows,
        } => render_table(headers, alignments, rows, style),
    }
}

fn rich_text_block(
    content: &[MarkdownInline],
    style: MarkdownStyle,
    force_bold: bool,
) -> RichTextBlock {
    RichTextBlock::new()
        .paragraphs(RichText::single_paragraph(render_inlines(
            content, force_bold,
        )))
        .font_size(style.body_font_size)
        .is_text_selection_enabled(true)
        .text_wrapping(TextWrapping::Wrap)
}

fn render_inlines(content: &[MarkdownInline], force_bold: bool) -> Vec<RichTextInline> {
    content
        .iter()
        .map(|inline| match inline {
            MarkdownInline::Text(text) => RichTextInline::Run(RichTextRun {
                text: text.clone(),
                is_bold: force_bold,
                ..RichTextRun::default()
            }),
            MarkdownInline::Bold(text) => RichTextInline::Run(RichTextRun {
                text: text.clone(),
                is_bold: true,
                ..RichTextRun::default()
            }),
            // The pinned RichTextRun surface has no font-family property.
            // Italic keeps inline code visibly distinct without retaining the
            // Markdown delimiters or sacrificing selection/wrapping.
            MarkdownInline::Code(text) => RichTextInline::Run(RichTextRun {
                text: text.clone(),
                is_bold: force_bold,
                is_italic: true,
            }),
            MarkdownInline::Link {
                label,
                target: Some(target),
            } => RichTextInline::Hyperlink(RichTextHyperlink {
                text: label.clone(),
                uri: target.clone(),
            }),
            MarkdownInline::Link {
                label,
                target: None,
            } => RichTextInline::Run(RichTextRun {
                text: label.clone(),
                is_bold: force_bold,
                ..RichTextRun::default()
            }),
        })
        .collect()
}

fn render_list(ordered: bool, items: &[Vec<MarkdownInline>], style: MarkdownStyle) -> View {
    let paragraphs = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let marker = if ordered {
                format!("{}. ", index + 1)
            } else {
                "• ".to_string()
            };
            let mut inlines = vec![RichTextInline::Run(RichTextRun::plain(marker))];
            inlines.extend(render_inlines(item, false));
            RichTextParagraph::new(inlines)
        })
        .collect::<Vec<_>>();
    RichTextBlock::new()
        .paragraphs(RichText::new(paragraphs))
        .font_size(style.body_font_size)
        .is_text_selection_enabled(true)
        .text_wrapping(TextWrapping::Wrap)
        .margin(Thickness::new(style.list_indent, 0.0, 0.0, 0.0))
        .into()
}

fn render_code_block(code: &str, style: MarkdownStyle) -> View {
    Border::new()
        .padding(style.code_padding)
        .background(style.code_background)
        .border_brush(style.border)
        .border_thickness(1.0)
        .corner_radius(style.code_corner_radius)
        .content(
            ScrollViewer::new()
                .horizontal_scroll_bar_visibility(ScrollBarVisibility::Auto)
                .vertical_scroll_bar_visibility(ScrollBarVisibility::Disabled)
                .content(
                    TextBlock::new()
                        .text(code)
                        .font_size(style.code_font_size)
                        .foreground(style.code_foreground)
                        .is_text_selection_enabled(true)
                        .text_wrapping(TextWrapping::NoWrap),
                ),
        )
}

fn render_table(
    headers: &[Vec<MarkdownInline>],
    alignments: &[MarkdownAlignment],
    rows: &[Vec<Vec<MarkdownInline>>],
    style: MarkdownStyle,
) -> View {
    if headers.is_empty() {
        return View::empty();
    }
    let mut cells = Vec::new();
    for (row_index, row) in std::iter::once(headers)
        .chain(rows.iter().map(Vec::as_slice))
        .enumerate()
    {
        for (column_index, content) in row.iter().enumerate() {
            let alignment = alignments.get(column_index).copied().unwrap_or_default();
            let rich = rich_text_block(content, style, row_index == 0).horizontal_alignment(
                match alignment {
                    MarkdownAlignment::Left => HorizontalAlignment::Left,
                    MarkdownAlignment::Center => HorizontalAlignment::Center,
                    MarkdownAlignment::Right => HorizontalAlignment::Right,
                },
            );
            cells.push(KeyedView::new(
                format!("{row_index}:{column_index}"),
                Border::new()
                    .grid_row(i32::try_from(row_index).unwrap_or(i32::MAX))
                    .grid_column(i32::try_from(column_index).unwrap_or(i32::MAX))
                    .padding(style.table_cell_padding)
                    .border_brush(style.border)
                    .border_thickness(0.5)
                    .content(rich),
            ));
        }
    }
    let grid = Grid::new()
        .columns(std::iter::repeat_n(GridLength::Auto, headers.len()))
        .rows(std::iter::repeat_n(GridLength::Auto, rows.len() + 1))
        .keyed_children(cells);
    ScrollViewer::new()
        .horizontal_scroll_bar_visibility(ScrollBarVisibility::Auto)
        .vertical_scroll_bar_visibility(ScrollBarVisibility::Disabled)
        .content(grid)
}
