// SPDX-License-Identifier: MPL-2.0

//! Bounded adjacent-lane routing. A crossing swaps two paths; a join removes
//! only a duplicate destination. Neither operation changes surviving colors.

use super::{CommitSummary, GraphLanes, GraphRow, MAX_NETWORK_LANES};

/// At most 65 text rows per commit, including the metadata row. If a dense
/// merge reaches this budget, label undrawn parents without replaying layout.
pub const MAX_CONNECTOR_ROWS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum Glyph {
    Blank,
    Node,
    Vertical,
    Left,
    Right,
    Crossing,
    Horizontal,
    CrossHorizontal,
    ForkLeft,
    ForkRight,
    TargetLeft,
    TargetRight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphCell {
    glyph: Glyph,
    pub color: u8,
}

impl GraphCell {
    pub fn character(self, ascii: bool) -> char {
        match (self.glyph, ascii) {
            (Glyph::Blank, _) => ' ',
            (Glyph::Node, true) => '*',
            (Glyph::Node, false) => '●',
            (Glyph::Vertical, true) => '|',
            (Glyph::Vertical, false) => '│',
            (Glyph::Left, true) => '/',
            (Glyph::Left, false) => '╱',
            (Glyph::Right, true) => '\\',
            (Glyph::Right, false) => '╲',
            (Glyph::Crossing, true) => 'x',
            (Glyph::Crossing, false) => '╳',
            (Glyph::Horizontal, true) => '-',
            (Glyph::Horizontal, false) => '─',
            (Glyph::CrossHorizontal, true) => 'x',
            (Glyph::CrossHorizontal, false) => '╫',
            (Glyph::ForkLeft | Glyph::ForkRight, true) => '+',
            (Glyph::ForkLeft, false) => '┤',
            (Glyph::ForkRight, false) => '├',
            (Glyph::TargetLeft, _) => '<',
            (Glyph::TargetRight, _) => '>',
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GraphLine {
    pub cells: Vec<GraphCell>,
}

impl GraphLine {
    fn blank(lanes: usize) -> Self {
        Self {
            cells: vec![
                GraphCell {
                    glyph: Glyph::Blank,
                    color: 0
                };
                (lanes * 2).saturating_sub(1)
            ],
        }
    }

    fn put(&mut self, column: usize, glyph: Glyph, color: u8) {
        self.cells[column] = GraphCell { glyph, color };
    }

    fn vertical(tracks: &[Track]) -> Self {
        let mut line = Self::blank(tracks.len());
        for (lane, track) in tracks.iter().enumerate() {
            line.put(lane * 2, Glyph::Vertical, track.color);
        }
        line
    }

    pub fn text(&self, ascii: bool, width: usize) -> String {
        let mut chars = self
            .cells
            .iter()
            .map(|cell| cell.character(ascii))
            .collect::<Vec<_>>();
        chars.resize((width.min(MAX_NETWORK_LANES) * 2).saturating_sub(1), ' ');
        chars.into_iter().collect()
    }
}

#[derive(Clone)]
struct Track {
    oid: String,
    color: u8,
    // Local identity distinguishes a newly routed edge from its destination.
    id: usize,
}

fn color(next: &mut u8, neighbors: impl Iterator<Item = u8>) -> u8 {
    let occupied = neighbors.fold(0_u8, |mask, color| mask | (1 << color));
    let chosen = (0..4)
        .map(|step| (*next + step) % 4)
        .find(|candidate| occupied & (1 << candidate) == 0)
        .unwrap_or(*next);
    *next = (chosen + 1) % 4;
    chosen
}

impl GraphLanes {
    pub fn row(&mut self, commit: CommitSummary, shallow: bool) -> GraphRow {
        let mut tracks = self
            .pending
            .iter()
            .enumerate()
            .filter_map(|(index, oid)| {
                oid.as_ref().map(|oid| Track {
                    oid: oid.clone(),
                    color: self.colors.get(index).copied().unwrap_or((index % 4) as u8),
                    id: index,
                })
            })
            .collect::<Vec<_>>();
        let mut next_color = self.next_color;
        let lane = tracks
            .iter()
            .position(|track| track.oid == commit.oid)
            .or_else(|| {
                if tracks.len() == MAX_NETWORK_LANES {
                    return None;
                }
                let color = color(
                    &mut next_color,
                    tracks.last().map(|track| track.color).into_iter(),
                );
                tracks.push(Track {
                    oid: commit.oid.clone(),
                    color,
                    id: MAX_NETWORK_LANES,
                });
                Some(tracks.len() - 1)
            });
        let mut node = GraphLine::vertical(&tracks);
        if let Some(lane) = lane {
            node.put(lane * 2, Glyph::Node, tracks[lane].color);
        }
        let Some(lane_index) = lane else {
            return GraphRow {
                edges: commit
                    .parents
                    .iter()
                    .map(|oid| (oid.clone(), None))
                    .collect(),
                commit,
                lane,
                shallow,
                node,
                connectors: Vec::new(),
            };
        };
        let (outgoing, connectors, drawn, next) =
            route(tracks, lane_index, &commit.parents, next_color);
        let edges = commit
            .parents
            .iter()
            .map(|oid| {
                let lane = drawn
                    .contains(oid)
                    .then(|| outgoing.iter().position(|track| &track.oid == oid))
                    .flatten();
                (oid.clone(), lane)
            })
            .collect();
        self.pending = outgoing
            .iter()
            .map(|track| Some(track.oid.clone()))
            .collect();
        self.colors = outgoing.iter().map(|track| track.color).collect();
        self.next_color = next;
        GraphRow {
            commit,
            lane,
            edges,
            shallow,
            node,
            connectors,
        }
    }
}

fn route(
    mut tracks: Vec<Track>,
    source: usize,
    parents: &[String],
    mut next: u8,
) -> (Vec<Track>, Vec<GraphLine>, Vec<String>, u8) {
    let mut rows = Vec::new();
    let mut drawn = Vec::new();
    if let Some(first) = parents.first() {
        tracks[source].oid = first.clone();
        let source_id = tracks[source].id;
        drawn.push(first.clone());
        for (index, parent) in parents.iter().enumerate().skip(1) {
            if drawn.contains(parent) {
                continue;
            }
            let source = tracks
                .iter()
                .position(|track| track.id == source_id)
                .unwrap();
            let target = tracks.iter().position(|track| &track.oid == parent);
            let first_is_shared = tracks
                .iter()
                .any(|track| track.id != source_id && &track.oid == first);
            // Reserve the remaining first-parent join before admitting another
            // route. No work is rendered and discarded to discover a limit.
            let reserve = if first_is_shared {
                2 * (tracks.len() - 1)
            } else {
                0
            };
            let expanded_target = target.map(|target| target + usize::from(target > source));
            let cost = 2 + expanded_target.map_or(0, |target| 2 * (source + 1).abs_diff(target));
            let expanded_reserve = if target.is_none() && first_is_shared {
                reserve + 2
            } else {
                reserve
            };
            if let Some(target) = target
                && (tracks.len() == MAX_NETWORK_LANES
                    || rows.len() + cost + expanded_reserve > MAX_CONNECTOR_ROWS + 1)
            {
                if rows.len() + 2 + reserve > MAX_CONNECTOR_ROWS + 1 {
                    continue;
                }
                // An existing lane needs no expansion slot. Give this one
                // edge its own directed row, with explicit pass-throughs.
                rows.push(existing_route(&tracks, source, target));
                rows.push(GraphLine::vertical(&tracks));
                drawn.push(parent.clone());
                continue;
            }
            if tracks.len() == MAX_NETWORK_LANES
                || rows.len() + cost + expanded_reserve > MAX_CONNECTOR_ROWS + 1
            {
                continue;
            }
            let edge_color = color(
                &mut next,
                tracks[source..].iter().take(2).map(|track| track.color),
            );
            let mut line = GraphLine::blank(tracks.len() + 1);
            for (lane, track) in tracks.iter().enumerate() {
                if lane <= source {
                    line.put(lane * 2, Glyph::Vertical, track.color);
                } else {
                    line.put(lane * 2 + 1, Glyph::Right, track.color);
                }
            }
            line.put(source * 2 + 1, Glyph::Right, edge_color);
            let target_id = target.map(|lane| tracks[lane].id);
            tracks.insert(
                source + 1,
                Track {
                    oid: parent.clone(),
                    color: edge_color,
                    id: MAX_NETWORK_LANES + 1 + index,
                },
            );
            rows.push(line);
            rows.push(GraphLine::vertical(&tracks));
            if let Some(target_id) = target_id {
                join_tracks(&mut tracks, &mut rows, source + 1, target_id);
            }
            drawn.push(parent.clone());
        }
        let from = tracks
            .iter()
            .position(|track| track.id == source_id)
            .unwrap();
        if let Some(target_id) = tracks
            .iter()
            .find(|track| track.id != source_id && &track.oid == first)
            .map(|track| track.id)
        {
            join_tracks(&mut tracks, &mut rows, from, target_id);
        }
    } else {
        // End this path and close its hole. Roots and omitted/overflow ancestry
        // are distinguished by the row's explicit parent-route note.
        let mut line = GraphLine::blank(tracks.len());
        for (lane, track) in tracks.iter().enumerate() {
            if lane < source {
                line.put(lane * 2, Glyph::Vertical, track.color);
            }
            if lane > source {
                line.put(lane * 2 - 1, Glyph::Left, track.color);
            }
        }
        tracks.remove(source);
        // Even a rightmost root leaves a blank below its node so a later
        // disconnected root reusing this column cannot look like its parent.
        rows.push(line);
        rows.push(GraphLine::vertical(&tracks));
    }
    // The next commit row supplies the final settled verticals, also across a
    // page boundary. Intermediate settled rows separate consecutive bends.
    rows.pop();
    (tracks, rows, drawn, next)
}

/// A single directed edge on a dedicated row avoids allocating a temporary
/// lane at capacity. The arrow identifies its target even across other paths.
fn existing_route(tracks: &[Track], source: usize, target: usize) -> GraphLine {
    let mut line = GraphLine::vertical(tracks);
    let edge_color = tracks[target].color;
    for column in source.min(target) * 2 + 1..source.max(target) * 2 {
        line.put(
            column,
            if column % 2 == 0 {
                Glyph::CrossHorizontal
            } else {
                Glyph::Horizontal
            },
            edge_color,
        );
    }
    line.put(
        source * 2,
        if target < source {
            Glyph::ForkLeft
        } else {
            Glyph::ForkRight
        },
        tracks[source].color,
    );
    line.put(
        target * 2,
        if target < source {
            Glyph::TargetLeft
        } else {
            Glyph::TargetRight
        },
        edge_color,
    );
    line
}

fn join_tracks(
    tracks: &mut Vec<Track>,
    rows: &mut Vec<GraphLine>,
    mut from: usize,
    target_id: usize,
) {
    loop {
        let target = tracks
            .iter()
            .position(|track| track.id == target_id)
            .unwrap();
        if from.abs_diff(target) == 1 {
            // Both paths enter the lower-numbered column. The existing
            // destination's identity and color survive the junction.
            let low = from.min(target);
            let high = from.max(target);
            let mut line = GraphLine::blank(tracks.len());
            for (lane, track) in tracks.iter().enumerate() {
                if lane <= low {
                    line.put(lane * 2, Glyph::Vertical, track.color);
                } else {
                    line.put(lane * 2 - 1, Glyph::Left, track.color);
                }
            }
            let survivor = tracks[target].clone();
            tracks[low] = survivor;
            tracks.remove(high);
            rows.push(line);
            rows.push(GraphLine::vertical(tracks));
            break;
        }
        // Adjacent swap, explicitly a crossing, never a join. The
        // shared cell has the moving edge's color; both paths retain
        // their own colors on either side of it.
        let neighbor = if target < from { from - 1 } else { from + 1 };
        let low = from.min(neighbor);
        let high = from.max(neighbor);
        let mut line = GraphLine::vertical(tracks);
        line.put(low * 2, Glyph::Blank, 0);
        line.put(high * 2, Glyph::Blank, 0);
        line.put(low * 2 + 1, Glyph::Crossing, tracks[from].color);
        tracks.swap(from, neighbor);
        from = neighbor;
        rows.push(line);
        rows.push(GraphLine::vertical(tracks));
    }
}
