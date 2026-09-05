// SPDX-License-Identifier: MPL-2.0

//! Markdown source rendered as a read-only page.
//!
//! The editor never conceals a character of a document being edited: a row and
//! a column mean the same thing at every pane size, and hiding `**` would move
//! every offset after it. Rendering therefore produces a separate page — plain
//! text plus the semantic spans over it — the way help, the manual, and the
//! about page already do. The markers are gone from that page because they
//! were never written into it, not because anything is hiding them.
//!
//! This module owns what Markdown *means* for a reader: which parts are
//! emphasis, which are structure, and what a list marker or a table column
//! looks like once it is being read rather than written. It knows nothing
//! about colour, terminal attributes, buffers, or panes. Every part it
//! recognises is named with a scope from [`crate::syntax::SCOPES`], so the
//! rendered page is themed by the same entries as a highlighted document.

use crate::{
    row_hints::display_cells,
    syntax::{Scope, Span},
};

/// Cells a horizontal rule is drawn with.
///
/// A rule separates sections rather than measuring the pane, and the page is
/// generated once for every width it will be shown at, so it cannot ask how
/// wide the pane is. This is narrow enough to fit a split.
const RULE_WIDTH: usize = 40;

/// Columns a code block is indented by on the page.
const CODE_INDENT: &str = "  ";

/// Columns the source spells an unfenced code block with.
const SOURCE_CODE_INDENT: usize = 4;

/// The most characters an inline scan looks ahead for a closing marker.
///
/// Every inline construct is closed by a marker somewhere to its right, and a
/// scan for one that is not there runs to the end of the paragraph. An opening
/// marker that never closes is ordinary text, but finding that out costs the
/// whole remaining length, so a document that is mostly unmatched `[` or
/// backticks would pay that for each of them in turn — quadratic work on the
/// main loop, which is a stalled editor rather than a slow page. A construct
/// whose closing marker is further away than this is read as literal text: no
/// emphasis, code span, or link a person wrote is this long, and one that is
/// was not going to read as one anyway.
const INLINE_SCAN_LIMIT: usize = 2048;

/// Plain page text together with its non-overlapping semantic spans.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RenderedMarkdown {
    text: String,
    spans: Vec<Span>,
}

impl RenderedMarkdown {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn spans(&self) -> &[Span] {
        &self.spans
    }
}

/// The scopes a rendered page paints with, resolved once.
///
/// Every name here is in [`crate::syntax::SCOPES`]; resolving them together
/// keeps the failure at one place rather than at each use.
#[derive(Clone, Copy)]
struct Palette {
    bold: Scope,
    heading: Scope,
    italic: Scope,
    link_text: Scope,
    link_url: Scope,
    list: Scope,
    quote: Scope,
    raw: Scope,
    strikethrough: Scope,
    /// Front matter, HTML comments, and rules: present in the source, but not
    /// prose. Every theme dims this one.
    aside: Scope,
}

impl Palette {
    fn new() -> Self {
        let scope = |name: &str| Scope::named(name).expect("rendered Markdown uses known scopes");
        Self {
            bold: scope("markup.bold"),
            heading: scope("markup.heading"),
            italic: scope("markup.italic"),
            link_text: scope("markup.link.text"),
            link_url: scope("markup.link.url"),
            list: scope("markup.list"),
            quote: scope("markup.quote"),
            raw: scope("markup.raw"),
            strikethrough: scope("markup.strikethrough"),
            aside: scope("comment"),
        }
    }
}

/// A run of rendered text and what it means.
#[derive(Clone, Debug)]
struct Piece {
    text: String,
    scope: Option<Scope>,
}

/// The page under construction, in character offsets rather than bytes.
#[derive(Default)]
struct Page {
    text: String,
    spans: Vec<Span>,
    chars: usize,
}

impl Page {
    fn push(&mut self, text: &str, scope: Option<Scope>) {
        if text.is_empty() {
            return;
        }
        let from = self.chars;
        self.text.push_str(text);
        self.chars += text.chars().count();
        let Some(scope) = scope else {
            return;
        };
        // Adjacent runs of one meaning are one span. Nothing downstream reads
        // the count, but a span per character would make the clip the frontend
        // performs on every frame proportional to the page rather than to the
        // rows on screen.
        if let Some(last) = self.spans.last_mut()
            && last.scope == scope
            && last.to == from
        {
            last.to = self.chars;
            return;
        }
        self.spans.push(Span {
            from,
            to: self.chars,
            scope,
        });
    }

    fn pieces(&mut self, pieces: &[Piece]) {
        for piece in pieces {
            self.push(&piece.text, piece.scope);
        }
    }

    fn repeat(&mut self, character: char, count: usize, scope: Option<Scope>) {
        self.push(&character.to_string().repeat(count), scope);
    }

    fn newline(&mut self) {
        self.push("\n", None);
    }

    /// Ends the current row and leaves one blank row, unless the page is empty
    /// or already ends in one.
    fn blank_line(&mut self) {
        if self.text.is_empty() {
            return;
        }
        if !self.text.ends_with('\n') {
            self.newline();
        }
        if !self.text.ends_with("\n\n") {
            self.newline();
        }
    }
}

/// Renders Markdown source as a page to read.
///
/// Structure that carries meaning is kept and given a scope; the characters
/// that only announce that structure to a parser — emphasis runs, backticks,
/// heading hashes, link brackets — are not written to the page at all.
/// Anything unrecognised is passed through as ordinary text, so a document
/// this module does not fully understand still reads as its own source.
pub fn render(source: &str) -> RenderedMarkdown {
    let palette = Palette::new();
    let lines = source
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect::<Vec<_>>();
    let mut page = Page::default();
    let mut index = 0;
    // A four-space indent is a code block after a paragraph and a continuation
    // line inside a list, and the two are told apart only by what came before.
    let mut in_list = false;

    if let Some(end) = front_matter_end(&lines) {
        for line in &lines[1..end] {
            page.push(line, Some(palette.aside));
            page.newline();
        }
        page.blank_line();
        index = end + 1;
    }

    while index < lines.len() {
        let line = lines[index];
        index += 1;

        if let Some(fence) = code_fence(line) {
            page.blank_line();
            while index < lines.len() && !closes_fence(lines[index], fence) {
                page.push(CODE_INDENT, None);
                page.push(lines[index], Some(palette.raw));
                page.newline();
                index += 1;
            }
            // A document may end inside an unclosed fence; there is simply no
            // closing line to step over then.
            index += usize::from(index < lines.len());
            page.blank_line();
            in_list = false;
            continue;
        }

        if line.trim().is_empty() {
            page.blank_line();
            continue;
        }

        if let Some((level, text)) = atx_heading(line) {
            page.blank_line();
            let rendered = inline(text, Some(palette.heading), palette);
            page.pieces(&rendered);
            page.newline();
            if level == 1 {
                page.repeat('─', width_of(&rendered), Some(palette.heading));
                page.newline();
            }
            page.blank_line();
            in_list = false;
            continue;
        }

        if is_thematic_break(line) {
            page.blank_line();
            page.repeat('─', RULE_WIDTH, Some(palette.aside));
            page.newline();
            page.blank_line();
            in_list = false;
            continue;
        }

        if let Some((depth, text)) = block_quote(line) {
            for _ in 0..depth {
                page.push("▌ ", Some(palette.quote));
            }
            page.pieces(&inline(text, Some(palette.quote), palette));
            page.newline();
            in_list = false;
            continue;
        }

        if let Some(item) = list_item(line) {
            page.push(&" ".repeat(item.indent), None);
            page.push(&item.marker, Some(palette.list));
            page.push(" ", None);
            page.pieces(&inline(&item.text, None, palette));
            page.newline();
            in_list = true;
            continue;
        }

        if line.starts_with('|') || line.trim_start().starts_with('|') {
            let mut rows = vec![line];
            while index < lines.len() && lines[index].trim_start().starts_with('|') {
                rows.push(lines[index]);
                index += 1;
            }
            write_table(&mut page, &rows, palette);
            in_list = false;
            continue;
        }

        // An indented block is code only where a code block can start. Inside
        // a list the same indentation is the rest of an item, and rendering it
        // as a literal would lose the emphasis in it.
        if !in_list && is_indented_code(line) && starts_block(&lines, index - 1) {
            page.blank_line();
            let start = index - 1;
            let mut last = start;
            for (offset, candidate) in lines.iter().enumerate().skip(start) {
                if !is_indented_code(candidate) && !candidate.trim().is_empty() {
                    break;
                }
                last = offset;
            }
            for candidate in &lines[start..=last] {
                if candidate.trim().is_empty() {
                    page.newline();
                    continue;
                }
                // The four columns that made this a code block are the
                // source's way of saying so, exactly as a fence is; the page
                // says it with its own indent instead. Anything past them is
                // the block's own shape and is kept.
                page.push(CODE_INDENT, None);
                page.push(
                    candidate[SOURCE_CODE_INDENT..].trim_end(),
                    Some(palette.raw),
                );
                page.newline();
            }
            index = last + 1;
            page.blank_line();
            continue;
        }

        if is_html_comment(line) {
            page.push(line.trim_end(), Some(palette.aside));
            page.newline();
            continue;
        }

        // A paragraph line underlined by `===` or `---` is a heading, and the
        // underline is consumed rather than drawn: it belongs to the source's
        // spelling of the heading, not to the page.
        if let Some(level) = setext_underline(lines.get(index).copied()) {
            page.blank_line();
            let rendered = inline(line.trim(), Some(palette.heading), palette);
            page.pieces(&rendered);
            page.newline();
            if level == 1 {
                page.repeat('─', width_of(&rendered), Some(palette.heading));
                page.newline();
            }
            page.blank_line();
            index += 1;
            in_list = false;
            continue;
        }

        // A paragraph is rendered as one run rather than a line at a time.
        // `**a\nb**` is one emphasis in the source, and rendering each line on
        // its own would leave the markers of a run that never closes.
        let start = index - 1;
        while index < lines.len() && continues_paragraph(lines[index]) {
            index += 1;
        }
        let (indent, _) = split_indent(line);
        let keep_indent = in_list;
        page.push(&" ".repeat(if keep_indent { indent } else { 0 }), None);
        let paragraph = lines[start..index]
            .iter()
            .enumerate()
            .map(|(offset, line)| {
                if offset > 0 && keep_indent {
                    line.trim_end().to_owned()
                } else {
                    line.trim().to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        page.pieces(&inline(&paragraph, None, palette));
        page.newline();
    }

    while page.text.ends_with("\n\n") {
        page.text.pop();
        page.chars -= 1;
    }

    RenderedMarkdown {
        text: page.text,
        spans: page.spans,
    }
}

/// Whether a line carries on the paragraph above it rather than opening a
/// block of its own.
fn continues_paragraph(line: &str) -> bool {
    !line.trim().is_empty()
        && code_fence(line).is_none()
        && atx_heading(line).is_none()
        && !is_thematic_break(line)
        && block_quote(line).is_none()
        && list_item(line).is_none()
        && !line.trim_start().starts_with('|')
        && !is_html_comment(line)
        && setext_underline(Some(line)).is_none()
}

/// The line index of a YAML front-matter block's closing fence.
fn front_matter_end(lines: &[&str]) -> Option<usize> {
    if lines.first()?.trim_end() != "---" {
        return None;
    }
    lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, line)| line.trim_end() == "---")
        .map(|(index, _)| index)
}

/// The fence character and length a fenced code block opens with.
fn code_fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let character = trimmed.chars().next()?;
    if character != '`' && character != '~' {
        return None;
    }
    let length = trimmed
        .chars()
        .take_while(|value| *value == character)
        .count();
    (length >= 3).then_some((character, length))
}

fn closes_fence(line: &str, fence: (char, usize)) -> bool {
    let (character, length) = fence;
    let trimmed = line.trim();
    trimmed.chars().all(|value| value == character)
        && trimmed.chars().count() >= length
        && !trimmed.is_empty()
}

/// The level and text of an ATX heading.
fn atx_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let level = trimmed.chars().take_while(|value| *value == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim_start().trim_end();
    // A closing run of hashes is decoration in the source rather than text.
    let text = text.trim_end_matches('#').trim_end();
    Some((level, text))
}

fn is_thematic_break(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    if !matches!(first, '-' | '*' | '_') {
        return false;
    }
    trimmed.chars().filter(|value| *value == first).count() >= 3
        && trimmed
            .chars()
            .all(|value| value == first || value == ' ' || value == '\t')
}

/// The heading level a setext underline gives the line above it.
fn setext_underline(line: Option<&str>) -> Option<usize> {
    let trimmed = line?.trim();
    if trimmed.len() < 2 {
        return None;
    }
    if trimmed.chars().all(|value| value == '=') {
        return Some(1);
    }
    trimmed.chars().all(|value| value == '-').then_some(2)
}

/// The nesting depth and remaining text of a block quote line.
fn block_quote(line: &str) -> Option<(usize, &str)> {
    let mut rest = line.trim_start();
    if !rest.starts_with('>') {
        return None;
    }
    let mut depth = 0;
    while let Some(remainder) = rest.strip_prefix('>') {
        depth += 1;
        rest = remainder.strip_prefix(' ').unwrap_or(remainder);
        rest = rest.trim_start_matches(' ');
    }
    Some((depth, rest))
}

struct ListItem {
    indent: usize,
    marker: String,
    text: String,
}

/// The parts of a list item, with its bullet already chosen.
fn list_item(line: &str) -> Option<ListItem> {
    let (indent, rest) = split_indent(line);
    let (marker, text) = if let Some(text) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        // Depth is spelled with the bullet rather than only with the indent,
        // so a nested item reads as nested in a pane too narrow to show the
        // indentation of the item above it.
        let bullet = match indent / 2 {
            0 => '•',
            1 => '◦',
            _ => '▪',
        };
        (bullet.to_string(), text)
    } else {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let delimiter = rest[digits..].chars().next()?;
        if !matches!(delimiter, '.' | ')') {
            return None;
        }
        let text = rest[digits + 1..].strip_prefix(' ')?;
        (format!("{}{delimiter}", &rest[..digits]), text)
    };

    // A task box is the item's state, so it stays in the marker column where
    // the reader is already looking rather than inside the text.
    let (marker, text) = match text.get(..4) {
        Some("[ ] ") => (format!("{marker} ☐"), &text[4..]),
        Some("[x] ") | Some("[X] ") => (format!("{marker} ☑"), &text[4..]),
        _ => (marker, text),
    };

    Some(ListItem {
        indent,
        marker,
        text: text.to_owned(),
    })
}

fn split_indent(line: &str) -> (usize, &str) {
    let trimmed = line.trim_start_matches(' ');
    (line.len() - trimmed.len(), trimmed.trim_end())
}

fn is_indented_code(line: &str) -> bool {
    line.starts_with("    ") && !line.trim().is_empty()
}

/// Whether a line begins a block rather than continuing the one above it.
fn starts_block(lines: &[&str], index: usize) -> bool {
    index == 0 || lines[index - 1].trim().is_empty()
}

fn is_html_comment(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("<!--") || trimmed.starts_with("-->")
}

fn width_of(pieces: &[Piece]) -> usize {
    pieces.iter().map(|piece| display_cells(&piece.text)).sum()
}

/// Renders a run of table rows with its columns aligned.
///
/// The widths are measured on the rendered cells rather than on the source,
/// because that is what the reader sees: a cell spelled `**yes**` occupies
/// three columns on the page, not seven.
fn write_table(page: &mut Page, rows: &[&str], palette: Palette) {
    let parsed = rows.iter().map(|row| table_cells(row)).collect::<Vec<_>>();
    let delimiter = parsed.iter().position(|cells| is_table_delimiter(cells));
    let Some(delimiter) = delimiter.filter(|index| *index == 1) else {
        // Not a table after all — pipes in ordinary prose. Render the lines as
        // what they are rather than inventing columns for them.
        for row in rows {
            page.pieces(&inline(row.trim(), None, palette));
            page.newline();
        }
        return;
    };

    let rendered = parsed
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != delimiter)
        .map(|(index, cells)| {
            cells
                .iter()
                .map(|cell| {
                    let base = (index == 0).then_some(palette.heading);
                    inline(cell, base, palette)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let columns = rendered.iter().map(Vec::len).max().unwrap_or_default();
    let widths = (0..columns)
        .map(|column| {
            rendered
                .iter()
                .filter_map(|row| row.get(column))
                .map(|cell| width_of(cell))
                .max()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();

    page.blank_line();
    for (index, row) in rendered.iter().enumerate() {
        for (column, width) in widths.iter().enumerate() {
            if column > 0 {
                page.push(" │ ", Some(palette.aside));
            }
            let cell = row.get(column);
            page.pieces(cell.map_or(&[][..], Vec::as_slice));
            let used = cell.map_or(0, |cell| width_of(cell));
            // The last column is not padded: trailing blanks would be text a
            // caret could sit past, and nothing is drawn to their right.
            if column + 1 < widths.len() {
                page.push(&" ".repeat(width.saturating_sub(used)), None);
            }
        }
        page.newline();
        if index == 0 {
            for (column, width) in widths.iter().enumerate() {
                if column > 0 {
                    page.push("─┼─", Some(palette.aside));
                }
                page.repeat('─', *width, Some(palette.aside));
            }
            page.newline();
        }
    }
    page.blank_line();
}

/// The cells of one table row, with the outer pipes dropped.
fn table_cells(row: &str) -> Vec<String> {
    let trimmed = row.trim();
    let trimmed = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix('|').unwrap_or(trimmed);
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for character in trimmed.chars() {
        match character {
            '\\' if !escaped => escaped = true,
            '|' if !escaped => cells.push(std::mem::take(&mut current)),
            _ => {
                if escaped && character != '|' {
                    current.push('\\');
                }
                escaped = false;
                current.push(character);
            }
        }
    }
    cells.push(current);
    cells.iter().map(|cell| cell.trim().to_owned()).collect()
}

fn is_table_delimiter(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let trimmed = cell.trim();
            trimmed.len() >= 3
                && trimmed
                    .chars()
                    .all(|value| matches!(value, '-' | ':' | ' '))
                && trimmed.contains('-')
        })
}

/// Renders the inline Markdown in `source`, with `base` as the meaning of
/// everything not claimed by a narrower one.
fn inline(source: &str, base: Option<Scope>, palette: Palette) -> Vec<Piece> {
    let characters = source.chars().collect::<Vec<_>>();
    let mut pieces = Vec::new();
    let mut plain = String::new();
    let mut index = 0;

    while index < characters.len() {
        let character = characters[index];
        let consumed = match character {
            '\\' if index + 1 < characters.len() && is_escapable(characters[index + 1]) => {
                plain.push(characters[index + 1]);
                Some(2)
            }
            '`' => code_span(&characters, index).map(|(text, length)| {
                flush(&mut pieces, &mut plain, base);
                pieces.push(Piece {
                    text,
                    scope: Some(palette.raw),
                });
                length
            }),
            '~' => delimited(&characters, index, '~', 2).map(|(content, length)| {
                flush(&mut pieces, &mut plain, base);
                pieces.extend(inline(&content, Some(palette.strikethrough), palette));
                length
            }),
            '*' | '_' => emphasis(&characters, index).map(|(content, run, length)| {
                flush(&mut pieces, &mut plain, base);
                // Three markers are bold and italic at once. The page carries
                // one meaning per character, so it keeps the stronger of the
                // two rather than silently dropping both. A longer run has to
                // close with the same number of markers, so it either means
                // one of these two or is not emphasis at all.
                let scope = if run == 1 {
                    palette.italic
                } else {
                    palette.bold
                };
                pieces.extend(inline(&content, Some(scope), palette));
                length
            }),
            '[' | '!' => link(&characters, index).map(|parsed| {
                flush(&mut pieces, &mut plain, base);
                // A link's destination is something a reader can act on; a
                // picture's is a file this terminal will not be showing, and
                // printing it buries the description that stands in for it.
                // Both spellings of a picture are read the same way: `!` says
                // so outright, and a plain link at an image file is one too,
                // which is how a pasted `[Image 1](…/1f0a.png)` reads as
                // `Image 1` rather than as its cache path.
                let picture = parsed.image || names_an_image(&parsed.url);
                // The description is all that survives, so it is emphasised
                // rather than left looking like the prose around it: bold is
                // what stands in for a picture that cannot be drawn.
                let scope = if picture {
                    palette.bold
                } else {
                    palette.link_text
                };
                if picture && parsed.text.is_empty() {
                    // `![](diagram.png)` describes itself with nothing at all.
                    // Dropping the destination too would leave the page with
                    // no trace that a picture is here, so the file name stands
                    // in for the description nobody wrote. It is pushed
                    // whole rather than read as inline markup, because it is a
                    // file name and a `*` in one is part of the name.
                    pieces.push(Piece {
                        text: destination_name(&parsed.url).to_owned(),
                        scope: Some(scope),
                    });
                } else {
                    pieces.extend(inline(&parsed.text, Some(scope), palette));
                }
                if !picture && !parsed.url.is_empty() {
                    pieces.push(Piece {
                        text: format!(" ({})", parsed.url),
                        scope: Some(palette.link_url),
                    });
                }
                parsed.length
            }),
            '<' => autolink(&characters, index).map(|(url, length)| {
                flush(&mut pieces, &mut plain, base);
                pieces.push(Piece {
                    text: url,
                    scope: Some(palette.link_url),
                });
                length
            }),
            _ => None,
        };
        match consumed {
            Some(length) => index += length,
            None => {
                plain.push(character);
                index += 1;
            }
        }
    }

    flush(&mut pieces, &mut plain, base);
    pieces
}

fn flush(pieces: &mut Vec<Piece>, plain: &mut String, base: Option<Scope>) {
    if plain.is_empty() {
        return;
    }
    pieces.push(Piece {
        text: std::mem::take(plain),
        scope: base,
    });
}

/// Markdown's escapable punctuation. Anything else after a backslash is a
/// backslash followed by that character, as it is in the source.
fn is_escapable(character: char) -> bool {
    "\\`*_{}[]()#+-.!|<>~".contains(character)
}

/// The contents and source length of a code span starting at `index`.
fn code_span(characters: &[char], index: usize) -> Option<(String, usize)> {
    let run = characters[index..]
        .iter()
        .take_while(|value| **value == '`')
        .count();
    let mut cursor = index + run;
    let limit = characters.len().min(index + INLINE_SCAN_LIMIT);
    while cursor < limit {
        if characters[cursor] != '`' {
            cursor += 1;
            continue;
        }
        let closing = characters[cursor..]
            .iter()
            .take_while(|value| **value == '`')
            .count();
        if closing == run {
            let content = characters[index + run..cursor].iter().collect::<String>();
            // One space on each side is padding that lets a span hold a
            // backtick, not content.
            let content = match content
                .strip_prefix(' ')
                .and_then(|rest| rest.strip_suffix(' '))
            {
                Some(stripped) if !stripped.trim().is_empty() => stripped.to_owned(),
                _ => content,
            };
            return Some((content, cursor + closing - index));
        }
        cursor += closing;
    }
    None
}

/// The contents and source length of a run delimited by `count` copies of
/// `marker` on both sides.
fn delimited(
    characters: &[char],
    index: usize,
    marker: char,
    count: usize,
) -> Option<(String, usize)> {
    let opening = characters[index..]
        .iter()
        .take_while(|value| **value == marker)
        .count();
    if opening < count {
        return None;
    }
    let start = index + count;
    let mut cursor = start;
    let limit = characters.len().min(index + INLINE_SCAN_LIMIT);
    while cursor + count <= limit {
        if characters[cursor..cursor + count]
            .iter()
            .all(|value| *value == marker)
        {
            if cursor == start {
                return None;
            }
            let content = characters[start..cursor].iter().collect::<String>();
            return Some((content, cursor + count - index));
        }
        cursor += 1;
    }
    None
}

/// The contents, marker run, and source length of an emphasis span.
fn emphasis(characters: &[char], index: usize) -> Option<(String, usize, usize)> {
    let marker = characters[index];
    let run = characters[index..]
        .iter()
        .take_while(|value| **value == marker)
        .count();
    // An underscore inside a word is part of the word. `snake_case_name` is
    // one identifier, and reading its middle as emphasis would both restyle it
    // and delete the underscores.
    if marker == '_' {
        let before = index.checked_sub(1).map(|previous| characters[previous]);
        if before.is_some_and(|value| value.is_alphanumeric()) {
            return None;
        }
    }
    if characters
        .get(index + run)
        .is_none_or(|value| value.is_whitespace())
    {
        return None;
    }
    let (content, length) = delimited(characters, index, marker, run)?;
    if content.starts_with(char::is_whitespace) || content.ends_with(char::is_whitespace) {
        return None;
    }
    if marker == '_'
        && characters
            .get(index + length)
            .is_some_and(|value| value.is_alphanumeric())
    {
        return None;
    }
    Some((content, run, length))
}

struct ParsedLink {
    text: String,
    url: String,
    image: bool,
    length: usize,
}

/// An inline link or image starting at `index`.
fn link(characters: &[char], index: usize) -> Option<ParsedLink> {
    let image = characters[index] == '!';
    let start = if image {
        if characters.get(index + 1) != Some(&'[') {
            return None;
        }
        index + 1
    } else {
        index
    };
    let mut depth = 0;
    let mut close = None;
    for (cursor, character) in characters
        .iter()
        .enumerate()
        .skip(start)
        .take(INLINE_SCAN_LIMIT)
    {
        match character {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(cursor);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    if characters.get(close + 1) != Some(&'(') {
        return None;
    }
    let open = close + 2;
    let scan_from = |from: usize, wanted: char| {
        characters[from..]
            .iter()
            .take(INLINE_SCAN_LIMIT)
            .position(|value| *value == wanted)
            .map(|offset| from + offset)
    };
    // A destination wrapped in angle brackets ends at its closing bracket, so
    // a space or a parenthesis inside it is part of the path rather than the
    // end of the link. That is the only spelling a path containing either can
    // be written in, and the one Runyte writes when it has to.
    //
    // The bracket has to be closed by the link itself: `>` must be followed
    // immediately by the `)`, and no second `<` may appear inside. Without
    // both, `[a](<x.png) and 5 > 3 (done)` would find the `>` of a comparison
    // further along the line and swallow everything up to a later `)` as one
    // enormous destination. A `<` that does not close this way is not a
    // bracketed destination and not a link, which is what CommonMark says too.
    let (url, end) = if characters.get(open) == Some(&'<') {
        let bracket = scan_from(open + 1, '>')?;
        if characters.get(bracket + 1) != Some(&')') || characters[open + 1..bracket].contains(&'<')
        {
            return None;
        }
        (
            characters[open + 1..bracket].iter().collect::<String>(),
            bracket + 1,
        )
    } else {
        let end = scan_from(open, ')')?;
        let target = characters[open..end].iter().collect::<String>();
        // A title after the destination is help for a mouse that is not here.
        let url = target
            .split_once(char::is_whitespace)
            .map_or(target.as_str(), |(url, _)| url)
            .trim()
            .to_owned();
        (url, end)
    };
    let text = characters[start + 1..close].iter().collect::<String>();
    Some(ParsedLink {
        text,
        url,
        image,
        length: end + 1 - index,
    })
}

/// File extensions a link destination is read as a picture by.
const IMAGE_EXTENSIONS: &[&str] = &[
    "apng", "avif", "bmp", "gif", "ico", "jpeg", "jpg", "png", "svg", "tif", "tiff", "webp",
];

/// The final path segment of a link destination.
///
/// A query or fragment belongs to the request rather than to the name, and
/// sits after the extension it would otherwise hide.
fn destination_name(url: &str) -> &str {
    url.split(['?', '#'])
        .next()
        .unwrap_or(url)
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(url)
}

/// Whether a link destination names an image file.
///
/// This is the extension and nothing else. Reading the file to find out would
/// make rendering a page depend on what is on disk, and a destination that is
/// a URL is not on this disk at all; a name ending in `.png` is the only thing
/// both cases share.
fn names_an_image(url: &str) -> bool {
    let Some((stem, extension)) = destination_name(url).rsplit_once('.') else {
        return false;
    };
    // A bare `.png` is a hidden file named for the format, not a picture
    // called nothing at all.
    !stem.is_empty()
        && IMAGE_EXTENSIONS
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
}

/// A bare `<https://…>` link.
fn autolink(characters: &[char], index: usize) -> Option<(String, usize)> {
    let end = characters[index + 1..]
        .iter()
        .take(INLINE_SCAN_LIMIT)
        .position(|value| *value == '>')
        .map(|offset| index + 1 + offset)?;
    let content = characters[index + 1..end].iter().collect::<String>();
    let scheme = content.split_once("://").map(|(scheme, _)| scheme);
    let looks_like_a_link = scheme.is_some_and(|scheme| {
        !scheme.is_empty() && scheme.chars().all(|value| value.is_ascii_alphanumeric())
    }) || content.starts_with("mailto:");
    looks_like_a_link.then(|| (content, end + 1 - index))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of every span with the given scope name, in order.
    fn scoped<'a>(rendered: &'a RenderedMarkdown, name: &str) -> Vec<&'a str> {
        let scope = Scope::named(name).unwrap();
        let characters = rendered.text().chars().collect::<Vec<_>>();
        rendered
            .spans()
            .iter()
            .filter(|span| span.scope == scope)
            .map(|span| {
                let start = characters[..span.from].iter().collect::<String>().len();
                let end = characters[..span.to].iter().collect::<String>().len();
                &rendered.text()[start..end]
            })
            .collect()
    }

    #[test]
    fn emphasis_markers_are_absent_from_the_page_but_named_by_its_spans() {
        let rendered = render("A **strong** and *soft* word.\n");
        assert_eq!(rendered.text(), "A strong and soft word.\n");
        assert_eq!(scoped(&rendered, "markup.bold"), ["strong"]);
        assert_eq!(scoped(&rendered, "markup.italic"), ["soft"]);
    }

    #[test]
    fn spans_stay_sorted_and_do_not_overlap() {
        let rendered = render(
            "# Title\n\nA `call()` and **bold** text.\n\n- item with *emphasis*\n\n> quoted **too**\n",
        );
        let mut previous = 0;
        for span in rendered.spans() {
            assert!(span.from < span.to, "empty span: {span:?}");
            assert!(span.from >= previous, "unsorted span: {span:?}");
            previous = span.to;
        }
        assert!(previous <= rendered.text().chars().count());
    }

    #[test]
    fn an_underscore_inside_a_word_is_part_of_the_word() {
        let rendered = render("The `snake_case_name` and plain snake_case_name stay whole.\n");
        assert_eq!(
            rendered.text(),
            "The snake_case_name and plain snake_case_name stay whole.\n"
        );
        assert!(scoped(&rendered, "markup.italic").is_empty());
    }

    #[test]
    fn headings_lose_their_hashes_and_the_first_level_is_underlined() {
        let rendered = render("# Title\n\n## Section\n\ntext\n");
        assert_eq!(rendered.text(), "Title\n─────\n\nSection\n\ntext\n");
        assert_eq!(
            scoped(&rendered, "markup.heading"),
            ["Title", "─────", "Section"]
        );
    }

    #[test]
    fn a_setext_underline_becomes_a_heading_rather_than_a_rule() {
        let rendered = render("Title\n=====\n\nSection\n-------\n\ntext\n");
        assert_eq!(rendered.text(), "Title\n─────\n\nSection\n\ntext\n");
    }

    #[test]
    fn fenced_code_keeps_its_text_and_drops_its_fences() {
        let rendered = render("Before\n\n```rust\nlet value = **x**;\n```\n\nAfter\n");
        assert_eq!(rendered.text(), "Before\n\n  let value = **x**;\n\nAfter\n");
        assert_eq!(scoped(&rendered, "markup.raw"), ["let value = **x**;"]);
    }

    #[test]
    fn an_indented_code_block_drops_the_columns_that_declared_it() {
        let rendered = render("Prose.\n\n    let value = **x**;\n        nested\n\nAfter\n");
        assert_eq!(
            rendered.text(),
            "Prose.\n\n  let value = **x**;\n      nested\n\nAfter\n"
        );
        assert_eq!(
            scoped(&rendered, "markup.raw"),
            ["let value = **x**;", "    nested"]
        );
    }

    #[test]
    fn a_marker_run_longer_than_three_closes_as_one_emphasis() {
        assert_eq!(render("****bold****\n").text(), "bold\n");
        assert_eq!(render("***both***\n").text(), "both\n");
    }

    /// An opening marker that never closes costs the rest of the paragraph to
    /// rule out, so a document made of them would be quadratic on the main
    /// loop. Past the limit the marker is text, which is both bounded and what
    /// an unclosable marker was anyway.
    #[test]
    fn a_closing_marker_further_than_the_scan_limit_leaves_the_opening_one_as_text() {
        let near = format!("`{}`", "x".repeat(INLINE_SCAN_LIMIT / 2));
        assert!(!render(&near).text().starts_with('`'));

        let far = format!("`{}`", "x".repeat(INLINE_SCAN_LIMIT * 2));
        assert!(render(&far).text().starts_with('`'));

        // The shape that motivated the limit: nothing here closes, and every
        // opening bracket used to look for a partner all the way to the end.
        let unmatched = "[".repeat(INLINE_SCAN_LIMIT * 4);
        assert_eq!(render(&unmatched).text(), format!("{unmatched}\n"));
    }

    #[test]
    fn list_markers_are_bullets_that_deepen_with_indentation() {
        let rendered = render("- one\n  - two\n    - three\n1. first\n");
        assert_eq!(rendered.text(), "• one\n  ◦ two\n    ▪ three\n1. first\n");
        assert_eq!(scoped(&rendered, "markup.list"), ["•", "◦", "▪", "1."]);
    }

    #[test]
    fn a_task_list_keeps_its_state_in_the_marker_column() {
        let rendered = render("- [ ] open\n- [x] done\n");
        assert_eq!(rendered.text(), "• ☐ open\n• ☑ done\n");
    }

    /// A pasted image is a plain link at a file this terminal cannot draw, so
    /// the page keeps the description and drops the cache path that would
    /// otherwise be most of the line.
    #[test]
    fn a_link_to_an_image_shows_only_its_description() {
        let rendered = render("Look at [Image 1](.runyte/cache/images/1f0a2b.png) here.\n");
        assert_eq!(rendered.text(), "Look at Image 1 here.\n");
        assert_eq!(scoped(&rendered, "markup.bold"), ["Image 1"]);
        assert_eq!(scoped(&rendered, "markup.link.url"), [] as [&str; 0]);

        // The `!` spelling of the same picture reads identically.
        assert_eq!(render("![Image 1](a/b.png)\n").text(), "Image 1\n");
        assert_eq!(
            scoped(&render("![Image 1](a/b.png)\n"), "markup.bold"),
            ["Image 1"]
        );

        // A destination that merely lives beside images is still a link.
        let rendered = render("[notes](.runyte/cache/images/readme.md)\n");
        assert_eq!(rendered.text(), "notes (.runyte/cache/images/readme.md)\n");
        assert_eq!(scoped(&rendered, "markup.link.text"), ["notes"]);
    }

    /// A picture nobody described would otherwise render as nothing at all,
    /// leaving the page with no trace that one is there.
    #[test]
    fn a_picture_with_no_description_falls_back_to_its_file_name() {
        let rendered = render("![](diagrams/flow.png)\n");
        assert_eq!(rendered.text(), "flow.png\n");
        assert_eq!(scoped(&rendered, "markup.bold"), ["flow.png"]);

        // The same holds for the plain-link spelling Runyte itself writes.
        assert_eq!(render("[](a/b/shot.jpeg)\n").text(), "shot.jpeg\n");

        // A described picture is unaffected.
        assert_eq!(render("![a plan](diagrams/flow.png)\n").text(), "a plan\n");
    }

    /// A destination in angle brackets ends at its bracket, which is the only
    /// way to write a path holding a space or a parenthesis — and the spelling
    /// Runyte writes when a pasted image lands under such a path.
    #[test]
    fn an_angle_bracketed_destination_may_hold_spaces_and_parentheses() {
        let rendered = render("[Image 1](<My Notes/a (copy).png>)\n");
        assert_eq!(rendered.text(), "Image 1\n");
        assert_eq!(scoped(&rendered, "markup.bold"), ["Image 1"]);

        // Whatever `pasted_image::destination` writes, this reads back: it
        // percent-encodes both brackets precisely so the wrapper it emits
        // survives the rule above.
        let written = crate::pasted_image::destination("odd<name/a (1).png");
        let rendered = render(&format!("[Image 1]({written})\n"));
        assert_eq!(rendered.text(), "Image 1\n");
        assert_eq!(scoped(&rendered, "markup.bold"), ["Image 1"]);

        // The same brackets around an ordinary link show the destination
        // without them.
        let rendered = render("[the guide](<My Docs/user guide.md>)\n");
        assert_eq!(rendered.text(), "the guide (My Docs/user guide.md)\n");
        assert_eq!(
            scoped(&rendered, "markup.link.url"),
            [" (My Docs/user guide.md)"]
        );

        // An opening bracket that never closes is not a link at all.
        assert_eq!(
            render("[a](<unclosed.png)\n").text(),
            "[a](<unclosed.png)\n"
        );

        // Nor is one whose `>` is a comparison further along the line. The
        // bracket has to be closed by the link itself, or a stray `>` would
        // swallow the rest of the line as one enormous destination.
        for literal in [
            "[a](<x.png) and 5 > 3 (done)\n",
            // A second `<` inside means the first never opened a destination.
            "[a](<x <y>.png)\n",
            // A title after a bracketed destination is not something this
            // reads, so the whole construct stays as its own source.
            "[a](<x.png> \"a title\")\n",
        ] {
            assert_eq!(render(literal).text(), literal, "{literal}");
        }
    }

    #[test]
    fn an_image_destination_is_recognised_by_its_name_alone() {
        for image in [
            "a.png",
            "A.PNG",
            "./deep/path/shot.jpeg",
            "https://example.com/a.webp?width=10",
            "https://example.com/a.svg#top",
            "C:\\pictures\\a.bmp",
        ] {
            assert!(names_an_image(image), "{image} should read as a picture");
        }
        for other in [
            "",
            "docs/user-guide.md",
            "https://example.com/",
            "a.png.md",
            // A hidden file named for a format is not a picture called
            // nothing at all.
            ".png",
            "images/.webp",
            // The extension has to be the whole final segment.
            "notpng",
        ] {
            assert!(!names_an_image(other), "{other} should read as a link");
        }
    }

    #[test]
    fn a_link_shows_its_text_and_its_destination() {
        let rendered = render("See [the guide](docs/user-guide.md) first.\n");
        assert_eq!(
            rendered.text(),
            "See the guide (docs/user-guide.md) first.\n"
        );
        assert_eq!(scoped(&rendered, "markup.link.text"), ["the guide"]);
        assert_eq!(
            scoped(&rendered, "markup.link.url"),
            [" (docs/user-guide.md)"]
        );
    }

    #[test]
    fn a_table_is_rendered_with_aligned_columns() {
        let rendered = render("| Key | Action |\n| --- | ------ |\n| `q` | Close |\n");
        assert_eq!(rendered.text(), "Key │ Action\n────┼───────\nq   │ Close\n");
    }

    #[test]
    fn pipes_in_prose_are_not_a_table() {
        let rendered = render("| not a table\n");
        assert_eq!(rendered.text(), "| not a table\n");
    }

    #[test]
    fn front_matter_is_kept_as_an_aside_without_its_fences() {
        let rendered = render("---\ntitle: \"Report\"\nstatus: resolved\n---\n\n# Body\n");
        assert_eq!(
            rendered.text(),
            "title: \"Report\"\nstatus: resolved\n\nBody\n────\n"
        );
        assert_eq!(
            scoped(&rendered, "comment"),
            ["title: \"Report\"", "status: resolved"]
        );
    }

    #[test]
    fn a_block_quote_is_marked_in_the_margin() {
        let rendered = render("> quoted **text**\n>> deeper\n");
        assert_eq!(rendered.text(), "▌ quoted text\n▌ ▌ deeper\n");
        assert_eq!(scoped(&rendered, "markup.bold"), ["text"]);
    }

    #[test]
    fn a_thematic_break_becomes_a_rule() {
        let rendered = render("above\n\n---\n\nbelow\n");
        assert_eq!(
            rendered.text(),
            format!("above\n\n{}\n\nbelow\n", "─".repeat(RULE_WIDTH))
        );
    }

    #[test]
    fn strikethrough_and_autolinks_are_recognised() {
        let rendered = render("~~gone~~ at <https://example.com>\n");
        assert_eq!(rendered.text(), "gone at https://example.com\n");
        assert_eq!(scoped(&rendered, "markup.strikethrough"), ["gone"]);
        assert_eq!(
            scoped(&rendered, "markup.link.url"),
            ["https://example.com"]
        );
    }

    #[test]
    fn an_unclosed_marker_is_ordinary_text() {
        let rendered = render("2 * 3 and a `stray backtick and **half\n");
        assert_eq!(rendered.text(), "2 * 3 and a `stray backtick and **half\n");
        assert!(rendered.spans().is_empty());
    }

    #[test]
    fn escaped_punctuation_survives_as_itself() {
        let rendered = render("\\*not emphasis\\* and \\\\ a slash\n");
        assert_eq!(rendered.text(), "*not emphasis* and \\ a slash\n");
    }

    #[test]
    fn an_empty_document_renders_as_an_empty_page() {
        assert_eq!(render("").text(), "");
        assert_eq!(render("\n\n\n").text(), "");
    }

    #[test]
    fn every_rendered_offset_stays_inside_the_page() {
        let source = std::fs::read_to_string("README.md").unwrap();
        let rendered = render(&source);
        let length = rendered.text().chars().count();
        for span in rendered.spans() {
            assert!(span.to <= length, "span past the page: {span:?}");
        }
    }
}
