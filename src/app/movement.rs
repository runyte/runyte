// SPDX-License-Identifier: MPL-2.0

//! Pure character-offset movement and selection-span calculations.

// Application-module dependencies:
use super::{
    Buffer, Change, DiffProjection, Motion, Offset, Position, Range, ResolvedFold, Selection,
    UnicodeWidthChar, next_visible_row, previous_visible_row, project_visible_rows,
};

pub(super) fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// True when a character occupies exactly one terminal cell.
///
/// A jump label is drawn in place of the character underneath it, so it can
/// only cover characters that are the same width it is.
pub(super) fn is_single_cell(ch: char) -> bool {
    UnicodeWidthChar::width(ch) == Some(1)
}

pub(super) fn word_class(character: char, long: bool) -> u8 {
    if character.is_whitespace() {
        0
    } else if long || is_word(character) {
        1
    } else {
        2
    }
}

pub(super) fn offsets_after(buffer: &Buffer, offset: Offset) -> impl Iterator<Item = Offset> + '_ {
    let len = buffer.len_chars();
    (offset + 1..len).filter(move |candidate| buffer.char_at(*candidate) != Some('\n'))
}

pub(super) fn offsets_before(buffer: &Buffer, offset: Offset) -> impl Iterator<Item = Offset> + '_ {
    (0..offset)
        .rev()
        .filter(move |candidate| buffer.char_at(*candidate) != Some('\n'))
}

pub(super) fn next_offset(buffer: &Buffer, offset: Offset) -> Option<Offset> {
    offsets_after(buffer, offset).next()
}

pub(super) fn previous_offset(buffer: &Buffer, offset: Offset) -> Option<Offset> {
    offsets_before(buffer, offset).next()
}

pub(super) fn class_at(buffer: &Buffer, offset: Offset, long: bool) -> Option<u8> {
    buffer
        .char_at(offset)
        .filter(|ch| *ch != '\n')
        .map(|ch| word_class(ch, long))
}

/// Character class for word motion, where a line terminator is whitespace
/// rather than nothing at all.
///
/// `class_at` hides newlines because the Normal-mode cursor cannot rest on
/// one, and the offset walk it pairs with skips them for the same reason.
/// Word motion needs the opposite: a line break has to break a run of word
/// characters, or the last word of a row and the first word of the next row
/// read as one word and `w`, `b`, and `e` all step over the boundary.
pub(super) fn word_class_at(buffer: &Buffer, offset: Offset, long: bool) -> Option<u8> {
    buffer.char_at(offset).map(|ch| word_class(ch, long))
}

/// Next offset for a word scan, line terminators included.
pub(super) fn word_scan_next(buffer: &Buffer, offset: Offset) -> Option<Offset> {
    (offset + 1 < buffer.len_chars()).then_some(offset + 1)
}

/// Previous offset for a word scan, line terminators included.
pub(super) fn word_scan_previous(offset: Offset) -> Option<Offset> {
    offset.checked_sub(1)
}

/// Offset of the last character in the document, for Normal-mode motion.
pub(super) fn document_end(buffer: &Buffer) -> Offset {
    buffer.row_end_offset(buffer.last_row(), false)
}

/// True when `offset` begins a word: it is non-whitespace and the preceding
/// character position has a different class.
pub(super) fn is_word_start(buffer: &Buffer, offset: Offset, long: bool) -> bool {
    let Some(class) = word_class_at(buffer, offset, long) else {
        return false;
    };
    if class == 0 {
        return false;
    }
    word_scan_previous(offset).and_then(|previous| word_class_at(buffer, previous, long))
        != Some(class)
}

pub(super) fn word_forward_kind(buffer: &Buffer, offset: Offset, long: bool) -> Offset {
    let mut previous_class = word_class_at(buffer, offset, long);
    let mut candidate = offset;
    while let Some(next) = word_scan_next(buffer, candidate) {
        let class = word_class_at(buffer, next, long).unwrap_or(0);
        if class != 0 && previous_class != Some(class) {
            return next;
        }
        previous_class = Some(class);
        candidate = next;
    }
    document_end(buffer)
}

pub(super) fn word_back_kind(buffer: &Buffer, offset: Offset, long: bool) -> Offset {
    let mut candidate = offset;
    while let Some(previous) = word_scan_previous(candidate) {
        if is_word_start(buffer, previous, long) {
            return previous;
        }
        candidate = previous;
    }
    0
}

pub(super) fn word_end(buffer: &Buffer, offset: Offset, long: bool) -> Offset {
    let mut candidate = offset;
    loop {
        let class = word_class_at(buffer, candidate, long).unwrap_or(0);
        if class != 0 && candidate != offset {
            let next_class = word_scan_next(buffer, candidate)
                .and_then(|next| word_class_at(buffer, next, long));
            if next_class != Some(class) {
                return candidate;
            }
        }
        match word_scan_next(buffer, candidate) {
            Some(next) => candidate = next,
            None => return document_end(buffer),
        }
    }
}

pub(super) fn word_end_back(buffer: &Buffer, offset: Offset, long: bool) -> Offset {
    let Some(mut candidate) = word_scan_previous(offset) else {
        return 0;
    };
    if let Some(class) = word_class_at(buffer, offset, long).filter(|class| *class != 0) {
        while word_class_at(buffer, candidate, long) == Some(class) {
            let Some(previous) = word_scan_previous(candidate) else {
                return 0;
            };
            candidate = previous;
        }
    }
    while word_class_at(buffer, candidate, long).unwrap_or(0) == 0 {
        let Some(previous) = word_scan_previous(candidate) else {
            return 0;
        };
        candidate = previous;
    }
    candidate
}

pub(super) fn insert_word_back(buffer: &Buffer, offset: Offset) -> Offset {
    let Some(mut candidate) = previous_offset(buffer, offset) else {
        return 0;
    };
    while buffer.char_at(candidate).is_some_and(char::is_whitespace) {
        let Some(previous) = previous_offset(buffer, candidate) else {
            return 0;
        };
        candidate = previous;
    }
    let class = class_at(buffer, candidate, false).unwrap_or(0);
    while let Some(previous) = previous_offset(buffer, candidate) {
        if class_at(buffer, previous, false) != Some(class) {
            break;
        }
        candidate = previous;
    }
    candidate
}

pub(super) fn insert_word_forward(buffer: &Buffer, offset: Offset) -> Offset {
    let len = buffer.len_chars();
    if offset >= len {
        return len;
    }
    let mut candidate = offset;
    while buffer.char_at(candidate).is_some_and(char::is_whitespace) {
        let Some(next) = next_offset(buffer, candidate) else {
            return len;
        };
        candidate = next;
    }
    let class = class_at(buffer, candidate, false).unwrap_or(0);
    while let Some(next) = next_offset(buffer, candidate) {
        if class_at(buffer, next, false) != Some(class) {
            break;
        }
        candidate = next;
    }
    (candidate + 1).min(len)
}

/// Inclusive-start, exclusive-end bounds of the word under `offset`, confined
/// to a single row.
pub(super) fn word_bounds(buffer: &Buffer, offset: Offset) -> (Offset, Offset) {
    let Some(class) = class_at(buffer, offset, false) else {
        return (offset, offset);
    };
    let row = buffer.offset_to_row(offset);
    let mut start = offset;
    let mut end = offset;
    while let Some(previous) = previous_offset(buffer, start) {
        if buffer.offset_to_row(previous) != row || class_at(buffer, previous, false) != Some(class)
        {
            break;
        }
        start = previous;
    }
    while let Some(next) = next_offset(buffer, end) {
        if buffer.offset_to_row(next) != row || class_at(buffer, next, false) != Some(class) {
            break;
        }
        end = next;
    }
    (start, end + 1)
}

/// The half-open span an operation acts on: the range plus the character under
/// the caret. A non-empty row stops before its line terminator, while an empty
/// row acts on the terminator itself so commands such as `d` can remove it.
/// Whether a periodic Git refresh replaces this buffer's whole text.
///
/// These are the projections rebuilt from provider output. A tracked source
/// file is not one of them: a refresh reconciles its gutter, not its text.
pub(super) fn is_refreshed_projection(buffer: &Buffer) -> bool {
    buffer.is_git_status()
        || buffer.is_git_branches()
        || buffer.is_git_worktrees()
        || buffer.is_git_log()
        || buffer.is_git_blame()
        || buffer.is_git_stash()
        || buffer.is_diff()
}

/// Whether a selection represents something the person chose rather than
/// where their cursor happens to sit.
///
/// This is the rule `App::scoping_region` already uses to decide whether a
/// search should narrow: a bare caret is a one-character range in this
/// grammar, so several ranges or a range of two or more characters is what
/// distinguishes a deliberate selection, such as the one `s` leaves behind
/// on every search match.
pub(super) fn selection_is_deliberate(buffer: &Buffer, selection: &Selection) -> bool {
    selection.ranges().len() > 1
        || selection.ranges().iter().any(|range| {
            let (from, to) = operative_span(buffer, range);
            to.saturating_sub(from) >= 2
        })
}

/// `spans` widened to whole rows, with those that then cover one run of rows
/// folded into a single span.
///
/// Two selections sitting on different columns of one row do not overlap until
/// they are widened, and `Transaction::new` drops a change overlapping an
/// earlier one — deliberately, since two cursors editing one region is a
/// selection-model bug. Leaving them apart would therefore skip rows the status
/// line had just reported as formatted.
///
/// Spans divided by nothing but a single line terminator are folded too. Their
/// rows are consecutive and every one of them is selected, so they are one run
/// of rows rather than two, and formatting them apart would give one table two
/// sets of column widths. A wider gap holds a row nobody selected, which stays
/// outside the change and keeps the two spans apart.
pub(super) fn merged_line_spans(
    buffer: &Buffer,
    spans: Vec<(Offset, Offset)>,
) -> Vec<(Offset, Offset)> {
    let mut spans: Vec<(Offset, Offset)> = spans
        .into_iter()
        .map(|(from, to)| whole_line_span(buffer, from, to))
        .collect();
    spans.sort_unstable();
    let mut merged: Vec<(Offset, Offset)> = Vec::with_capacity(spans.len());
    for (from, to) in spans {
        match merged.last_mut() {
            Some(previous)
                if from <= previous.1
                    || matches!(buffer.slice(previous.1, from).as_str(), "\n" | "\r\n") =>
            {
                previous.1 = previous.1.max(to);
            }
            _ => merged.push((from, to)),
        }
    }
    merged
}

/// `from`..`to` widened to cover in full every row it touches, and no other.
///
/// The row a span ends *at* is not one it touches: a span stopping at column
/// zero of the following row, which a pointer drag produces, ends on the row
/// before it. Neither is the row after a trailing terminator, which is why the
/// last selected character rather than the span's end decides the final row.
pub(super) fn whole_line_span(buffer: &Buffer, from: Offset, to: Offset) -> (Offset, Offset) {
    let last = if to > from { to - 1 } else { from };
    (
        buffer.line_to_offset(buffer.offset_to_row(from)),
        buffer.row_end_offset(buffer.offset_to_row(last), true),
    )
}

/// `to` with a trailing line terminator held back, keeping it out of a change
/// that would otherwise pull the following row up.
pub(super) fn without_trailing_line_terminator(
    buffer: &Buffer,
    from: Offset,
    to: Offset,
) -> Offset {
    if to <= from || buffer.char_at(to - 1) != Some('\n') {
        return to;
    }
    let to = to - 1;
    if to > from && buffer.char_at(to - 1) == Some('\r') {
        to - 1
    } else {
        to
    }
}

/// Changes that strip trailing spaces and tabs from each of `rows`.
///
/// Shared by the save-time trim and the `_` command so the two cannot disagree
/// about what counts as trailing whitespace. Rows must be unique: two changes
/// covering one line would overlap inside a single transaction.
pub(super) fn trailing_whitespace_changes(
    buffer: &Buffer,
    rows: impl Iterator<Item = usize>,
) -> Vec<Change> {
    rows.filter_map(|row| {
        let line = buffer.line_string(row);
        let trimmed = line.trim_end_matches([' ', '\t']);
        let trimmed_len = trimmed.chars().count();
        let line_len = line.chars().count();
        (trimmed_len < line_len).then(|| {
            let start = buffer.line_to_offset(row);
            Change::new(start + trimmed_len, start + line_len, "")
        })
    })
    .collect()
}

pub(super) fn operative_span(buffer: &Buffer, range: &Range) -> (Offset, Offset) {
    let from = range.from();
    let to = range.to();
    let row = buffer.offset_to_row(to);
    if range.is_empty() && buffer.line_len(row) == 0 {
        if row < buffer.last_row() {
            return (from, buffer.line_to_offset(row + 1));
        }
        if row > 0 {
            let previous_end = buffer.line_to_offset(row - 1) + buffer.line_len(row - 1);
            return (previous_end, from);
        }
    }
    let row_end = buffer.line_to_offset(row) + buffer.line_len(row);
    (from, (to + 1).min(row_end).max(from))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn move_offset_projected(
    buffer: &Buffer,
    offset: Offset,
    motion: Motion,
    viewport: (usize, usize, usize),
    width: usize,
    tab_width: usize,
    soft_wrap: bool,
    folds: &[ResolvedFold],
    diff: Option<DiffProjection<'_>>,
) -> Offset {
    let (viewport_height, scroll_row, scroll_wrap) = viewport;
    let position = buffer.position_of(offset);
    let desired = if soft_wrap {
        crate::wrap::screen_column(
            &buffer.line_string(position.row),
            position.col,
            width,
            tab_width,
        )
    } else {
        position.col
    };
    let move_once = |position: Position, down: bool| {
        let segments = crate::wrap::segments(&buffer.line_string(position.row), width, tab_width);
        let current = if soft_wrap {
            crate::wrap::segment_index(
                &buffer.line_string(position.row),
                position.col,
                width,
                tab_width,
            )
        } else {
            0
        };
        let (row, segment) = if down {
            if soft_wrap && current + 1 < segments.len() {
                (position.row, current + 1)
            } else {
                let row = next_visible_row(folds, position.row, buffer.last_row());
                if row == position.row {
                    return position;
                }
                (row, 0)
            }
        } else if soft_wrap && current > 0 {
            (position.row, current - 1)
        } else {
            let row = previous_visible_row(folds, position.row);
            if row == position.row {
                return position;
            }
            let segment = if soft_wrap {
                crate::wrap::segments(&buffer.line_string(row), width, tab_width)
                    .len()
                    .saturating_sub(1)
            } else {
                0
            };
            (row, segment)
        };
        let col = if soft_wrap {
            crate::wrap::column_for_screen(
                &buffer.line_string(row),
                segment,
                desired,
                width,
                tab_width,
            )
        } else {
            desired.min(buffer.line_len(row))
        };
        Position::new(row, col)
    };
    let target = match motion {
        Motion::Up | Motion::Down => move_once(position, matches!(motion, Motion::Down)),
        Motion::PageUp | Motion::PageDown | Motion::HalfPageUp | Motion::HalfPageDown => {
            let down = matches!(motion, Motion::PageDown | Motion::HalfPageDown);
            let amount = if matches!(motion, Motion::PageUp | Motion::PageDown) {
                viewport_height
            } else {
                (viewport_height / 2).max(1)
            };
            (0..amount).fold(position, |position, _| move_once(position, down))
        }
        Motion::WindowTop | Motion::WindowCenter | Motion::WindowBottom => {
            // Filler is not somewhere a caret can go, so the top, middle,
            // and bottom of the window are the first, middle, and last rows
            // that actually show a line.
            let rows = project_visible_rows(
                buffer,
                folds,
                scroll_row,
                scroll_wrap,
                viewport_height,
                width,
                tab_width,
                soft_wrap,
                diff,
            )
            .into_iter()
            .filter_map(|visual| Some((visual.document_row?, visual.segment)))
            .collect::<Vec<_>>();
            if rows.is_empty() {
                position
            } else {
                let index = match motion {
                    Motion::WindowTop => 0,
                    Motion::WindowCenter => (rows.len() - 1) / 2,
                    Motion::WindowBottom => rows.len() - 1,
                    _ => unreachable!(),
                };
                let (document_row, segment) = rows[index];
                let col = segment.map_or_else(
                    || desired.min(buffer.line_len(document_row)),
                    |segment| {
                        let segment_index = crate::wrap::segment_index(
                            &buffer.line_string(document_row),
                            segment.start,
                            width,
                            tab_width,
                        );
                        crate::wrap::column_for_screen(
                            &buffer.line_string(document_row),
                            segment_index,
                            desired,
                            width,
                            tab_width,
                        )
                    },
                );
                Position::new(document_row, col)
            }
        }
        _ => position,
    };
    buffer.clamp_offset(buffer.offset_of(target), false)
}

pub(super) fn move_offset(
    buffer: &Buffer,
    offset: Offset,
    motion: Motion,
    viewport_height: usize,
    scroll_row: usize,
) -> Offset {
    let position = buffer.position_of(offset);
    let last_row = buffer.last_row();
    let on_row = |row: usize| {
        let row = row.min(last_row);
        let col = position.col.min(buffer.line_len(row));
        buffer.clamp_offset(buffer.line_to_offset(row) + col, false)
    };
    match motion {
        Motion::Left => {
            if position.col > 0 {
                offset - 1
            } else if position.row > 0 {
                buffer.row_end_offset(position.row - 1, false)
            } else {
                offset
            }
        }
        Motion::Right => {
            if position.col + 1 < buffer.line_len(position.row) {
                offset + 1
            } else if position.row < last_row {
                buffer.line_to_offset(position.row + 1)
            } else {
                offset
            }
        }
        Motion::Up => on_row(position.row.saturating_sub(1)),
        Motion::Down => on_row(position.row + 1),
        Motion::LineStart => buffer.line_to_offset(position.row),
        Motion::LineEnd => buffer.row_end_offset(position.row, false),
        Motion::FileStart => 0,
        Motion::FileEnd => document_end(buffer),
        Motion::WordForward => word_forward_kind(buffer, offset, false),
        Motion::WordBack => word_back_kind(buffer, offset, false),
        Motion::WordEnd => word_end(buffer, offset, false),
        Motion::WordEndBack => word_end_back(buffer, offset, false),
        Motion::LongWordForward => word_forward_kind(buffer, offset, true),
        Motion::LongWordBack => word_back_kind(buffer, offset, true),
        Motion::LongWordEnd => word_end(buffer, offset, true),
        Motion::LongWordEndBack => word_end_back(buffer, offset, true),
        Motion::NextParagraph => next_paragraph(buffer, offset),
        Motion::PreviousParagraph => previous_paragraph(buffer, offset),
        Motion::FirstNonWhitespace => {
            let column = buffer
                .line_string(position.row)
                .chars()
                .position(|character| !character.is_whitespace())
                .unwrap_or(0);
            buffer.clamp_offset(buffer.line_to_offset(position.row) + column, false)
        }
        Motion::LastNonWhitespace => {
            let column = buffer
                .line_string(position.row)
                .chars()
                .enumerate()
                .filter_map(|(column, character)| (!character.is_whitespace()).then_some(column))
                .last()
                .unwrap_or(0);
            buffer.clamp_offset(buffer.line_to_offset(position.row) + column, false)
        }
        Motion::PageUp => on_row(position.row.saturating_sub(viewport_height)),
        Motion::PageDown => on_row(position.row + viewport_height),
        Motion::HalfPageUp => on_row(position.row.saturating_sub((viewport_height / 2).max(1))),
        Motion::HalfPageDown => on_row(position.row + (viewport_height / 2).max(1)),
        Motion::WindowTop => on_row(scroll_row),
        Motion::WindowCenter => on_row(scroll_row + viewport_height / 2),
        Motion::WindowBottom => on_row(scroll_row + viewport_height.saturating_sub(1)),
    }
}

pub(super) fn next_paragraph(buffer: &Buffer, offset: Offset) -> Offset {
    let mut row = buffer.offset_to_row(offset);
    let last_row = buffer.last_row();

    while row <= last_row && buffer.line_len(row) > 0 {
        row += 1;
    }
    while row <= last_row && buffer.line_len(row) == 0 {
        row += 1;
    }

    if row <= last_row {
        buffer.line_to_offset(row)
    } else {
        document_end(buffer)
    }
}

pub(super) fn previous_paragraph(buffer: &Buffer, offset: Offset) -> Offset {
    let position = buffer.position_of(offset);
    if buffer.line_len(position.row) > 0 && position.col > 0 {
        return buffer.line_to_offset(position.row);
    }

    let mut row = position.row;
    if row == 0 {
        return 0;
    }
    row -= 1;
    while row > 0 && buffer.line_len(row) == 0 {
        row -= 1;
    }
    while row > 0 && buffer.line_len(row - 1) > 0 {
        row -= 1;
    }
    buffer.line_to_offset(row)
}
