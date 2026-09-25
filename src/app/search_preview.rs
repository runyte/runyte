// SPDX-License-Identifier: MPL-2.0

//! The matches an open `s` or `/` prompt would select if it were accepted now.
//!
//! The preview is drawn over the pane without touching its selection. The
//! selection, the jumplist, and the committed query stay what they were until
//! Enter, so abandoning the prompt has nothing to undo apart from the scroll
//! the preview used to show its match.

use super::{
    App, Mode, Pane, PromptKind, Range, SearchMode, primary_match, region_spans, text_matches,
};
use crate::snapshot::TextRole;

/// Where the pane was looking when the prompt opened.
#[derive(Clone, Copy, Debug)]
struct PreviewOrigin {
    scroll_row: usize,
    scroll_wrap: usize,
    scroll_col: usize,
    preserve_scroll: bool,
}

impl PreviewOrigin {
    fn restore(self, pane: &mut Pane) {
        pane.scroll_row = self.scroll_row;
        pane.scroll_wrap = self.scroll_wrap;
        pane.scroll_col = self.scroll_col;
        pane.preserve_scroll = self.preserve_scroll;
    }
}

#[derive(Debug)]
pub(crate) struct SearchPreview {
    pane: usize,
    buffer: usize,
    mode: SearchMode,
    origin: PreviewOrigin,
    /// The pattern `matches` was computed for. With `revision`, lets a frame
    /// that changes neither reuse them.
    pattern: String,
    /// The buffer revision `matches` and `text` were computed for.
    revision: u64,
    /// The buffer's text at `revision`. Each keystroke reruns the pattern, and
    /// flattening a large rope for every one of them would cost more than the
    /// search itself.
    text: Option<String>,
    matches: Vec<Range>,
    primary: usize,
}

impl SearchPreview {
    /// The role the match covering `offset` gives it, if one does.
    ///
    /// Mirrors what the pane draws once the search is accepted: the primary
    /// match in the primary colour with its caret on its head, and every other
    /// match in the secondary colour. Matches are ordered and never overlap,
    /// so this is a binary search rather than a walk over every match for
    /// every visible character.
    pub(crate) fn role_at(&self, offset: usize) -> TextRole {
        let index = self.matches.partition_point(|range| range.to() < offset);
        match self.matches.get(index) {
            Some(range) if range.from() <= offset => {
                if index != self.primary {
                    TextRole::Selected
                } else if range.head == offset {
                    TextRole::PrimaryCaret
                } else {
                    TextRole::PrimarySelected
                }
            }
            _ => TextRole::Plain,
        }
    }

    /// The offset the viewport follows while the preview is shown.
    fn head(&self) -> usize {
        self.matches[self.primary].head
    }
}

impl App {
    /// Starts previewing for a search prompt that has just opened.
    pub(super) fn begin_search_preview(&mut self) {
        let PromptKind::Search(mode) = self.prompt_kind else {
            return;
        };
        let pane = self.active();
        self.search_preview = Some(SearchPreview {
            pane: self.active_pane,
            buffer: pane.buffer,
            mode,
            origin: PreviewOrigin {
                scroll_row: pane.scroll_row,
                scroll_wrap: pane.scroll_wrap,
                scroll_col: pane.scroll_col,
                preserve_scroll: pane.preserve_scroll,
            },
            pattern: String::new(),
            revision: self.active_buffer().revision(),
            text: None,
            matches: Vec::new(),
            primary: 0,
        });
    }

    /// Stops previewing and puts the pane back where it was looking.
    pub(super) fn abandon_search_preview(&mut self) {
        if let Some(preview) = self.search_preview.take() {
            self.restore_preview_origin(&preview);
        }
    }

    /// Ends the preview once the search it showed has been accepted.
    ///
    /// A committed search scrolls to the same primary match the preview was
    /// already showing, so the pane keeps its view rather than returning to
    /// the origin and scrolling out again. A search that found nothing leaves
    /// the selection where it was, and the view goes back to it.
    pub(super) fn finish_search_preview(&mut self, preview: Option<SearchPreview>, found: bool) {
        if let Some(preview) = preview
            && !found
        {
            self.restore_preview_origin(&preview);
        }
    }

    pub(super) fn take_search_preview(&mut self) -> Option<SearchPreview> {
        self.search_preview.take()
    }

    fn restore_preview_origin(&mut self, preview: &SearchPreview) {
        if let Some(pane) = self.panes.get_mut(&preview.pane)
            && pane.buffer == preview.buffer
        {
            preview.origin.restore(pane);
        }
    }

    /// Brings the preview up to date with the prompt text and buffer.
    ///
    /// Called once per frame rather than from each prompt edit, so typing,
    /// deleting a word, pasting, and a buffer changing underneath the prompt
    /// all reach the preview by the same path.
    ///
    /// An empty pattern or one found nowhere shows nothing and returns the
    /// view to where the prompt opened. An unfinished regular expression, such
    /// as `foo(` on the way to `foo(bar)`, keeps the last preview rather than
    /// flashing back to the origin between keystrokes.
    pub(super) fn refresh_search_preview(&mut self) {
        if self.mode != Mode::Command || !matches!(self.prompt_kind, PromptKind::Search(_)) {
            self.abandon_search_preview();
            return;
        }
        let Some(preview) = self.search_preview.as_ref() else {
            return;
        };
        let Some(pane) = self
            .panes
            .get(&preview.pane)
            .filter(|pane| pane.buffer == preview.buffer)
        else {
            self.search_preview = None;
            return;
        };
        let buffer = &self.buffers[pane.buffer];
        let revision = buffer.revision();
        if preview.pattern == self.command && preview.revision == revision {
            return;
        }
        // Read from the pane on every refresh, as Enter reads them, so an
        // edit arriving under the open prompt cannot leave the preview
        // answering for a region or a caret that has since moved.
        let spans = self
            .pane_scoping_region(preview.pane)
            .map(|region| region_spans(buffer, &region));
        let cursor = pane.selection.primary().from();
        let stale = preview.text.is_none() || preview.revision != revision;
        let fresh = stale.then(|| buffer.to_string());
        let preview = self.search_preview.as_mut().unwrap();
        if let Some(text) = fresh {
            preview.text = Some(text);
        }
        let text = preview.text.as_deref().unwrap_or_default();
        let result = text_matches(text, &self.command, preview.mode, spans.as_deref());
        preview.pattern.clone_from(&self.command);
        preview.revision = revision;
        let Ok(matches) = result else {
            return;
        };
        preview.primary = primary_match(&matches, cursor);
        preview.matches = matches;
        let (pane, origin, found) = (preview.pane, preview.origin, !preview.matches.is_empty());
        let Some(pane) = self.panes.get_mut(&pane) else {
            return;
        };
        if found {
            // A pane the mouse wheel had scrolled away from its caret would
            // otherwise keep the match out of sight.
            pane.preserve_scroll = false;
        } else {
            origin.restore(pane);
        }
    }

    /// The preview drawn over `pane`, when it has something to draw.
    pub(crate) fn search_preview_for(&self, pane: usize, buffer: usize) -> Option<&SearchPreview> {
        self.search_preview
            .as_ref()
            .filter(|preview| preview.pane == pane && preview.buffer == buffer)
            .filter(|preview| !preview.matches.is_empty())
    }

    /// The offset `pane`'s viewport should keep in sight this frame.
    pub(super) fn search_preview_head(&self, pane: usize, buffer: usize) -> Option<usize> {
        self.search_preview_for(pane, buffer)
            .map(SearchPreview::head)
    }
}
