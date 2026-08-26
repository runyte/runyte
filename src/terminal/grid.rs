// SPDX-License-Identifier: MPL-2.0

//! The styled cell grid a terminal child paints into.
//!
//! Nothing here knows about panes, buffers, or drawing. A grid is a rectangle
//! of cells plus a bounded scrollback of the lines that have left the top of
//! it, and the operations are the ones an escape sequence names: write a
//! character, move the cursor, erase a region, scroll a region. The parser in
//! [`super::parser`] is the only caller.
//!
//! Deliberately not a [`crate::text::Text`]. Writes here are addressed rather
//! than appended, they are lossy by design, and there is no transaction to
//! undo — which is exactly why a terminal is a pane content type rather than a
//! buffer kind.

use std::collections::VecDeque;

use unicode_width::UnicodeWidthChar;

/// How many lines of scrollback one terminal keeps.
///
/// Bounded because a terminal that never forgets is a memory leak with a
/// cursor. Five thousand lines is about what a person scrolls back through
/// looking for the command before last, and costs a few megabytes at most.
pub const SCROLLBACK_LIMIT: usize = 5_000;

/// A colour a cell names.
///
/// `Default` defers to the frontend, which resolves it against the editor
/// theme, so a terminal in a light theme is not stuck painting on black.
/// `Indexed` is the 256-colour palette; `Rgb` is direct colour. The grid never
/// resolves any of them.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Cell attributes, as a bit set so a cell stays small.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Attributes(u16);

impl Attributes {
    pub const NONE: Self = Self(0);
    pub const BOLD: Self = Self(1 << 0);
    pub const DIM: Self = Self(1 << 1);
    pub const ITALIC: Self = Self(1 << 2);
    pub const UNDERLINE: Self = Self(1 << 3);
    pub const BLINK: Self = Self(1 << 4);
    pub const REVERSE: Self = Self(1 << 5);
    pub const HIDDEN: Self = Self(1 << 6);
    pub const STRIKETHROUGH: Self = Self(1 << 7);

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    pub const fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// The colour and attribute state new characters are written with.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Pen {
    pub foreground: Color,
    pub background: Color,
    pub attributes: Attributes,
}

/// One grid cell.
///
/// A double-width character occupies two cells: the first carries the
/// character with `width == 2`, and the one after it is a spacer with
/// `width == 0`. A frontend draws only cells whose width is non-zero, which
/// keeps the column arithmetic here and on screen identical.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cell {
    pub character: char,
    /// Bounded combining marks attached to `character`. Three covers the
    /// common canonical and emoji-modifier cases without making a cell own a
    /// heap allocation or weakening the workspace memory bound.
    pub combining: [char; 3],
    pub combining_len: u8,
    pub width: u8,
    pub foreground: Color,
    pub background: Color,
    pub attributes: Attributes,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            character: ' ',
            combining: ['\0'; 3],
            combining_len: 0,
            width: 1,
            foreground: Color::Default,
            background: Color::Default,
            attributes: Attributes::NONE,
        }
    }
}

impl Cell {
    pub fn text(self) -> String {
        let mut text = String::with_capacity(1 + usize::from(self.combining_len));
        text.push(self.character);
        text.extend(self.combining[..usize::from(self.combining_len)].iter());
        text
    }

    fn blank(pen: Pen) -> Self {
        Self {
            character: ' ',
            combining: ['\0'; 3],
            combining_len: 0,
            width: 1,
            // Erasing paints the current background but never the current
            // foreground: there is no glyph to colour, and carrying it would
            // make a cleared region change colour when the pen later did.
            foreground: Color::Default,
            background: pen.background,
            attributes: Attributes::NONE,
        }
    }

    fn spacer(pen: Pen) -> Self {
        Self {
            character: ' ',
            combining: ['\0'; 3],
            combining_len: 0,
            width: 0,
            foreground: pen.foreground,
            background: pen.background,
            attributes: pen.attributes,
        }
    }

    /// Whether this cell holds nothing a reader would miss.
    pub fn is_blank(&self) -> bool {
        self.width != 0
            && self.character == ' '
            && self.background == Color::Default
            && self.attributes.is_empty()
    }
}

pub type Line = Vec<Cell>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Cursor {
    pub row: usize,
    pub column: usize,
    /// Set when the last write filled the final column. The cursor stays on
    /// that column until one more character arrives, which is what makes a
    /// line exactly as wide as the screen wrap once rather than twice.
    pub pending_wrap: bool,
}

/// One screen: its cells, its cursor, its scroll region, and — for the primary
/// screen only — the lines that have scrolled off the top.
#[derive(Clone, Debug)]
pub struct Grid {
    columns: usize,
    rows: usize,
    lines: Vec<Line>,
    scrollback: VecDeque<Line>,
    /// Whether lines leaving the top are kept. False for the alternate screen,
    /// which by definition has no history.
    keeps_history: bool,
    /// How many lines have ever left the top of the screen.
    ///
    /// Monotonic, and therefore not the same question as how long the
    /// scrollback is: once the limit is reached a line joins the back for every
    /// line dropped from the front, and the length stops moving while the
    /// content keeps sliding. A reader holding a position in history needs the
    /// sliding, not the length.
    retired: u64,
    pub cursor: Cursor,
    saved_cursor: Option<(Cursor, Pen)>,
    /// Inclusive top and bottom rows of the scrolling region.
    scroll_top: usize,
    scroll_bottom: usize,
}

impl Grid {
    pub fn new(columns: usize, rows: usize, keeps_history: bool) -> Self {
        let columns = columns.max(1);
        let rows = rows.max(1);
        Self {
            columns,
            rows,
            lines: vec![vec![Cell::default(); columns]; rows],
            scrollback: VecDeque::new(),
            keeps_history,
            retired: 0,
            cursor: Cursor::default(),
            saved_cursor: None,
            scroll_top: 0,
            scroll_bottom: rows - 1,
        }
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn line(&self, row: usize) -> Option<&Line> {
        self.lines.get(row)
    }

    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// How many lines have left the top of the screen since it was created.
    pub fn retired(&self) -> u64 {
        self.retired
    }

    pub fn scrollback_line(&self, index: usize) -> Option<&Line> {
        self.scrollback.get(index)
    }

    /// Retained primary history followed by the current screen, with stable
    /// identities derived from the monotonic retirement counter.
    pub fn retained_lines(&self) -> impl Iterator<Item = (u64, &Line)> {
        let oldest = self.retired.saturating_sub(self.scrollback.len() as u64);
        self.scrollback
            .iter()
            .enumerate()
            .map(move |(index, line)| (oldest + index as u64, line))
            .chain(
                self.lines
                    .iter()
                    .enumerate()
                    .map(move |(index, line)| (self.retired + index as u64, line)),
            )
    }

    pub fn scrollback_cells(&self) -> usize {
        self.scrollback.iter().map(Vec::len).sum()
    }

    /// Drops the oldest retained line for the workspace-wide memory budget.
    pub fn drop_oldest_scrollback(&mut self) -> bool {
        self.scrollback.pop_front().is_some()
    }

    pub fn scroll_region(&self) -> (usize, usize) {
        (self.scroll_top, self.scroll_bottom)
    }

    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        if top < bottom && bottom < self.rows {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
        } else {
            self.scroll_top = 0;
            self.scroll_bottom = self.rows - 1;
        }
    }

    pub fn save_cursor(&mut self, pen: Pen) {
        self.saved_cursor = Some((self.cursor, pen));
    }

    /// Restores the cursor saved by `DECSC`, reporting the pen to restore with
    /// it. A restore with nothing saved homes the cursor, as VT100 does.
    pub fn restore_cursor(&mut self) -> Option<Pen> {
        match self.saved_cursor {
            Some((cursor, pen)) => {
                self.cursor = cursor;
                self.clamp_cursor();
                Some(pen)
            }
            None => {
                self.cursor = Cursor::default();
                None
            }
        }
    }

    fn clamp_cursor(&mut self) {
        self.cursor.row = self.cursor.row.min(self.rows - 1);
        self.cursor.column = self.cursor.column.min(self.columns - 1);
    }

    pub fn move_to(&mut self, row: usize, column: usize) {
        self.cursor.row = row.min(self.rows - 1);
        self.cursor.column = column.min(self.columns - 1);
        self.cursor.pending_wrap = false;
    }

    pub fn move_column(&mut self, column: usize) {
        self.cursor.column = column.min(self.columns - 1);
        self.cursor.pending_wrap = false;
    }

    pub fn move_row(&mut self, row: usize) {
        self.cursor.row = row.min(self.rows - 1);
        self.cursor.pending_wrap = false;
    }

    pub fn move_up(&mut self, count: usize) {
        // Cursor motion stops at the scroll region's edge only when it starts
        // inside it; outside, the screen edge is the boundary.
        let limit = if self.cursor.row >= self.scroll_top {
            self.scroll_top
        } else {
            0
        };
        self.cursor.row = self.cursor.row.saturating_sub(count).max(limit);
        self.cursor.pending_wrap = false;
    }

    pub fn move_down(&mut self, count: usize) {
        let limit = if self.cursor.row <= self.scroll_bottom {
            self.scroll_bottom
        } else {
            self.rows - 1
        };
        self.cursor.row = (self.cursor.row + count).min(limit);
        self.cursor.pending_wrap = false;
    }

    pub fn move_left(&mut self, count: usize) {
        self.cursor.column = self.cursor.column.saturating_sub(count);
        self.cursor.pending_wrap = false;
    }

    pub fn move_right(&mut self, count: usize) {
        self.cursor.column = (self.cursor.column + count).min(self.columns - 1);
        self.cursor.pending_wrap = false;
    }

    pub fn carriage_return(&mut self) {
        self.cursor.column = 0;
        self.cursor.pending_wrap = false;
    }

    pub fn backspace(&mut self) {
        if self.cursor.pending_wrap {
            self.cursor.pending_wrap = false;
        } else {
            self.cursor.column = self.cursor.column.saturating_sub(1);
        }
    }

    /// Index (LF): down one row, scrolling the region when already at its foot.
    pub fn index(&mut self, pen: Pen) {
        if self.cursor.row == self.scroll_bottom {
            self.scroll_up(1, pen);
        } else if self.cursor.row + 1 < self.rows {
            self.cursor.row += 1;
        }
        self.cursor.pending_wrap = false;
    }

    /// Reverse index (RI): up one row, scrolling the region down at its head.
    pub fn reverse_index(&mut self, pen: Pen) {
        if self.cursor.row == self.scroll_top {
            self.scroll_down(1, pen);
        } else {
            self.cursor.row = self.cursor.row.saturating_sub(1);
        }
        self.cursor.pending_wrap = false;
    }

    /// Moves the region's lines up, retiring the top one to scrollback.
    pub fn scroll_up(&mut self, count: usize, pen: Pen) {
        let count = count.min(self.scroll_bottom - self.scroll_top + 1);
        for _ in 0..count {
            let line = self.lines.remove(self.scroll_top);
            // A region anchored at the first row still pushes its top line
            // out of the terminal and into history. Inline TUIs use this to
            // commit completed output while keeping a composer below it in
            // place. A region starting farther down belongs to an application's
            // internal layout, so retaining that would interleave status-area
            // updates into the scrollback.
            if self.keeps_history && self.scroll_top == 0 {
                self.retire(line);
            }
            self.lines.insert(self.scroll_bottom, self.blank_line(pen));
        }
    }

    /// Moves the region's lines down, discarding the ones pushed off its foot.
    pub fn scroll_down(&mut self, count: usize, pen: Pen) {
        let count = count.min(self.scroll_bottom - self.scroll_top + 1);
        for _ in 0..count {
            self.lines.remove(self.scroll_bottom);
            self.lines.insert(self.scroll_top, self.blank_line(pen));
        }
    }

    fn retire(&mut self, line: Line) {
        self.scrollback.push_back(line);
        self.retired = self.retired.wrapping_add(1);
        while self.scrollback.len() > SCROLLBACK_LIMIT {
            self.scrollback.pop_front();
        }
    }

    fn blank_line(&self, pen: Pen) -> Line {
        vec![Cell::blank(pen); self.columns]
    }

    /// Writes one character at the cursor, wrapping and scrolling as needed.
    pub fn write(&mut self, character: char, pen: Pen, autowrap: bool) {
        self.write_with_insert(character, pen, autowrap, false);
    }

    /// Writes one character, applying IRM after its actual destination has
    /// been resolved across delayed wrap and wide-glyph overflow.
    pub(super) fn write_with_insert(
        &mut self,
        character: char,
        pen: Pen,
        autowrap: bool,
        insert: bool,
    ) {
        let width = UnicodeWidthChar::width(character)
            .unwrap_or(0)
            .min(self.columns);
        if width == 0 {
            let mut column = if self.cursor.pending_wrap {
                self.columns.saturating_sub(1)
            } else {
                self.cursor.column.saturating_sub(1)
            };
            if self.lines[self.cursor.row][column].width == 0 && column > 0 {
                column -= 1;
            }
            let cell = &mut self.lines[self.cursor.row][column];
            if cell.character != ' ' && usize::from(cell.combining_len) < cell.combining.len() {
                cell.combining[usize::from(cell.combining_len)] = character;
                cell.combining_len += 1;
            }
            return;
        }
        if self.cursor.pending_wrap && autowrap {
            self.cursor.column = 0;
            self.index(pen);
            self.cursor.pending_wrap = false;
        }
        if self.cursor.column + width > self.columns {
            if autowrap {
                self.cursor.column = 0;
                self.index(pen);
            } else {
                self.cursor.column = self.columns - width.min(self.columns);
            }
        }
        let row = self.cursor.row;
        let column = self.cursor.column;
        if insert {
            self.insert_characters(width, pen);
        }
        // Overwriting half of a double-width character leaves the other half
        // orphaned; blank it so no stale glyph survives.
        self.clear_partner(row, column, pen);
        if width == 2 {
            self.clear_partner(row, column + 1, pen);
        }
        self.lines[row][column] = Cell {
            character,
            combining: ['\0'; 3],
            combining_len: 0,
            width: width as u8,
            foreground: pen.foreground,
            background: pen.background,
            attributes: pen.attributes,
        };
        if width == 2 && column + 1 < self.columns {
            self.lines[row][column + 1] = Cell::spacer(pen);
        }
        let advanced = column + width;
        if advanced >= self.columns {
            self.cursor.column = self.columns - 1;
            self.cursor.pending_wrap = autowrap;
        } else {
            self.cursor.column = advanced;
            self.cursor.pending_wrap = false;
        }
    }

    /// Blanks the other half of a double-width character at `column`.
    ///
    /// The grid's one structural invariant is that a width-2 cell is always
    /// followed by a width-0 spacer and a width-0 spacer is always preceded by
    /// a width-2 cell. Every operation that can land on one half alone has to
    /// take the other with it: the renderer skips width-0 cells, so an
    /// orphaned spacer would silently shorten the row and shift everything
    /// after it one column left.
    fn clear_partner(&mut self, row: usize, column: usize, pen: Pen) {
        if column >= self.columns {
            return;
        }
        if self.lines[row][column].width == 0 && column > 0 {
            self.lines[row][column - 1] = Cell::blank(pen);
        }
        if self.lines[row][column].width == 2 && column + 1 < self.columns {
            self.lines[row][column + 1] = Cell::blank(pen);
        }
    }

    /// Blanks `start..end` on `row`, taking with it either half of a
    /// double-width character the range covers only part of.
    fn blank_span(&mut self, row: usize, start: usize, end: usize, pen: Pen) {
        let end = end.min(self.columns);
        if start >= end {
            return;
        }
        self.clear_partner(row, start, pen);
        self.clear_partner(row, end - 1, pen);
        for column in start..end {
            self.lines[row][column] = Cell::blank(pen);
        }
    }

    /// Blanks a trailing width-2 cell whose spacer has been pushed off the
    /// end of the line by a shift or a narrowing resize.
    fn clear_trailing_lead(&mut self, row: usize, pen: Pen) {
        if self.lines[row][self.columns - 1].width == 2 {
            self.lines[row][self.columns - 1] = Cell::blank(pen);
        }
    }

    /// Blanks both halves of a double-width character that the boundary
    /// *before* `column` falls inside.
    ///
    /// A shift moves everything from a boundary and leaves everything before
    /// it, so a character straddling one is about to have its halves separated
    /// by however far the shift goes. Unlike [`Self::clear_partner`], which
    /// serves a caller that is going to overwrite the cell it names anyway,
    /// this has to remove both.
    fn split_before(&mut self, row: usize, column: usize, pen: Pen) {
        if column == 0 || column >= self.columns {
            return;
        }
        if self.lines[row][column].width == 0 {
            self.lines[row][column - 1] = Cell::blank(pen);
            self.lines[row][column] = Cell::blank(pen);
        }
    }

    /// Erase in line: 0 to the end, 1 from the start, 2 the whole line.
    pub fn erase_line(&mut self, mode: u16, pen: Pen) {
        let row = self.cursor.row;
        let column = self.cursor.column.min(self.columns - 1);
        let (start, end) = match mode {
            1 => (0, column + 1),
            2 => (0, self.columns),
            _ => (column, self.columns),
        };
        self.blank_span(row, start, end, pen);
        self.cursor.pending_wrap = false;
    }

    /// Erase in display: 0 below, 1 above, 2 all, 3 scrollback only.
    pub fn erase_display(&mut self, mode: u16, pen: Pen) {
        match mode {
            1 => {
                for row in 0..self.cursor.row {
                    self.lines[row] = self.blank_line(pen);
                }
                self.erase_line(1, pen);
            }
            2 => {
                for row in 0..self.rows {
                    self.lines[row] = self.blank_line(pen);
                }
            }
            3 => {
                self.scrollback.clear();
            }
            _ => {
                self.erase_line(0, pen);
                for row in self.cursor.row + 1..self.rows {
                    self.lines[row] = self.blank_line(pen);
                }
            }
        }
        self.cursor.pending_wrap = false;
    }

    /// Erase characters (ECH): blank `count` cells from the cursor, no shift.
    pub fn erase_characters(&mut self, count: usize, pen: Pen) {
        let row = self.cursor.row;
        let start = self.cursor.column;
        self.blank_span(row, start, start + count, pen);
    }

    /// Delete characters (DCH): shift the rest of the line left.
    pub fn delete_characters(&mut self, count: usize, pen: Pen) {
        let row = self.cursor.row;
        let column = self.cursor.column;
        let count = count.min(self.columns - column);
        if count == 0 {
            return;
        }
        // Both ends of what is about to be removed can fall inside a
        // character, whose halves the shift would then separate.
        self.split_before(row, column, pen);
        self.split_before(row, column + count, pen);
        for _ in 0..count {
            self.lines[row].remove(column);
            self.lines[row].push(Cell::blank(pen));
        }
    }

    /// Insert characters (ICH): shift the rest of the line right.
    pub fn insert_characters(&mut self, count: usize, pen: Pen) {
        let row = self.cursor.row;
        let column = self.cursor.column;
        let count = count.min(self.columns - column);
        if count == 0 {
            return;
        }
        // Inserting inside a character separates its halves; the truncation
        // at the far end can push one off the line entirely.
        self.split_before(row, column, pen);
        for _ in 0..count {
            self.lines[row].insert(column, Cell::blank(pen));
            self.lines[row].truncate(self.columns);
        }
        self.clear_trailing_lead(row, pen);
    }

    /// Insert lines (IL) at the cursor, inside the scrolling region.
    pub fn insert_lines(&mut self, count: usize, pen: Pen) {
        if self.cursor.row < self.scroll_top || self.cursor.row > self.scroll_bottom {
            return;
        }
        let count = count.min(self.scroll_bottom - self.cursor.row + 1);
        for _ in 0..count {
            self.lines.remove(self.scroll_bottom);
            self.lines.insert(self.cursor.row, self.blank_line(pen));
        }
        self.cursor.pending_wrap = false;
    }

    /// Delete lines (DL) at the cursor, inside the scrolling region.
    pub fn delete_lines(&mut self, count: usize, pen: Pen) {
        if self.cursor.row < self.scroll_top || self.cursor.row > self.scroll_bottom {
            return;
        }
        let count = count.min(self.scroll_bottom - self.cursor.row + 1);
        for _ in 0..count {
            self.lines.remove(self.cursor.row);
            self.lines.insert(self.scroll_bottom, self.blank_line(pen));
        }
        self.cursor.pending_wrap = false;
    }

    /// Resizes the screen without reflowing.
    ///
    /// Reflow is deliberately not attempted: emulators disagree about what a
    /// resized wrapped line should become, and a wrong guess corrupts a live
    /// full-screen program worse than a truncated one. Shrinking the height
    /// retires lines from the top into scrollback so the cursor's own row is
    /// never the one thrown away.
    pub fn resize(&mut self, columns: usize, rows: usize, pen: Pen) {
        let columns = columns.max(1);
        let rows = rows.max(1);
        if columns == self.columns && rows == self.rows {
            return;
        }
        let narrowing = columns < self.columns;
        for line in self.lines.iter_mut().chain(self.scrollback.iter_mut()) {
            line.resize(columns, Cell::blank(pen));
            // Narrowing can cut a double-width character in half. The lead is
            // what survives, and a lead with no spacer is not a cell any
            // renderer can place.
            if narrowing && line[columns - 1].width == 2 {
                line[columns - 1] = Cell::blank(pen);
            }
        }
        self.columns = columns;
        if rows < self.rows {
            // Retire from the top only as far as the cursor allows, then trim
            // the foot, so a shell prompt at the bottom stays on screen.
            let mut removed = self.rows - rows;
            let above = self.cursor.row.min(removed);
            for _ in 0..above {
                let line = self.lines.remove(0);
                if self.keeps_history {
                    self.retire(line);
                }
            }
            self.cursor.row -= above;
            removed -= above;
            for _ in 0..removed {
                self.lines.pop();
            }
        } else {
            for _ in self.rows..rows {
                self.lines.push(vec![Cell::blank(pen); columns]);
            }
        }
        self.rows = rows;
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.clamp_cursor();
        self.cursor.pending_wrap = false;
    }

    /// The grid as plain text: scrollback first, then the screen, each line
    /// with its trailing blanks removed.
    pub fn plain_text(&self) -> String {
        let mut text = String::new();
        for line in self.scrollback.iter().chain(self.lines.iter()) {
            push_line_text(&mut text, line);
            text.push('\n');
        }
        while text.ends_with("\n\n") {
            text.pop();
        }
        text
    }
}

fn push_line_text(text: &mut String, line: &Line) {
    let end = line
        .iter()
        .rposition(|cell| cell.width != 0 && cell.character != ' ')
        .map_or(0, |index| index + 1);
    for cell in &line[..end] {
        if cell.width != 0 {
            text.push(cell.character);
            text.extend(cell.combining[..usize::from(cell.combining_len)].iter());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combining_marks_stay_on_their_base_cell_without_taking_a_column() {
        let mut grid = Grid::new(4, 1, false);
        let pen = Pen::default();
        grid.write('e', pen, true);
        grid.write('\u{301}', pen, true);
        grid.write('x', pen, true);
        assert_eq!(grid.plain_text(), "e\u{301}x\n");
        assert_eq!(grid.cursor.column, 2);
        assert_eq!(grid.lines[0][0].combining_len, 1);
    }

    fn row_text(grid: &Grid, row: usize) -> String {
        let mut text = String::new();
        push_line_text(&mut text, grid.line(row).unwrap());
        text
    }

    fn write(grid: &mut Grid, text: &str) {
        for character in text.chars() {
            grid.write(character, Pen::default(), true);
        }
    }

    #[test]
    fn a_line_exactly_as_wide_as_the_screen_wraps_once() {
        let mut grid = Grid::new(4, 3, true);
        write(&mut grid, "abcd");
        assert_eq!(row_text(&grid, 0), "abcd");
        assert_eq!(grid.cursor.row, 0);
        assert!(grid.cursor.pending_wrap);
        write(&mut grid, "e");
        assert_eq!(row_text(&grid, 1), "e");
        assert_eq!(grid.cursor.row, 1);
    }

    #[test]
    fn a_double_width_character_occupies_two_cells_and_leaves_no_orphan() {
        let mut grid = Grid::new(4, 2, true);
        write(&mut grid, "漢a");
        assert_eq!(grid.line(0).unwrap()[0].width, 2);
        assert_eq!(grid.line(0).unwrap()[1].width, 0);
        assert_eq!(row_text(&grid, 0), "漢a");
        grid.move_to(0, 1);
        write(&mut grid, "x");
        assert_eq!(row_text(&grid, 0), " xa");
    }

    #[test]
    fn a_one_column_grid_never_keeps_an_orphaned_wide_lead() {
        let mut grid = Grid::new(1, 1, false);
        write(&mut grid, "漢");

        assert_eq!(grid.line(0).unwrap()[0].character, '漢');
        assert_eq!(grid.line(0).unwrap()[0].width, 1);
    }

    #[test]
    fn disabled_autowrap_overwrites_the_final_cell_without_pending_wrap() {
        let mut grid = Grid::new(2, 1, false);
        let pen = Pen::default();
        grid.write('a', pen, false);
        grid.write('b', pen, false);
        assert!(!grid.cursor.pending_wrap);
        grid.write('c', pen, false);

        assert_eq!(row_text(&grid, 0), "ac");
        assert!(!grid.cursor.pending_wrap);
    }

    #[test]
    fn scrolling_a_top_anchored_region_keeps_history_and_a_lower_region_does_not() {
        let mut grid = Grid::new(4, 3, true);
        write(&mut grid, "one");
        grid.index(Pen::default());
        grid.carriage_return();
        write(&mut grid, "two");
        grid.move_to(2, 0);
        write(&mut grid, "six");
        assert_eq!(grid.scrollback_len(), 0);

        grid.move_to(2, 0);
        grid.index(Pen::default());
        assert_eq!(grid.scrollback_len(), 1);

        grid.set_scroll_region(0, 1);
        grid.move_to(1, 0);
        grid.index(Pen::default());
        assert_eq!(grid.scrollback_len(), 2);

        grid.set_scroll_region(1, 2);
        grid.move_to(2, 0);
        grid.index(Pen::default());
        assert_eq!(grid.scrollback_len(), 2);
    }

    #[test]
    fn shrinking_retires_lines_above_the_cursor_before_trimming_the_foot() {
        let mut grid = Grid::new(8, 4, true);
        for (row, text) in ["first", "second", "third", "fourth"].iter().enumerate() {
            grid.move_to(row, 0);
            write(&mut grid, text);
        }
        grid.move_to(3, 0);
        grid.resize(8, 2, Pen::default());
        assert_eq!(grid.scrollback_len(), 2);
        assert_eq!(row_text(&grid, 0), "third");
        assert_eq!(row_text(&grid, 1), "fourth");
        assert_eq!(grid.cursor.row, 1);
    }

    /// A range that covers one half of a double-width character has to take
    /// the other half with it. The renderer skips width-0 cells, so an orphan
    /// left behind would shorten the row and shift everything after it.
    #[test]
    fn erasing_part_of_a_double_width_character_takes_the_whole_of_it() {
        let mut grid = Grid::new(6, 1, true);
        write(&mut grid, "a漢b");
        // Erase from the spacer: the lead before it must go too.
        grid.move_to(0, 2);
        grid.erase_line(0, Pen::default());
        assert_eq!(row_text(&grid, 0), "a");
        assert!(grid.line(0).unwrap().iter().all(|cell| cell.width == 1));

        let mut grid = Grid::new(6, 1, true);
        write(&mut grid, "a漢b");
        // Erase up to the lead: the spacer after it must go too.
        grid.move_to(0, 1);
        grid.erase_line(1, Pen::default());
        assert_eq!(row_text(&grid, 0), "   b");
        assert!(grid.line(0).unwrap().iter().all(|cell| cell.width == 1));

        let mut grid = Grid::new(6, 1, true);
        write(&mut grid, "a漢b");
        grid.move_to(0, 2);
        grid.erase_characters(1, Pen::default());
        assert_eq!(row_text(&grid, 0), "a  b");
        assert!(grid.line(0).unwrap().iter().all(|cell| cell.width == 1));
    }

    #[test]
    fn shifting_across_a_double_width_character_leaves_no_orphan() {
        let mut grid = Grid::new(6, 1, true);
        write(&mut grid, "a漢b");
        grid.move_to(0, 1);
        grid.delete_characters(1, Pen::default());
        assert!(grid.line(0).unwrap().iter().all(|cell| cell.width == 1));
        assert_eq!(row_text(&grid, 0), "a b");

        let mut grid = Grid::new(6, 1, true);
        write(&mut grid, "a漢b");
        grid.move_to(0, 2);
        grid.insert_characters(1, Pen::default());
        assert!(grid.line(0).unwrap().iter().all(|cell| cell.width == 1));

        // Shifting right can also push a lead's spacer off the end.
        let mut grid = Grid::new(4, 1, true);
        write(&mut grid, "ab漢");
        grid.move_to(0, 0);
        grid.insert_characters(1, Pen::default());
        assert!(grid.line(0).unwrap().iter().all(|cell| cell.width == 1));
    }

    #[test]
    fn narrowing_never_keeps_half_a_character() {
        let mut grid = Grid::new(4, 1, true);
        write(&mut grid, "ab漢");
        grid.resize(3, 1, Pen::default());
        assert!(grid.line(0).unwrap().iter().all(|cell| cell.width == 1));
        assert_eq!(row_text(&grid, 0), "ab");
    }

    /// A reader holding a position in history needs to know that a line slid
    /// past, and once the limit is reached the length stops saying so.
    #[test]
    fn retirement_keeps_counting_after_the_scrollback_limit() {
        let mut grid = Grid::new(4, 1, true);
        for _ in 0..SCROLLBACK_LIMIT + 10 {
            grid.index(Pen::default());
        }
        assert_eq!(grid.scrollback_len(), SCROLLBACK_LIMIT);
        assert_eq!(grid.retired(), SCROLLBACK_LIMIT as u64 + 10);
    }

    #[test]
    fn erasing_paints_the_pen_background_but_not_its_foreground() {
        let mut grid = Grid::new(4, 1, true);
        let pen = Pen {
            foreground: Color::Indexed(1),
            background: Color::Indexed(4),
            attributes: Attributes::BOLD,
        };
        grid.erase_line(2, pen);
        let cell = grid.line(0).unwrap()[0];
        assert_eq!(cell.background, Color::Indexed(4));
        assert_eq!(cell.foreground, Color::Default);
        assert!(cell.attributes.is_empty());
    }

    #[test]
    fn plain_text_puts_scrollback_before_the_screen() {
        let mut grid = Grid::new(8, 2, true);
        write(&mut grid, "one");
        grid.index(Pen::default());
        grid.carriage_return();
        write(&mut grid, "two");
        grid.index(Pen::default());
        grid.carriage_return();
        write(&mut grid, "three");
        assert_eq!(grid.plain_text(), "one\ntwo\nthree\n");
    }

    #[test]
    fn erase_display_three_clears_only_scrollback() {
        let mut grid = Grid::new(8, 2, true);
        write(&mut grid, "one");
        grid.index(Pen::default());
        grid.carriage_return();
        write(&mut grid, "two");
        grid.index(Pen::default());
        grid.carriage_return();
        write(&mut grid, "three");
        assert_eq!(grid.scrollback_len(), 1);

        grid.erase_display(3, Pen::default());

        assert_eq!(grid.scrollback_len(), 0);
        assert_eq!(row_text(&grid, 0), "two");
        assert_eq!(row_text(&grid, 1), "three");
    }
}
