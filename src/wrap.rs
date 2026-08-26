// SPDX-License-Identifier: MPL-2.0

//! Presentation-neutral soft-wrap geometry.
//!
//! All columns are character offsets until the terminal boundary. Cell widths
//! are derived here so rendering, cursor placement, scrolling, and vertical
//! motion share exactly the same wrapping decisions.

use unicode_width::UnicodeWidthChar;

use crate::buffer::{Buffer, Position};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReflowKind {
    Plain,
    Markdown,
    Source,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    pub start_cell: usize,
    pub end_cell: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisualRow {
    pub row: usize,
    pub segment: usize,
    pub span: Segment,
}

pub fn segments(line: &str, width: usize, tab_width: usize) -> Vec<Segment> {
    let width = width.max(1);
    let tab_width = tab_width.max(1);
    let mut result = Vec::new();
    let mut start = 0;
    let mut start_cell = 0;
    let mut cell = 0;
    let mut length = 0;
    // The cell of a candidate break is carried alongside its column. Deriving
    // it by rescanning the line instead would make wrapping quadratic in line
    // length, which a minified one-line document turns into a stall.
    let mut last_word_boundary: Option<(usize, usize)> = None;
    for (column, character) in line.chars().enumerate() {
        let character_width = cell_width(character, cell, tab_width);
        if column > start && cell - start_cell + character_width > width {
            // `cell` is the running offset of `column` itself, because the
            // character's own width is only added once the break is decided.
            let (end, end_cell) = if character.is_whitespace() {
                (column, cell)
            } else {
                last_word_boundary
                    .filter(|(boundary, _)| *boundary > start)
                    .unwrap_or((column, cell))
            };
            result.push(Segment {
                start,
                end,
                start_cell,
                end_cell,
            });
            start = end;
            start_cell = end_cell;
            last_word_boundary = None;

            // A word may already be wider than a complete row after wrapping
            // at preceding whitespace. Only that case is allowed to split a
            // word, and it may need several character-boundary segments.
            if column > start && cell - start_cell + character_width > width {
                result.push(Segment {
                    start,
                    end: column,
                    start_cell,
                    end_cell: cell,
                });
                start = column;
                start_cell = cell;
            }
        }
        cell += character_width;
        length = column + 1;
        if character.is_whitespace() {
            // Prefer placing a consumed separator on the preceding row.
            // Keeping every character in one of the contiguous segments is
            // essential for cursor mapping.
            last_word_boundary = Some((column + 1, cell));
        }
    }
    result.push(Segment {
        start,
        end: length,
        start_cell,
        end_cell: cell,
    });
    result
}

/// Inserts hard line breaks without splitting words when whitespace offers a
/// legal boundary. Existing newlines remain paragraph boundaries. A word
/// longer than `width` is split only because no word boundary can satisfy the
/// requested limit.
pub fn hard_wrap(text: &str, width: usize) -> String {
    let width = width.max(1);
    let mut output = String::with_capacity(text.len());
    for part in text.split_inclusive('\n') {
        let (line, newline) = part.strip_suffix('\n').map_or((part, ""), |line| {
            line.strip_suffix('\r')
                .map_or((line, "\n"), |line| (line, "\r\n"))
        });
        let wrapped = hard_wrap_line(line, width);
        if newline == "\r\n" {
            output.push_str(&wrapped.replace('\n', "\r\n"));
        } else {
            output.push_str(&wrapped);
        }
        output.push_str(newline);
    }
    output
}

fn hard_wrap_line(line: &str, width: usize) -> String {
    let characters: Vec<char> = line.chars().collect();
    if characters.len() <= width {
        return line.to_owned();
    }

    let mut output = String::with_capacity(line.len() + line.len() / width);
    let mut start = 0;
    while characters.len().saturating_sub(start) > width {
        let limit = start + width;
        if characters
            .get(limit)
            .is_some_and(|character| character.is_whitespace())
        {
            output.extend(&characters[start..limit]);
            output.push('\n');
            start = limit;
            while start < characters.len() && characters[start].is_whitespace() {
                start += 1;
            }
            continue;
        }
        let boundary = (start + 1..=limit)
            .rev()
            .find(|index| characters[*index - 1].is_whitespace());
        if let Some(boundary) = boundary {
            let mut run_start = boundary - 1;
            while run_start > start && characters[run_start - 1].is_whitespace() {
                run_start -= 1;
            }
            if run_start > start {
                output.extend(&characters[start..run_start]);
                output.push('\n');
                start = boundary;
                while start < characters.len() && characters[start].is_whitespace() {
                    start += 1;
                }
                continue;
            }
        }

        output.extend(&characters[start..limit]);
        output.push('\n');
        start = limit;
    }
    output.extend(&characters[start..]);
    output
}

/// Removes every line break inside `text`, putting `delimiter` where each one
/// was. The inverse of `hard_wrap`, and the only text transform here that takes
/// its shape from the person rather than from a configured width.
///
/// The whitespace sitting against a removed break goes with it, so joining
/// indented lines with an empty delimiter concatenates them rather than
/// producing a run of spaces. Nothing else is touched: the first line keeps its
/// indentation and the last keeps any trailing whitespace, because neither
/// adjoins a break being removed.
///
/// Every break is joined, including one that ends `text`. Which breaks belong to
/// the join is the caller's decision and cannot be recovered from the text: a
/// span ending in a break may end on a selected empty row, whose break has to
/// go, or at a row nobody selected, whose break has to stay. The caller hands
/// over the span it means. Text holding no break at all is returned unchanged,
/// including its whitespace, so callers can treat an identical result as
/// "nothing to join".
pub fn join_lines(text: &str, delimiter: &str) -> String {
    let mut lines = text.split('\n');
    let Some(first) = lines.next() else {
        return text.to_owned();
    };
    let mut lines = lines.peekable();
    if lines.peek().is_none() {
        return text.to_owned();
    }

    let mut output = String::with_capacity(text.len());
    output.push_str(first.trim_end());
    while let Some(line) = lines.next() {
        output.push_str(delimiter);
        if lines.peek().is_some() {
            output.push_str(line.trim());
        } else {
            output.push_str(line.trim_start());
        }
    }
    output
}

/// Refills prose while retaining common document structure.
///
/// Blank lines always delimit paragraphs. In Markdown, list items receive a
/// hanging indent, block quotes are refilled behind a repeated `>` leader, and
/// syntax-only lines such as headings and fences are left alone. Outside
/// Markdown, consecutive `#` and `//` line comments are refilled while
/// repeating their leader on every output line.
pub fn reflow(text: &str, width: usize, kind: ReflowKind) -> String {
    let crlf = text
        .char_indices()
        .filter(|(_, character)| *character == '\n')
        .try_fold(false, |_, (index, _)| {
            (index > 0 && text.as_bytes()[index - 1] == b'\r').then_some(true)
        })
        .unwrap_or(false);
    if crlf {
        return reflow_lf(&text.replace("\r\n", "\n"), width, kind).replace('\n', "\r\n");
    }
    reflow_lf(text, width, kind)
}

fn reflow_lf(text: &str, width: usize, kind: ReflowKind) -> String {
    let markdown = kind == ReflowKind::Markdown;
    let lines: Vec<&str> = text.split('\n').collect();
    let mut output = Vec::with_capacity(lines.len());
    let mut index = 0;
    let mut fence = None;

    while index < lines.len() {
        let line = lines[index];
        if markdown {
            if let Some(marker) = markdown_fence(line) {
                output.push(line.to_owned());
                index += 1;
                if fence == Some(marker) {
                    fence = None;
                } else if fence.is_none() {
                    fence = Some(marker);
                }
                continue;
            }
            if fence.is_some()
                || (markdown_protected(line)
                    && markdown_list_item(line).is_none()
                    && block_quote(line).is_none())
            {
                output.push(line.to_owned());
                index += 1;
                continue;
            }
        }
        if line.trim().is_empty() {
            output.push(line.to_owned());
            index += 1;
            continue;
        }

        if kind == ReflowKind::Source && line_comment(line).is_none() {
            output.push(line.to_owned());
            index += 1;
            continue;
        }

        if !markdown && let Some(comment) = line_comment(line) {
            let (next, rendered) = reflow_comment_block(&lines, index, comment, width);
            output.extend(rendered);
            index = next;
            continue;
        }

        if markdown && let Some(quote) = block_quote(line) {
            let (next, rendered) = reflow_quote_block(&lines, index, quote, width);
            output.extend(rendered);
            index = next;
            continue;
        }

        if markdown && let Some(item) = markdown_list_item(line) {
            let mut content = vec![item.content];
            index += 1;
            while index < lines.len() {
                let next = lines[index];
                if next.trim().is_empty()
                    || markdown_fence(next).is_some()
                    || markdown_protected(next)
                    || markdown_list_item(next).is_some()
                {
                    break;
                }
                content.push(next.trim());
                index += 1;
            }
            output.extend(fill_block(
                &item.first_prefix,
                &item.continuation_prefix,
                &content,
                width,
            ));
            continue;
        }

        let indent_len = line
            .chars()
            .take_while(|character| character.is_whitespace())
            .count();
        let indent: String = line.chars().take(indent_len).collect();
        let mut content = vec![line.trim()];
        index += 1;
        while index < lines.len() {
            let next = lines[index];
            let next_indent = next
                .chars()
                .take_while(|character| character.is_whitespace())
                .count();
            if next.trim().is_empty()
                || next_indent != indent_len
                || (!markdown && line_comment(next).is_some())
                || (markdown
                    && (markdown_fence(next).is_some()
                        || markdown_protected(next)
                        || markdown_list_item(next).is_some()))
            {
                break;
            }
            content.push(next.trim());
            index += 1;
        }
        output.extend(fill_block(&indent, &indent, &content, width));
    }

    output.join("\n")
}

#[derive(Clone, Copy)]
struct CommentLine<'a> {
    indent: &'a str,
    marker: &'a str,
    content: &'a str,
}

fn line_comment(line: &str) -> Option<CommentLine<'_>> {
    let body = line.trim_start_matches([' ', '\t']);
    let indent = &line[..line.len() - body.len()];
    let marker_len = if body.starts_with("///") || body.starts_with("//!") {
        3
    } else if body.starts_with("//") {
        2
    } else if body.starts_with('#') {
        body.bytes().take_while(|byte| *byte == b'#').count()
    } else {
        return None;
    };
    let after = &body[marker_len..];
    if !after.is_empty() && !after.starts_with([' ', '\t']) {
        return None;
    }
    Some(CommentLine {
        indent,
        marker: &body[..marker_len],
        content: after.trim(),
    })
}

fn reflow_comment_block(
    lines: &[&str],
    start: usize,
    first: CommentLine<'_>,
    width: usize,
) -> (usize, Vec<String>) {
    let prefix = format!("{}{} ", first.indent, first.marker);
    let bare_prefix = format!("{}{}", first.indent, first.marker);
    if first.content.is_empty() {
        return (start + 1, vec![bare_prefix]);
    }

    if let Some(item) = list_item_in(first.content, &prefix) {
        let mut content = vec![item.content];
        let mut index = start + 1;
        while index < lines.len() {
            let Some(next) = line_comment(lines[index]) else {
                break;
            };
            if next.indent != first.indent
                || next.marker != first.marker
                || next.content.is_empty()
                || list_item_in(next.content, &prefix).is_some()
            {
                break;
            }
            content.push(next.content);
            index += 1;
        }
        return (
            index,
            fill_block(
                &item.first_prefix,
                &item.continuation_prefix,
                &content,
                width,
            ),
        );
    }

    let mut content = vec![first.content];
    let mut index = start + 1;
    while index < lines.len() {
        let Some(next) = line_comment(lines[index]) else {
            break;
        };
        if next.indent != first.indent
            || next.marker != first.marker
            || next.content.is_empty()
            || list_item_in(next.content, &prefix).is_some()
        {
            break;
        }
        content.push(next.content);
        index += 1;
    }
    (index, fill_block(&prefix, &prefix, &content, width))
}

#[derive(Clone, Copy)]
struct QuoteLine<'a> {
    indent: &'a str,
    marker: &'a str,
    content: &'a str,
}

/// Splits a Markdown block-quote line into its indent, its run of `>` markers,
/// and the text behind them. Nesting is part of the marker, so `> >` and `>`
/// are different leaders and never share a paragraph.
fn block_quote(line: &str) -> Option<QuoteLine<'_>> {
    let body = line.trim_start_matches([' ', '\t']);
    let indent = &line[..line.len() - body.len()];
    if !body.starts_with('>') {
        return None;
    }
    let marker_len = body
        .bytes()
        .take_while(|byte| matches!(byte, b'>' | b' ' | b'\t'))
        .count();
    Some(QuoteLine {
        indent,
        marker: body[..marker_len].trim_end(),
        content: body[marker_len..].trim(),
    })
}

/// Refills one quoted paragraph, repeating the `>` leader on every line it
/// produces. A following line without a leader is a lazy continuation, which
/// Markdown reads as part of the same quote, so it joins the paragraph and
/// comes back out with the leader made explicit.
fn reflow_quote_block(
    lines: &[&str],
    start: usize,
    first: QuoteLine<'_>,
    width: usize,
) -> (usize, Vec<String>) {
    let prefix = format!("{}{} ", first.indent, first.marker);
    if first.content.is_empty() {
        return (start + 1, vec![format!("{}{}", first.indent, first.marker)]);
    }

    let item = list_item_in(first.content, &prefix);
    let (first_prefix, continuation_prefix) = match &item {
        Some(item) => (item.first_prefix.clone(), item.continuation_prefix.clone()),
        None => (prefix.clone(), prefix.clone()),
    };
    let mut content = vec![item.map_or(first.content, |item| item.content)];

    let mut index = start + 1;
    while index < lines.len() {
        let line = lines[index];
        let next = match block_quote(line) {
            Some(quote) => {
                if quote.indent != first.indent
                    || quote.marker != first.marker
                    || quote.content.is_empty()
                {
                    break;
                }
                quote.content
            }
            None => {
                if line.trim().is_empty()
                    || markdown_fence(line).is_some()
                    || markdown_protected(line)
                {
                    break;
                }
                line.trim()
            }
        };
        if list_item_in(next, "").is_some() {
            break;
        }
        content.push(next);
        index += 1;
    }
    (
        index,
        fill_block(&first_prefix, &continuation_prefix, &content, width),
    )
}

struct ListItem<'a> {
    first_prefix: String,
    continuation_prefix: String,
    content: &'a str,
}

fn markdown_list_item(line: &str) -> Option<ListItem<'_>> {
    let body = line.trim_start_matches([' ', '\t']);
    let indent = &line[..line.len() - body.len()];
    list_item_in(body, indent)
}

fn list_item_in<'a>(body: &'a str, outer_prefix: &str) -> Option<ListItem<'a>> {
    let inner = body.trim_start_matches([' ', '\t']);
    let inner_indent = &body[..body.len() - inner.len()];
    let marker_end = if inner
        .chars()
        .next()
        .is_some_and(|character| matches!(character, '-' | '*' | '+'))
        && inner.chars().nth(1).is_some_and(char::is_whitespace)
    {
        1
    } else {
        let digits = inner.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0
            || !inner[digits..].starts_with(['.', ')'])
            || !inner[digits + 1..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
        {
            return None;
        }
        digits + 1
    };
    let marker = &inner[..marker_end];
    let first_prefix = format!("{outer_prefix}{inner_indent}{marker} ");
    let continuation_prefix = format!(
        "{outer_prefix}{inner_indent}{}",
        " ".repeat(marker.chars().count() + 1)
    );
    Some(ListItem {
        first_prefix,
        continuation_prefix,
        content: inner[marker_end..].trim(),
    })
}

fn markdown_fence(line: &str) -> Option<char> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") {
        Some('`')
    } else if trimmed.starts_with("~~~") {
        Some('~')
    } else {
        None
    }
}

fn markdown_protected(line: &str) -> bool {
    let trimmed = line.trim_start();
    let indent = line
        .chars()
        .take_while(|character| *character == ' ')
        .count();
    (trimmed.starts_with('#')
        && trimmed
            .chars()
            .find(|character| *character != '#')
            .is_some_and(char::is_whitespace))
        || indent >= 4
        || trimmed.starts_with('>')
        || trimmed.contains('|')
        || is_thematic_break(trimmed)
}

fn is_thematic_break(line: &str) -> bool {
    let compact: String = line.chars().filter(|character| *character != ' ').collect();
    compact.len() >= 3
        && compact
            .chars()
            .next()
            .is_some_and(|marker| matches!(marker, '-' | '*' | '_'))
        && compact
            .chars()
            .all(|character| character == compact.chars().next().unwrap())
}

fn fill_block(
    first_prefix: &str,
    continuation_prefix: &str,
    fragments: &[&str],
    width: usize,
) -> Vec<String> {
    let words: Vec<&str> = fragments
        .iter()
        .flat_map(|fragment| fragment.split_whitespace())
        .collect();
    if words.is_empty() {
        return vec![first_prefix.trim_end().to_owned()];
    }

    let mut output = Vec::new();
    let mut prefix = first_prefix;
    let mut line = prefix.to_owned();
    let mut line_content_len = 0;
    for word in words {
        let word_len = word.chars().count();
        let capacity = width.saturating_sub(prefix.chars().count()).max(1);
        if line_content_len > 0 && line_content_len + 1 + word_len > capacity {
            output.push(line);
            prefix = continuation_prefix;
            line = prefix.to_owned();
            line_content_len = 0;
        }
        if line_content_len > 0 {
            line.push(' ');
            line_content_len += 1;
        }
        line.push_str(word);
        line_content_len += word_len;
    }
    output.push(line);
    output
}

pub fn segment_index(line: &str, column: usize, width: usize, tab_width: usize) -> usize {
    index_in(&segments(line, width, tab_width), column)
}

fn index_in(spans: &[Segment], column: usize) -> usize {
    spans
        .iter()
        .position(|segment| column >= segment.start && column < segment.end)
        .unwrap_or_else(|| spans.len().saturating_sub(1))
}

pub fn screen_column(line: &str, column: usize, width: usize, tab_width: usize) -> usize {
    let spans = segments(line, width, tab_width);
    let segment = spans[index_in(&spans, column)];
    cell_at(line, column, tab_width).saturating_sub(segment.start_cell)
}

/// Display cells between two character columns when `start_column` is drawn
/// at cell zero.
///
/// Hard-scrolled rows use this relative origin at the terminal boundary, so a
/// tab after the clipped prefix advances to a stop in the visible row rather
/// than one derived from text that is no longer drawn.
pub fn cells_from_column(
    line: &str,
    start_column: usize,
    end_column: usize,
    tab_width: usize,
) -> usize {
    let start_column = start_column.min(line.chars().count());
    let end_column = end_column.max(start_column);
    let mut cell = 0;
    for character in line
        .chars()
        .skip(start_column)
        .take(end_column - start_column)
    {
        cell += cell_width(character, cell, tab_width.max(1));
    }
    cell
}

/// Absolute display-cell column of a character boundary in an unscrolled row.
pub fn display_column(line: &str, column: usize, tab_width: usize) -> usize {
    cell_at(line, column, tab_width)
}

pub fn move_vertical(
    buffer: &Buffer,
    position: Position,
    down: bool,
    width: usize,
    tab_width: usize,
) -> Position {
    let line = buffer.line_string(position.row);
    let current_segments = segments(&line, width, tab_width);
    let current = segment_index(&line, position.col, width, tab_width);
    let desired = screen_column(&line, position.col, width, tab_width);
    let (row, segment) = if down {
        if current + 1 < current_segments.len() {
            (position.row, current + 1)
        } else if position.row < buffer.last_row() {
            (position.row + 1, 0)
        } else {
            return position;
        }
    } else if current > 0 {
        (position.row, current - 1)
    } else if position.row > 0 {
        let row = position.row - 1;
        let count = segments(&buffer.line_string(row), width, tab_width).len();
        (row, count.saturating_sub(1))
    } else {
        return position;
    };
    let target = buffer.line_string(row);
    Position::new(
        row,
        column_for_screen(&target, segment, desired, width, tab_width),
    )
}

pub fn visible_rows(
    buffer: &Buffer,
    start_row: usize,
    start_segment: usize,
    height: usize,
    width: usize,
    tab_width: usize,
) -> Vec<VisualRow> {
    let mut result = Vec::with_capacity(height);
    let mut row = start_row.min(buffer.last_row());
    let mut segment = start_segment;
    while result.len() < height && row < buffer.len_lines() {
        let line = buffer.line_string(row);
        let spans = segments(&line, width, tab_width);
        while segment < spans.len() && result.len() < height {
            result.push(VisualRow {
                row,
                segment,
                span: spans[segment],
            });
            segment += 1;
        }
        row += 1;
        segment = 0;
    }
    result
}

/// Visual rows between a viewport start and a target, counted no further than
/// `limit`.
///
/// Every logical row between the two has to be wrapped to be counted, so an
/// unbounded walk costs a pass over the document when the caret is far below
/// the viewport. Callers only compare the result against thresholds inside one
/// screen, so a distance past `limit` is reported as soon as it is reached
/// rather than resolved exactly.
#[allow(clippy::too_many_arguments)]
pub fn visual_distance(
    buffer: &Buffer,
    start_row: usize,
    start_segment: usize,
    target_row: usize,
    target_segment: usize,
    width: usize,
    tab_width: usize,
    limit: usize,
) -> Option<usize> {
    if (target_row, target_segment) < (start_row, start_segment) {
        return None;
    }
    let mut distance = 0;
    for row in start_row..=target_row {
        let count = segments(&buffer.line_string(row), width, tab_width).len();
        let first = if row == start_row { start_segment } else { 0 };
        let last = if row == target_row {
            target_segment
        } else {
            count
        };
        distance += last.saturating_sub(first);
        if distance > limit {
            return Some(distance);
        }
    }
    Some(distance)
}

pub fn move_visual_start_backward(
    buffer: &Buffer,
    mut row: usize,
    mut segment: usize,
    mut amount: usize,
    width: usize,
    tab_width: usize,
) -> (usize, usize) {
    while amount > 0 {
        if segment > 0 {
            let step = segment.min(amount);
            segment -= step;
            amount -= step;
        } else if row > 0 {
            row -= 1;
            segment = segments(&buffer.line_string(row), width, tab_width)
                .len()
                .saturating_sub(1);
            amount -= 1;
        } else {
            break;
        }
    }
    (row, segment)
}

pub fn column_for_screen(
    line: &str,
    segment_index: usize,
    desired: usize,
    width: usize,
    tab_width: usize,
) -> usize {
    let spans = segments(line, width, tab_width);
    let segment = spans[segment_index.min(spans.len() - 1)];
    let target_cell = segment.start_cell + desired;
    let mut cell = segment.start_cell;
    for (column, character) in line
        .chars()
        .enumerate()
        .take(segment.end)
        .skip(segment.start)
    {
        let next = cell + cell_width(character, cell, tab_width);
        if next > target_cell {
            return column;
        }
        cell = next;
    }
    segment.end
}

fn cell_at(line: &str, column: usize, tab_width: usize) -> usize {
    let tab_width = tab_width.max(1);
    let mut cell = 0;
    for character in line.chars().take(column) {
        cell += cell_width(character, cell, tab_width);
    }
    cell
}

/// Converts a display-cell offset from a character boundary back to a
/// character column. Clicking inside a wide glyph resolves to that glyph;
/// positions past the row clamp to its end.
pub fn column_for_cell_from(
    line: &str,
    start_column: usize,
    screen_cell: usize,
    tab_width: usize,
) -> usize {
    let start_column = start_column.min(line.chars().count());
    let start_cell = cell_at(line, start_column, tab_width);
    let target = start_cell.saturating_add(screen_cell);
    let mut cell = start_cell;
    for (column, character) in line.chars().enumerate().skip(start_column) {
        let next = cell.saturating_add(cell_width(character, cell, tab_width.max(1)));
        if target < next {
            return column;
        }
        cell = next;
    }
    line.chars().count()
}

fn cell_width(character: char, cell: usize, tab_width: usize) -> usize {
    let tab_width = tab_width.max(1);
    if character == '\t' {
        tab_width - cell % tab_width
    } else {
        UnicodeWidthChar::width(character).unwrap_or(0).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{buffer::Buffer, text::Transaction};

    #[test]
    fn segments_respect_unicode_width_and_tab_stops() {
        assert_eq!(
            segments("ab界cd", 4, 4),
            vec![
                Segment {
                    start: 0,
                    end: 3,
                    start_cell: 0,
                    end_cell: 4,
                },
                Segment {
                    start: 3,
                    end: 5,
                    start_cell: 4,
                    end_cell: 6,
                },
            ]
        );
        assert_eq!(segments("a\tb", 4, 4).len(), 2);
        assert_eq!(segments("\t", 4, 0).len(), 1);
        assert_eq!(column_for_cell_from("\t", 0, 1, 0), 1);
    }

    #[test]
    fn segments_wrap_at_words_and_split_only_overlong_words() {
        let wrapped = segments("alpha beta gamma", 10, 4);
        assert_eq!(
            wrapped
                .iter()
                .map(|span| (span.start, span.end))
                .collect::<Vec<_>>(),
            vec![(0, 10), (10, 16)]
        );

        let overlong = segments("ab abcdef", 4, 4);
        assert_eq!(
            overlong
                .iter()
                .map(|span| (span.start, span.end))
                .collect::<Vec<_>>(),
            vec![(0, 3), (3, 7), (7, 9)]
        );
    }

    #[test]
    fn hard_wrap_uses_word_boundaries_and_preserves_existing_newlines() {
        assert_eq!(
            hard_wrap("alpha beta gamma\nnext", 10),
            "alpha beta\ngamma\nnext"
        );
        assert_eq!(hard_wrap("abcdefghij", 4), "abcd\nefgh\nij");
        assert_eq!(hard_wrap("zażółć gęślą", 6), "zażółć\ngęślą");
        assert_eq!(hard_wrap("abcd\r\nnext\r\n", 3), "abc\r\nd\r\nnex\r\nt\r\n");
    }

    #[test]
    fn join_lines_replaces_breaks_and_the_whitespace_around_them() {
        assert_eq!(join_lines("alpha\nbeta\ngamma", ""), "alphabetagamma");
        assert_eq!(join_lines("alpha\nbeta\ngamma", " "), "alpha beta gamma");
        assert_eq!(join_lines("alpha\nbeta", ", "), "alpha, beta");
        assert_eq!(
            join_lines("    let first = 1;\n    let second = 2;", " "),
            "    let first = 1; let second = 2;"
        );
        assert_eq!(join_lines("alpha   \n\tbeta", ""), "alphabeta");
        assert_eq!(join_lines("zażółć\ngęślą", "-"), "zażółć-gęślą");
    }

    #[test]
    fn join_lines_keeps_the_outer_edges_of_the_text_it_is_given() {
        // Leading indentation and trailing whitespace do not adjoin a removed
        // break, so they stay.
        assert_eq!(join_lines("  alpha\nbeta  ", " "), "  alpha beta  ");
        assert_eq!(join_lines("alpha\r\nbeta", " "), "alpha beta");
        // Every break goes, including one ending the text: the caller hands
        // over exactly the span it means to join.
        assert_eq!(join_lines("alpha\nbeta\n", " "), "alpha beta ");
        assert_eq!(join_lines("alpha\n", "-"), "alpha-");
    }

    #[test]
    fn join_lines_returns_text_without_a_break_unchanged() {
        assert_eq!(join_lines("  alpha  ", " "), "  alpha  ");
        assert_eq!(join_lines("", " "), "");
        // A blank line is still a line: joining one contributes an empty piece
        // rather than being skipped.
        assert_eq!(join_lines("alpha\n\nbeta", "-"), "alpha--beta");
    }

    #[test]
    fn reflow_fills_paragraphs_without_crossing_blank_lines() {
        assert_eq!(
            reflow(
                "alpha beta\ngamma delta\n\nsecond paragraph is here",
                12,
                ReflowKind::Plain
            ),
            "alpha beta\ngamma delta\n\nsecond\nparagraph is\nhere"
        );
    }

    #[test]
    fn reflow_keeps_markdown_lists_and_protects_non_prose_blocks() {
        let source = "# Heading\n\n- alpha beta\n  gamma delta epsilon\n- second item stays separate\n\n1. numbered item with more words\n2. next item\n\n```rust\nlet untouched = true;\n```";
        assert_eq!(
            reflow(source, 18, ReflowKind::Markdown),
            "# Heading\n\n- alpha beta gamma\n  delta epsilon\n- second item\n  stays separate\n\n1. numbered item\n   with more words\n2. next item\n\n```rust\nlet untouched = true;\n```"
        );
    }

    #[test]
    fn reflow_refills_block_quotes_and_repeats_their_leader() {
        assert_eq!(
            reflow(
                "> alpha beta gamma delta\n  epsilon zeta\n\nafter",
                14,
                ReflowKind::Markdown
            ),
            "> alpha beta\n> gamma delta\n> epsilon zeta\n\nafter"
        );
        assert_eq!(
            reflow("> alpha beta\n>\n> gamma delta", 14, ReflowKind::Markdown),
            "> alpha beta\n>\n> gamma delta"
        );
        assert_eq!(
            reflow(
                "> - first item with words\n> - second stays separate\n> > nested quote text",
                18,
                ReflowKind::Markdown
            ),
            "> - first item\n>   with words\n> - second stays\n>   separate\n> > nested quote\n> > text"
        );
    }

    #[test]
    fn reflow_repeats_source_comment_leaders_and_keeps_comment_paragraphs() {
        let source = "    // alpha beta\n    // gamma delta epsilon\n    //\n    // - first list item has words\n    // - second stays separate\ncode();\n# shell comments can\n# also be filled";
        assert_eq!(
            reflow(source, 24, ReflowKind::Plain),
            "    // alpha beta gamma\n    // delta epsilon\n    //\n    // - first list item\n    //   has words\n    // - second stays\n    //   separate\ncode();\n# shell comments can\n# also be filled"
        );
    }

    #[test]
    fn reflow_preserves_consistent_crlf_line_endings() {
        assert_eq!(
            reflow(
                "// alpha beta\r\n// gamma delta epsilon\r\n",
                18,
                ReflowKind::Source,
            ),
            "// alpha beta\r\n// gamma delta\r\n// epsilon\r\n"
        );
    }

    #[test]
    fn source_reflow_changes_comments_without_joining_code_lines() {
        assert_eq!(
            reflow(
                "let first = 1;\nlet second = 2;\n// prose on one line\n// continues on another",
                30,
                ReflowKind::Source,
            ),
            "let first = 1;\nlet second = 2;\n// prose on one line continues".to_owned()
                + "\n// on another"
        );
    }

    #[test]
    fn vertical_motion_crosses_wrapped_and_logical_lines() {
        let mut buffer = Buffer::scratch();
        buffer.apply(&Transaction::insert(0, "abcdef\nxy"));
        assert_eq!(
            move_vertical(&buffer, Position::new(0, 1), true, 3, 4),
            Position::new(0, 4)
        );
        assert_eq!(
            move_vertical(&buffer, Position::new(0, 4), true, 3, 4),
            Position::new(1, 1)
        );
        assert_eq!(
            move_vertical(&buffer, Position::new(1, 1), false, 3, 4),
            Position::new(0, 4)
        );
    }

    #[test]
    fn visible_rows_can_start_inside_one_long_line() {
        let mut buffer = Buffer::scratch();
        buffer.apply(&Transaction::insert(0, "abcdefgh\nlast"));
        let rows = visible_rows(&buffer, 0, 1, 3, 3, 4);
        assert_eq!(
            rows.iter()
                .map(|row| (row.row, row.segment, row.span.start, row.span.end))
                .collect::<Vec<_>>(),
            [(0, 1, 3, 6), (0, 2, 6, 8), (1, 0, 0, 3)]
        );
    }
}
