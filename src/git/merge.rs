// SPDX-License-Identifier: MPL-2.0

//! Reviewed merge and resolution values. Index stages, rather than marker labels,
//! identify the sides. Plans are intentionally opaque: only their provider can
//! construct a mutation authority from a repository observation.

use super::{BufferRevisionGuard, Repository, RepositoryFingerprint};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepositoryOperation {
    Idle,
    Merge { parents: Vec<String> },
    Rebase,
    CherryPick,
    Revert,
    Bisect,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictStage {
    pub mode: String,
    pub oid: String,
    pub stage: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictEntry {
    /// Repository-relative path; never a marker label or a lossy display name.
    pub path: PathBuf,
    pub marker_width: usize,
    pub base: Option<ConflictStage>,
    pub current: Option<ConflictStage>,
    pub other: Option<ConflictStage>,
}

impl ConflictEntry {
    pub fn kind(&self) -> &'static str {
        let stages = [&self.base, &self.current, &self.other];
        if stages
            .into_iter()
            .flatten()
            .any(|stage| stage.mode == "160000")
        {
            return "submodule";
        }
        if stages
            .into_iter()
            .flatten()
            .any(|stage| stage.mode == "120000")
        {
            return "symlink";
        }
        match (&self.base, &self.current, &self.other) {
            (None, Some(_), Some(_)) => "add/add",
            (Some(_), Some(_), None) | (Some(_), None, Some(_)) => "modify/delete or structural",
            (Some(_), Some(a), Some(b)) if a.mode != b.mode => "mode/type",
            (Some(_), Some(_), Some(_)) => "content",
            _ => "structural",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictInventory {
    pub operation: RepositoryOperation,
    pub head_oid: Option<String>,
    pub entries: Vec<ConflictEntry>,
    pub index_identity: String,
    pub current_identity: String,
    pub other_identity: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeConflictMessage {
    pub paths: Vec<PathBuf>,
    pub kind: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MergePreviewOutcome {
    AlreadyContained,
    FastForward,
    Merge {
        has_conflicts: bool,
        tree_oid: String,
        conflicts: Vec<ConflictEntry>,
        messages: Vec<MergeConflictMessage>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergePlan {
    pub(crate) repository: Repository,
    pub destination_reference: String,
    pub destination_oid: String,
    pub source_reference: String,
    pub source_oid: String,
    pub outcome: MergePreviewOutcome,
    pub comparison: super::RevisionComparison,
    pub(crate) fingerprint: RepositoryFingerprint,
    pub(crate) disk_identity: String,
    pub(crate) settings_identity: String,
    pub(crate) guard: BufferRevisionGuard,
}

impl MergePlan {
    pub fn invalidate(&self) {
        self.guard.invalidate();
    }
    pub fn is_valid(&self) -> bool {
        self.guard.is_valid()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeApplied {
    AlreadyContained,
    FastForward,
    PendingCommit,
    Conflicted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeApplyResult {
    pub diagnostic: Option<String>,
    pub outcome: MergeApplied,
    pub inventory: ConflictInventory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionChoice {
    SavedFile { allow_literal_markers: bool },
    Current,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionPlan {
    pub(crate) repository: Repository,
    pub path: PathBuf,
    pub choice: ResolutionChoice,
    pub entry: ConflictEntry,
    pub(crate) inventory: ConflictInventory,
    pub(crate) disk_identity: String,
    pub(crate) guard: BufferRevisionGuard,
}
impl ResolutionPlan {
    pub fn invalidate(&self) {
        self.guard.invalidate();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeCompletionPlan {
    pub(crate) repository: Repository,
    pub inventory: ConflictInventory,
    pub message: String,
    pub staged_diff: String,
    pub(crate) disk_identity: String,
    pub(crate) markers_identity: String,
    pub(crate) settings_identity: String,
    pub(crate) guard: BufferRevisionGuard,
}
impl MergeCompletionPlan {
    pub fn invalidate(&self) {
        self.guard.invalidate();
    }
}

// Mutation identity uses the provider-issued guard, retaining bounded hashing
// even when the reviewed comparison contains thousands of files.
impl std::hash::Hash for MergePlan {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.guard, state);
    }
}
impl std::hash::Hash for ResolutionPlan {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.guard, state);
    }
}
impl std::hash::Hash for MergeCompletionPlan {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.guard, state);
    }
}
