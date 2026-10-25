// SPDX-License-Identifier: MPL-2.0

use super::*;

impl Layout {
    /// An aligned pair can own only part of an ancestor's boundary. Retile
    /// that pair before resizing the ancestor, which would move other panes.
    pub(super) fn resize_aligned_segment(
        &mut self,
        first_pane: usize,
        second_pane: usize,
        area: Rect,
        delta: i16,
    ) -> Option<bool> {
        let Self::Split {
            axis,
            first,
            second,
            ..
        } = self
        else {
            return None;
        };
        let forward = first.contains(first_pane) && second.contains(second_pane);
        let reverse = first.contains(second_pane) && second.contains(first_pane);
        if !forward && !reverse {
            return None;
        }
        let axis = *axis;
        let mut rectangles = HashMap::new();
        self.rectangles(area, &mut rectangles);
        let (leading, trailing) = if forward {
            (first_pane, second_pane)
        } else {
            (second_pane, first_pane)
        };
        let mut a = rectangles[&leading];
        let mut b = rectangles[&trailing];
        let aligned_segment = match axis {
            Axis::Horizontal => {
                a.y == b.y
                    && a.height == b.height
                    && a.x.checked_add(a.width) == Some(b.x)
                    && a.height < area.height
            }
            Axis::Vertical => {
                a.x == b.x
                    && a.width == b.width
                    && a.y.checked_add(a.height) == Some(b.y)
                    && a.width < area.width
            }
        };
        if !aligned_segment {
            return None;
        }
        let total = extent(a, axis).checked_add(extent(b, axis))?;
        if total < 2 {
            return Some(true);
        }
        let minimum = if total >= 2 * MIN_DRAWABLE_EXTENT {
            MIN_DRAWABLE_EXTENT
        } else {
            1
        };
        let delta = if forward {
            i32::from(delta)
        } else {
            -i32::from(delta)
        };
        let new_extent = (i32::from(extent(a, axis)) + delta)
            .clamp(i32::from(minimum), i32::from(total - minimum)) as u16;
        if new_extent == extent(a, axis) {
            return Some(true);
        }
        match axis {
            Axis::Horizontal => {
                a.width = new_extent;
                b.x = a.x + new_extent;
                b.width = total - new_extent;
            }
            Axis::Vertical => {
                a.height = new_extent;
                b.y = a.y + new_extent;
                b.height = total - new_extent;
            }
        }
        rectangles.insert(leading, a);
        rectangles.insert(trailing, b);
        let mut ids = Vec::new();
        self.panes(&mut ids);
        let panes: Vec<_> = ids.into_iter().map(|id| (id, rectangles[&id])).collect();
        // Do not fall back to moving unrelated panes if these rectangles
        // cannot be represented by a slicing tree.
        match super::expand::rebuild(area, &panes) {
            Some(layout) => {
                *self = layout;
                Some(true)
            }
            None => Some(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned_segments_clamp_without_moving_other_panes() {
        for axis in [Axis::Horizontal, Axis::Vertical] {
            for delta in [i16::MIN, i16::MAX] {
                for reversed in [false, true] {
                    let cross = if axis == Axis::Horizontal {
                        Axis::Vertical
                    } else {
                        Axis::Horizontal
                    };
                    let mut layout = Layout::Pane(0);
                    layout.split(0, 1, axis);
                    layout.split(0, 2, cross);
                    layout.split(1, 3, cross);
                    let area = Rect {
                        x: 7,
                        y: 9,
                        width: 81,
                        height: 31,
                    };
                    let mut before = HashMap::new();
                    layout.rectangles(area, &mut before);
                    let pair = if reversed { (1, 0) } else { (0, 1) };
                    assert!(layout.resize_between_cells(pair.0, pair.1, area, delta));
                    let mut after = HashMap::new();
                    layout.rectangles(area, &mut after);
                    assert_eq!(before[&2], after[&2]);
                    assert_eq!(before[&3], after[&3]);
                    let smaller = if (delta < 0) != reversed { 0 } else { 1 };
                    assert_eq!(extent(after[&smaller], axis), MIN_DRAWABLE_EXTENT);
                    assert_eq!(
                        extent(after[&0], axis) + extent(after[&1], axis),
                        extent(area, axis)
                    );
                }
            }
        }
    }
}
