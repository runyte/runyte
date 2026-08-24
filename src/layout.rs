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
                    *ratio = moved_ratio(
                        area,
                        *axis,
                        *ratio,
                        i32::from(delta),
                        first.minimum_extent(*axis),
                        second.minimum_extent(*axis),
                    );
                    true
                } else if first_has_second && second_has_first {
                    *ratio = moved_ratio(
                        area,
                        *axis,
                        *ratio,
                        -i32::from(delta),
                        first.minimum_extent(*axis),
                        second.minimum_extent(*axis),
                    );
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

    fn contains(&self, pane: usize) -> bool {
        match self {
            Self::Pane(candidate) => *candidate == pane,
            Self::Split { first, second, .. } => first.contains(pane) || second.contains(pane),
        }
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

fn split_extent(total: u16, ratio: u16) -> u16 {
    if total < 2 {
        return total;
    }
    ((u32::from(total) * u32::from(ratio) / u32::from(RATIO_SCALE)) as u16).clamp(1, total - 1)
}

fn moved_ratio(
    area: Rect,
    axis: Axis,
    ratio: u16,
    delta: i32,
    first_minimum: u16,
    second_minimum: u16,
) -> u16 {
    let total = match axis {
        Axis::Horizontal => area.width,
        Axis::Vertical => area.height,
    };
    if total < 2 {
        return ratio;
    }
    let (minimum, maximum) = if total >= first_minimum.saturating_add(second_minimum) {
        (first_minimum, total - second_minimum)
    } else {
        (1, total - 1)
    };
    let current = split_extent(total, ratio);
    let desired = (i32::from(current) + delta).clamp(i32::from(minimum), i32::from(maximum)) as u16;
    // `split_extent` floors. The ceiling here chooses the smallest ratio that
    // maps back to the requested cell boundary.
    ((u32::from(desired) * u32::from(RATIO_SCALE)).div_ceil(u32::from(total)))
        .clamp(1, u32::from(RATIO_SCALE - 1)) as u16
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
}
