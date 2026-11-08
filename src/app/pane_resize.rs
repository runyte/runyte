// SPDX-License-Identifier: MPL-2.0

//! Keyboard layout drafts never mutate live pane views or PTY geometry.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const FADE_DURATION: Duration = Duration::from_millis(160);

use super::{App, InputEvent, InputGrammar, Layout, Mode, Rect};
use crate::keymap::ui::{Action, Owner};
use crate::snapshot::{PaneSnapshot, ResizePaneSnapshot, ResizeSnapshot};

/// Clockwise, starting at the preferred right edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResizeEdge {
    Right,
    Bottom,
    Left,
    Top,
}

impl ResizeEdge {
    const ALL: [Self; 4] = [Self::Right, Self::Bottom, Self::Left, Self::Top];
    fn vertical(self) -> bool {
        matches!(self, Self::Right | Self::Left)
    }
    fn start(self) -> bool {
        matches!(self, Self::Left | Self::Top)
    }
    fn name(self) -> &'static str {
        match self {
            Self::Right => "Right",
            Self::Bottom => "Bottom",
            Self::Left => "Left",
            Self::Top => "Top",
        }
    }
    fn position(self, r: Rect) -> u16 {
        match self {
            Self::Right => r.x + r.width,
            Self::Bottom => r.y + r.height,
            Self::Left => r.x,
            Self::Top => r.y,
        }
    }
    fn touches(self, source: Rect, other: Rect) -> bool {
        let opposite = match self {
            Self::Right => Self::Left,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
            Self::Top => Self::Bottom,
        };
        let overlap = if self.vertical() {
            source.y.max(other.y) < (source.y + source.height).min(other.y + other.height)
        } else {
            source.x.max(other.x) < (source.x + source.width).min(other.x + other.width)
        };
        overlap && self.position(source) == opposite.position(other)
    }
}

#[derive(Clone)]
struct Draft {
    layout: Layout,
    tree_width: u16,
    edge: Option<ResizeEdge>,
}

pub(super) struct ResizePreview {
    original: Layout,
    editor: Rect,
    tree: Option<Rect>,
    tree_focused: bool,
    pane: usize,
    draft: Draft,
    history: Vec<Draft>,
    feedback: String,
    fade_started: Option<Instant>,
}

impl ResizePreview {
    fn area(&self) -> Rect {
        Rect {
            x: self.editor.x + self.draft.tree_width,
            width: self.editor.width - self.draft.tree_width,
            ..self.editor
        }
    }
    fn rectangles(&self) -> HashMap<usize, Rect> {
        let mut areas = HashMap::new();
        self.draft.layout.rectangles(self.area(), &mut areas);
        areas
    }
    fn source(&self) -> Rect {
        if self.tree_focused {
            Rect {
                width: self.draft.tree_width,
                ..self.editor
            }
        } else {
            self.rectangles()[&self.pane]
        }
    }
    fn tree_edge(&self, edge: ResizeEdge) -> bool {
        self.tree.is_some()
            && if self.tree_focused {
                edge == ResizeEdge::Right
            } else {
                edge == ResizeEdge::Left && self.source().x == self.area().x
            }
    }
    fn edges(&self) -> Vec<ResizeEdge> {
        ResizeEdge::ALL
            .into_iter()
            .filter(|&edge| {
                self.tree_edge(edge)
                    || (!self.tree_focused
                        && edge.position(self.source()) != edge.position(self.area()))
            })
            .collect()
    }
    fn settle_edge(&mut self) {
        let edges = self.edges();
        if !self.draft.edge.is_some_and(|edge| edges.contains(&edge)) {
            self.draft.edge = edges.first().copied();
        }
    }
    fn remember(&mut self, before: Draft) {
        // Holding a key must not grow unbounded history.
        if self.history.len() == 256 {
            self.history.remove(0);
        }
        self.history.push(before);
    }
    fn move_edge(&mut self, direction: ResizeEdge) {
        let Some(edge) = self.draft.edge else {
            self.feedback = "No movable edge · undo an extension to restore panes".into();
            return;
        };
        if edge.vertical() != direction.vertical() {
            self.feedback = if edge.vertical() {
                "Use left/right for this edge"
            } else {
                "Use up/down for this edge"
            }
            .into();
            return;
        }
        let before = self.draft.clone();
        let step = if direction.start() { -1 } else { 1 };
        let old_areas = self.rectangles();
        if self.tree_edge(edge) {
            self.draft.tree_width = (i32::from(self.draft.tree_width) + step)
                .clamp(12, i32::from(self.editor.width.saturating_sub(24).max(12)))
                as u16;
        } else {
            let source = self.source();
            let neighbour = old_areas
                .iter()
                .filter(|(id, r)| **id != self.pane && edge.touches(source, **r))
                .map(|(&id, _)| id)
                .min();
            if let Some(neighbour) = neighbour {
                let delta = step * if edge.start() { -1 } else { 1 };
                self.draft.layout.resize_between_cells(
                    self.pane,
                    neighbour,
                    self.area(),
                    delta as i16,
                );
            }
        }
        if old_areas != self.rectangles() || before.tree_width != self.draft.tree_width {
            self.remember(before);
        } else {
            self.draft = before;
            self.feedback = "Minimum pane size reached".into();
        }
    }
    fn extend(&mut self, edge: ResizeEdge) {
        if self.tree_focused || self.tree_edge(edge) {
            self.feedback = "The directory tree can be resized, but not absorbed".into();
            return;
        }
        let source = self.source();
        let mut candidates: Vec<_> = self
            .rectangles()
            .into_iter()
            .filter(|(id, r)| *id != self.pane && edge.touches(source, *r))
            .map(|(_, r)| edge.position(r))
            .collect();
        candidates.sort_unstable_by_key(|&position| position.abs_diff(edge.position(source)));
        candidates.dedup();
        for position in candidates {
            let target = match edge {
                ResizeEdge::Left => Rect {
                    x: position,
                    width: source.x + source.width - position,
                    ..source
                },
                ResizeEdge::Right => Rect {
                    width: position - source.x,
                    ..source
                },
                ResizeEdge::Top => Rect {
                    y: position,
                    height: source.y + source.height - position,
                    ..source
                },
                ResizeEdge::Bottom => Rect {
                    height: position - source.y,
                    ..source
                },
            };
            let mut layout = self.draft.layout.clone();
            if layout.expand_pane(self.pane, self.area(), target) {
                let before = self.draft.clone();
                self.draft.layout = layout;
                self.draft.edge = Some(edge);
                self.settle_edge();
                self.remember(before);
                return;
            }
        }
        self.feedback = "No valid extension in that direction".into();
    }
}

impl App {
    /// Existing presentation ticks run only while a finite transition or job needs them.
    pub fn has_presentation_animation(&self) -> bool {
        self.has_long_running_action()
            || self
                .pane_resize
                .as_ref()
                .is_some_and(|p| p.fade_started.is_some())
    }

    pub(super) fn advance_resize_fade(&mut self, now: Instant) {
        if let Some(preview) = &mut self.pane_resize
            && preview
                .fade_started
                .is_some_and(|started| now.saturating_duration_since(started) >= FADE_DURATION)
        {
            preview.fade_started = None;
        }
    }

    pub(super) fn begin_resize(&mut self) {
        if self.maximized.is_some() {
            self.status("Leave fullscreen or Zen before resizing panes");
            return;
        }
        let Some(area) = super::enclosing_area(self.areas.values().copied()) else {
            self.status("Pane layout is not ready");
            return;
        };
        let tree = self.directory_tree_geometry.map(|(tree, _)| tree);
        let editor = self
            .directory_tree_geometry
            .map_or(area, |(_, editor)| editor);
        let mut preview = ResizePreview {
            original: self.layout.clone(),
            editor,
            tree,
            tree_focused: self.directory_tree.focused,
            pane: self.active_pane,
            draft: Draft {
                layout: self.layout.clone(),
                tree_width: tree.map_or(0, |r| r.width),
                edge: None,
            },
            history: Vec::new(),
            feedback: String::new(),
            fade_started: Some(Instant::now()),
        };
        preview.settle_edge();
        if preview.draft.edge.is_none() {
            self.status("Only one pane; no boundary to resize");
            return;
        }
        self.dismiss_popups();
        self.grammar.reset();
        self.jump = None;
        self.mode = Mode::Normal;
        self.pane_resize = Some(preview);
    }

    pub(super) fn validate_resize_preview(&mut self, editor: Option<Rect>) -> bool {
        let valid = self.pane_resize.as_ref().is_some_and(|p| {
            p.original == self.layout
                && p.pane == self.active_pane
                && self.maximized.is_none()
                && p.tree_focused == self.directory_tree.focused
                && p.tree.is_some() == (self.directory_tree.visible && p.editor.width >= 36)
                && p.tree
                    .is_none_or(|tree| tree.width == self.tree_width(p.editor.width))
                && editor.is_none_or(|area| area == p.editor)
        });
        if self.pane_resize.is_some() && !valid {
            self.pane_resize = None;
            self.status("Resize cancelled: pane layout or screen size changed");
        }
        valid
    }

    pub(super) fn handle_resize_input(&mut self, input: InputEvent) {
        if !self.validate_resize_preview(None) {
            return;
        }
        let InputEvent::Key(key) = input else {
            return;
        };
        let Some(action) = self.keymap.ui().lookup(Owner::PaneResize, key) else {
            return;
        };
        let mut preview = self.pane_resize.take().expect("validated preview");
        preview.feedback.clear();
        preview.fade_started = None;
        match action {
            Action::Cancel => {
                self.status("Resize cancelled");
                return;
            }
            Action::Accept => {
                let mut survivors = Vec::new();
                preview.draft.layout.panes(&mut survivors);
                let mut before = Vec::new();
                self.layout.panes(&mut before);
                for closing in before.into_iter().filter(|id| !survivors.contains(id)) {
                    if !self.close_temporary_comparison(closing) {
                        self.remove_pane(closing);
                    }
                }
                self.layout = preview.draft.layout;
                if self.active_pane != preview.pane {
                    self.activate_pane(preview.pane);
                }
                if preview.tree.is_some() {
                    self.resize_tree(i32::from(preview.draft.tree_width), preview.editor.width);
                }
                self.status("Pane layout applied");
                return;
            }
            Action::Next | Action::Previous => {
                let edges = preview.edges();
                if !edges.is_empty() {
                    let index = edges
                        .iter()
                        .position(|e| Some(*e) == preview.draft.edge)
                        .unwrap_or(0);
                    let step = if action == Action::Next {
                        1
                    } else {
                        edges.len() - 1
                    };
                    preview.draft.edge = Some(edges[(index + step) % edges.len()]);
                }
            }
            Action::Back => {
                if let Some(draft) = preview.history.pop() {
                    preview.draft = draft;
                } else {
                    preview.feedback = "Nothing to undo".into();
                }
            }
            Action::MoveLeft => preview.move_edge(ResizeEdge::Left),
            Action::MoveRight => preview.move_edge(ResizeEdge::Right),
            Action::MoveUp => preview.move_edge(ResizeEdge::Top),
            Action::MoveDown => preview.move_edge(ResizeEdge::Bottom),
            Action::ExtendLeft => preview.extend(ResizeEdge::Left),
            Action::ExtendRight => preview.extend(ResizeEdge::Right),
            Action::ExtendUp => preview.extend(ResizeEdge::Top),
            Action::ExtendDown => preview.extend(ResizeEdge::Bottom),
            _ => {}
        }
        self.pane_resize = Some(preview);
    }

    pub(crate) fn resize_snapshot(&self, panes: &[PaneSnapshot]) -> Option<ResizeSnapshot> {
        let preview = self.pane_resize.as_ref()?;
        let areas = preview.rectangles();
        let source = preview.source();
        let selected_edge = preview.draft.edge.map(|edge| match edge {
            ResizeEdge::Left => Rect { width: 1, ..source },
            ResizeEdge::Right => Rect {
                x: source.x + source.width.saturating_sub(1),
                width: 1,
                ..source
            },
            ResizeEdge::Top => Rect {
                height: 1,
                ..source
            },
            ResizeEdge::Bottom => Rect {
                y: source.y + source.height.saturating_sub(1),
                height: 1,
                ..source
            },
        });
        let mut tiles: Vec<_> = panes
            .iter()
            .filter_map(|pane| {
                Some(ResizePaneSnapshot {
                    area: *areas.get(&pane.pane_id)?,
                    title: pane.title.name.clone(),
                    active: !preview.tree_focused && pane.pane_id == preview.pane,
                })
            })
            .collect();
        if preview.tree.is_some() {
            tiles.push(ResizePaneSnapshot {
                area: Rect {
                    width: preview.draft.tree_width,
                    ..preview.editor
                },
                title: "[dir tree]".into(),
                active: preview.tree_focused,
            });
        }
        let label = |a| self.keymap.ui().label(Owner::PaneResize, a);
        let factory_motion = [
            Action::MoveLeft,
            Action::MoveRight,
            Action::MoveUp,
            Action::MoveDown,
            Action::ExtendLeft,
            Action::ExtendRight,
            Action::ExtendUp,
            Action::ExtendDown,
        ]
        .iter()
        .all(|&a| !self.keymap.ui().explicitly_configured(Owner::PaneResize, a));
        let motions = if factory_motion {
            "hjkl/Arrows resize · Shift+direction extend".into()
        } else {
            format!(
                "{}/{}/{}/{} resize · {}/{}/{}/{} extend",
                label(Action::MoveLeft),
                label(Action::MoveDown),
                label(Action::MoveUp),
                label(Action::MoveRight),
                label(Action::ExtendLeft),
                label(Action::ExtendDown),
                label(Action::ExtendUp),
                label(Action::ExtendRight)
            )
        };
        let removed = panes.len().saturating_sub(areas.len());
        let detail = if preview.feedback.is_empty() {
            format!(
                "{} edge · {} × {} · {} pane views removed",
                preview.draft.edge.map_or("No movable", ResizeEdge::name),
                source.width,
                source.height,
                removed
            )
        } else {
            preview.feedback.clone()
        };
        Some(ResizeSnapshot {
            fade: preview.fade_started.map_or(255, |started| {
                (started.elapsed().as_millis() * 255 / FADE_DURATION.as_millis()).min(255) as u8
            }),
            panes: tiles,
            selected_edge,
            controls: format!(
                "{} edge · {} · {} undo",
                label(Action::Next),
                motions,
                label(Action::Back)
            ),
            decisions: format!(
                "{} apply · {} cancel",
                label(Action::Accept),
                label(Action::Cancel)
            ),
            detail,
        })
    }
}
