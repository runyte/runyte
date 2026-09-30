// SPDX-License-Identifier: MPL-2.0

//! Captured multi-root history and pure sparse-lane graph layout.
//!
//! Lane numbers are stable for a generation. Edges which cannot be drawn
//! unambiguously on one row name their destination lanes instead; commit
//! detail always exposes the complete parent identities.

use unicode_segmentation::UnicodeSegmentation;

use super::CommitSummary;

pub const NETWORK_PAGE_SIZE: usize = 200;
pub const MAX_NETWORK_COMMITS: usize = 10_000;
pub const MAX_NETWORK_PAGES: usize = MAX_NETWORK_COMMITS / NETWORK_PAGE_SIZE;
pub const MAX_NETWORK_LANES: usize = 16;
pub const MAX_NETWORK_ROOTS: usize = 256;
pub const MAX_NETWORK_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub enum NetworkScope {
    #[default]
    All,
    Head,
    Ref(String),
}

impl NetworkScope {
    pub fn label(&self) -> &str {
        match self {
            Self::All => "all cached refs and HEAD",
            Self::Head => "current HEAD",
            Self::Ref(reference) => reference,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct NetworkRoot {
    pub oid: String,
    pub labels: Vec<String>,
    pub references: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct GraphLanes {
    pub pending: Vec<Option<String>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct NetworkCursor {
    pub roots: Vec<NetworkRoot>,
    pub offset: usize,
    pub lanes: GraphLanes,
    pub roots_limited: bool,
    pub shallow_fingerprint: String,
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct NetworkRequest {
    pub scope: NetworkScope,
    pub cursor: Option<NetworkCursor>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphRow {
    pub commit: CommitSummary,
    pub lane: Option<usize>,
    /// Exact parent-to-lane mapping. `None` means the bounded layout cannot
    /// represent the edge, never that the parent disappeared.
    pub edges: Vec<(String, Option<usize>)>,
    pub active: Vec<usize>,
    pub shallow: bool,
}

impl GraphLanes {
    fn assign(&mut self, oid: &str, preferred: Option<usize>) -> Option<usize> {
        if let Some(lane) = self
            .pending
            .iter()
            .position(|value| value.as_deref() == Some(oid))
        {
            return Some(lane);
        }
        let lane = preferred
            .filter(|&lane| self.pending.get(lane).is_some_and(Option::is_none))
            .or_else(|| self.pending.iter().position(Option::is_none));
        if let Some(lane) = lane {
            self.pending[lane] = Some(oid.to_owned());
            Some(lane)
        } else if self.pending.len() < MAX_NETWORK_LANES {
            self.pending.push(Some(oid.to_owned()));
            Some(self.pending.len() - 1)
        } else {
            None
        }
    }

    pub fn row(&mut self, commit: CommitSummary, shallow: bool) -> GraphRow {
        let lane = self.assign(&commit.oid, None);
        let active = self
            .pending
            .iter()
            .enumerate()
            .filter_map(|(index, oid)| oid.as_ref().map(|_| index))
            .collect();
        if let Some(lane) = lane {
            self.pending[lane] = None;
        }
        let edges = commit
            .parents
            .iter()
            .enumerate()
            .map(|(index, parent)| {
                let assigned = self.assign(parent, (index == 0).then_some(lane).flatten());
                (parent.clone(), assigned)
            })
            .collect();
        GraphRow {
            commit,
            lane,
            edges,
            active,
            shallow,
        }
    }
}

impl GraphRow {
    pub fn width(&self) -> usize {
        self.active
            .iter()
            .copied()
            .chain(self.lane)
            .chain(self.edges.iter().filter_map(|(_, lane)| *lane))
            .max()
            .map_or(1, |lane| lane + 1)
    }

    pub fn graph(&self, ascii: bool) -> String {
        self.graph_with_width(ascii, self.width())
    }

    pub fn graph_with_width(&self, ascii: bool, width: usize) -> String {
        let width = width.clamp(self.width(), MAX_NETWORK_LANES);
        let mut cells = vec![' '; width * 2 - 1];
        for &lane in &self.active {
            cells[lane * 2] = if ascii { '|' } else { '│' };
        }
        if let Some(lane) = self.lane {
            cells[lane * 2] = if ascii { '*' } else { '●' };
            let external = self
                .edges
                .iter()
                .filter_map(|(_, target)| target.filter(|target| *target != lane))
                .collect::<Vec<_>>();
            if external.len() == 1
                && self.edges.iter().all(|(_, lane)| lane.is_some())
                && external[0].abs_diff(lane) == 1
            {
                let target = external[0];
                cells[lane.min(target) * 2 + 1] = if ascii { '-' } else { '─' };
                let occupied = self.active.contains(&target);
                cells[target * 2] = if ascii {
                    '+'
                } else {
                    match (target > lane, occupied) {
                        (true, false) => '╮',
                        (false, false) => '╭',
                        (true, true) => '┤',
                        (false, true) => '├',
                    }
                };
            }
        }
        cells.into_iter().collect()
    }

    pub fn route_note(&self) -> String {
        let ambiguous = self.lane.is_none()
            || self.edges.iter().any(|(_, target)| target.is_none())
            || self
                .edges
                .iter()
                .filter(|(_, target)| *target != self.lane)
                .count()
                > 1
            || self.edges.iter().any(|(_, target)| {
                target
                    .zip(self.lane)
                    .is_some_and(|(a, b)| a.abs_diff(b) > 1)
            });
        let mut note = if ambiguous {
            format!(
                " [parent lanes: {}; Enter inspects parents]",
                self.edges
                    .iter()
                    .map(|(_, lane)| lane
                        .map_or_else(|| "overflow".to_owned(), |lane| (lane + 1).to_string()))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        } else {
            String::new()
        };
        if self.lane.is_none() {
            note.push_str(" [unplaced commit: lane limit]");
        }
        if self.shallow {
            note.push_str(" [shallow boundary]");
        }
        note
    }

    pub fn text(&self, ascii: bool) -> String {
        self.text_with_width(ascii, self.width())
    }

    pub fn text_with_width(&self, ascii: bool, width: usize) -> String {
        use unicode_width::UnicodeWidthStr;
        let initials = author_initials(&self.commit.author);
        let initials_padding = " ".repeat(4usize.saturating_sub(initials.width()));
        let labels = if self.commit.decorations.is_empty() {
            String::new()
        } else {
            format!(
                "[{}] ",
                self.commit
                    .decorations
                    .iter()
                    .map(|label| if ascii {
                        label.replace("HEAD → ", "HEAD -> ")
                    } else {
                        label.clone()
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!(
            "{:<12}  {}{}  {}  {}{}{}",
            self.commit.oid.chars().take(12).collect::<String>(),
            initials,
            initials_padding,
            self.graph_with_width(ascii, width),
            labels,
            self.commit.subject,
            self.route_note()
        )
    }
}

pub fn author_initials(author: &str) -> String {
    let words = author.split_whitespace().collect::<Vec<_>>();
    let initials = words
        .first()
        .into_iter()
        .chain((words.len() > 1).then(|| words.last().unwrap()))
        .filter_map(|word| word.graphemes(true).next())
        .collect::<String>();
    if initials.chars().any(char::is_control)
        || initials.is_empty()
        || unicode_width::UnicodeWidthStr::width(initials.as_str()) > 4
    {
        "??".to_owned()
    } else {
        initials
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPage {
    pub rows: Vec<GraphRow>,
    pub next: Option<NetworkCursor>,
    pub roots: Vec<NetworkRoot>,
    pub stale: bool,
    pub limited: bool,
    pub roots_limited: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn commit(oid: &str, parents: &[&str]) -> CommitSummary {
        CommitSummary {
            oid: oid.into(),
            abbreviated: oid.into(),
            parents: parents.iter().map(|p| (*p).into()).collect(),
            author: "A B".into(),
            author_time: 0,
            author_date: String::new(),
            author_datetime: String::new(),
            subject: String::new(),
            decorations: vec![],
        }
    }
    #[test]
    fn branches_octopus_disconnected_roots_and_joins_retain_exact_parent_routes() {
        let mut lanes = GraphLanes::default();
        for (oid, parents) in [
            ("merge", vec!["a", "b", "c"]),
            ("a", vec!["root"]),
            ("b", vec!["root"]),
            ("c", vec!["root"]),
            ("other", vec![]),
            ("root", vec![]),
        ] {
            let row = lanes.row(commit(oid, &parents), false);
            assert_eq!(
                row.edges
                    .iter()
                    .map(|(oid, _)| oid.as_str())
                    .collect::<Vec<_>>(),
                parents
            );
            for (parent, lane) in &row.edges {
                assert_eq!(
                    lanes.pending[lane.unwrap()].as_deref(),
                    Some(parent.as_str())
                );
            }
        }
        assert!(lanes.pending.iter().all(Option::is_none));
    }
    #[test]
    fn page_continuation_and_back_replay_are_identical() {
        let mut lanes = GraphLanes::default();
        lanes.row(commit("merge", &["a", "b"]), false);
        let checkpoint = lanes.clone();
        let row = lanes.row(commit("a", &["root"]), false);
        assert_eq!(row, checkpoint.clone().row(commit("a", &["root"]), false));
        assert_eq!(
            lanes.row(commit("b", &["root"]), false).edges[0].1,
            row.edges[0].1
        );
    }
    #[test]
    fn overflow_and_shallow_boundaries_are_explicit_and_bounded() {
        let parents = (0..MAX_NETWORK_LANES + 2)
            .map(|i| i.to_string())
            .collect::<Vec<_>>();
        let mut lanes = GraphLanes::default();
        let row = lanes.row(
            commit("m", &parents.iter().map(String::as_str).collect::<Vec<_>>()),
            true,
        );
        assert_eq!(lanes.pending.len(), MAX_NETWORK_LANES);
        assert_eq!(row.edges.len(), parents.len());
        assert!(row.route_note().contains("overflow"));
        assert!(row.route_note().contains("shallow boundary"));
        assert!(row.graph(true).is_ascii());
    }
    #[test]
    fn adjacent_forks_and_joins_have_connected_downward_routes() {
        let mut lanes = GraphLanes::default();
        let merge = lanes.row(commit("merge", &["a", "b"]), false);
        assert_eq!(merge.graph(false), "●─╮");
        assert_eq!(merge.graph(true), "*-+");
        assert_eq!(
            merge
                .edges
                .iter()
                .map(|(_, lane)| *lane)
                .collect::<Vec<_>>(),
            [Some(0), Some(1)]
        );
        let a = lanes.row(commit("a", &["root"]), false);
        assert_eq!(a.graph(false), "● │");
        let b = lanes.row(commit("b", &["root"]), false);
        assert_eq!(b.graph(false), "├─●");
        assert_eq!(b.graph(true), "+-*");
        assert_eq!(b.edges, [("root".into(), Some(0))]);
        // The tee retains root's vertical connection below this join.
        assert_eq!(lanes.pending[0].as_deref(), Some("root"));
        let root = lanes.row(commit("root", &[]), false);
        assert_eq!(root.lane, Some(0));
        assert!(lanes.pending.iter().all(Option::is_none));
    }
    #[test]
    fn unplaced_commits_never_replace_or_reassign_an_existing_lane() {
        let mut lanes = GraphLanes {
            pending: (0..MAX_NETWORK_LANES)
                .map(|index| Some(format!("pending{index}")))
                .collect(),
        };
        let before = lanes.clone();
        let row = lanes.row(commit("unrelated", &["unplaced-parent"]), false);
        assert_eq!(row.lane, None);
        assert_eq!(lanes, before);
        assert_eq!(row.graph(false), vec!["│"; MAX_NETWORK_LANES].join(" "));
        assert!(row.route_note().contains("unplaced commit"));
        assert_eq!(row.edges, [("unplaced-parent".into(), None)]);
    }
    #[test]
    fn hash_initials_and_subject_columns_are_fixed_in_display_cells() {
        use unicode_width::UnicodeWidthStr;
        for author in ["A B", "李 小龍", "A\u{301}lpha Beta", "👩‍💻 Coder"] {
            let mut summary = commit(&"a".repeat(64), &[]);
            summary.abbreviated = "a".repeat(16);
            summary.author = author.into();
            summary.subject = "SUBJECT".into();
            let row = GraphLanes::default().row(summary, false);
            let text = row.text_with_width(false, 2);
            assert_eq!(text[..text.find("SUBJECT").unwrap()].width(), 25);
            assert_eq!(&text[..12], "aaaaaaaaaaaa");
        }
    }
    #[test]
    fn initials_use_graphemes_and_stable_fallback() {
        assert_eq!(author_initials("  "), "??");
        assert_eq!(author_initials("A\u{301}lpha Beta"), "A\u{301}B");
        assert_eq!(author_initials("李 小龍"), "李小");
    }
}
