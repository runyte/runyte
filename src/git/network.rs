// SPDX-License-Identifier: MPL-2.0

//! Captured multi-root history and bounded path routing.
//!
//! Captured pages contain commit blocks with independently colored paths and
//! connector rows. Cursor state preserves lane order and colors across pages.

mod routing;
pub use routing::{GraphCell, GraphLine, MAX_CONNECTOR_ROWS};

use unicode_segmentation::UnicodeSegmentation;

use super::CommitSummary;

pub const HASH_COLUMNS: usize = 12;
const INITIALS_COLUMNS: usize = 4;
const COLUMN_GAP: &str = "  ";
pub const GRAPH_COLUMN: usize = HASH_COLUMNS + INITIALS_COLUMNS + 2 * COLUMN_GAP.len();

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
    /// Colors travel with pending paths, independently of column positions.
    pub colors: Vec<u8>,
    pub next_color: u8,
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
    /// Parent-to-lane mapping at block exit, after compaction. These are not
    /// column annotations for the commit row. `None` means an undrawn route,
    /// never that the parent disappeared.
    pub edges: Vec<(String, Option<usize>)>,
    pub shallow: bool,
    pub node: GraphLine,
    pub connectors: Vec<GraphLine>,
}

impl GraphRow {
    pub fn width(&self) -> usize {
        std::iter::once(&self.node)
            .chain(&self.connectors)
            .map(|line| line.cells.len().div_ceil(2))
            .max()
            .unwrap_or(1)
            .max(1)
    }

    pub fn graph(&self, ascii: bool) -> String {
        self.graph_with_width(ascii, self.width())
    }

    pub fn graph_with_width(&self, ascii: bool, width: usize) -> String {
        self.node.text(ascii, width.max(self.width()))
    }

    pub fn route_note(&self) -> String {
        let ambiguous = self.edges.iter().any(|(_, target)| target.is_none());
        let mut note = if ambiguous {
            format!(
                " [overflow parents: {}; Enter inspects all parents]",
                self.edges
                    .iter()
                    .filter(|(_, lane)| lane.is_none())
                    .map(|(oid, _)| oid.chars().take(HASH_COLUMNS).collect::<String>())
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
        let labels = if self.commit.decorations.is_empty() {
            String::new()
        } else {
            format!(
                "[{}] ",
                self.commit
                    .decorations
                    .iter()
                    .map(|label| display_label(label))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!(
            "{}{}{COLUMN_GAP}{}{}{}",
            self.metadata_prefix(),
            self.graph_with_width(ascii, width),
            labels,
            self.commit.subject,
            self.route_note()
        )
    }

    pub fn metadata_prefix(&self) -> String {
        use unicode_width::UnicodeWidthStr;
        let initials = author_initials(&self.commit.author);
        let padding = " ".repeat(INITIALS_COLUMNS.saturating_sub(initials.width()));
        let hash = self
            .commit
            .oid
            .chars()
            .take(HASH_COLUMNS)
            .collect::<String>();
        format!("{hash:<HASH_COLUMNS$}{COLUMN_GAP}{initials}{padding}{COLUMN_GAP}")
    }
}

pub fn connector_text(line: &GraphLine, ascii: bool, width: usize) -> String {
    format!("{}{}", " ".repeat(GRAPH_COLUMN), line.text(ascii, width))
}

/// Labels are metadata, not graph glyphs. Keep their spelling and coordinates
/// identical while toggling the graph between Unicode and ASCII.
pub fn display_label(label: &str) -> String {
    label.replace("HEAD → ", "HEAD -> ")
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
        || unicode_width::UnicodeWidthStr::width(initials.as_str()) > INITIALS_COLUMNS
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
#[path = "network/tests.rs"]
mod tests;
