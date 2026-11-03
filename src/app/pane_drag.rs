// SPDX-License-Identifier: MPL-2.0

//! Alt-drag rearranges panes with the same tree moves `Ctrl-w H/J/K/L` make.
//!
//! A press with Alt held picks up the pane under the pointer; releasing it
//! lays the pane along a side of the layout, splits another pane to make room
//! for it, or swaps contents with another pane. Shared borders keep resizing
//! whether Alt is held or not, so the two gestures never compete for a cell.
//! Shift-Alt-drag instead extends the pane in place, shrinking or closing
//! neighbours, with the same preview, cancellation and release ownership.

use std::collections::HashMap;

use super::{
    App, Axis, Edge, InputGrammar, Layout, PointerButton, PointerDrag, PointerEvent,
    PointerEventKind, PreparedView, Rect, pointer_pane, pointer_resize_pair, rect_contains,
};

/// Columns along the left and right of the layout that lay a pane along
/// that whole side. Cells are about twice as tall as they are wide, so the
/// side bands are twice as wide as the top and bottom ones are tall.
const SIDE_BAND_COLUMNS: u16 = 2;
const SIDE_BAND_ROWS: u16 = 1;

/// Where an Alt-dragged pane would land if it were released now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PaneDrop {
    /// Along one whole side of the layout, spanning everything else.
    Edge { axis: Axis, edge: Edge },
    /// Into one half of another pane, which is split to make room.
    Beside {
        target: usize,
        axis: Axis,
        edge: Edge,
    },
    /// Into another pane's place, exchanging contents with it.
    Swap { target: usize },
    /// Grow in place to this rectangle, shrinking or closing neighbours.
    Expand { area: Rect },
}

impl App {
    /// A stale configuration must not reinterpret a new press or deliver its
    /// orphaned motion/release to terminal content beneath it.
    pub(crate) fn discard_pointer_gesture(&mut self, button: PointerButton) {
        self.cancel_pointer_drag();
        self.pane_pointer_owned = true;
        self.pane_pointer_button = Some(button);
    }

    pub(super) fn begin_pointer_drag(&mut self, drag: PointerDrag, button: PointerButton) {
        self.pointer_drag = Some(drag);
        self.pane_pointer_button = Some(button);
        self.pane_pointer_owned = button != PointerButton::Left;
    }

    pub(crate) fn pointer_drag_matches(&self, event: PointerEvent) -> bool {
        event.kind
            == PointerEventKind::Drag(self.pane_pointer_button.unwrap_or(PointerButton::Left))
    }

    pub(crate) fn release_pointer_drag(&mut self, event: PointerEvent) -> bool {
        let retain = self.pane_pointer_owned
            && self.pane_pointer_button.is_some_and(|button| {
                button != PointerButton::Left && event.kind != PointerEventKind::Up(button)
            });
        let owned = self.cancel_pointer_drag();
        self.pane_pointer_owned = retain;
        owned
    }

    /// A stale drop cannot commit geometry, but an unrelated button release
    /// must still leave the initiating button's remaining events captured.
    pub(crate) fn cancel_pane_drop(&mut self, event: PointerEvent) {
        let retain = matches!(self.pointer_drag, Some(PointerDrag::MovePane { button, .. })
            if button != PointerButton::Left && event.kind != PointerEventKind::Up(button));
        self.cancel_pointer_drag();
        self.pane_pointer_owned = retain;
    }

    /// Whether a release needs to resolve a pane drop against its visible frame.
    pub fn pointer_pane_drag_active(&self) -> bool {
        matches!(self.pointer_drag, Some(PointerDrag::MovePane { .. }))
    }

    /// Keyboard and semantic commands cancel the move, but cannot hand the
    /// remainder of its physical gesture to a terminal or another pane.
    pub(super) fn cancel_pointer_drag_preserving_pane_release(&mut self) {
        let owned = self.pane_pointer_owned;
        self.cancel_pointer_drag();
        self.pane_pointer_owned = owned;
    }

    pub(crate) fn consume_cancelled_pane_pointer(&mut self, event: PointerEvent) -> bool {
        if !self.pane_pointer_owned || self.pointer_drag.is_some() {
            return false;
        }
        match event.kind {
            PointerEventKind::Down(_) => {
                self.pane_pointer_owned = false;
                false
            }
            PointerEventKind::Up(button) => {
                if self
                    .pane_pointer_button
                    .is_none_or(|captured| captured == PointerButton::Left || captured == button)
                {
                    self.pane_pointer_owned = false;
                }
                true
            }
            _ => true,
        }
    }

    /// Starts, follows, or completes an Alt-drag of a pane.
    ///
    /// Returns whether the event belonged to one, in which case nothing else
    /// may interpret it: not even a terminal child that asked for the mouse.
    pub(super) fn handle_pane_drag(&mut self, event: PointerEvent, view: &PreparedView) -> bool {
        if let Some(PointerDrag::MovePane {
            pane,
            expand,
            button,
            ..
        }) = self.pointer_drag
        {
            match event.kind {
                PointerEventKind::Drag(pressed) if pressed == button => {
                    let drop = self.pane_drop_at(view, pane, expand, event.column, event.row);
                    self.pointer_drag = Some(PointerDrag::MovePane {
                        pane,
                        expand,
                        drop,
                        button,
                    });
                    self.describe_pane_drop(drop);
                }
                PointerEventKind::Up(_) => {
                    self.pane_pointer_owned =
                        button != PointerButton::Left && event.kind != PointerEventKind::Up(button);
                    self.pointer_drag = None;
                    let drop = if button == PointerButton::Left
                        || event.kind == PointerEventKind::Up(button)
                    {
                        self.pane_drop_at(view, pane, expand, event.column, event.row)
                    } else {
                        None
                    };
                    self.drop_pane(pane, drop, view.layout_area());
                }
                _ => {}
            }
            return true;
        }
        let Some(action) = self.keymap.pane_gestures().action(event) else {
            return false;
        };
        if self
            .keymap
            .pointer()
            .press(crate::keymap::pointer_actions::Action::Resize, event)
            && pointer_resize_pair(view, event.column, event.row).is_some()
        {
            return false;
        }
        let Some(pane) = pointer_pane(view, event.column, event.row) else {
            return false;
        };
        self.pane_pointer_owned = true;
        let PointerEventKind::Down(button) = event.kind else {
            unreachable!("pane action requires button down")
        };
        self.pane_pointer_button = Some(button);
        self.grammar.reset();
        self.jump = None;
        if let Some(maximized) = self.maximized {
            self.action_failed(format!(
                "{} keeps the current pane maximized",
                maximized.view.label()
            ));
            return true;
        }
        if matches!(self.layout, Layout::Pane(_)) {
            self.status("only one pane");
            return true;
        }
        self.activate_pane_from_pointer(pane);
        self.dismiss_popups();
        let expand = action == crate::keymap::pointer::PaneAction::Extend;
        self.pointer_drag = Some(PointerDrag::MovePane {
            button,
            pane,
            expand,
            drop: None,
        });
        self.describe_pane_drop(None);
        true
    }

    /// The area the dragged pane would occupy if it were released now, found
    /// by making the move on a copy of the layout.
    pub(crate) fn pane_drop_preview(&self, view: &PreparedView) -> Option<Rect> {
        let Some(PointerDrag::MovePane {
            pane,
            drop: Some(drop),
            ..
        }) = self.pointer_drag
        else {
            return None;
        };
        if let PaneDrop::Swap { target } = drop {
            return view.pane(target).map(|prepared| prepared.area);
        }
        let mut layout = self.layout.clone();
        if !apply_drop(&mut layout, pane, drop, view.layout_area()) {
            return None;
        }
        let mut areas = HashMap::new();
        layout.rectangles(view.layout_area(), &mut areas);
        areas.get(&pane).copied()
    }

    /// Where `pane` would land at this cell, or nothing when releasing there
    /// would leave the layout as it is.
    fn pane_drop_at(
        &self,
        view: &PreparedView,
        pane: usize,
        expand: bool,
        column: u16,
        row: u16,
    ) -> Option<PaneDrop> {
        let drop = if expand {
            expansion_at(view, pane, column, row)?
        } else {
            drop_at(view, pane, column, row)?
        };
        if let PaneDrop::Swap { target } = drop {
            return self.panes.contains_key(&target).then_some(drop);
        }
        apply_drop(&mut self.layout.clone(), pane, drop, view.layout_area()).then_some(drop)
    }

    /// Says on the status line what releasing the pane here would do.
    fn describe_pane_drop(&mut self, drop: Option<PaneDrop>) {
        let message = match drop {
            None if matches!(
                self.pointer_drag,
                Some(PointerDrag::MovePane { expand: true, .. })
            ) =>
            {
                "extending pane: drag into a neighbour; release to grow to its far edge".to_owned()
            }
            None => "moving pane: release on a layout side, a pane's side, or a pane's middle"
                .to_owned(),
            Some(PaneDrop::Expand { .. }) => {
                "release to extend pane; covered panes close".to_owned()
            }
            Some(PaneDrop::Edge { axis, edge }) => {
                format!("release to move pane to the {} edge", edge_name(axis, edge))
            }
            Some(PaneDrop::Beside { axis, edge, .. }) => format!(
                "release to move pane into the {} half of this pane",
                edge_name(axis, edge)
            ),
            Some(PaneDrop::Swap { .. }) => "release to swap pane contents".to_owned(),
        };
        self.status(message);
    }

    fn drop_pane(&mut self, pane: usize, drop: Option<PaneDrop>, layout_area: Rect) {
        let Some(drop) = drop else {
            self.status("pane left where it was");
            return;
        };
        if !self.panes.contains_key(&pane) {
            return;
        }
        if self.active_pane != pane {
            self.activate_pane(pane);
        }
        match drop {
            PaneDrop::Edge { axis, edge } => self.move_pane_to_edge(axis, edge),
            PaneDrop::Beside { target, axis, edge } => self.move_pane_beside(target, axis, edge),
            PaneDrop::Swap { target } => self.swap_pane_contents(target),
            PaneDrop::Expand { area } => {
                let mut layout = self.layout.clone();
                if !layout.expand_pane(pane, layout_area, area) {
                    return;
                }
                let mut survivors = Vec::new();
                layout.panes(&mut survivors);
                let mut before = Vec::new();
                self.layout.panes(&mut before);
                for closing in before.into_iter().filter(|id| !survivors.contains(id)) {
                    if !self.close_temporary_comparison(closing) {
                        self.remove_pane(closing);
                    }
                }
                self.layout = layout;
                if self.active_pane != pane {
                    self.activate_pane(pane);
                }
                self.status("extended pane");
            }
        }
    }
}

impl PreparedView {
    /// The area the split tree is laid out in: the editor area less the
    /// directory tree sidebar.
    pub(crate) fn layout_area(&self) -> Rect {
        let tree = self.tree_area.map_or(0, |tree| tree.width);
        Rect {
            x: self.geometry.editor.x.saturating_add(tree),
            width: self.geometry.editor.width.saturating_sub(tree),
            ..self.geometry.editor
        }
    }
}

/// How a status message names one side of the layout or of a pane.
pub(super) fn edge_name(axis: Axis, edge: Edge) -> &'static str {
    match (axis, edge) {
        (Axis::Horizontal, Edge::Start) => "left",
        (Axis::Horizontal, Edge::End) => "right",
        (Axis::Vertical, Edge::Start) => "top",
        (Axis::Vertical, Edge::End) => "bottom",
    }
}

fn apply_drop(layout: &mut Layout, pane: usize, drop: PaneDrop, area: Rect) -> bool {
    match drop {
        PaneDrop::Edge { axis, edge } => layout.move_to_edge(pane, axis, edge),
        PaneDrop::Beside { target, axis, edge } => layout.move_beside(pane, target, axis, edge),
        PaneDrop::Swap { target } => target != pane,
        PaneDrop::Expand { area: target } => layout.expand_pane(pane, area, target),
    }
}

/// Reads a cell as a drop target, before asking whether it changes anything.
///
/// The bands along the outside of the layout come first, so a pane against
/// a side can still be carried along that whole side. Inside another pane,
/// the quarter nearest each of its sides splits it on that side, and the
/// middle swaps; a corner belongs to whichever side it is relatively nearer.
fn drop_at(view: &PreparedView, dragged: usize, column: u16, row: u16) -> Option<PaneDrop> {
    let area = view.layout_area();
    if !rect_contains(area, column, row) {
        return None;
    }
    let right = area.x.saturating_add(area.width);
    let bottom = area.y.saturating_add(area.height);
    let edge = if column < area.x.saturating_add(SIDE_BAND_COLUMNS) {
        Some((Axis::Horizontal, Edge::Start))
    } else if column >= right.saturating_sub(SIDE_BAND_COLUMNS) {
        Some((Axis::Horizontal, Edge::End))
    } else if row < area.y.saturating_add(SIDE_BAND_ROWS) {
        Some((Axis::Vertical, Edge::Start))
    } else if row >= bottom.saturating_sub(SIDE_BAND_ROWS) {
        Some((Axis::Vertical, Edge::End))
    } else {
        None
    };
    if let Some((axis, edge)) = edge {
        return Some(PaneDrop::Edge { axis, edge });
    }

    let target = pointer_pane(view, column, row).filter(|target| *target != dragged)?;
    let pane = view.pane(target)?.area;
    // Distances to each side as a share of the pane's own extent, measured
    // from cell centres so a pane's first and last cells weigh the same.
    let across = |offset: u16, extent: u16| (f32::from(offset) + 0.5) / f32::from(extent.max(1));
    let x = across(column - pane.x, pane.width);
    let y = across(row - pane.y, pane.height);
    let (distance, axis, edge) = [
        (x, Axis::Horizontal, Edge::Start),
        (1.0 - x, Axis::Horizontal, Edge::End),
        (y, Axis::Vertical, Edge::Start),
        (1.0 - y, Axis::Vertical, Edge::End),
    ]
    .into_iter()
    .min_by(|left, right| left.0.total_cmp(&right.0))?;
    Some(if distance < 0.25 {
        PaneDrop::Beside { target, axis, edge }
    } else {
        PaneDrop::Swap { target }
    })
}

/// Stretch one side of the original pane to the far edge of the pane under
/// the pointer. Diagonal motion uses the larger displacement relative to the
/// original pane size, so terminal cell aspect ratios do not favour a direction.
fn expansion_at(view: &PreparedView, dragged: usize, column: u16, row: u16) -> Option<PaneDrop> {
    if !rect_contains(view.layout_area(), column, row) {
        return None;
    }
    let source = view.pane(dragged)?.area;
    if rect_contains(source, column, row) {
        return None;
    }
    let target = view
        .panes
        .iter()
        .find(|p| rect_contains(p.area, column, row))?
        .area;
    let right = source.x.saturating_add(source.width);
    let bottom = source.y.saturating_add(source.height);
    let directions = [
        (
            u32::from(source.x.saturating_sub(column)) * u32::from(source.height),
            Axis::Horizontal,
            Edge::Start,
        ),
        (
            u32::from(column.saturating_add(1).saturating_sub(right)) * u32::from(source.height),
            Axis::Horizontal,
            Edge::End,
        ),
        (
            u32::from(source.y.saturating_sub(row)) * u32::from(source.width),
            Axis::Vertical,
            Edge::Start,
        ),
        (
            u32::from(row.saturating_add(1).saturating_sub(bottom)) * u32::from(source.width),
            Axis::Vertical,
            Edge::End,
        ),
    ];
    let &(_, axis, edge) = directions.iter().max_by_key(|(distance, _, _)| distance)?;
    let area = match (axis, edge) {
        (Axis::Horizontal, Edge::Start) => Rect {
            x: target.x,
            width: right - target.x,
            ..source
        },
        (Axis::Horizontal, Edge::End) => Rect {
            width: target.x + target.width - source.x,
            ..source
        },
        (Axis::Vertical, Edge::Start) => Rect {
            y: target.y,
            height: bottom - target.y,
            ..source
        },
        (Axis::Vertical, Edge::End) => Rect {
            height: target.y + target.height - source.y,
            ..source
        },
    };
    Some(PaneDrop::Expand { area })
}
