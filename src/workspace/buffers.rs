// SPDX-License-Identifier: MPL-2.0

//! Stable host-lifetime buffer and wait-request identities.

use std::{fmt, path::PathBuf};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BufferId(u64);

impl BufferId {
    pub(crate) fn from_index(index: usize) -> Self {
        Self(index as u64 + 1)
    }

    pub(crate) const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub(crate) fn index(self) -> Option<usize> {
        usize::try_from(self.0).ok()?.checked_sub(1)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for BufferId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BufferRevision(u64);

impl BufferRevision {
    pub(crate) const fn from_raw(revision: u64) -> Self {
        Self(revision)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BufferMetadata {
    pub id: BufferId,
    pub revision: BufferRevision,
    pub path: Option<PathBuf>,
    pub name: String,
    pub dirty: bool,
    pub read_only: bool,
    pub closed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BufferContents {
    pub metadata: BufferMetadata,
    pub text: String,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WaitToken(u64);

impl WaitToken {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for WaitToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WaitStatus {
    Pending {
        buffers: Vec<BufferId>,
        remaining: Vec<BufferId>,
    },
    Completed,
    Cancelled {
        reason: String,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct WaitRequest {
    pub buffers: Vec<BufferId>,
    pub completed: Vec<BufferId>,
    pub status: WaitStatus,
}
