// SPDX-License-Identifier: MPL-2.0

//! One-row rectangular routing. Pending paths retain their columns and colors
//! until they end; holes can be reused without shifting surviving paths.

use super::{CommitSummary, GraphLanes, GraphRow, MAX_NETWORK_LANES, NetworkRoot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum Glyph {
    Blank,
    Node,
    Vertical,
    Horizontal,
    Junction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphCell {
    glyph: Glyph,
    pub color: u8,
}

impl GraphCell {
    pub fn character(self) -> char {
        match self.glyph {
            Glyph::Blank => ' ',
            Glyph::Node => '*',
            Glyph::Vertical => '|',
            Glyph::Horizontal => '-',
            Glyph::Junction => '+',
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GraphLine {
    pub cells: Vec<GraphCell>,
}

impl GraphLine {
    pub fn text(&self, width: usize) -> String {
        let mut chars = self
            .cells
            .iter()
            .map(|cell| cell.character())
            .collect::<Vec<_>>();
        chars.resize((width.min(MAX_NETWORK_LANES) * 2).saturating_sub(1), ' ');
        chars.into_iter().collect()
    }
}

impl GraphLanes {
    /// Prefer a nearby empty column, but never move an existing path.
    fn assign(&mut self, oid: &str, near: usize) -> Option<usize> {
        if let Some(lane) = self
            .pending
            .iter()
            .position(|value| value.as_deref() == Some(oid))
        {
            return Some(lane);
        }
        let lane = (0..self.pending.len())
            .filter(|&lane| self.pending[lane].is_none())
            .min_by_key(|&lane| (lane.abs_diff(near), lane))
            .unwrap_or(self.pending.len());
        if lane == MAX_NETWORK_LANES {
            return None;
        }
        let occupied = self
            .pending
            .iter()
            .enumerate()
            .filter(|(_, oid)| oid.is_some())
            .fold(0_u8, |mask, (lane, _)| mask | (1 << self.colors[lane]));
        let chosen = (0..4)
            .map(|step| (self.next_color + step) % 4)
            .find(|candidate| occupied & (1 << candidate) == 0)
            .unwrap_or(self.next_color);
        self.next_color = (chosen + 1) % 4;
        if lane == self.pending.len() {
            self.pending.push(None);
            self.colors.push(chosen);
            self.branches.push(None);
        }
        self.pending[lane] = Some(oid.to_owned());
        self.colors[lane] = chosen;
        self.branches[lane] = None;
        Some(lane)
    }

    pub fn row(&mut self, commit: CommitSummary, shallow: bool) -> GraphRow {
        self.row_with_roots(commit, shallow, &[])
    }

    pub fn row_with_roots(
        &mut self,
        commit: CommitSummary,
        shallow: bool,
        roots: &[NetworkRoot],
    ) -> GraphRow {
        let branch_at = |oid: &str| {
            let root = roots.iter().find(|root| root.oid == oid)?;
            root.labels
                .iter()
                .find_map(|label| label.strip_prefix("HEAD → "))
                .or_else(|| {
                    root.references
                        .iter()
                        .find_map(|reference| reference.strip_prefix("refs/heads/"))
                })
                .or_else(|| {
                    root.references
                        .iter()
                        .find_map(|reference| reference.strip_prefix("refs/remotes/"))
                })
                .map(str::to_owned)
        };
        let lane = self.assign(&commit.oid, 0);
        if let Some(lane) = lane
            && self.branches[lane].is_none()
        {
            self.branches[lane] = branch_at(&commit.oid);
        }
        let branch = lane.and_then(|lane| self.branches[lane].clone());
        let mut edges = Vec::with_capacity(commit.parents.len());
        if let Some(source) = lane {
            for (index, parent) in commit.parents.iter().enumerate() {
                let existing = self
                    .pending
                    .iter()
                    .position(|oid| oid.as_ref() == Some(parent));
                let target = if index == 0 && existing.is_none() {
                    // The first parent continues this path with its existing color.
                    self.pending[source] = Some(parent.clone());
                    Some(source)
                } else {
                    self.assign(parent, source)
                };
                if let Some(target) = target
                    && self.branches[target].is_none()
                {
                    self.branches[target] = branch_at(parent);
                }
                edges.push((parent.clone(), target));
            }
        } else {
            edges.extend(commit.parents.iter().map(|oid| (oid.clone(), None)));
        }
        let mut node = GraphLine {
            cells: vec![
                GraphCell {
                    glyph: Glyph::Blank,
                    color: 0
                };
                (self.pending.len() * 2).saturating_sub(1)
            ],
        };
        for (lane, oid) in self.pending.iter().enumerate() {
            if oid.is_some() {
                node.cells[lane * 2] = GraphCell {
                    glyph: Glyph::Vertical,
                    color: self.colors[lane],
                };
            }
        }
        if let Some(source) = lane {
            // Draw farther endpoints first: shared horizontal segments take
            // the nearest endpoint's color. Vertical crossings keep their own
            // path color, and only parent endpoints become '+' junctions.
            let mut targets = edges
                .iter()
                .filter_map(|(_, lane)| *lane)
                .collect::<Vec<_>>();
            targets.sort_by_key(|&target| std::cmp::Reverse(target.abs_diff(source)));
            for target in targets {
                if target == source {
                    continue;
                }
                for column in source.min(target) * 2 + 1..source.max(target) * 2 {
                    if matches!(node.cells[column].glyph, Glyph::Blank | Glyph::Horizontal) {
                        node.cells[column] = GraphCell {
                            glyph: Glyph::Horizontal,
                            color: self.colors[target],
                        };
                    }
                }
                node.cells[target * 2] = GraphCell {
                    glyph: Glyph::Junction,
                    color: self.colors[target],
                };
            }
            node.cells[source * 2] = GraphCell {
                glyph: Glyph::Node,
                color: self.colors[source],
            };
            if self.pending[source].as_ref() == Some(&commit.oid) {
                // Keep this column reserved for the whole row, so another
                // parent cannot appear to continue a path that just ended.
                self.pending[source] = None;
                self.branches[source] = None;
            }
        }
        while self.pending.last().is_some_and(Option::is_none) {
            self.pending.pop();
            self.colors.pop();
            self.branches.pop();
        }
        GraphRow {
            commit,
            lane,
            edges,
            shallow,
            node,
            branch,
            containing_branch: None,
        }
    }
}
