// SPDX-License-Identifier: MPL-2.0

//! Local application operations. Wire values are independent of filesystem-plan values.

use super::application::{Error, ErrorCode};
use crate::fs_plan::{DesiredEntry, DirectorySnapshot, EntryKind, FsPlan};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

pub const MAX_ENTRIES: usize = 1024;
pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
pub const DIRECTORY_CHARGE: usize = 8 * 1024 * 1024;
pub const MAX_DIRECTORIES: usize = 2;
pub const MAX_PLANS: usize = 2;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Intent {
    CreateFile { destination: String },
    CreateDirectory { destination: String },
    Rename { entry: String, destination: String },
    Copy { entry: String, destination: String },
    Trash { entry: String },
}

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub entry: String,
    pub name: String,
    pub kind: &'static str,
    pub bytes: u64,
}

#[derive(Clone, Debug)]
pub struct Directory {
    pub(crate) path: PathBuf,
    pub(crate) snapshot: DirectorySnapshot,
    pub(crate) revision: String,
}

impl Directory {
    pub fn entry_id(&self, name: &Path) -> String {
        format!(
            "n:{}",
            &crate::hash::sha256_hex(name.to_string_lossy().as_bytes())[..32]
        )
    }

    pub fn page(&self, offset: usize, limit: usize) -> Vec<Entry> {
        self.snapshot
            .entries()
            .iter()
            .skip(offset)
            .take(limit)
            .map(|entry| Entry {
                entry: self.entry_id(&entry.path),
                name: entry.path.to_string_lossy().into_owned(),
                kind: match entry.kind {
                    EntryKind::File => "file",
                    EntryKind::Directory => "directory",
                    EntryKind::Symlink => "symlink",
                    EntryKind::Other => "other",
                },
                bytes: entry.source_fingerprint().detail_fields().len,
            })
            .collect()
    }
}

#[derive(Debug)]
pub(crate) enum Task {
    Create {
        root: PathBuf,
        path: String,
        text: String,
    },
    List {
        root: PathBuf,
        path: String,
    },
    Prepare {
        root: PathBuf,
        directory: Directory,
        intent: Intent,
    },
    Open {
        root: PathBuf,
        path: String,
    },
}

#[derive(Debug)]
pub enum Prepared {
    Directory(Directory),
    Plan(FsPlan),
    Document(Box<crate::buffer::Buffer>),
}

pub(crate) fn project_path(root: &Path, path: &str) -> Result<PathBuf, Error> {
    if path.len() > 4096
        || path.chars().any(char::is_control)
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(Error::new(
            ErrorCode::InvalidArgument,
            "Expected a relative workspace path without parent components",
        ));
    }
    let candidate = root.join(path);
    crate::path_safety::ensure_within_root(root, &candidate).map_err(|_| {
        Error::new(
            ErrorCode::InvalidArgument,
            "Path resolves outside the workspace",
        )
    })?;
    crate::path_safety::canonicalize_existing_prefix(&candidate)
        .map_err(|_| Error::new(ErrorCode::InvalidArgument, "Path could not be resolved"))
}

impl Task {
    pub fn run(self) -> Result<Prepared, Error> {
        let io = |error: anyhow::Error| {
            if error.is::<crate::buffer::ReadLimitExceeded>()
                || error.is::<crate::fs_plan::DirectoryLimitExceeded>()
            {
                Error::new(
                    ErrorCode::LimitExceeded,
                    "Local filesystem input exceeds the application limit",
                )
            } else if error.is::<crate::buffer::BinaryFileError>() {
                Error::new(
                    ErrorCode::Unsupported,
                    "Binary files require an external application",
                )
            } else {
                Error::new(
                    ErrorCode::Conflict,
                    "Local filesystem operation could not be prepared; refresh and retry",
                )
            }
        };
        match self {
            Self::Create { root, path, text } => {
                let path = project_path(&root, &path)?;
                if std::fs::symlink_metadata(&path).is_ok() {
                    return Err(Error::new(
                        ErrorCode::Conflict,
                        "Document path already exists",
                    ));
                }
                let buffer = crate::buffer::Buffer::unsaved_document(path, text);
                Ok(Prepared::Document(Box::new(buffer)))
            }
            Self::List { root, path } => {
                let path = project_path(&root, &path)?
                    .canonicalize()
                    .map_err(|_| Error::new(ErrorCode::NotFound, "Directory does not exist"))?;
                let snapshot =
                    DirectorySnapshot::read_bounded(&path, true, MAX_ENTRIES).map_err(io)?;
                let revision = format!(
                    "d:{}",
                    &crate::hash::sha256_hex(format!("{snapshot:?}").as_bytes())[..32]
                );
                Ok(Prepared::Directory(Directory {
                    path,
                    snapshot,
                    revision,
                }))
            }
            Self::Open { root, path } => {
                let path = project_path(&root, &path)?;
                let path = path
                    .canonicalize()
                    .map_err(|_| Error::new(ErrorCode::NotFound, "Document does not exist"))?;
                crate::path_safety::ensure_within_root(&root, &path).map_err(io)?;
                let buffer =
                    crate::buffer::Buffer::open_bounded(&path, MAX_DOCUMENT_BYTES).map_err(io)?;
                Ok(Prepared::Document(Box::new(buffer)))
            }
            Self::Prepare {
                root,
                directory,
                intent,
            } => {
                crate::path_safety::ensure_within_root(&root, &directory.path).map_err(io)?;
                let source = match &intent {
                    Intent::Rename { entry, .. }
                    | Intent::Copy { entry, .. }
                    | Intent::Trash { entry } => Some(
                        directory
                            .snapshot
                            .entries()
                            .iter()
                            .find(|e| directory.entry_id(&e.path) == *entry)
                            .ok_or_else(|| {
                                Error::new(ErrorCode::NotFound, "Unknown directory entry")
                            })?,
                    ),
                    _ => None,
                };
                // This adapter initially admits regular files. Recursive directory
                // operations need the subsequent asynchronous mutation/job adapter.
                if let Some(source) = source {
                    if source.kind != EntryKind::File {
                        return Err(Error::new(
                            ErrorCode::Unsupported,
                            "This application adapter mutates regular files only",
                        ));
                    }
                    if source.source_fingerprint().detail_fields().len > MAX_DOCUMENT_BYTES as u64 {
                        return Err(Error::new(
                            ErrorCode::LimitExceeded,
                            "File exceeds application operation limit",
                        ));
                    }
                }
                let destination = match &intent {
                    Intent::CreateFile { destination }
                    | Intent::CreateDirectory { destination }
                    | Intent::Rename { destination, .. }
                    | Intent::Copy { destination, .. } => Some(
                        crate::fs_plan::relative_from_root(
                            &directory.path,
                            &project_path(&root, destination)?,
                        )
                        .map_err(io)?,
                    ),
                    _ => None,
                };
                let mut desired = directory
                    .snapshot
                    .entries()
                    .iter()
                    .filter(|entry| {
                        !source.is_some_and(|s| {
                            s.id == entry.id
                                && matches!(intent, Intent::Rename { .. } | Intent::Trash { .. })
                        })
                    })
                    .map(|entry| DesiredEntry::existing(entry, entry.path.clone()))
                    .collect::<Vec<_>>();
                match intent {
                    Intent::CreateFile { .. } => {
                        desired.push(DesiredEntry::create(destination.unwrap(), EntryKind::File))
                    }
                    Intent::CreateDirectory { .. } => desired.push(DesiredEntry::create(
                        destination.unwrap(),
                        EntryKind::Directory,
                    )),
                    Intent::Rename { .. } => desired.push(DesiredEntry::existing(
                        source.unwrap(),
                        destination.unwrap(),
                    )),
                    Intent::Copy { .. } => desired.push(DesiredEntry::existing(
                        source.unwrap(),
                        destination.unwrap(),
                    )),
                    Intent::Trash { .. } => {}
                }
                let plan =
                    FsPlan::build(directory.path, directory.snapshot, desired).map_err(io)?;
                for operation in plan.operations() {
                    use crate::fs_plan::FsOperation;
                    let paths = match operation {
                        FsOperation::Create { path, .. } | FsOperation::Delete { path, .. } => {
                            vec![path]
                        }
                        FsOperation::Rename { from, to, .. }
                        | FsOperation::Move { from, to, .. }
                        | FsOperation::Copy { from, to, .. } => vec![from, to],
                    };
                    for path in paths {
                        crate::path_safety::ensure_within_root(&root, &plan.root().join(path))
                            .map_err(io)?;
                    }
                }
                Ok(Prepared::Plan(plan))
            }
        }
    }
}

pub(crate) struct Pending {
    pub creating: bool,
    pub invocation: Option<String>,
    pub offset: usize,
    pub limit: usize,
    pub charge: usize,
    pub expected_revision: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finished {
    pub plan: String,
    pub state: &'static str,
    pub applied: usize,
    pub recovery: bool,
}
