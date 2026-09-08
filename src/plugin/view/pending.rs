// SPDX-License-Identifier: MPL-2.0

use super::{PreparedModel, ReadSnapshot};
use std::sync::{Arc, atomic::AtomicBool};

pub(crate) struct Pending {
    pub view: String,
    pub creating: bool,
    pub buffer: Option<usize>,
    pub revision: u64,
    pub buffer_revision: u64,
    pub charge: usize,
    pub source_charge: usize,
    pub stage: Option<String>,
    pub cancelled: Arc<AtomicBool>,
}

#[derive(Debug)]
pub struct Prepared {
    pub(crate) model: PreparedModel,
    pub(crate) text: crate::buffer::PreparedPluginProjection,
    pub(crate) spans: Vec<crate::syntax::Span>,
    pub(crate) remap: Vec<usize>,
}

pub(crate) struct OwnedSnapshot {
    pub view: String,
    pub snapshot: ReadSnapshot,
}
