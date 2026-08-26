// SPDX-License-Identifier: MPL-2.0

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Debug)]
pub enum Layout {
    Pane(usize),
    Split {
        axis: Axis,
        /// Share of the available extent assigned to `first`, scaled across
        /// the full `u16` range for cell-accurate resizing on wide terminals.
        ratio: u16,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

const RATIO_SCALE: u16 = u16::MAX;
const DEFAULT_RATIO: u16 = RATIO_SCALE / 2 + 1;
const MIN_DRAWABLE_EXTENT: u16 = 3;

/// Which side of a subtree's extent moved while the rest of it stayed still.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Edge {
    Start,
    End,
}

impl Layout {
    pub fn split(&mut self, pane: usize, new_pane: usize, axis: Axis) -> bool {
        match self {
            Self::Pane(id) if *id == pane => {
                *self = Self::Split {
                    axis,
                    ratio: DEFAULT_RATIO,
                    first: Box::new(Self::Pane(pane)),
                    second: Box::new(Self::Pane(new_pane)),
                };
                true
            }
            Self::Pane(_) => false,
            Self::Split { first, second, .. } => {
                first.split(pane, new_pane, axis) || second.split(pane, new_pane, axis)
            }
        }
    }

    pub fn without(self, pane: usize) -> Option<Self> {
        match self {
            Self::Pane(id) => (id != pane).then_some(Self::Pane(id)),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => match (first.without(pane), second.without(pane)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    axis,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(node), None) | (None, Some(node)) => Some(node),
                (None, None) => None,
            },
        }
    }

    pub fn panes(&self, output: &mut Vec<usize>) {
        match self {
            Self::Pane(id) => output.push(*id),
            Self::Split { first, second, .. } => {
                first.panes(output);
                second.panes(output);
            }
        }
    }

    pub fn rectangles(&self, area: Rect, output: &mut HashMap<usize, Rect>) {
        match self {
            Self::Pane(id) => {
                output.insert(*id, area);
            }
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (first_area, second_area) = split_areas(area, *axis, *ratio);
                first.rectangles(first_area, output);
                second.rectangles(second_area, output);
            }
        }
    }

    /// Moves the nearest split separating two panes by an exact number of
    /// screen cells within `area`.
    ///
    /// Positive deltas grow the subtree containing `first_pane`; negative
    /// deltas grow the subtree containing `second_pane`. When the split is at
    /// least six cells wide/high, both children retain the three cells needed
    /// by viewport preparation.
    pub fn resize_between_cells(
        &mut self,
        first_pane: usize,
        second_pane: usize,
        area: Rect,
        delta: i16,
    ) -> bool {
        match self {
            Self::Pane(_) => false,
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let first_has_first = first.contains(first_pane);
                let first_has_second = first.contains(second_pane);
                let second_has_first = second.contains(first_pane);
                let second_has_second = second.contains(second_pane);
                if first_has_first && second_has_second {
                    move_boundary(area, *axis, ratio, first, second, i32::from(delta));
                    true
                } else if first_has_second && second_has_first {
                    move_boundary(area, *axis, ratio, first, second, -i32::from(delta));
                    true
                } else if first_has_first && first_has_second {
                    let (first_area, _) = split_areas(area, *axis, *ratio);
                    first.resize_between_cells(first_pane, second_pane, first_area, delta)
                } else if second_has_first && second_has_second {
                    let (_, second_area) = split_areas(area, *axis, *ratio);
                    second.resize_between_cells(first_pane, second_pane, second_area, delta)
                } else {
                    false
                }
            }
        }
    }

    /// Gives every pane the same width, and then every pane sharing a column
    /// the same height, without moving a pane in the tree.
    ///
    /// Each boundary is placed by how many pane-wide slots lie on either side
    /// of it along its own axis. A split along that axis needs as many slots
    /// as its two children need together; a split across it needs as many as
    /// its hungrier child, because both children occupy the same slots rather
    /// than separate ones. One pane beside a stack of two therefore puts the
    /// boundary a third of the way along, which is what leaves the three of
    /// them equal.
    ///
    /// A ratio is a share of its parent's extent, so this needs no area: the
    /// same tree equalizes to the same shape at every terminal size.
    pub fn equalize(&mut self) {
        let Self::Split {
            axis,
            ratio,
            first,
            second,
        } = self
        else {
            return;
        };
        *ratio = ratio_for_equal_slots(first.slots(*axis), second.slots(*axis));
        first.equalize();
        second.equalize();
    }

    /// How many pane-wide slots this subtree occupies along `axis`.
    fn slots(&self, axis: Axis) -> u32 {
        match self {
            Self::Pane(_) => 1,
            Self::Split {
                axis: split_axis,
                first,
                second,
                ..
            } if *split_axis == axis => first.slots(axis).saturating_add(second.slots(axis)),
            Self::Split { first, second, .. } => first.slots(axis).max(second.slots(axis)),
        }
    }

    fn contains(&self, pane: usize) -> bool {
        match self {
            Self::Pane(candidate) => *candidate == pane,
            Self::Split { first, second, .. } => first.contains(pane) || second.contains(pane),
        }
    }

    /// Rewrites the descendant ratios so that every boundary inside this
    /// subtree keeps the screen position it had while the subtree's extent
    /// along `axis` changed from `old_extent` to `new_extent`.
    ///
    /// `edge` names the side of the subtree that moved. Everything anchored
    /// to the opposite side stays put, so the whole change is absorbed by the
    /// one child touching `edge`, and only that child recurses into a further
    /// adjustment. Perpendicular splits give both children the full extent
    /// along `axis`, so both of them see the same change.
    fn preserve_boundaries(&mut self, axis: Axis, old_extent: u16, new_extent: u16, edge: Edge) {
        let Self::Split {
            axis: split_axis,
            ratio,
            first,
            second,
        } = self
        else {
            return;
        };
        if *split_axis != axis {
            first.preserve_boundaries(axis, old_extent, new_extent, edge);
            second.preserve_boundaries(axis, old_extent, new_extent, edge);
            return;
        }
        if old_extent < 2 || new_extent < 2 {
            return;
        }
        let old_first = split_extent(old_extent, *ratio);
        let old_second = old_extent - old_first;
        let held = match edge {
            Edge::End => i32::from(old_first),
            Edge::Start => i32::from(new_extent) - i32::from(old_second),
        };
        *ratio = ratio_for_first_extent(
            new_extent,
            held,
            first.minimum_extent(axis),
            second.minimum_extent(axis),
        );
        let new_first = split_extent(new_extent, *ratio);
        first.preserve_boundaries(axis, old_first, new_first, edge);
        second.preserve_boundaries(axis, old_second, new_extent - new_first, edge);
    }

    fn minimum_extent(&self, axis: Axis) -> u16 {
        match self {
            Self::Pane(_) => MIN_DRAWABLE_EXTENT,
            Self::Split {
                axis: split_axis,
                first,
                second,
                ..
            } if *split_axis == axis => first
                .minimum_extent(axis)
                .saturating_add(second.minimum_extent(axis)),
            Self::Split { first, second, .. } => {
                first.minimum_extent(axis).max(second.minimum_extent(axis))
            }
        }
    }
}

fn split_areas(area: Rect, axis: Axis, ratio: u16) -> (Rect, Rect) {
    match axis {
        Axis::Horizontal => {
            let first_width = split_extent(area.width, ratio);
            (
                Rect {
                    width: first_width,
                    ..area
                },
                Rect {
                    x: area.x + first_width,
                    width: area.width - first_width,
                    ..area
                },
            )
        }
        Axis::Vertical => {
            let first_height = split_extent(area.height, ratio);
            (
                Rect {
                    height: first_height,
                    ..area
                },
                Rect {
                    y: area.y + first_height,
                    height: area.height - first_height,
                    ..area
                },
            )
        }
    }
}

fn extent(area: Rect, axis: Axis) -> u16 {
    match axis {
        Axis::Horizontal => area.width,
        Axis::Vertical => area.height,
    }
}

fn split_extent(total: u16, ratio: u16) -> u16 {
    if total < 2 {
        return total;
    }
    ((u32::from(total) * u32::from(ratio) / u32::from(RATIO_SCALE)) as u16).clamp(1, total - 1)
}

/// Moves one split's own boundary by `delta` cells and then holds every
/// other boundary in the two subtrees where it already was.
///
/// A ratio is a share of its parent's extent, so moving this boundary
/// rescales both children's areas and would otherwise drag every nested
/// boundary along with it. Dragging one edge is meant to move that edge
/// alone, so the children re-derive their ratios against their new extents.
fn move_boundary(
    area: Rect,
    axis: Axis,
    ratio: &mut u16,
    first: &mut Layout,
    second: &mut Layout,
    delta: i32,
) {
    let total = extent(area, axis);
    if total < 2 {
        return;
    }
    let old_first = split_extent(total, *ratio);
    let old_second = total - old_first;
    *ratio = ratio_for_first_extent(
        total,
        i32::from(old_first) + delta,
        first.minimum_extent(axis),
        second.minimum_extent(axis),
    );
    let new_first = split_extent(total, *ratio);
    // The first child's own start edge and the second child's own end edge
    // are the outer sides of `area`, which did not move; the boundary between
    // them is the one that did.
    first.preserve_boundaries(axis, old_first, new_first, Edge::End);
    second.preserve_boundaries(axis, old_second, total - new_first, Edge::Start);
}

/// The ratio placing the boundary at `desired` cells from the start of a
/// `total`-cell extent, keeping each side at its subtree's minimum.
fn ratio_for_first_extent(
    total: u16,
    desired: i32,
    first_minimum: u16,
    second_minimum: u16,
) -> u16 {
    let (minimum, maximum) = if total >= first_minimum.saturating_add(second_minimum) {
        (first_minimum, total - second_minimum)
    } else {
        (1, total - 1)
    };
    let desired = desired.clamp(i32::from(minimum), i32::from(maximum)) as u16;
    // `split_extent` floors. The ceiling here chooses the smallest ratio that
    // maps back to the requested cell boundary.
    ((u32::from(desired) * u32::from(RATIO_SCALE)).div_ceil(u32::from(total)))
        .clamp(1, u32::from(RATIO_SCALE - 1)) as u16
}

/// The ratio that hands `first` its share of `first + second` equal slots.
///
/// Every subtree occupies at least one slot, so the total is never zero.
fn ratio_for_equal_slots(first: u32, second: u32) -> u16 {
    let total = u64::from(first) + u64::from(second);
    // `split_extent` floors, so the ceiling here is what lands the boundary
    // on the intended cell rather than one short of it.
    (u64::from(first) * u64::from(RATIO_SCALE))
        .div_ceil(total)
        .clamp(1, u64::from(RATIO_SCALE - 1)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_layout_allocates_all_panes() {
        let mut layout = Layout::Pane(0);
        assert!(layout.split(0, 1, Axis::Horizontal));
        assert!(layout.split(1, 2, Axis::Vertical));
        let mut areas = HashMap::new();
        layout.rectangles(
            Rect {
                width: 100,
                height: 40,
                ..Rect::default()
            },
            &mut areas,
        );
        assert_eq!(areas[&0].width, 50);
        assert_eq!(areas[&1].height, 20);
        assert_eq!(areas[&2].y, 20);
    }

    #[test]
    fn removing_a_pane_collapses_its_split() {
        let layout = Layout::Split {
            axis: Axis::Horizontal,
            ratio: DEFAULT_RATIO,
            first: Box::new(Layout::Pane(0)),
            second: Box::new(Layout::Pane(1)),
        };
        assert!(matches!(layout.without(1), Some(Layout::Pane(0))));
    }

    #[test]
    fn resizing_changes_only_the_split_that_separates_the_pair() {
        let mut layout = Layout::Pane(0);
        assert!(layout.split(0, 1, Axis::Horizontal));
        assert!(layout.split(1, 2, Axis::Vertical));
        assert!(layout.resize_between_cells(
            1,
            2,
            Rect {
                width: 100,
                height: 40,
                ..Rect::default()
            },
            1,
        ));

        let mut areas = HashMap::new();
        layout.rectangles(
            Rect {
                width: 100,
                height: 40,
                ..Rect::default()
            },
            &mut areas,
        );
        assert_eq!(areas[&0].width, 50);
        assert_eq!(areas[&1].height, 21);
        assert_eq!(areas[&2].height, 19);
    }

    #[test]
    fn resize_ratio_is_bounded_so_both_sides_remain_visible() {
        let mut layout = Layout::Pane(0);
        assert!(layout.split(0, 1, Axis::Horizontal));
        assert!(layout.resize_between_cells(
            0,
            1,
            Rect {
                width: 10,
                height: 4,
                ..Rect::default()
            },
            i16::MAX,
        ));
        let mut areas = HashMap::new();
        layout.rectangles(
            Rect {
                width: 10,
                height: 4,
                ..Rect::default()
            },
            &mut areas,
        );
        assert_eq!(areas[&0].width, 7);
        assert_eq!(areas[&1].width, 3);
    }

    #[test]
    fn resizing_tracks_cells_at_narrow_and_wide_extents() {
        for width in [20, 80, 2_000] {
            let mut layout = Layout::Pane(0);
            assert!(layout.split(0, 1, Axis::Horizontal));
            let area = Rect {
                width,
                height: 10,
                ..Rect::default()
            };
            assert!(layout.resize_between_cells(0, 1, area, 1));
            let mut areas = HashMap::new();
            layout.rectangles(area, &mut areas);
            assert_eq!(areas[&0].width, width / 2 + 1);
            assert_eq!(areas[&1].width, width / 2 - 1);
        }
    }

    #[test]
    fn resizing_a_stacked_boundary_leaves_the_other_boundaries_where_they_were() {
        // Three panes stacked vertically. Dragging the boundary between the
        // top and middle panes must resize those two only; the middle-to-
        // bottom boundary keeps its row.
        let mut layout = Layout::Pane(0);
        assert!(layout.split(0, 1, Axis::Vertical));
        assert!(layout.split(1, 2, Axis::Vertical));
        let area = Rect {
            width: 80,
            height: 60,
            ..Rect::default()
        };
        let mut before = HashMap::new();
        layout.rectangles(area, &mut before);

        assert!(layout.resize_between_cells(0, 1, area, 5));
        let mut after = HashMap::new();
        layout.rectangles(area, &mut after);

        assert_eq!(after[&0].height, before[&0].height + 5);
        assert_eq!(after[&1].height, before[&1].height - 5);
        assert_eq!(after[&2].y, before[&2].y);
        assert_eq!(after[&2].height, before[&2].height);
    }

    #[test]
    fn resizing_a_side_by_side_boundary_leaves_the_other_boundaries_where_they_were() {
        let mut layout = Layout::Pane(0);
        assert!(layout.split(0, 1, Axis::Horizontal));
        assert!(layout.split(1, 2, Axis::Horizontal));
        let area = Rect {
            width: 120,
            height: 40,
            ..Rect::default()
        };
        let mut before = HashMap::new();
        layout.rectangles(area, &mut before);

        assert!(layout.resize_between_cells(1, 2, area, -4));
        let mut after = HashMap::new();
        layout.rectangles(area, &mut after);

        assert_eq!(after[&0].width, before[&0].width);
        assert_eq!(after[&1].width, before[&1].width - 4);
        assert_eq!(after[&2].width, before[&2].width + 4);
        assert_eq!(after[&2].x, before[&2].x - 4);
    }

    #[test]
    fn resizing_holds_boundaries_nested_across_the_perpendicular_axis() {
        // The second column is split into two rows, then split again into
        // three. Widening the first column must not disturb any row.
        let mut layout = Layout::Pane(0);
        assert!(layout.split(0, 1, Axis::Horizontal));
        assert!(layout.split(1, 2, Axis::Vertical));
        assert!(layout.split(2, 3, Axis::Vertical));
        let area = Rect {
            width: 100,
            height: 60,
            ..Rect::default()
        };
        let mut before = HashMap::new();
        layout.rectangles(area, &mut before);

        assert!(layout.resize_between_cells(0, 1, area, 7));
        let mut after = HashMap::new();
        layout.rectangles(area, &mut after);

        assert_eq!(after[&0].width, before[&0].width + 7);
        for pane in [1, 2, 3] {
            assert_eq!(after[&pane].y, before[&pane].y);
            assert_eq!(after[&pane].height, before[&pane].height);
            assert_eq!(after[&pane].width, before[&pane].width - 7);
        }
    }

    #[test]
    fn repeated_single_cell_resizes_do_not_drift_the_far_boundary() {
        let mut layout = Layout::Pane(0);
        assert!(layout.split(0, 1, Axis::Vertical));
        assert!(layout.split(1, 2, Axis::Vertical));
        let area = Rect {
            width: 80,
            height: 47,
            ..Rect::default()
        };
        let mut before = HashMap::new();
        layout.rectangles(area, &mut before);

        for _ in 0..9 {
            assert!(layout.resize_between_cells(0, 1, area, 1));
        }
        let mut after = HashMap::new();
        layout.rectangles(area, &mut after);

        assert_eq!(after[&0].height, before[&0].height + 9);
        assert_eq!(after[&2].y, before[&2].y);
        assert_eq!(after[&2].height, before[&2].height);
    }

    #[test]
    fn resize_preserves_nested_descendant_minimums_and_handles_minimum_delta() {
        let mut layout = Layout::Pane(0);
        assert!(layout.split(0, 1, Axis::Horizontal));
        assert!(layout.split(0, 2, Axis::Horizontal));
        let area = Rect {
            width: 30,
            height: 10,
            ..Rect::default()
        };
        assert!(layout.resize_between_cells(0, 1, area, i16::MIN));
        let mut areas = HashMap::new();
        layout.rectangles(area, &mut areas);
        assert_eq!(areas[&0].width, 3);
        assert_eq!(areas[&2].width, 3);
        assert_eq!(areas[&1].width, 24);

        assert!(layout.resize_between_cells(1, 0, area, i16::MIN));
        areas.clear();
        layout.rectangles(area, &mut areas);
        assert!(areas[&0].width >= 3);
        assert!(areas[&2].width >= 3);
        assert_eq!(areas[&1].width, 3);
    }

    #[test]
    fn equalizing_levels_a_row_of_columns() {
        let mut layout = Layout::Pane(0);
        assert!(layout.split(0, 1, Axis::Horizontal));
        assert!(layout.split(1, 2, Axis::Horizontal));
        let area = Rect {
            width: 90,
            height: 30,
            ..Rect::default()
        };
        assert!(layout.resize_between_cells(0, 1, area, 20));

        layout.equalize();
        let mut areas = HashMap::new();
        layout.rectangles(area, &mut areas);
        for pane in [0, 1, 2] {
            assert_eq!(areas[&pane].width, 30, "pane {pane}");
            assert_eq!(areas[&pane].height, 30, "pane {pane}");
        }
    }

    #[test]
    fn equalizing_levels_each_column_against_its_own_rows() {
        // One column on the left, two stacked panes in the middle column, and
        // three stacked in the right one.
        let mut layout = Layout::Pane(0);
        assert!(layout.split(0, 1, Axis::Horizontal));
        assert!(layout.split(1, 2, Axis::Horizontal));
        assert!(layout.split(1, 3, Axis::Vertical));
        assert!(layout.split(2, 4, Axis::Vertical));
        assert!(layout.split(4, 5, Axis::Vertical));

        layout.equalize();
        let area = Rect {
            width: 90,
            height: 60,
            ..Rect::default()
        };
        let mut areas = HashMap::new();
        layout.rectangles(area, &mut areas);
        for pane in 0..=5 {
            assert_eq!(areas[&pane].width, 30, "pane {pane}");
        }
        assert_eq!(areas[&0].height, 60);
        for pane in [1, 3] {
            assert_eq!(areas[&pane].height, 30, "pane {pane}");
        }
        for pane in [2, 4, 5] {
            assert_eq!(areas[&pane].height, 20, "pane {pane}");
        }
    }

    #[test]
    fn equalizing_keeps_a_pane_that_spans_the_full_width_spanning_it() {
        let mut layout = Layout::Pane(0);
        assert!(layout.split(0, 1, Axis::Vertical));
        assert!(layout.split(1, 2, Axis::Horizontal));
        let area = Rect {
            width: 80,
            height: 40,
            ..Rect::default()
        };
        assert!(layout.resize_between_cells(1, 2, area, 15));

        layout.equalize();
        let mut areas = HashMap::new();
        layout.rectangles(area, &mut areas);
        assert_eq!(
            areas[&0].width, 80,
            "the arrangement into columns is unchanged"
        );
        assert_eq!(areas[&0].height, 20);
        assert_eq!(areas[&1].width, 40);
        assert_eq!(areas[&2].width, 40);
        assert_eq!(areas[&1].height, 20);
        assert_eq!(areas[&2].height, 20);
    }

    #[test]
    fn equalizing_an_already_level_layout_changes_nothing() {
        let mut layout = Layout::Pane(0);
        assert!(layout.split(0, 1, Axis::Horizontal));
        assert!(layout.split(1, 2, Axis::Vertical));
        let area = Rect {
            width: 100,
            height: 40,
            ..Rect::default()
        };
        let mut before = HashMap::new();
        layout.rectangles(area, &mut before);

        layout.equalize();
        let mut after = HashMap::new();
        layout.rectangles(area, &mut after);
        assert_eq!(before, after);
    }

    #[test]
    fn equalizing_a_single_pane_is_a_no_op() {
        let mut layout = Layout::Pane(7);
        layout.equalize();
        assert!(matches!(layout, Layout::Pane(7)));
    }
}
