//! Safe Markdown-lite parsing for `WFDiag` shells.
//!
//! The parser intentionally mirrors the small surface used by the shipping
//! React UI. It never interprets HTML, and it is deliberately free of any
//! view-framework type so the untrusted-text and URL policy stays testable on
//! every host. Native rendering of a [`MarkdownDocument`] lives in the shell.

const MAX_LINK_CHARS: usize = 2_048;

/// Upper bound on one inline span search (a bold span or a link label).
/// Longer candidates render as plain text. Without this bound the inline
/// scanner rescans to the end of the document for every unmatched `**` or `[`,
/// which is quadratic on model output that repeats opening markers.
const MAX_INLINE_SPAN_CHARS: usize = 512;

/// The at-most-`max_chars`-character tail of `text` starting at byte `start`
/// (always sliced on a char boundary). Lookupahead costs stay linear.
fn bounded_tail(text: &str, start: usize, max_chars: usize) -> &str {
    let tail = text.get(start..).unwrap_or("");
    match tail.char_indices().nth(max_chars) {
        Some((offset, _)) => &tail[..offset],
        None => tail,
    }
}

/// Parsed Markdown-lite document. Keeping parsing separate from native view
/// construction makes the untrusted-text and URL policy directly testable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkdownDocument {
    pub blocks: Vec<MarkdownBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkdownBlock {
    Heading {
        level: u8,
        content: Vec<MarkdownInline>,
    },
    Paragraph(Vec<MarkdownInline>),
    List {
        ordered: bool,
        items: Vec<Vec<MarkdownInline>>,
    },
    CodeBlock(String),
    Table {
        headers: Vec<Vec<MarkdownInline>>,
        alignments: Vec<MarkdownAlignment>,
        rows: Vec<Vec<Vec<MarkdownInline>>>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkdownAlignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkdownInline {
    Text(String),
    Bold(String),
    Code(String),
    /// `target == None` means the label remains visible but the untrusted URL
    /// was rejected and will never be materialized as a `WinUI` `Hyperlink`.
    Link {
        label: String,
        target: Option<String>,
    },
}

/// Parse the deliberately bounded Markdown subset used for model-authored
/// content. Embedded HTML is always ordinary text.
#[must_use]
pub fn parse_markdown_lite(text: &str) -> MarkdownDocument {
    let mut blocks = Vec::new();
    for segment in fenced_segments(text) {
        if segment.code {
            blocks.push(MarkdownBlock::CodeBlock(
                segment
                    .text
                    .strip_suffix('\n')
                    .unwrap_or(segment.text)
                    .to_string(),
            ));
        } else {
            parse_prose_blocks(segment.text, &mut blocks);
        }
    }
    MarkdownDocument { blocks }
}

/// Return an absolute link only when `WinUI` navigation cannot dispatch an
/// executable or local-file scheme. Relative, protocol-relative, file, data,
/// JavaScript, and custom-protocol targets stay visible as inert label text.
#[must_use]
pub fn safe_markdown_link_target(raw: &str) -> Option<String> {
    let target = raw.trim();
    if target.is_empty()
        || target.len() > MAX_LINK_CHARS
        || target
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        || target.contains('\\')
        || target.contains(['"', '\'', '<', '>', '{', '}', '|', '^', '`'])
        || !valid_percent_encoding(target)
    {
        return None;
    }
    let (scheme, remainder) = target.split_once(':')?;
    if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
        let authority_and_path = remainder.strip_prefix("//")?;
        let authority = authority_and_path
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default();
        if authority.is_empty() {
            return None;
        }
    } else if scheme.eq_ignore_ascii_case("mailto") {
        if remainder.is_empty() || remainder.starts_with("//") {
            return None;
        }
    } else {
        return None;
    }
    Some(target.to_string())
}

fn valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if !bytes
                .get(index + 1..index + 3)
                .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
            {
                return false;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    true
}

#[derive(Clone, Copy)]
struct FencedSegment<'a> {
    code: bool,
    text: &'a str,
}

/// Match the shipping split expression: every triple-backtick toggles code,
/// and an optional ASCII-word language plus newline is consumed with it.
fn fenced_segments(text: &str) -> Vec<FencedSegment<'_>> {
    let mut segments = Vec::new();
    let mut cursor = 0;
    let mut code = false;
    while let Some(relative) = text[cursor..].find("```") {
        let fence = cursor + relative;
        segments.push(FencedSegment {
            code,
            text: &text[cursor..fence],
        });
        let mut after = fence + 3;
        if let Some(newline) = text[after..].find('\n') {
            let label = &text[after..after + newline];
            if label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                after += newline + 1;
            }
        }
        cursor = after;
        code = !code;
    }
    segments.push(FencedSegment {
        code,
        text: &text[cursor..],
    });
    segments
}

#[derive(Default)]
struct PendingList {
    ordered: bool,
    items: Vec<String>,
}

fn parse_prose_blocks(text: &str, output: &mut Vec<MarkdownBlock>) {
    let lines = text.split('\n').collect::<Vec<_>>();
    let mut paragraph = Vec::<String>::new();
    let mut list = None::<PendingList>;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();

        if trimmed.contains('|') && index + 1 < lines.len() && is_table_delimiter(lines[index + 1])
        {
            flush_paragraph(&mut paragraph, output);
            flush_list(&mut list, output);
            let header_cells = split_table_row(trimmed);
            let alignments = split_table_row(lines[index + 1])
                .into_iter()
                .map(|cell| alignment_of(&cell))
                .collect::<Vec<_>>();
            let headers = header_cells
                .iter()
                .map(|cell| parse_inline(cell))
                .collect::<Vec<_>>();
            let mut rows = Vec::new();
            let mut next = index + 2;
            while next < lines.len() && !lines[next].trim().is_empty() && lines[next].contains('|')
            {
                let mut cells = split_table_row(lines[next]);
                cells.resize(header_cells.len(), String::new());
                cells.truncate(header_cells.len());
                rows.push(
                    cells
                        .iter()
                        .map(|cell| parse_inline(cell))
                        .collect::<Vec<_>>(),
                );
                next += 1;
            }
            output.push(MarkdownBlock::Table {
                headers,
                alignments,
                rows,
            });
            index = next;
            continue;
        }

        if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, output);
            flush_list(&mut list, output);
        } else if let Some((level, heading)) = heading(trimmed) {
            flush_paragraph(&mut paragraph, output);
            flush_list(&mut list, output);
            output.push(MarkdownBlock::Heading {
                level,
                content: parse_inline(heading),
            });
        } else if let Some(item) = unordered_item(trimmed) {
            flush_paragraph(&mut paragraph, output);
            if list.as_ref().is_some_and(|pending| pending.ordered) {
                flush_list(&mut list, output);
            }
            list.get_or_insert_with(|| PendingList {
                ordered: false,
                items: Vec::new(),
            })
            .items
            .push(item.to_string());
        } else if let Some(item) = ordered_item(trimmed) {
            flush_paragraph(&mut paragraph, output);
            if list.as_ref().is_some_and(|pending| !pending.ordered) {
                flush_list(&mut list, output);
            }
            list.get_or_insert_with(|| PendingList {
                ordered: true,
                items: Vec::new(),
            })
            .items
            .push(item.to_string());
        } else {
            flush_list(&mut list, output);
            paragraph.push(trimmed.to_string());
        }
        index += 1;
    }
    flush_paragraph(&mut paragraph, output);
    flush_list(&mut list, output);
}

fn flush_paragraph(paragraph: &mut Vec<String>, output: &mut Vec<MarkdownBlock>) {
    if !paragraph.is_empty() {
        output.push(MarkdownBlock::Paragraph(parse_inline(&paragraph.join(" "))));
        paragraph.clear();
    }
}

fn flush_list(list: &mut Option<PendingList>, output: &mut Vec<MarkdownBlock>) {
    if let Some(list) = list.take() {
        output.push(MarkdownBlock::List {
            ordered: list.ordered,
            items: list.items.iter().map(|item| parse_inline(item)).collect(),
        });
    }
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let count = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=4).contains(&count) {
        return None;
    }
    let rest = &line[count..];
    rest.chars()
        .next()
        .filter(|character| character.is_whitespace())?;
    Some((u8::try_from(count).ok()?, rest.trim_start()))
}

fn unordered_item(line: &str) -> Option<&str> {
    let marker = line.chars().next()?;
    if !matches!(marker, '-' | '*') {
        return None;
    }
    let rest = &line[marker.len_utf8()..];
    rest.chars()
        .next()
        .filter(|character| character.is_whitespace())?;
    Some(rest.trim_start())
}

fn ordered_item(line: &str) -> Option<&str> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let punctuation = *line.as_bytes().get(digits)?;
    if !matches!(punctuation, b'.' | b')') {
        return None;
    }
    let rest = &line[digits + 1..];
    rest.chars()
        .next()
        .filter(|character| character.is_whitespace())?;
    Some(rest.trim_start())
}

fn parse_inline(text: &str) -> Vec<MarkdownInline> {
    let mut output = Vec::new();
    let mut plain_start = 0;
    let mut search_start = 0;
    while let Some(relative) = text[search_start..].find('`') {
        let open = search_start + relative;
        let content_start = open + 1;
        let Some(close_relative) = text[content_start..].find('`') else {
            break;
        };
        let close = content_start + close_relative;
        if close == content_start {
            search_start = close + 1;
            continue;
        }
        parse_non_code_inline(&text[plain_start..open], &mut output);
        output.push(MarkdownInline::Code(text[content_start..close].to_string()));
        plain_start = close + 1;
        search_start = plain_start;
    }
    parse_non_code_inline(&text[plain_start..], &mut output);
    output
}

fn parse_non_code_inline(text: &str, output: &mut Vec<MarkdownInline>) {
    let mut cursor = 0;
    let mut plain_start = 0;
    while cursor < text.len() {
        if text[cursor..].starts_with("**")
            && let Some(relative) = bounded_tail(text, cursor + 2, MAX_INLINE_SPAN_CHARS).find("**")
        {
            let close = cursor + 2 + relative;
            let content = &text[cursor + 2..close];
            if !content.is_empty() && !content.contains('*') {
                push_text(output, &text[plain_start..cursor]);
                output.push(MarkdownInline::Bold(content.to_string()));
                cursor = close + 2;
                plain_start = cursor;
                continue;
            }
        }
        if text[cursor..].starts_with('[')
            && let Some(label_end_relative) =
                bounded_tail(text, cursor + 1, MAX_INLINE_SPAN_CHARS).find("](")
        {
            let label_end = cursor + 1 + label_end_relative;
            let target_start = label_end + 2;
            if let Some(target_end_relative) = link_target_end(&text[target_start..]) {
                let target_end = target_start + target_end_relative;
                let label = &text[cursor + 1..label_end];
                let target = &text[target_start..target_end];
                if !label.is_empty() && !target.is_empty() {
                    push_text(output, &text[plain_start..cursor]);
                    output.push(MarkdownInline::Link {
                        label: label.to_string(),
                        target: safe_markdown_link_target(target),
                    });
                    cursor = target_end + 1;
                    plain_start = cursor;
                    continue;
                }
            }
        }
        let Some(character) = text[cursor..].chars().next() else {
            break;
        };
        cursor += character.len_utf8();
    }
    push_text(output, &text[plain_start..]);
}

fn link_target_end(target_and_suffix: &str) -> Option<usize> {
    let mut nested = 0_u32;
    for (index, character) in target_and_suffix.char_indices() {
        match character {
            '(' => nested = nested.saturating_add(1),
            ')' if nested == 0 => return Some(index),
            ')' => nested -= 1,
            _ => {}
        }
    }
    None
}

fn push_text(output: &mut Vec<MarkdownInline>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(MarkdownInline::Text(existing)) = output.last_mut() {
        existing.push_str(text);
    } else {
        output.push(MarkdownInline::Text(text.to_string()));
    }
}

fn split_table_row(line: &str) -> Vec<String> {
    let mut row = line.trim();
    if let Some(stripped) = row.strip_prefix('|') {
        row = stripped;
    }
    if let Some(stripped) = row.strip_suffix('|') {
        row = stripped;
    }
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut chars = row.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\\' && chars.peek() == Some(&'|') {
            current.push('|');
            chars.next();
        } else if character == '|' {
            cells.push(current.trim().to_string());
            current.clear();
        } else {
            current.push(character);
        }
    }
    cells.push(current.trim().to_string());
    cells
}

fn is_table_delimiter(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.contains('|')
        && trimmed.contains('-')
        && split_table_row(trimmed)
            .iter()
            .all(|cell| delimiter_cell(cell))
}

fn delimiter_cell(cell: &str) -> bool {
    let cell = cell.strip_prefix(':').unwrap_or(cell);
    let cell = cell.strip_suffix(':').unwrap_or(cell);
    !cell.is_empty() && cell.bytes().all(|byte| byte == b'-')
}

fn alignment_of(cell: &str) -> MarkdownAlignment {
    match (cell.starts_with(':'), cell.ends_with(':')) {
        (true, true) => MarkdownAlignment::Center,
        (false, true) => MarkdownAlignment::Right,
        _ => MarkdownAlignment::Left,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> MarkdownInline {
        MarkdownInline::Text(value.to_string())
    }

    #[test]
    fn paragraphs_join_lines_and_html_remains_inert_text() {
        let document =
            parse_markdown_lite("first line\ncontinues <img src=x onerror=alert(1)>\n\nsecond");
        assert_eq!(
            document.blocks,
            [
                MarkdownBlock::Paragraph(vec![text(
                    "first line continues <img src=x onerror=alert(1)>"
                )]),
                MarkdownBlock::Paragraph(vec![text("second")]),
            ]
        );
    }

    #[test]
    fn inline_code_is_verbatim_before_bold_and_links_are_typed() {
        let document = parse_markdown_lite(
            "a **bold** and `**literal**` plus [Docs](https://learn.microsoft.com/windows/)",
        );
        assert_eq!(
            document.blocks,
            [MarkdownBlock::Paragraph(vec![
                text("a "),
                MarkdownInline::Bold("bold".to_string()),
                text(" and "),
                MarkdownInline::Code("**literal**".to_string()),
                text(" plus "),
                MarkdownInline::Link {
                    label: "Docs".to_string(),
                    target: Some("https://learn.microsoft.com/windows/".to_string()),
                },
            ])]
        );
    }

    #[test]
    fn unsafe_and_relative_links_remain_inert() {
        for target in [
            "javascript:alert(1)",
            "data:text/html,x",
            "file:///C:/secret",
            "custom:payload",
            "//example.com/path",
            "/relative/path",
            "https://example.com/space here",
            "https:\\\\example.com",
            "https://example.com/%zz",
        ] {
            assert_eq!(safe_markdown_link_target(target), None, "{target}");
        }
        assert_eq!(
            safe_markdown_link_target(" HTTPS://example.com/path?q=1#part "),
            Some("HTTPS://example.com/path?q=1#part".to_string())
        );
        assert_eq!(
            safe_markdown_link_target("mailto:support@example.com"),
            Some("mailto:support@example.com".to_string())
        );

        let document = parse_markdown_lite("[click](javascript:alert(1))");
        assert_eq!(
            document.blocks,
            [MarkdownBlock::Paragraph(vec![MarkdownInline::Link {
                label: "click".to_string(),
                target: None,
            }])]
        );
    }

    #[test]
    fn headings_and_list_type_transitions_match_the_shipping_groups() {
        let document = parse_markdown_lite(
            "### Title\nbody\n- one\n* **two**\n3) three\n4. four\n##### not a heading",
        );
        assert_eq!(
            document.blocks,
            [
                MarkdownBlock::Heading {
                    level: 3,
                    content: vec![text("Title")],
                },
                MarkdownBlock::Paragraph(vec![text("body")]),
                MarkdownBlock::List {
                    ordered: false,
                    items: vec![
                        vec![text("one")],
                        vec![MarkdownInline::Bold("two".to_string())],
                    ],
                },
                MarkdownBlock::List {
                    ordered: true,
                    items: vec![vec![text("three")], vec![text("four")]],
                },
                MarkdownBlock::Paragraph(vec![text("##### not a heading")]),
            ]
        );
    }

    #[test]
    fn fenced_code_is_verbatim_and_discards_the_language_tag() {
        let document =
            parse_markdown_lite("before\n```rust\nlet x = **1**;\n  keep indent\n```\nafter");
        assert_eq!(
            document.blocks,
            [
                MarkdownBlock::Paragraph(vec![text("before")]),
                MarkdownBlock::CodeBlock("let x = **1**;\n  keep indent".to_string()),
                MarkdownBlock::Paragraph(vec![text("after")]),
            ]
        );
    }

    #[test]
    fn gfm_tables_preserve_alignment_escaped_pipes_and_ragged_rows() {
        let document = parse_markdown_lite(
            "| Name | Center | Value |\n| :--- | :---: | ---: |\n| CPU Usage | a\\|b | 91% |\n| OnlyOne |",
        );
        assert_eq!(
            document.blocks,
            [MarkdownBlock::Table {
                headers: vec![
                    vec![text("Name")],
                    vec![text("Center")],
                    vec![text("Value")]
                ],
                alignments: vec![
                    MarkdownAlignment::Left,
                    MarkdownAlignment::Center,
                    MarkdownAlignment::Right,
                ],
                rows: vec![
                    vec![
                        vec![text("CPU Usage")],
                        vec![text("a|b")],
                        vec![text("91%")]
                    ],
                    vec![vec![text("OnlyOne")], Vec::new(), Vec::new()],
                ],
            }]
        );
    }

    #[test]
    fn a_pipe_without_a_delimiter_is_an_ordinary_paragraph() {
        assert_eq!(
            parse_markdown_lite("a | b without a delimiter row").blocks,
            [MarkdownBlock::Paragraph(vec![text(
                "a | b without a delimiter row"
            )])]
        );
    }

    #[test]
    fn malformed_or_empty_inline_delimiters_remain_literal() {
        assert_eq!(
            parse_markdown_lite("empty `` and unmatched `code and **bold").blocks,
            [MarkdownBlock::Paragraph(vec![text(
                "empty `` and unmatched `code and **bold"
            )])]
        );
    }
}
