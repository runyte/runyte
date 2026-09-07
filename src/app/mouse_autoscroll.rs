// SPDX-License-Identifier: MPL-2.0

//! Selection drags advance only while the pointer is at a vertical pane edge.

use std::time::{Duration, Instant};

use super::{App, FrameGeometry, Mode, Offset, PointerDrag, PointerEvent, PreparedView};

const INTERVAL: Duration = Duration::from_millis(60);

#[derive(Clone, Copy, Debug)]
pub(super) struct Autoscroll {
    drag: PointerDrag,
    event: PointerEvent,
    direction: i32,
    due: Instant,
}

impl App {
    /// A continued document drag belongs to its original pane, even when the
    /// pointer crosses another pane or the displayed frame has scrolled.
    pub fn pointer_selection_drag_active(&self) -> bool {
        matches!(self.pointer_drag, Some(PointerDrag::Selection { .. }))
    }

    /// Ends a pointer gesture when keyboard input or attachment loss takes over.
    /// Returns whether Runyte owned a gesture, rather than the terminal child.
    pub fn cancel_pointer_drag(&mut self) -> bool {
        let owned = self.pointer_drag.take().is_some();
        self.pointer_autoscroll = None;
        owned
    }

    fn valid_pointer_autoscroll(&self) -> Option<Autoscroll> {
        let scroll = self.pointer_autoscroll?;
        let PointerDrag::Selection { pane, buffer, .. } = scroll.drag else {
            return None;
        };
        (self.pointer_drag == Some(scroll.drag)
            && self.active_pane == pane
            && self
                .panes
                .get(&pane)
                .is_some_and(|p| p.buffer == buffer && p.terminal.is_none())
            && self.mode != Mode::Command
            && !self.has_input_overlay()
            && !self.macro_replay_pending())
        .then_some(scroll)
    }

    /// No idle wakeup is needed when there is no active edge drag.
    pub fn pointer_autoscroll_delay(&self, now: Instant) -> Option<Duration> {
        self.valid_pointer_autoscroll()
            .map(|scroll| scroll.due.saturating_duration_since(now))
    }

    pub(super) fn update_pointer_autoscroll(&mut self, event: PointerEvent, view: &PreparedView) {
        let Some(drag @ PointerDrag::Selection { pane, .. }) = self.pointer_drag else {
            self.pointer_autoscroll = None;
            return;
        };
        let Some(prepared) = view.pane(pane).filter(|p| p.drawable && p.body.height > 1) else {
            self.pointer_autoscroll = None;
            return;
        };
        let direction = if event.row <= prepared.body.y {
            -1
        } else if event.row >= prepared.body.y + prepared.body.height - 1 {
            1
        } else {
            self.pointer_autoscroll = None;
            return;
        };
        let due = self
            .pointer_autoscroll
            .filter(|scroll| scroll.drag == drag && scroll.direction == direction)
            .map_or_else(|| Instant::now() + INTERVAL, |scroll| scroll.due);
        self.pointer_autoscroll = Some(Autoscroll {
            drag,
            event,
            direction,
            due,
        });
    }

    /// Unlike a press, a drag outside the body extends to the nearest text row.
    /// Padding and diff filler have no offsets of their own.
    pub(super) fn pointer_drag_offset(
        &self,
        view: &PreparedView,
        pane: usize,
        column: u16,
        row: u16,
    ) -> Option<Offset> {
        let prepared = view.pane(pane).filter(|p| p.drawable && p.body.width > 0)?;
        let target = usize::from(row.saturating_sub(prepared.body.y));
        let nearest = prepared
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.document_row.is_some())
            .min_by_key(|(index, _)| index.abs_diff(target))?
            .0;
        self.pointer_offset(
            view,
            pane,
            column.clamp(prepared.body.x, prepared.body.x + prepared.body.width - 1),
            prepared.body.y + nearest as u16,
            false,
        )
    }

    /// Advances one visual row, then resolves the held pointer against the new
    /// projection. Late ticks never catch up in a burst.
    pub fn advance_pointer_autoscroll(&mut self, now: Instant, geometry: FrameGeometry) -> bool {
        let Some(scroll) = self.valid_pointer_autoscroll() else {
            self.pointer_autoscroll = None;
            return false;
        };
        if now < scroll.due {
            return false;
        }
        let PointerDrag::Selection { pane, anchor, .. } = scroll.drag else {
            return false;
        };
        let view = self.prepare_view(geometry);
        self.update_pointer_autoscroll(scroll.event, &view);
        let Some(current) = self.pointer_autoscroll else {
            return false;
        };
        let before = (self.panes[&pane].scroll_row, self.panes[&pane].scroll_wrap);
        self.scroll_pane(pane, current.direction);
        let view = self.prepare_view(geometry);
        let after = (self.panes[&pane].scroll_row, self.panes[&pane].scroll_wrap);
        if before == after {
            self.pointer_autoscroll = None;
            return false;
        }
        if let Some(offset) =
            self.pointer_drag_offset(&view, pane, scroll.event.column, scroll.event.row)
        {
            let (selection, semantics) = self.pointer_selection(anchor, offset);
            let candidate = self.panes.get_mut(&pane).unwrap();
            candidate.replace_selection(selection);
            candidate.mark_selection_semantics(semantics);
            candidate.preserve_scroll = true;
            self.mode = if anchor == offset {
                Mode::Normal
            } else {
                Mode::Select
            };
        }
        self.pointer_autoscroll = Some(Autoscroll {
            due: now + INTERVAL,
            ..current
        });
        true
    }
}
