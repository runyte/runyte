// SPDX-License-Identifier: MPL-2.0

//! Pane-owned selection and viewport positions for retained generated views.

use crate::selection::Selection;

#[derive(Clone, Debug)]
pub(super) struct ViewPosition<S = Selection> {
    pub selection: S,
    pub scroll_row: usize,
    pub scroll_wrap: usize,
    pub scroll_col: usize,
}
impl<S> ViewPosition<S> {
    pub(super) fn capture_with(pane: &super::Pane, selection: S) -> Self {
        Self {
            selection,
            scroll_row: pane.scroll_row,
            scroll_wrap: pane.scroll_wrap,
            scroll_col: pane.scroll_col,
        }
    }
    pub(super) fn restore_with(&self, pane: &mut super::Pane, selection: Selection) {
        pane.replace_selection(selection);
        pane.scroll_row = self.scroll_row;
        pane.scroll_wrap = self.scroll_wrap;
        pane.scroll_col = self.scroll_col;
        pane.preserve_scroll = true;
    }
}
impl ViewPosition {
    pub(super) fn capture(pane: &super::Pane) -> Self {
        Self::capture_with(pane, pane.selection.clone())
    }
    pub(super) fn restore(&self, pane: &mut super::Pane) {
        self.restore_with(pane, self.selection.clone());
    }
}
