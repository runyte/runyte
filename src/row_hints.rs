// SPDX-License-Identifier: MPL-2.0

//! Read-only annotations shown after the text of a row.
//!
//! A hint is not part of the buffer. It cannot be selected, edited, saved, or
//! parsed back out of a line, so a producer can say something about a row —
//! what a symlink points at, and whatever later views need — without that
//! sentence becoming text the person has to edit around.
//!
//! The layout rule lives here rather than in each producer so hints read the
//! same wherever they appear. [`RowHints::aligned`] lines every annotated row
//! in one buffer up in the same column, [`GAP`] cells past the longest of
//! them — the right choice when rows are close enough in length that a
//! shared column reads as a table, as filenames in an explorer usually are.
//! [`RowHints::trailing`] instead sits a row's hint one space past that row's
//! own text with no shared column, which keeps every hint reachable even
//! when one row (an overlong commit subject, say) is far longer than the
//! rest. Either way a frontend paints every hint in one muted style.

use std::collections::HashMap;

use unicode_width::UnicodeWidthChar;

/// Cells between the longest annotated row and the hint column.
pub const GAP: usize = 2;

/// The hints one buffer carries, and the column they share.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RowHints {
    texts: HashMap<usize, String>,
    column: usize,
}

impl RowHints {
    /// Collects hints and the column they align on.
    ///
    /// Each entry is the row, the display width of that row's text, and the
    /// hint itself. Widths come from the producer because only it knows which
    /// text the hint is being placed after.
    pub fn aligned(entries: impl IntoIterator<Item = (usize, usize, String)>) -> Self {
        Self::aligned_with_gap(entries, GAP)
    }

    /// Collects hints with an explicit minimum gap after the row text.
    ///
    /// Most annotations use [`GAP`]. A producer may use one cell when the
    /// buffer text already ends in punctuation that separates the hint, such
    /// as the final `|` in a generated page heading.
    pub fn aligned_with_gap(
        entries: impl IntoIterator<Item = (usize, usize, String)>,
        gap: usize,
    ) -> Self {
        let mut texts = HashMap::new();
        let mut column = 0;
        for (row, cells, text) in entries {
            if text.is_empty() {
                continue;
            }
            column = column.max(cells + gap.max(1));
            texts.insert(row, text);
        }
        Self { texts, column }
    }

    /// Collects hints that sit one space past each row's own text, with no
    /// shared column.
    ///
    /// Unlike [`Self::aligned`], one row's hint never moves because another
    /// row's text or hint is longer — the right choice when rows vary widely
    /// in length, so a handful of short rows never lose their hints off the
    /// right edge just because one row elsewhere ran long.
    pub fn trailing(entries: impl IntoIterator<Item = (usize, String)>) -> Self {
        let texts = entries
            .into_iter()
            .filter(|(_, text)| !text.is_empty())
            .collect();
        Self { texts, column: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }

    /// The column, in cells from the start of the row's text, where every hint
    /// in this buffer begins.
    pub fn column(&self) -> usize {
        self.column
    }

    pub fn text(&self, row: usize) -> Option<&str> {
        self.texts.get(&row).map(String::as_str)
    }

    /// The padding and hint to draw after `used_cells` of already-drawn text,
    /// within `remaining_cells` of viewport.
    ///
    /// A row longer than the shared column still keeps one space, so a hint is
    /// never mistaken for a continuation of the text. Nothing is returned when
    /// the viewport leaves no room for the hint itself; a run of trailing
    /// blanks would say less than the clipped text it replaced.
    pub fn rendered(
        &self,
        row: usize,
        used_cells: usize,
        remaining_cells: usize,
    ) -> Option<String> {
        let text = self.texts.get(&row)?;
        let padding = self.column.saturating_sub(used_cells).max(1);
        if padding >= remaining_cells {
            return None;
        }
        let mut rendered = " ".repeat(padding);
        let mut width = padding;
        for character in text.chars() {
            let cells = display_cells_of(character);
            if width + cells > remaining_cells {
                break;
            }
            rendered.push(character);
            width += cells;
        }
        Some(rendered)
    }
}

/// The display width of `text` in terminal cells.
pub fn display_cells(text: &str) -> usize {
    text.chars().map(display_cells_of).sum()
}

fn display_cells_of(character: char) -> usize {
    UnicodeWidthChar::width(character).unwrap_or(0).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_align_past_the_longest_annotated_row() {
        let hints = RowHints::aligned([
            (0, 4, "→ a".to_owned()),
            (2, 10, "→ b".to_owned()),
            (3, 6, "→ c".to_owned()),
        ]);

        assert_eq!(hints.column(), 12);
        assert_eq!(hints.rendered(0, 4, 40).as_deref(), Some("        → a"));
        assert_eq!(hints.rendered(2, 10, 40).as_deref(), Some("  → b"));
        assert_eq!(hints.text(1), None);
        assert_eq!(hints.rendered(1, 4, 40), None);
    }

    #[test]
    fn punctuation_can_separate_a_hint_with_one_cell() {
        let hints = RowHints::aligned_with_gap([(0, 12, "(keys)".to_owned())], 1);

        assert_eq!(hints.column(), 13);
        assert_eq!(hints.rendered(0, 12, 20).as_deref(), Some(" (keys)"));
    }

    #[test]
    fn an_unannotated_buffer_has_no_hints() {
        let hints = RowHints::aligned([]);

        assert!(hints.is_empty());
        assert_eq!(hints.column(), 0);
    }

    #[test]
    fn a_row_past_the_shared_column_keeps_one_space() {
        let hints = RowHints::aligned([(0, 4, "→ a".to_owned())]);

        assert_eq!(hints.rendered(0, 20, 40).as_deref(), Some(" → a"));
    }

    #[test]
    fn trailing_hints_sit_one_space_past_each_rows_own_text_regardless_of_others() {
        let hints = RowHints::trailing([
            (0, "(short)".to_owned()),
            (1, "(a much, much longer hint on another row)".to_owned()),
        ]);

        assert_eq!(hints.column(), 0);
        // Neither row's placement depends on how long the other row's text
        // or hint is, unlike `aligned`.
        assert_eq!(hints.rendered(0, 4, 40).as_deref(), Some(" (short)"));
        assert_eq!(hints.rendered(0, 60, 40).as_deref(), Some(" (short)"));
    }

    #[test]
    fn trailing_skips_empty_hints_and_drops_them_from_the_map() {
        let hints = RowHints::trailing([(0, String::new()), (1, "(main)".to_owned())]);

        assert_eq!(hints.text(0), None);
        assert_eq!(hints.text(1), Some("(main)"));
    }

    #[test]
    fn a_hint_is_clipped_to_the_remaining_viewport() {
        let hints = RowHints::aligned([(0, 2, "→ long-target".to_owned())]);

        assert_eq!(hints.rendered(0, 2, 8).as_deref(), Some("  → long"));
        assert_eq!(hints.rendered(0, 2, 3).as_deref(), Some("  →"));
        assert_eq!(hints.rendered(0, 2, 2), None, "no room past the padding");
    }

    #[test]
    fn wide_characters_count_as_two_cells() {
        assert_eq!(display_cells("あ"), 2);
        assert_eq!(display_cells("ab"), 2);

        let hints = RowHints::aligned([(0, 2, "→ あい".to_owned())]);

        // A wide glyph is never split across the edge of the viewport.
        assert_eq!(hints.rendered(0, 2, 7).as_deref(), Some("  → あ"));
        assert_eq!(hints.rendered(0, 2, 8).as_deref(), Some("  → あい"));
    }
}
