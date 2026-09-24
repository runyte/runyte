// SPDX-License-Identifier: MPL-2.0

//! Bounded semantic application projections. Plugin row IDs never become offsets.
use super::application::{Error, ErrorCode};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

mod decoding;
mod patch;
mod pending;
mod projection;
mod query;
mod staging;
pub(crate) use decoding::operations as decode_operations;
pub use patch::{Operation, Patch};
pub use pending::Prepared;
pub(crate) use pending::{OwnedSnapshot, Pending};
pub use projection::{PreparedModel, ProjectedRow, Projection};
pub use query::{MAX_QUERY_BYTES, QUERY_CHARGE, Query};
pub(crate) use query::{QueryState, check_query};
pub use staging::StageKind;
pub(crate) use staging::{ReadSnapshot, Stage};

pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DOCUMENT_LINES: usize = 250_000;
pub const MAX_DOCUMENT_MODEL_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_DOCUMENT_RETAINED_BYTES: usize = 64 * 1024 * 1024;
pub const DOCUMENT_PREPARE_CHARGE: usize = 96 * 1024 * 1024;
pub const MAX_MODEL_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ROWS: usize = 10_000;
pub const MAX_PATCH_REFERENCES: usize = 20_000;
pub const MAX_COLUMNS: usize = 8;
pub const MAX_BLOCK_BYTES: usize = 64 * 1024;
pub const MAX_BLOCK_LINES: usize = 1024;
pub const MAX_PROJECTION_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_RETAINED_BYTES: usize = 16 * 1024 * 1024;
pub const PREPARE_CHARGE: usize = 32 * 1024 * 1024;
pub const MAX_CHUNK_BYTES: usize = 128 * 1024;
pub const MAX_STAGES: usize = 2;
pub const MAX_SNAPSHOTS: usize = 2;
pub const IDLE_SECONDS: u64 = 30;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub title: String,
    pub purpose: Purpose,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "decoding::document"
    )]
    pub document: Option<String>,
    #[serde(deserialize_with = "decoding::rows")]
    pub rows: Vec<Row>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "decoding::columns"
    )]
    pub columns: Vec<Column>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Block>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<Block>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Block>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "decoding::metadata"
    )]
    pub metadata: Option<Vec<Metadata>>,
    /// Negotiated `view-help`: the registered topic `Space ?` opens here.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::presentation::authored"
    )]
    pub help: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "decoding::presentations"
    )]
    pub action_presentation: Option<BTreeMap<String, super::presentation::Presentation>>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "decoding::actions"
    )]
    pub actions: Vec<String>,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    #[default]
    Document,
    List,
    Dashboard,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: String,
    pub text: String,
    pub role: Role,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "decoding::cells"
    )]
    pub cells: Vec<Cell>,
    /// Negotiated override: absent inherits the model, empty offers no actions.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "decoding::row_actions"
    )]
    pub actions: Option<Vec<String>>,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    Ordinary,
    Muted,
    Heading,
    Warning,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub id: String,
    pub label: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    pub text: String,
    pub role: Role,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub text: String,
    pub role: Role,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub label: String,
    pub value: String,
}

/// A patch can replace the complete header without retransmitting stable rows.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub title: String,
    pub purpose: Purpose,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "decoding::document"
    )]
    pub document: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "decoding::columns"
    )]
    pub columns: Vec<Column>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Block>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<Block>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Block>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "decoding::metadata"
    )]
    pub metadata: Option<Vec<Metadata>>,
    /// Negotiated `view-help`: the registered topic `Space ?` opens here.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::presentation::authored"
    )]
    pub help: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "decoding::presentations"
    )]
    pub action_presentation: Option<BTreeMap<String, super::presentation::Presentation>>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "decoding::actions"
    )]
    pub actions: Vec<String>,
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidArgument, message)
}
fn limited(message: &str) -> Error {
    Error::new(ErrorCode::LimitExceeded, message)
}
fn safe(value: &str, bytes: usize) -> bool {
    !value.is_empty() && value.len() <= bytes && !value.chars().any(char::is_control)
}
fn cancelled(flag: &std::sync::atomic::AtomicBool) -> Result<(), Error> {
    if flag.load(std::sync::atomic::Ordering::Acquire) {
        Err(Error::new(
            ErrorCode::Cancelled,
            "View preparation was cancelled",
        ))
    } else {
        Ok(())
    }
}

impl Model {
    /// Effective restriction for row indices resolved from the retained projection.
    /// None preserves the legacy unrestricted model; multiple rows intersect.
    pub fn selected_actions(&self, selected: &[usize]) -> Option<BTreeSet<&str>> {
        let inherited = || {
            (!self.actions.is_empty()).then(|| self.actions.iter().map(String::as_str).collect())
        };
        if selected.is_empty() {
            return inherited();
        }
        let mut allowed: Option<BTreeSet<&str>> = None;
        for index in selected {
            let Some(row) = self.rows.get(*index) else {
                return Some(BTreeSet::new());
            };
            let restriction = row.actions.as_ref().map_or_else(inherited, |actions| {
                Some(actions.iter().map(String::as_str).collect())
            });
            if let Some(restriction) = restriction {
                if let Some(allowed) = &mut allowed {
                    allowed.retain(|action| restriction.contains(action));
                } else {
                    allowed = Some(restriction);
                }
            }
        }
        allowed
    }

    /// Bounded retained storage estimate, including container allocations.
    pub fn payload_bytes(&self) -> usize {
        self.title.capacity()
            + self.document.as_ref().map_or(0, String::capacity)
            + self.help.as_ref().map_or(0, String::capacity)
            + std::mem::size_of::<Self>()
            + self.rows.capacity() * std::mem::size_of::<Row>()
            + self
                .rows
                .iter()
                .map(|row| {
                    row.id.capacity()
                        + row.text.capacity()
                        + row.actions.as_ref().map_or(0, |actions| {
                            actions.capacity() * std::mem::size_of::<String>()
                                + actions.iter().map(String::capacity).sum::<usize>()
                        })
                        + row.cells.capacity() * std::mem::size_of::<Cell>()
                        + row
                            .cells
                            .iter()
                            .map(|cell| cell.text.capacity())
                            .sum::<usize>()
                })
                .sum::<usize>()
            + self.columns.capacity() * std::mem::size_of::<Column>()
            + self
                .columns
                .iter()
                .map(|column| column.id.capacity() + column.label.capacity())
                .sum::<usize>()
            + self.metadata.as_ref().map_or(0, |entries| {
                entries.capacity() * std::mem::size_of::<Metadata>()
                    + entries
                        .iter()
                        .map(|entry| entry.label.capacity() + entry.value.capacity())
                        .sum::<usize>()
            })
            + self.action_presentation.as_ref().map_or(0, |entries| {
                entries
                    .iter()
                    .map(|(name, presentation)| {
                        name.capacity() + presentation.payload_bytes() + 128
                    })
                    .sum::<usize>()
            })
            + self.actions.capacity() * std::mem::size_of::<String>()
            + self.actions.iter().map(String::capacity).sum::<usize>()
            + [&self.detail, &self.preview, &self.status]
                .into_iter()
                .flatten()
                .map(|block| block.text.capacity())
                .sum::<usize>()
    }

    pub fn encoded_limit(&self) -> usize {
        if self.document.is_some() {
            MAX_DOCUMENT_MODEL_BYTES
        } else {
            MAX_MODEL_BYTES
        }
    }

    pub fn retained_limit(&self) -> usize {
        if self.document.is_some() {
            MAX_DOCUMENT_RETAINED_BYTES
        } else {
            MAX_RETAINED_BYTES
        }
    }

    pub fn validate(&self) -> Result<(), Error> {
        self.checked_json().map(|_| ())
    }

    fn checked_json(&self) -> Result<String, Error> {
        if !safe(&self.title, 160) {
            return Err(invalid("Invalid view title"));
        }
        if self.rows.len() > MAX_ROWS
            || self.columns.len() > MAX_COLUMNS
            || self.actions.len() > super::application::MAX_COMMANDS
        {
            return Err(limited("View row, column or action limit exceeded"));
        }
        if self.payload_bytes() > self.retained_limit() {
            return Err(limited("View storage limit exceeded"));
        }
        if let Some(document) = &self.document {
            if self.purpose != Purpose::Document
                || !self.rows.is_empty()
                || !self.columns.is_empty()
                || self.detail.is_some()
                || self.preview.is_some()
            {
                return Err(invalid(
                    "Document views require document purpose and no rows, columns, detail or preview",
                ));
            }
            if document.len() > MAX_DOCUMENT_BYTES
                || document.split('\n').count() > MAX_DOCUMENT_LINES
            {
                return Err(limited("View document limit exceeded"));
            }
            if document
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                return Err(invalid("Invalid view document text"));
            }
        }
        if let Some(metadata) = &self.metadata {
            if metadata.len() > 16 {
                return Err(limited("View metadata limit exceeded"));
            }
            for entry in metadata {
                if !safe(&entry.label, 64)
                    || entry.value.len() > 1024
                    || entry.value.chars().any(char::is_control)
                {
                    return Err(invalid("Invalid view metadata"));
                }
            }
        }
        if self
            .help
            .as_deref()
            .is_some_and(|topic| !super::valid_name(topic))
        {
            return Err(invalid("Invalid view help topic"));
        }
        if let Some(presentations) = &self.action_presentation {
            if presentations.len() > super::application::MAX_COMMANDS {
                return Err(limited("View action presentation limit exceeded"));
            }
            let mut groups = BTreeSet::new();
            for (name, presentation) in presentations {
                if !super::valid_name(name) {
                    return Err(invalid("Invalid view action presentation command"));
                }
                presentation.validate()?;
                if let Some(group) = &presentation.group {
                    groups.insert(group);
                }
            }
            if groups.len() > 16 {
                return Err(limited("View action group limit exceeded"));
            }
        }
        let mut columns = BTreeSet::new();
        for column in &self.columns {
            if !safe(&column.id, 64) || !safe(&column.label, 160) || !columns.insert(&column.id) {
                return Err(invalid("Invalid or duplicate view column"));
            }
        }
        let mut actions = BTreeSet::new();
        if self
            .actions
            .iter()
            .any(|action| !super::valid_name(action) || !actions.insert(action))
        {
            return Err(invalid("Invalid or duplicate view action"));
        }
        for block in [&self.detail, &self.preview].into_iter().flatten() {
            if block.text.len() > MAX_BLOCK_BYTES
                || block.text.split('\n').count() > MAX_BLOCK_LINES
            {
                return Err(limited("View detail or preview limit exceeded"));
            }
            if block.text.chars().any(|c| c.is_control() && c != '\n') {
                return Err(invalid("Invalid view detail or preview text"));
            }
        }
        if let Some(status) = &self.status {
            if status.text.len() > 1024 {
                return Err(limited("View status limit exceeded"));
            }
            if status.text.chars().any(char::is_control) {
                return Err(invalid("Invalid view status text"));
            }
        }
        let mut ids = BTreeSet::new();
        let mut bytes = self.title.len();
        for row in &self.rows {
            if let Some(actions) = &row.actions {
                if actions.len() > super::application::MAX_COMMANDS {
                    return Err(limited("View row action limit exceeded"));
                }
                let mut unique = BTreeSet::new();
                if actions
                    .iter()
                    .any(|action| !super::valid_name(action) || !unique.insert(action))
                {
                    return Err(invalid("Invalid or duplicate view row action"));
                }
            }
            if !safe(&row.id, 64)
                || !ids.insert(&row.id)
                || row.text.chars().any(char::is_control)
                || row
                    .cells
                    .iter()
                    .any(|cell| cell.text.chars().any(char::is_control))
                || (self.columns.is_empty() && !row.cells.is_empty())
                || (!self.columns.is_empty()
                    && (row.cells.len() != self.columns.len() || !row.text.is_empty()))
            {
                return Err(invalid("Invalid or duplicate view row"));
            }
            bytes += row.id.len()
                + row.text.len()
                + row.cells.iter().map(|cell| cell.text.len()).sum::<usize>()
                + 1;
            if bytes > MAX_MODEL_BYTES {
                return Err(limited("View payload limit exceeded"));
            }
        }
        let encoded = serde_json::to_string(self).map_err(|_| invalid("View encoding failed"))?;
        if encoded.len() > self.encoded_limit() {
            return Err(limited("Encoded view model exceeds limit"));
        }
        Ok(encoded)
    }
}

pub(crate) struct View {
    pub buffer: usize,
    pub model: Arc<Model>,
    pub encoded: Arc<str>,
    pub projection: Arc<Projection>,
    pub charge: usize,
    pub revision: u64,
    pub published: Option<std::time::Instant>,
    pub query: Option<QueryState>,
    pub accepted_actions: u64,
}

#[cfg(test)]
#[path = "tests/view_models.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/view_queries.rs"]
mod query_tests;
