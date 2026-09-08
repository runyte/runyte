// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};

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
