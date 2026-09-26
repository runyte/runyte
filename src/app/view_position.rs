// SPDX-License-Identifier: MPL-2.0

//! Pane-owned selection and viewport positions for retained generated views.

use crate::selection::Selection;

#[derive(Clone, Debug)]
pub(super) struct ViewPosition {
    pub selection: Selection,
    pub scroll_row: usize,
    pub scroll_wrap: usize,
    pub scroll_col: usize,
}
impl ViewPosition {
    pub(super) fn capture(pane: &super::Pane) -> Self {
        Self {
            selection: pane.selection.clone(),
            scroll_row: pane.scroll_row,
            scroll_wrap: pane.scroll_wrap,
            scroll_col: pane.scroll_col,
        }
    }
    pub(super) fn restore(&self, pane: &mut super::Pane) {
        pane.replace_selection(self.selection.clone());
        pane.scroll_row = self.scroll_row;
        pane.scroll_wrap = self.scroll_wrap;
        pane.scroll_col = self.scroll_col;
        pane.preserve_scroll = true;
    }
}
