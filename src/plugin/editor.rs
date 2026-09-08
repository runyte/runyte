// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};

pub const MAX_CHUNK_BYTES: usize = 256 * 1024;
pub const MAX_CHANGES: usize = 1024;
pub const MAX_REPLACEMENT_BYTES: usize = 512 * 1024;
pub const MAX_SNAPSHOTS: usize = 2;
pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
pub const SNAPSHOT_IDLE_SECONDS: u64 = 30;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub from: usize,
    pub to: usize,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub anchor: usize,
    pub head: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct Buffer {
    pub buffer: String,
    pub revision: String,
    pub name: String,
    pub chars: usize,
    pub read_only: bool,
    pub dirty: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Pane {
    pub pane: String,
    pub buffer: Option<String>,
    pub selection_revision: String,
}

pub(crate) struct Snapshot {
    pub buffer: usize,
    pub text: crate::text::Text,
    pub revision: String,
}
