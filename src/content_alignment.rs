// SPDX-License-Identifier: MPL-2.0

//! Where a generated buffer's content sits inside the pane showing it.
//!
//! Alignment is presentation, not text. A page that wants to be centred says
//! so once, and the blank space in front of it is recomputed from the live
//! pane geometry every frame; the buffer keeps the columns and rows it was
//! generated with however wide or tall the pane becomes. Nothing here is
//! reachable by a caret, a search, or a save, because none of it is in the
//! buffer.
//!
//! That boundary is the point. A row and column in an aligned buffer mean the
//! same thing at every pane size, so anything a producer anchors to them — a
//! row hint today, an activatable region in a later interactive page — stays
//! valid across a resize, and a frontend translates once, in one place, to
//! decide what a person pointed at.
//!
//! Content is placed as one block: the whole page shifts together, so relative
//! indentation inside it survives. Centring each line on its own would pull an
//! ASCII logo apart, and the shape of such a block is exactly what the
//! producer meant.

use crate::row_hints::display_cells;

/// Where a block of content sits across the width of a pane.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Horizontal {
    #[default]
    Left,
    Center,
}

/// Where a block of content sits down the height of a pane.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Vertical {
    #[default]
    Top,
    Center,
}

/// How one buffer asks to be placed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ContentAlignment {
    pub horizontal: Horizontal,
    pub vertical: Vertical,
}

impl ContentAlignment {
    /// Centred on both axes: the placement a generated front page wants.
    pub const CENTERED: Self = Self {
        horizontal: Horizontal::Center,
        vertical: Vertical::Center,
    };
}

/// An alignment together with the width of the content it describes.
///
/// The width is measured when the text is set rather than every frame, so a
/// pane resize costs the two divisions below and nothing else. It is the width
/// of the whole buffer, not of the visible rows: measuring what is on screen
/// would slide the page sideways as it scrolled.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ContentLayout {
    alignment: ContentAlignment,
    cells: usize,
    maximum_width: Option<usize>,
}

impl ContentLayout {
    /// The layout of `text` under `alignment`, measured in display cells.
    pub fn measured(alignment: ContentAlignment, text: &str) -> Self {
        Self {
            alignment,
            cells: text.lines().map(display_cells).max().unwrap_or_default(),
            maximum_width: None,
        }
    }

    /// A horizontally centred viewport with a fixed maximum text width.
    ///
    /// Unlike [`Self::measured`], this describes space the editor is allowed
    /// to use rather than the current shape of a buffer. Editable prose can
    /// therefore stay still as its longest line changes.
    pub const fn viewport(width: usize) -> Self {
        Self {
            alignment: ContentAlignment {
                horizontal: Horizontal::Center,
                vertical: Vertical::Top,
            },
            cells: width,
            maximum_width: Some(width),
        }
    }

    /// The same alignment, measured against replacement text.
    pub fn remeasured(self, text: &str) -> Self {
        Self::measured(self.alignment, text)
    }

    /// The blank cells drawn before every row, given the cells a pane leaves
    /// for text.
    ///
    /// Zero once the content is wider than the pane: a block that does not fit
    /// is shown from its first column, so scrolling right still reaches the
    /// rest of it.
    pub fn indent(&self, text_width: usize) -> usize {
        match self.alignment.horizontal {
            Horizontal::Left => 0,
            Horizontal::Center => text_width.saturating_sub(self.cells) / 2,
        }
    }

    /// The cells available to the placed text after horizontal alignment.
    ///
    /// Generated pages keep the remaining pane width so an unusually wide
    /// row can still be reached by horizontal scrolling. A fixed viewport is
    /// capped at its requested width, leaving equal blank space on both sides
    /// whenever the pane is wide enough.
    pub fn width(&self, available: usize) -> usize {
        self.maximum_width.map_or_else(
            || available.saturating_sub(self.indent(available)),
            |width| width.min(available),
        )
    }

    /// The blank rows drawn above `content_rows` of content in a pane
    /// `body_height` rows tall.
    ///
    /// Zero when the content is at least as tall as the pane, which is also
    /// the only case in which it has anywhere to scroll.
    pub fn top(&self, content_rows: usize, body_height: usize) -> usize {
        match self.alignment.vertical {
            Vertical::Top => 0,
            Vertical::Center => body_height.saturating_sub(content_rows) / 2,
        }
    }

    /// Whether this content is placed down the pane rather than from its top.
    pub fn centers_vertically(&self) -> bool {
        self.alignment.vertical == Vertical::Center
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_centered_block_is_measured_at_its_widest_line() {
        let layout = ContentLayout::measured(ContentAlignment::CENTERED, "ab\nabcd\nabc\n");

        assert_eq!(layout.indent(10), 3);
        assert_eq!(layout.width(10), 7);
        assert_eq!(layout.top(3, 9), 3);
    }

    #[test]
    fn a_fixed_viewport_is_centered_and_capped_without_measuring_text() {
        let layout = ContentLayout::viewport(6);

        assert_eq!(layout.indent(10), 2);
        assert_eq!(layout.width(10), 6);
        assert_eq!(layout.indent(4), 0);
        assert_eq!(layout.width(4), 4);
        assert!(!layout.centers_vertically());
    }

    #[test]
    fn content_wider_or_taller_than_the_pane_starts_at_its_first_cell() {
        let layout = ContentLayout::measured(ContentAlignment::CENTERED, "abcdef\n");

        assert_eq!(layout.indent(6), 0);
        assert_eq!(layout.indent(4), 0);
        assert_eq!(layout.top(9, 9), 0);
        assert_eq!(layout.top(12, 9), 0);
    }

    #[test]
    fn wide_glyphs_are_measured_in_cells_rather_than_characters() {
        let layout = ContentLayout::measured(ContentAlignment::CENTERED, "ああ\n");

        assert_eq!(layout.indent(10), 3);
    }

    #[test]
    fn an_unaligned_buffer_is_never_padded() {
        let layout = ContentLayout::default();

        assert_eq!(layout.indent(80), 0);
        assert_eq!(layout.top(1, 40), 0);
        assert!(!layout.centers_vertically());
    }

    #[test]
    fn each_axis_is_chosen_on_its_own() {
        let down_the_left = ContentLayout::measured(
            ContentAlignment {
                horizontal: Horizontal::Left,
                vertical: Vertical::Center,
            },
            "ab\n",
        );

        assert_eq!(down_the_left.indent(10), 0);
        assert_eq!(down_the_left.top(1, 9), 4);
    }

    #[test]
    fn replacement_text_is_measured_again() {
        let layout = ContentLayout::measured(ContentAlignment::CENTERED, "ab\n");

        assert_eq!(layout.indent(10), 4);
        assert_eq!(layout.remeasured("abcdef\n").indent(10), 2);
    }
}
