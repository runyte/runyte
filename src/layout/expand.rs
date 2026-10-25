// SPDX-License-Identifier: MPL-2.0

//! Expand one pane over its neighbours while retaining rectangular panes.
use super::{Axis, Edge, Layout, MIN_DRAWABLE_EXTENT, Rect, ratio_for_first_extent};
use std::collections::HashMap;

impl Layout {
    /// Keep `pane` anchored and grow it to `target`. Covered panes disappear;
    /// a neighbour survives only if its remainder is one drawable rectangle.
    /// Neighbours fill holes left by fragmented panes. Install the result only
    /// when it tiles the editor exactly and can still be represented by splits.
    pub fn expand_pane(&mut self, pane: usize, area: Rect, target: Rect) -> bool {
        let mut original = HashMap::new();
        self.rectangles(area, &mut original);
        let Some(source) = original.get(&pane).copied() else {
            return false;
        };
        if source == target
            || intersection(source, target) != Some(source)
            || intersection(area, target) != Some(target)
        {
            return false;
        }
        let direction = if target.x < source.x {
            (Axis::Horizontal, Edge::Start)
        } else if right(target) > right(source) {
            (Axis::Horizontal, Edge::End)
        } else if target.y < source.y {
            (Axis::Vertical, Edge::Start)
        } else {
            (Axis::Vertical, Edge::End)
        };
        let mut order = Vec::new();
        self.panes(&mut order);
        let mut panes = Vec::with_capacity(order.len());
        for id in order {
            let old = original[&id];
            if id == pane {
                panes.push((id, target));
            } else if let Some(remaining) = remainder(old, target)
                && (remaining == old
                    || (remaining.width >= MIN_DRAWABLE_EXTENT
                        && remaining.height >= MIN_DRAWABLE_EXTENT))
            {
                panes.push((id, remaining));
            }
        }
        // Grow neighbours toward the same edge first. In AAA / BCD, extending
        // C upward closes the fragmented A and grows B and D into its remnants.
        let directions = [
            direction,
            (Axis::Horizontal, Edge::Start),
            (Axis::Horizontal, Edge::End),
            (Axis::Vertical, Edge::Start),
            (Axis::Vertical, Edge::End),
        ];
        loop {
            let mut changed = false;
            for &(axis, edge) in &directions {
                for index in 0..panes.len() {
                    if panes[index].0 == pane {
                        continue;
                    }
                    let grown = fill_gap(index, &panes, area, axis, edge);
                    changed |= grown != panes[index].1;
                    panes[index].1 = grown;
                }
            }
            if !changed {
                break;
            }
        }
        let covered: u64 = panes
            .iter()
            .map(|(_, r)| u64::from(r.width) * u64::from(r.height))
            .sum();
        if covered != u64::from(area.width) * u64::from(area.height) {
            return false;
        }
        let Some(layout) = rebuild(area, &panes) else {
            return false;
        };
        *self = layout;
        true
    }
}

fn right(r: Rect) -> u16 {
    r.x.saturating_add(r.width)
}
fn bottom(r: Rect) -> u16 {
    r.y.saturating_add(r.height)
}
fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let end_x = right(a).min(right(b));
    let end_y = bottom(a).min(bottom(b));
    (x < end_x && y < end_y).then(|| Rect {
        x,
        y,
        width: end_x - x,
        height: end_y - y,
    })
}

/// None means fully covered, or a remainder that would split one pane into
/// multiple pieces or an L shape. Such a pane closes instead of being cloned.
fn remainder(old: Rect, target: Rect) -> Option<Rect> {
    let Some(hit) = intersection(old, target) else {
        return Some(old);
    };
    if hit.y == old.y && hit.height == old.height {
        if hit.x == old.x && right(hit) < right(old) {
            return Some(Rect {
                x: right(hit),
                width: right(old) - right(hit),
                ..old
            });
        }
        if right(hit) == right(old) && hit.x > old.x {
            return Some(Rect {
                width: hit.x - old.x,
                ..old
            });
        }
    }
    if hit.x == old.x && hit.width == old.width {
        if hit.y == old.y && bottom(hit) < bottom(old) {
            return Some(Rect {
                y: bottom(hit),
                height: bottom(old) - bottom(hit),
                ..old
            });
        }
        if bottom(hit) == bottom(old) && hit.y > old.y {
            return Some(Rect {
                height: hit.y - old.y,
                ..old
            });
        }
    }
    None
}

fn span(r: Rect, axis: Axis) -> (u16, u16) {
    match axis {
        Axis::Horizontal => (r.x, right(r)),
        Axis::Vertical => (r.y, bottom(r)),
    }
}
fn across(axis: Axis) -> Axis {
    match axis {
        Axis::Horizontal => Axis::Vertical,
        Axis::Vertical => Axis::Horizontal,
    }
}
fn fill_gap(index: usize, panes: &[(usize, Rect)], area: Rect, axis: Axis, edge: Edge) -> Rect {
    let rect = panes[index].1;
    let (start, end) = span(rect, axis);
    let (cross_start, cross_end) = span(rect, across(axis));
    let (mut low, mut high) = span(area, axis);
    for (other_index, &(_, other)) in panes.iter().enumerate() {
        if index == other_index {
            continue;
        }
        let (a, b) = span(other, across(axis));
        if a >= cross_end || b <= cross_start {
            continue;
        }
        let (a, b) = span(other, axis);
        if b <= start {
            low = low.max(b);
        }
        if a >= end {
            high = high.min(a);
        }
    }
    let (start, end) = match edge {
        Edge::Start => (low, end),
        Edge::End => (start, high),
    };
    match axis {
        Axis::Horizontal => Rect {
            x: start,
            width: end - start,
            ..rect
        },
        Axis::Vertical => Rect {
            y: start,
            height: end - start,
            ..rect
        },
    }
}

/// Reconstruct a slicing tree from the final rectangles. Cuts are existing
/// pane edges, and ratios round back to the exact previewed cell boundaries.
pub(super) fn rebuild(area: Rect, panes: &[(usize, Rect)]) -> Option<Layout> {
    if let [(id, rect)] = panes {
        return (*rect == area).then_some(Layout::Pane(*id));
    }
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let (start, end) = span(area, axis);
        for &(_, rect) in panes {
            let cut = span(rect, axis).1;
            if cut <= start
                || cut >= end
                || panes.iter().any(|(_, r)| {
                    let (a, b) = span(*r, axis);
                    a < cut && b > cut
                })
            {
                continue;
            }
            let (first, second): (Vec<_>, Vec<_>) = panes
                .iter()
                .copied()
                .partition(|(_, r)| span(*r, axis).1 <= cut);
            let ratio = ratio_for_first_extent(end - start, i32::from(cut - start), 1, 1);
            let (a, b) = super::split_areas(area, axis, ratio);
            // Any full cut is compatible with a slicing tiling. If one side
            // cannot be sliced, trying other full cuts cannot repair it.
            return Some(Layout::Split {
                axis,
                ratio,
                first: Box::new(rebuild(a, &first)?),
                second: Box::new(rebuild(b, &second)?),
            });
        }
    }
    None
}

#[cfg(test)]
#[path = "tests/expand.rs"]
mod tests;
