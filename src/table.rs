// SPDX-License-Identifier: MPL-2.0

//! Pipe-table recognition and alignment.
//!
//! A table here is the shape people actually type: rows opening with `|`, cells
//! divided by `|`, and a separator row of dashes somewhere among them. Which
//! characters the author used to draw it are theirs to keep — a separator
//! written `+---+---+` comes back as a `+` row, and GitHub's `:---:` alignment
//! colons survive and decide where the content sits — so formatting a table
//! changes its spacing and nothing else.
//!
//! Text is measured the way `wrap` and `ui` measure it, one cell per character
//! with the Unicode width where there is one, so a formatted table lines up in
//! the pane it was formatted in.

use unicode_width::UnicodeWidthChar;

/// Where a column's content sits inside its cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Alignment {
    /// A separator cell of bare dashes. Renders like `Left`, but keeps its
    /// plain run rather than growing a colon its author never wrote.
    Default,
    Left,
    Center,
    Right,
}

/// One line of the selection, in the order it was written.
#[derive(Debug, Eq, PartialEq)]
enum Row {
    /// A line holding nothing but whitespace, reproduced exactly as given. The
    /// blank lines around a table are allowed inside the selection precisely so
    /// selecting the table does not have to be pixel-exact.
    Blank(String),
    Cells(Vec<String>),
    Separator {
        /// The character drawn at each boundary, in order, so a `+` table stays
        /// a `+` table and a row mixing the two keeps each character where it
        /// was. A boundary past the end of this list repeats the last one,
        /// which is how a separator narrower than the table is widened.
        boundaries: Vec<char>,
        alignments: Vec<Alignment>,
    },
}

#[derive(Debug, Eq, PartialEq)]
struct Line {
    row: Row,
    /// Whether the line ended `\r\n`, so a CRLF document keeps its terminators.
    carriage_return: bool,
}

/// Re-lays the table in `text`, padding every column to its widest cell.
///
/// Returns `None` when `text` is not a table: any line that is neither blank nor
/// a row, or no separator row among them. The caller reports that rather than
/// editing. Text that is already formatted comes back unchanged, so an identical
/// result means there was nothing to do.
///
/// Every row is padded to the widest row's column count, so a table whose rows
/// disagree is squared up rather than rejected — no cell is ever dropped. All
/// rows take the indentation of the first one; content alignment comes from the
/// first separator row, while each separator row keeps its own colons.
///
/// `tab_width` is the editor's, because a tab inside a cell is expanded here
/// rather than measured.
pub fn format_table(text: &str, tab_width: usize) -> Option<String> {
    let mut lines = Vec::new();
    let mut indent = None;
    for raw in text.split('\n') {
        let (body, carriage_return) = match raw.strip_suffix('\r') {
            Some(body) => (body, true),
            None => (raw, false),
        };
        let row = if body.trim().is_empty() {
            Row::Blank(body.to_owned())
        } else {
            let trimmed = body.trim_start();
            if indent.is_none() {
                indent = Some(body[..body.len() - trimmed.len()].to_owned());
            }
            parse_row(trimmed.trim_end(), tab_width)?
        };
        lines.push(Line {
            row,
            carriage_return,
        });
    }
    // Absent only when nothing but blank lines was selected, which is no more a
    // table than a paragraph of prose is.
    let indent = indent?;

    let columns = lines
        .iter()
        .map(|line| match &line.row {
            Row::Blank(_) => 0,
            Row::Cells(cells) => cells.len(),
            Row::Separator { alignments, .. } => alignments.len(),
        })
        .max()
        .unwrap_or(0);

    // A column nobody put content in still gets a cell wide enough to see.
    let mut widths = vec![1; columns];
    for line in &lines {
        if let Row::Cells(cells) = &line.row {
            for (width, cell) in widths.iter_mut().zip(cells) {
                *width = (*width).max(display_width(cell));
            }
        }
    }
    // A run of pipe rows is not yet a table: a Rust closure, a diff hunk, and a
    // line of prose all open with `|` often enough that accepting them would
    // reformat text nobody called a table and leave the detection error with
    // nothing to catch. The dash separator is what settles it, so a selection
    // holding none is refused even though every line in it parsed.
    let content_alignments = lines.iter().find_map(|line| match &line.row {
        Row::Separator { alignments, .. } => Some(alignments.as_slice()),
        _ => None,
    })?;

    let mut output = String::with_capacity(text.len());
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        match &line.row {
            Row::Blank(body) => output.push_str(body),
            Row::Cells(cells) => {
                output.push_str(&indent);
                output.push('|');
                for (column, width) in widths.iter().copied().enumerate() {
                    output.push(' ');
                    let cell = cells.get(column).map(String::as_str).unwrap_or("");
                    pad(
                        &mut output,
                        cell,
                        width,
                        alignment_at(content_alignments, column),
                    );
                    output.push_str(" |");
                }
            }
            Row::Separator {
                boundaries,
                alignments,
            } => {
                output.push_str(&indent);
                output.push(boundary_at(boundaries, 0));
                for (column, width) in widths.iter().copied().enumerate() {
                    // The dashes stand in for the cell and the space on either
                    // side of it, so a separator is exactly as wide as the rows
                    // it divides.
                    dashes(&mut output, width + 2, alignment_at(alignments, column));
                    output.push(boundary_at(boundaries, column + 1));
                }
            }
        }
        if line.carriage_return {
            output.push('\r');
        }
    }
    Some(output)
}

/// A separator row is recognized wherever it appears rather than only as the
/// table's second line, because the selection handed here is a hand-picked run
/// of rows: the separator may be the first of them, or a rule under a footer.
/// One of them has to be a separator, which `format_table` rather than this
/// checks, since a row cannot see its neighbours.
fn parse_row(row: &str, tab_width: usize) -> Option<Row> {
    if let Some(separator) = parse_separator(row) {
        return Some(separator);
    }
    if !row.starts_with('|') {
        return None;
    }
    let cells = split_cells(row, tab_width);
    (!cells.is_empty()).then_some(Row::Cells(cells))
}

/// The row read as a separator, or `None` when any cell holds something other
/// than dashes and alignment colons.
fn parse_separator(row: &str) -> Option<Row> {
    let mut characters = row.chars();
    let first = characters.next()?;
    if first != '|' && first != '+' {
        return None;
    }
    let mut boundaries = vec![first];
    let mut alignments = Vec::new();
    let mut cell = String::new();
    for character in characters {
        if character == '|' || character == '+' {
            alignments.push(parse_alignment(&cell)?);
            boundaries.push(character);
            cell.clear();
        } else {
            cell.push(character);
        }
    }
    // Whatever follows the last boundary is a cell only if it holds anything; a
    // row closing with a boundary leaves nothing behind.
    if !cell.trim().is_empty() {
        alignments.push(parse_alignment(&cell)?);
    }
    (!alignments.is_empty()).then_some(Row::Separator {
        boundaries,
        alignments,
    })
}

fn parse_alignment(cell: &str) -> Option<Alignment> {
    let cell = cell.trim();
    let left = cell.starts_with(':');
    let right = cell.len() > 1 && cell.ends_with(':');
    let dashes = cell.trim_start_matches(':').trim_end_matches(':');
    if dashes.is_empty() || !dashes.chars().all(|character| character == '-') {
        return None;
    }
    Some(match (left, right) {
        (true, true) => Alignment::Center,
        (true, false) => Alignment::Left,
        (false, true) => Alignment::Right,
        (false, false) => Alignment::Default,
    })
}

/// Splits a row on its unescaped `|`, trimming each cell.
///
/// A `\|` is content rather than a boundary, and stays escaped so the cell it
/// belongs to survives the round trip. The empty piece a closing `|` leaves is
/// dropped; a row that does not close keeps its tail as a final cell.
fn split_cells(row: &str, tab_width: usize) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    for character in row.strip_prefix('|').unwrap_or(row).chars() {
        if escaped {
            escaped = false;
            cell.push(character);
        } else if character == '\\' {
            escaped = true;
            cell.push(character);
        } else if character == '|' {
            cells.push(expand_tabs(cell.trim(), tab_width));
            cell.clear();
        } else {
            cell.push(character);
        }
    }
    let tail = cell.trim();
    if !tail.is_empty() {
        cells.push(expand_tabs(tail, tab_width));
    }
    cells
}

/// `cell` with its tabs expanded to spaces at `tab_width` stops, counted from
/// the cell's own first character.
///
/// A tab is as wide as the distance to the next stop, so its width depends on
/// the column it lands on — and the column each cell lands on is exactly what
/// this module is working out. Measuring one would mean solving for it. Writing
/// spaces instead settles it: the row that comes out holds no tab left to
/// re-measure, and it is as wide here as it will be in the pane. This is the
/// one thing beyond spacing the command changes, and it changes whitespace into
/// whitespace, in a cell whose surrounding whitespace is trimmed regardless.
fn expand_tabs(cell: &str, tab_width: usize) -> String {
    if !cell.contains('\t') {
        return cell.to_owned();
    }
    let tab_width = tab_width.max(1);
    let mut expanded = String::with_capacity(cell.len());
    let mut column = 0;
    for character in cell.chars() {
        if character == '\t' {
            let run = tab_width - column % tab_width;
            for _ in 0..run {
                expanded.push(' ');
            }
            column += run;
        } else {
            expanded.push(character);
            column += character_width(character);
        }
    }
    expanded
}

fn alignment_at(alignments: &[Alignment], column: usize) -> Alignment {
    alignments
        .get(column)
        .copied()
        .unwrap_or(Alignment::Default)
}

fn boundary_at(boundaries: &[char], index: usize) -> char {
    boundaries
        .get(index)
        .or_else(|| boundaries.last())
        .copied()
        .unwrap_or('|')
}

fn pad(output: &mut String, cell: &str, width: usize, alignment: Alignment) {
    let slack = width.saturating_sub(display_width(cell));
    let (before, after) = match alignment {
        Alignment::Default | Alignment::Left => (0, slack),
        Alignment::Right => (slack, 0),
        Alignment::Center => (slack / 2, slack - slack / 2),
    };
    for _ in 0..before {
        output.push(' ');
    }
    output.push_str(cell);
    for _ in 0..after {
        output.push(' ');
    }
}

fn dashes(output: &mut String, run: usize, alignment: Alignment) {
    let (left, right) = match alignment {
        Alignment::Default => (false, false),
        Alignment::Left => (true, false),
        Alignment::Right => (false, true),
        Alignment::Center => (true, true),
    };
    if left {
        output.push(':');
    }
    for _ in 0..run.saturating_sub(usize::from(left) + usize::from(right)) {
        output.push('-');
    }
    if right {
        output.push(':');
    }
}

/// The cells `text` occupies, measured the way `wrap` and `ui` measure it.
///
/// Tabs never reach here: `expand_tabs` has already turned them into the spaces
/// they stand for, which is why this needs no column to count from.
fn display_width(text: &str) -> usize {
    text.chars().map(character_width).sum()
}

fn character_width(character: char) -> usize {
    UnicodeWidthChar::width(character).unwrap_or(0).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_table_pads_every_column_to_its_widest_cell() {
        assert_eq!(
            format_table(
                "| Column 1 | Column 2 |\n\
                 |---|---|\n\
                 | Value | abc |\n\
                 | Longer text | Very very long text |",
                4
            )
            .as_deref(),
            Some(
                "| Column 1    | Column 2            |\n\
                 |-------------|---------------------|\n\
                 | Value       | abc                 |\n\
                 | Longer text | Very very long text |"
            )
        );
    }

    #[test]
    fn format_table_keeps_the_characters_the_separator_was_drawn_with() {
        assert_eq!(
            format_table(
                "| Column 1 | Column 2 |\n\
                 +---+---+\n\
                 | Value | abc |\n\
                 | Longer text | Very very long text |",
                4
            )
            .as_deref(),
            Some(
                "| Column 1    | Column 2            |\n\
                 +-------------+---------------------+\n\
                 | Value       | abc                 |\n\
                 | Longer text | Very very long text |"
            )
        );
        // Every boundary is kept where it was, so a row mixing the two survives
        // as written.
        assert_eq!(
            format_table("| a | b |\n|---+---|\n| c | d |", 4).as_deref(),
            Some("| a | b |\n|---+---|\n| c | d |")
        );
    }

    #[test]
    fn format_table_leaves_an_already_formatted_table_untouched() {
        let table = "| a  | bb |\n|----|----|\n| cc | d  |";
        assert_eq!(format_table(table, 4).as_deref(), Some(table));
    }

    #[test]
    fn format_table_honours_alignment_colons() {
        assert_eq!(
            format_table(
                "| a | b | c | d |\n|:-|:-:|-:|-|\n| one | two | three | four |",
                4
            )
            .as_deref(),
            Some(
                "| a   |  b  |     c | d    |\n\
                 |:----|:---:|------:|------|\n\
                 | one | two | three | four |"
            )
        );
    }

    #[test]
    fn format_table_allows_blank_lines_around_the_table() {
        assert_eq!(
            format_table("\n| a | bbb |\n|--|--|\n   \n", 4).as_deref(),
            Some("\n| a | bbb |\n|---|-----|\n   \n")
        );
        // Blank lines alone are no more a table than prose is.
        assert_eq!(format_table("\n  \n", 4), None);
        assert_eq!(format_table("", 4), None);
    }

    #[test]
    fn format_table_rejects_a_line_that_is_not_a_row() {
        assert_eq!(format_table("| a | b |\n|---|---|\nprose", 4), None);
        assert_eq!(format_table("not a table at all", 4), None);
        // A lone boundary character holds no cell.
        assert_eq!(format_table("|", 4), None);
        // `+` opens a separator, never a content row.
        assert_eq!(format_table("+ a + b +", 4), None);
    }

    #[test]
    fn format_table_rejects_pipe_rows_with_no_separator_among_them() {
        // Every line here parses as a row, and the result would even look
        // tidier, but nothing in the selection says it is a table.
        assert_eq!(format_table("| a | bb |\n| ccc | d |", 4), None);
        assert_eq!(format_table("| a | bb |", 4), None);
        // The shapes this rule exists to keep out.
        assert_eq!(format_table("|value| the closure argument", 4), None);
        assert_eq!(format_table("|item| accumulate(item)", 4), None);
    }

    #[test]
    fn format_table_squares_up_rows_that_disagree_on_column_count() {
        assert_eq!(
            format_table("| a | b | c |\n|---|\n| d |", 4).as_deref(),
            Some("| a | b | c |\n|---|---|---|\n| d |   |   |")
        );
    }

    #[test]
    fn format_table_keeps_indentation_and_carriage_returns() {
        assert_eq!(
            format_table("    | a | bb |\n    |-|-|\n| ccc | d |", 4).as_deref(),
            Some("    | a   | bb |\n    |-----|----|\n    | ccc | d  |")
        );
        assert_eq!(
            format_table("| a | bb |\r\n|-|-|\r\n| ccc | d |\r\n", 4).as_deref(),
            Some("| a   | bb |\r\n|-----|----|\r\n| ccc | d  |\r\n")
        );
    }

    #[test]
    fn format_table_measures_cells_the_way_the_pane_draws_them() {
        // The wide characters take two cells each, so the ASCII column below is
        // padded to match rather than to the character count.
        assert_eq!(
            format_table("| 界界 | b |\n|---|---|\n| ab | c |", 4).as_deref(),
            Some("| 界界 | b |\n|------|---|\n| ab   | c |")
        );
    }

    #[test]
    fn format_table_expands_a_tab_inside_a_cell_to_the_configured_stops() {
        // `ab` sits at columns 0 and 1, so the tab runs to the stop at 4 and the
        // cell is six wide. Measuring the tab as one character would pad `dddddd`
        // to the same width and draw the two rows differently.
        assert_eq!(
            format_table("| ab\tc | b |\n|-|-|\n| dddddd | c |", 4).as_deref(),
            Some("| ab  c  | b |\n|--------|---|\n| dddddd | c |")
        );
        // The stops are the editor's, not a constant.
        assert_eq!(
            format_table("| ab\tc | b |\n|-|-|\n| dddd | c |", 2).as_deref(),
            Some("| ab  c | b |\n|-------|---|\n| dddd  | c |")
        );
    }

    #[test]
    fn format_table_treats_an_escaped_pipe_as_content() {
        assert_eq!(
            format_table("| a \\| b | c |\n|-|-|", 4).as_deref(),
            Some("| a \\| b | c |\n|--------|---|")
        );
    }

    #[test]
    fn format_table_pads_a_row_that_ends_without_a_boundary() {
        assert_eq!(
            format_table("| a | bbb\n|-|-|\n| cccc | d |", 4).as_deref(),
            Some("| a    | bbb |\n|------|-----|\n| cccc | d   |")
        );
    }
}
