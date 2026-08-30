// SPDX-License-Identifier: MPL-2.0

//! Explicit filesystem plans for editable directory buffers.
//!
//! Planning and application are deliberately separate. Building a plan only
//! compares an edited projection with the snapshot captured when the directory
//! was opened. Applying it first verifies that snapshot, then performs the
//! already-confirmed operations in a collision-safe order.

use std::{
    cmp::Reverse,
    collections::HashSet,
    fmt,
    fs::{self, OpenOptions},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result, anyhow, bail, ensure};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EntryId(u64);

impl EntryId {
    pub fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

impl EntryKind {
    pub fn marker(self) -> &'static str {
        match self {
            Self::Directory => "/",
            Self::File | Self::Symlink | Self::Other => "",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EntryFingerprint {
    kind: EntryKind,
    len: u64,
    modified_nanos: Option<u128>,
    symlink_target: Option<PathBuf>,
    readonly: bool,
    unix: Option<UnixFingerprint>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UnixFingerprint {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFingerprint {
    entry: EntryFingerprint,
    descendants: Vec<(PathBuf, EntryFingerprint)>,
}

impl SourceFingerprint {
    pub fn capture(path: &Path) -> Result<Self> {
        let entry = EntryFingerprint::capture(path, "transfer source")?;
        let mut descendants = Vec::new();
        if entry.kind == EntryKind::Directory {
            capture_descendants(path, Path::new(""), &mut descendants)?;
        }
        Ok(Self { entry, descendants })
    }

    fn shallow(path: &Path, purpose: &str) -> Result<Self> {
        Ok(Self {
            entry: EntryFingerprint::capture(path, purpose)?,
            descendants: Vec::new(),
        })
    }

    pub fn kind(&self) -> EntryKind {
        self.entry.kind
    }

    /// The path this entry links to, exactly as the link stores it.
    pub fn symlink_target(&self) -> Option<&Path> {
        self.entry.symlink_target.as_deref()
    }
}

impl EntryFingerprint {
    fn capture(path: &Path, purpose: &str) -> Result<Self> {
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("failed to inspect {purpose} {}", path.display()))?;
        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            EntryKind::Symlink
        } else if file_type.is_dir() {
            EntryKind::Directory
        } else if file_type.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        let modified_nanos = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos());
        let symlink_target = (kind == EntryKind::Symlink)
            .then(|| fs::read_link(path))
            .transpose()
            .with_context(|| format!("failed to read link {}", path.display()))?;
        Ok(Self {
            kind,
            len: metadata.len(),
            modified_nanos,
            symlink_target,
            readonly: metadata.permissions().readonly(),
            unix: unix_fingerprint(&metadata),
        })
    }
}

#[cfg(unix)]
fn unix_fingerprint(metadata: &fs::Metadata) -> Option<UnixFingerprint> {
    use std::os::unix::fs::MetadataExt;

    Some(UnixFingerprint {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        uid: metadata.uid(),
        gid: metadata.gid(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    })
}

#[cfg(not(unix))]
fn unix_fingerprint(_metadata: &fs::Metadata) -> Option<UnixFingerprint> {
    None
}

fn capture_descendants(
    root: &Path,
    relative: &Path,
    captured: &mut Vec<(PathBuf, EntryFingerprint)>,
) -> Result<()> {
    let directory = root.join(relative);
    let mut children = fs::read_dir(&directory)
        .with_context(|| format!("failed to inspect directory source {}", directory.display()))?
        .map(|entry| {
            entry
                .map(|entry| (entry.file_name(), entry.path()))
                .with_context(|| format!("failed to inspect entry in {}", directory.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    children.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, path) in children {
        let child = relative.join(name);
        let fingerprint = EntryFingerprint::capture(&path, "directory source entry")?;
        let recurse = fingerprint.kind == EntryKind::Directory;
        captured.push((child.clone(), fingerprint));
        if recurse {
            capture_descendants(root, &child, captured)?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotEntry {
    pub id: EntryId,
    pub path: PathBuf,
    pub kind: EntryKind,
    fingerprint: EntryFingerprint,
}

impl SnapshotEntry {
    fn from_path(id: EntryId, root: &Path, path: PathBuf) -> Result<Self> {
        let absolute = root.join(&path);
        let fingerprint = EntryFingerprint::capture(&absolute, "directory entry")?;
        Ok(Self {
            id,
            path,
            kind: fingerprint.kind,
            fingerprint,
        })
    }

    pub fn display_name(&self) -> Result<String> {
        let name = self
            .path
            .to_str()
            .with_context(|| format!("{} is not valid UTF-8", self.path.display()))?;
        Ok(format!("{name}{}", self.kind.marker()))
    }

    /// The path this entry links to, exactly as the link stores it.
    pub fn symlink_target(&self) -> Option<&Path> {
        self.fingerprint.symlink_target.as_deref()
    }

    pub fn source_fingerprint(&self) -> SourceFingerprint {
        SourceFingerprint {
            entry: self.fingerprint.clone(),
            descendants: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectorySnapshot {
    entries: Vec<SnapshotEntry>,
    show_hidden: bool,
}

impl DirectorySnapshot {
    /// Reads every entry, including dotfiles.
    pub fn read(root: &Path) -> Result<Self> {
        Self::read_with(root, true)
    }

    /// Reads the entries a listing covers.
    ///
    /// Dotfiles are left out unless `show_hidden`, and the choice is kept on
    /// the snapshot so a later re-read compares the same set of entries. A
    /// plan built from a listing without them is therefore about the entries
    /// it showed: an unlisted dotfile is neither deleted for being absent nor
    /// reported as a change when it appears.
    pub fn read_with(root: &Path, show_hidden: bool) -> Result<Self> {
        ensure!(root.is_dir(), "{} is not a directory", root.display());
        let mut paths = fs::read_dir(root)
            .with_context(|| format!("failed to read directory {}", root.display()))?
            .filter_map(|entry| {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        return Some(Err(error).with_context(|| {
                            format!("failed to read entry in {}", root.display())
                        }));
                    }
                };
                let name = entry.file_name();
                let Some(text) = name.to_str() else {
                    return Some(Err(anyhow::anyhow!(
                        "{} contains a non-UTF-8 entry",
                        root.display()
                    )));
                };
                (show_hidden || !text.starts_with('.')).then(|| {
                    ensure!(
                        !text.chars().any(char::is_control),
                        "{} contains a filename with control characters that the editable directory explorer cannot represent",
                        root.display()
                    );
                    ensure!(
                        !text.chars().last().is_some_and(char::is_whitespace),
                        "{} contains a filename ending in whitespace that the editable directory explorer cannot represent",
                        root.display()
                    );
                    Ok(PathBuf::from(&name))
                })
            })
            .collect::<Result<Vec<_>>>()?;
        paths.sort();
        let entries = paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                SnapshotEntry::from_path(EntryId::new(index as u64 + 1), root, path)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            entries,
            show_hidden,
        })
    }

    /// Whether this snapshot covers dotfiles.
    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    pub fn entries(&self) -> &[SnapshotEntry] {
        &self.entries
    }

    pub fn entry(&self, id: EntryId) -> Option<&SnapshotEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// Compares the directory's visible entries without treating changes
    /// inside child directories as a change to this directory.
    ///
    /// A child directory's length and modification time describe its own
    /// contents. Those values can change while the entry in this explorer is
    /// still the same directory, so sources used by the plan are checked
    /// separately before application.
    fn matches_current(&self, current: &Self) -> bool {
        self.entries.len() == current.entries.len()
            && self
                .entries
                .iter()
                .zip(&current.entries)
                .all(|(expected, current)| {
                    expected.id == current.id
                        && expected.path == current.path
                        && expected.kind == current.kind
                })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesiredEntry {
    pub id: Option<EntryId>,
    pub path: PathBuf,
    pub kind: EntryKind,
    transfer: Option<TransferSource>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferMode {
    Copy,
    Move,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TransferSource {
    path: PathBuf,
    mode: TransferMode,
    expected: SourceFingerprint,
}

impl DesiredEntry {
    pub fn existing(entry: &SnapshotEntry, path: impl Into<PathBuf>) -> Self {
        Self {
            id: Some(entry.id),
            path: path.into(),
            kind: entry.kind,
            transfer: None,
        }
    }

    pub fn identified(id: EntryId, path: impl Into<PathBuf>, kind: EntryKind) -> Self {
        Self {
            id: Some(id),
            path: path.into(),
            kind,
            transfer: None,
        }
    }

    pub fn create(path: impl Into<PathBuf>, kind: EntryKind) -> Self {
        Self {
            id: None,
            path: path.into(),
            kind,
            transfer: None,
        }
    }

    pub fn transfer(
        source: impl Into<PathBuf>,
        path: impl Into<PathBuf>,
        kind: EntryKind,
        mode: TransferMode,
        expected: SourceFingerprint,
    ) -> Self {
        Self {
            id: None,
            path: path.into(),
            kind,
            transfer: Some(TransferSource {
                path: source.into(),
                mode,
                expected,
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FsOperation {
    Create {
        path: PathBuf,
        kind: EntryKind,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
        kind: EntryKind,
    },
    Move {
        from: PathBuf,
        to: PathBuf,
        kind: EntryKind,
    },
    Copy {
        from: PathBuf,
        to: PathBuf,
        kind: EntryKind,
    },
    Delete {
        path: PathBuf,
        kind: EntryKind,
    },
}

impl FsOperation {
    pub fn description(&self) -> String {
        match self {
            Self::Create { path, kind } => {
                format!("create {}{}", path.display(), kind.marker())
            }
            Self::Rename { from, to, kind } => {
                format!(
                    "rename {}{} → {}{}",
                    from.display(),
                    kind.marker(),
                    to.display(),
                    kind.marker()
                )
            }
            Self::Move { from, to, kind } => {
                format!(
                    "move {}{} → {}{}",
                    from.display(),
                    kind.marker(),
                    to.display(),
                    kind.marker()
                )
            }
            Self::Copy { from, to, kind } => {
                format!(
                    "copy {}{} → {}{}",
                    from.display(),
                    kind.marker(),
                    to.display(),
                    kind.marker()
                )
            }
            Self::Delete { path, kind } => {
                format!("delete {}{}", path.display(), kind.marker())
            }
        }
    }

    fn staged_source(&self) -> Option<&Path> {
        match self {
            Self::Rename { from, .. } | Self::Move { from, .. } => Some(from),
            Self::Create { .. } | Self::Copy { .. } | Self::Delete { .. } => None,
        }
    }

    fn target(&self) -> Option<&Path> {
        match self {
            Self::Create { path, .. } => Some(path),
            Self::Rename { to, .. } | Self::Move { to, .. } | Self::Copy { to, .. } => Some(to),
            Self::Delete { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeletionMode {
    Trash,
    Permanent,
}

/// Boundary that moves a confirmed deletion into a recoverable trash.
///
/// The live editor uses [`SystemTrash`]. Tests and embedders may provide an
/// isolated implementation without touching the person's platform trash.
pub trait TrashBackend: Send + Sync {
    fn delete(&self, path: &Path) -> Result<()>;
}

/// Platform-native trash used by the live editor.
pub struct SystemTrash;

impl TrashBackend for SystemTrash {
    fn delete(&self, path: &Path) -> Result<()> {
        trash::delete(path).map_err(anyhow::Error::from)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApplyReport {
    pub applied: Vec<FsOperation>,
}

impl ApplyReport {
    pub fn summary(&self) -> String {
        match self.applied.as_slice() {
            [] => "no operations applied".to_owned(),
            operations => operations
                .iter()
                .map(FsOperation::description)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

#[derive(Debug)]
pub struct ApplyError {
    pub report: ApplyReport,
    pub failed: Option<FsOperation>,
    message: String,
}

impl ApplyError {
    fn new(report: ApplyReport, failed: Option<FsOperation>, error: impl fmt::Display) -> Self {
        Self {
            report,
            failed,
            message: error.to_string(),
        }
    }
}

impl fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(operation) = &self.failed {
            write!(
                formatter,
                "filesystem plan stopped at {}: {} · applied: {}",
                operation.description(),
                self.message,
                self.report.summary()
            )
        } else {
            write!(
                formatter,
                "{} · applied: {}",
                self.message,
                self.report.summary()
            )
        }
    }
}

impl std::error::Error for ApplyError {}

#[derive(Clone, Debug)]
pub struct FsPlan {
    root: PathBuf,
    expected: DirectorySnapshot,
    operations: Vec<FsOperation>,
    transfer_sources: Vec<(PathBuf, SourceFingerprint)>,
    confirmed_sources: Vec<(PathBuf, Option<SourceFingerprint>)>,
}

impl FsPlan {
    pub fn build(
        root: PathBuf,
        expected: DirectorySnapshot,
        desired: Vec<DesiredEntry>,
    ) -> Result<Self> {
        ensure!(root.is_absolute(), "directory plan root must be absolute");
        let root = fs::canonicalize(&root)
            .with_context(|| format!("failed to resolve directory plan root {}", root.display()))?;
        let desired = desired
            .into_iter()
            .map(|mut entry| {
                entry.path = normalize_desired_path(&root, &entry.path)?;
                if let Some(transfer) = entry.transfer.as_mut() {
                    ensure!(
                        transfer.path.is_absolute(),
                        "directory transfer source must be absolute"
                    );
                    transfer.path = relative_from_root(&root, &lexical_normalize(&transfer.path))?;
                }
                Ok(entry)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut paths = HashSet::new();
        for entry in &desired {
            ensure!(
                paths.insert(entry.path.clone()),
                "duplicate directory entry: {}",
                entry.path.display()
            );
            if let Some(id) = entry.id {
                ensure!(
                    expected.entry(id).is_some(),
                    "directory entry identity is stale"
                );
            } else {
                ensure!(
                    entry.transfer.is_some()
                        || matches!(entry.kind, EntryKind::File | EntryKind::Directory),
                    "new entries must be files or directories"
                );
                if let Some(transfer) = &entry.transfer {
                    ensure!(
                        transfer.expected.kind() == entry.kind,
                        "directory transfer source kind changed"
                    );
                }
            }
        }

        let transfer_sources = desired
            .iter()
            .filter_map(|entry| {
                entry
                    .transfer
                    .as_ref()
                    .map(|transfer| (transfer.path.clone(), transfer.expected.clone()))
            })
            .collect::<Vec<_>>();

        let mut creates = Vec::new();
        let mut changes = Vec::new();
        let mut copies = Vec::new();
        let mut deletes = Vec::new();

        for entry in desired.iter().filter(|entry| entry.id.is_none()) {
            let Some(transfer) = &entry.transfer else {
                creates.push(FsOperation::Create {
                    path: entry.path.clone(),
                    kind: entry.kind,
                });
                continue;
            };
            ensure!(
                transfer.path != entry.path,
                "cannot {} {} onto itself",
                match transfer.mode {
                    TransferMode::Copy => "copy",
                    TransferMode::Move => "move",
                },
                transfer.path.display()
            );
            let source = lexical_normalize(&root.join(&transfer.path));
            let target = lexical_normalize(&root.join(&entry.path));
            ensure!(
                entry.kind != EntryKind::Directory || !target.starts_with(&source),
                "cannot {} {} inside itself",
                match transfer.mode {
                    TransferMode::Copy => "copy",
                    TransferMode::Move => "move",
                },
                transfer.path.display()
            );
            match transfer.mode {
                TransferMode::Copy => copies.push(FsOperation::Copy {
                    from: transfer.path.clone(),
                    to: entry.path.clone(),
                    kind: entry.kind,
                }),
                TransferMode::Move => changes.push(FsOperation::Move {
                    from: transfer.path.clone(),
                    to: entry.path.clone(),
                    kind: entry.kind,
                }),
            }
        }
        for original in expected.entries() {
            let entries = desired
                .iter()
                .filter(|entry| entry.id == Some(original.id))
                .collect::<Vec<_>>();
            let Some(primary) = entries
                .iter()
                .copied()
                .find(|entry| entry.path == original.path)
                .or_else(|| entries.first().copied())
            else {
                deletes.push(FsOperation::Delete {
                    path: original.path.clone(),
                    kind: original.kind,
                });
                continue;
            };
            if original.path != primary.path {
                ensure!(
                    original.kind != EntryKind::Directory
                        || !primary.path.starts_with(&original.path),
                    "cannot move {} inside itself",
                    original.path.display()
                );
                let operation = if original.path.parent() == primary.path.parent() {
                    FsOperation::Rename {
                        from: original.path.clone(),
                        to: primary.path.clone(),
                        kind: original.kind,
                    }
                } else {
                    FsOperation::Move {
                        from: original.path.clone(),
                        to: primary.path.clone(),
                        kind: original.kind,
                    }
                };
                changes.push(operation);
            }
            for duplicate in entries
                .into_iter()
                .filter(|entry| !std::ptr::eq(*entry, primary))
            {
                ensure!(
                    original.kind != EntryKind::Directory
                        || !duplicate.path.starts_with(&primary.path),
                    "cannot copy {} inside itself",
                    primary.path.display()
                );
                copies.push(FsOperation::Copy {
                    from: primary.path.clone(),
                    to: duplicate.path.clone(),
                    kind: original.kind,
                });
            }
        }

        for source in desired.iter().filter_map(|entry| {
            entry
                .transfer
                .as_ref()
                .filter(|transfer| transfer.mode == TransferMode::Copy)
                .map(|transfer| &transfer.path)
        }) {
            let destroys_source = changes.iter().chain(&deletes).any(|operation| {
                let vacated = match operation {
                    FsOperation::Rename { from, .. } | FsOperation::Move { from, .. } => from,
                    FsOperation::Delete { path, .. } => path,
                    FsOperation::Create { .. } | FsOperation::Copy { .. } => return false,
                };
                source.starts_with(vacated)
            });
            ensure!(
                !destroys_source,
                "copy source will be moved or deleted by the same plan: {}",
                source.display()
            );
        }

        creates.sort_by_key(operation_order);
        changes.sort_by_key(operation_order);
        copies.sort_by_key(operation_order);
        deletes.sort_by_key(|operation| Reverse(operation_order(operation)));
        creates.extend(changes);
        creates.extend(copies);
        creates.extend(deletes);

        let mut seen_sources = HashSet::new();
        let confirmed_sources = creates
            .iter()
            .filter_map(|operation| match operation {
                FsOperation::Rename { from, .. }
                | FsOperation::Move { from, .. }
                | FsOperation::Copy { from, .. } => Some(from),
                FsOperation::Delete { path, .. } => Some(path),
                FsOperation::Create { .. } => None,
            })
            .filter(|source| {
                expected
                    .entries()
                    .iter()
                    .any(|entry| &entry.path == *source)
            })
            .filter(|source| seen_sources.insert((*source).clone()))
            .map(|source| {
                (
                    source.clone(),
                    SourceFingerprint::capture(&root.join(source)).ok(),
                )
            })
            .collect::<Vec<_>>();

        Ok(Self {
            root,
            expected,
            operations: creates,
            transfer_sources,
            confirmed_sources,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn operations(&self) -> &[FsOperation] {
        &self.operations
    }

    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    pub fn lines(&self) -> Vec<String> {
        self.operations
            .iter()
            .map(FsOperation::description)
            .collect()
    }

    pub fn apply(&self, deletion: DeletionMode) -> Result<ApplyReport, ApplyError> {
        self.apply_with_trash(deletion, &SystemTrash)
    }

    pub fn apply_with_trash(
        &self,
        deletion: DeletionMode,
        trash: &dyn TrashBackend,
    ) -> Result<ApplyReport, ApplyError> {
        for (source, expected) in &self.transfer_sources {
            let absolute = self.root.join(source);
            let current = SourceFingerprint::capture(&absolute)
                .map_err(|error| ApplyError::new(ApplyReport::default(), None, error))?;
            if &current != expected {
                return Err(ApplyError::new(
                    ApplyReport::default(),
                    None,
                    format!(
                        "transfer source changed on disk; select it again before applying: {}",
                        absolute.display()
                    ),
                ));
            }
        }
        let current = DirectorySnapshot::read_with(&self.root, self.expected.show_hidden())
            .map_err(|error| ApplyError::new(ApplyReport::default(), None, error))?;
        if !self.expected.matches_current(&current)
            || !self.operation_sources_unchanged()
            || !self.confirmed_sources_unchanged()
        {
            return Err(ApplyError::new(
                ApplyReport::default(),
                None,
                "directory changed on disk; reopen it before applying",
            ));
        }
        self.preflight()
            .map_err(|error| ApplyError::new(ApplyReport::default(), None, error))?;

        let mut report = ApplyReport::default();
        let moves = self
            .operations
            .iter()
            .filter(|operation| operation.staged_source().is_some())
            .cloned()
            .collect::<Vec<_>>();
        let mut staged = Vec::new();
        for operation in &moves {
            let source = operation.staged_source().expect("move source");
            let temporary = match self.temporary_path() {
                Ok(temporary) => temporary,
                Err(error) => {
                    let cleanup = rollback_staged(&self.root, &staged);
                    let message = combine_error(error, cleanup);
                    return Err(ApplyError::new(report, Some(operation.clone()), message));
                }
            };
            if let Err(error) = fs::rename(self.root.join(source), self.root.join(&temporary)) {
                let cleanup = rollback_staged(&self.root, &staged);
                let message = combine_error(error, cleanup);
                return Err(ApplyError::new(report, Some(operation.clone()), message));
            }
            staged.push((operation.clone(), temporary));
        }

        for operation in self
            .operations
            .iter()
            .filter(|operation| matches!(operation, FsOperation::Delete { .. }))
        {
            let FsOperation::Delete { path, kind } = operation else {
                unreachable!();
            };
            if let Err(error) = delete_path(&self.root.join(path), *kind, deletion, trash) {
                let cleanup = rollback_staged(&self.root, &staged);
                let message = combine_error(error, cleanup);
                return Err(ApplyError::new(report, Some(operation.clone()), message));
            }
            report.applied.push(operation.clone());
        }

        // Creates and staged moves depend on each other in both directions: a
        // moved entry may land inside a directory this plan creates, and a new
        // entry may name a directory this plan renames into place. Neither
        // order is correct on its own, so each pass runs every remaining step
        // whose parent directory is already on disk and defers the rest.
        let mut pending = self
            .operations
            .iter()
            .filter(|operation| matches!(operation, FsOperation::Create { .. }))
            .map(|operation| PendingStep::Create(operation.clone()))
            .chain(staged.iter().map(|(operation, temporary)| {
                PendingStep::Finalize(operation.clone(), temporary.clone())
            }))
            .collect::<Vec<_>>();
        let mut unfinalized = staged.clone();
        while !pending.is_empty() {
            let mut deferred = Vec::new();
            let mut progressed = false;
            for step in pending {
                if !self.parent_is_ready(step.target()) {
                    deferred.push(step);
                    continue;
                }
                let (operation, result) = match &step {
                    PendingStep::Create(operation) => {
                        let FsOperation::Create { path, kind } = operation else {
                            unreachable!();
                        };
                        let result = match kind {
                            EntryKind::Directory => fs::create_dir(self.root.join(path)),
                            EntryKind::File => OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .open(self.root.join(path))
                                .map(drop),
                            EntryKind::Symlink | EntryKind::Other => {
                                Err(std::io::Error::other("unsupported create kind"))
                            }
                        };
                        (operation, result)
                    }
                    PendingStep::Finalize(operation, temporary) => {
                        let target = operation.target().expect("move target");
                        let result = fs::rename(self.root.join(temporary), self.root.join(target));
                        (operation, result)
                    }
                };
                if let Err(error) = result {
                    let cleanup = rollback_staged(&self.root, &unfinalized);
                    let message = combine_error(error, cleanup);
                    return Err(ApplyError::new(report, Some(operation.clone()), message));
                }
                if let PendingStep::Finalize(finalized, _) = &step {
                    unfinalized.retain(|(candidate, _)| candidate != finalized);
                }
                report.applied.push(operation.clone());
                progressed = true;
            }
            if !progressed {
                let blocked = deferred
                    .first()
                    .map(|step| step.target().display().to_string())
                    .unwrap_or_default();
                let cleanup = rollback_staged(&self.root, &unfinalized);
                let message = combine_error(
                    std::io::Error::other(format!(
                        "no parent directory for {blocked}; reopen the directory before applying"
                    )),
                    cleanup,
                );
                return Err(ApplyError::new(report, None, message));
            }
            pending = deferred;
        }

        for operation in self
            .operations
            .iter()
            .filter(|operation| matches!(operation, FsOperation::Copy { .. }))
        {
            let FsOperation::Copy { from, to, .. } = operation else {
                unreachable!();
            };
            if let Err(error) = copy_path(&self.root.join(from), &self.root.join(to)) {
                return Err(ApplyError::new(report, Some(operation.clone()), error));
            }
            report.applied.push(operation.clone());
        }
        Ok(report)
    }

    fn operation_sources_unchanged(&self) -> bool {
        self.operations
            .iter()
            .filter_map(|operation| match operation {
                FsOperation::Rename { from, .. }
                | FsOperation::Move { from, .. }
                | FsOperation::Copy { from, .. } => Some(from),
                FsOperation::Delete { path, .. } => Some(path),
                FsOperation::Create { .. } => None,
            })
            .filter_map(|source| {
                self.expected
                    .entries()
                    .iter()
                    .find(|entry| &entry.path == source)
            })
            .all(|expected| {
                SourceFingerprint::shallow(&self.root.join(&expected.path), "plan source")
                    .is_ok_and(|current| current == expected.source_fingerprint())
            })
    }

    fn confirmed_sources_unchanged(&self) -> bool {
        self.confirmed_sources.iter().all(|(source, expected)| {
            expected.as_ref().is_some_and(|expected| {
                SourceFingerprint::capture(&self.root.join(source))
                    .is_ok_and(|current| &current == expected)
            })
        })
    }

    fn preflight(&self) -> Result<()> {
        let vacated = self
            .operations
            .iter()
            .filter_map(|operation| match operation {
                FsOperation::Rename { from, .. } | FsOperation::Move { from, .. } => {
                    Some(from.clone())
                }
                FsOperation::Delete { path, .. } => Some(path.clone()),
                FsOperation::Create { .. } | FsOperation::Copy { .. } => None,
            })
            .collect::<HashSet<_>>();
        for operation in &self.operations {
            let Some(target) = operation.target() else {
                continue;
            };
            match fs::symlink_metadata(self.root.join(target)) {
                Ok(_) if !vacated.contains(target) => {
                    bail!("target already exists: {}", target.display());
                }
                Ok(_) => {}
                Err(_) => {}
            }
        }
        let final_directory_targets = self
            .operations
            .iter()
            .filter_map(|operation| match operation {
                FsOperation::Create {
                    path,
                    kind: EntryKind::Directory,
                }
                | FsOperation::Rename {
                    to: path,
                    kind: EntryKind::Directory,
                    ..
                }
                | FsOperation::Move {
                    to: path,
                    kind: EntryKind::Directory,
                    ..
                }
                | FsOperation::Copy {
                    to: path,
                    kind: EntryKind::Directory,
                    ..
                } => Some(path.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        for target in self.operations.iter().filter_map(FsOperation::target) {
            let mut relative = PathBuf::new();
            let Some(parent) = target.parent() else {
                continue;
            };
            for component in parent.components() {
                relative.push(component.as_os_str());
                let absolute = self.root.join(&relative);
                ensure!(
                    !vacated.contains(&relative) || final_directory_targets.contains(&relative),
                    "target parent will be moved or deleted: {}",
                    relative.display()
                );
                match fs::symlink_metadata(&absolute) {
                    Ok(metadata) => ensure!(
                        metadata.is_dir() && !metadata.file_type().is_symlink(),
                        "target parent is not a real directory: {}",
                        relative.display()
                    ),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => ensure!(
                        final_directory_targets.contains(&relative),
                        "target parent does not exist: {}",
                        relative.display()
                    ),
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("failed to inspect target parent {}", absolute.display())
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether the directory that will hold `target` already exists.
    ///
    /// A target directly in the plan's root always qualifies; the root itself
    /// was verified before any operation ran.
    fn parent_is_ready(&self, target: &Path) -> bool {
        match target.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => self.root.join(parent).is_dir(),
            _ => true,
        }
    }

    fn temporary_path(&self) -> Result<PathBuf> {
        loop {
            let value = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let candidate = PathBuf::from(format!(".runyte-move-{}-{value}", std::process::id()));
            let absolute = self.root.join(&candidate);
            match fs::symlink_metadata(&absolute) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(candidate);
                }
                Ok(_) => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "failed to inspect temporary move path {}",
                            absolute.display()
                        )
                    });
                }
            }
        }
    }
}

fn normalize_desired_path(root: &Path, path: &Path) -> Result<PathBuf> {
    ensure!(
        !path.as_os_str().is_empty(),
        "directory entries cannot be empty"
    );
    ensure!(!path.is_absolute(), "directory entries must be relative");
    let target = lexical_normalize(&root.join(path));
    let relative = relative_from_root(root, &target)?;
    ensure!(
        !relative.as_os_str().is_empty(),
        "directory entries cannot name the explorer root"
    );
    Ok(relative)
}

fn relative_from_root(root: &Path, target: &Path) -> Result<PathBuf> {
    let root_components = root.components().collect::<Vec<_>>();
    let target_components = target.components().collect::<Vec<_>>();
    let common = root_components
        .iter()
        .zip(&target_components)
        .take_while(|(left, right)| left == right)
        .count();
    ensure!(
        common > 0
            && root_components[common..]
                .iter()
                .all(|component| matches!(component, Component::Normal(_)))
            && target_components[common..]
                .iter()
                .all(|component| matches!(component, Component::Normal(_))),
        "path is not reachable from its explorer root: {}",
        target.display()
    );
    let mut relative = PathBuf::new();
    for _ in &root_components[common..] {
        relative.push("..");
    }
    for component in &target_components[common..] {
        relative.push(component.as_os_str());
    }
    Ok(relative)
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn operation_order(operation: &FsOperation) -> (usize, PathBuf) {
    let path = match operation {
        FsOperation::Create { path, .. } | FsOperation::Delete { path, .. } => path,
        FsOperation::Rename { to, .. }
        | FsOperation::Move { to, .. }
        | FsOperation::Copy { to, .. } => to,
    };
    (path.components().count(), path.clone())
}

fn delete_path(
    path: &Path,
    kind: EntryKind,
    deletion: DeletionMode,
    trash: &dyn TrashBackend,
) -> Result<()> {
    match deletion {
        DeletionMode::Trash => trash
            .delete(path)
            .map_err(|error| anyhow!("failed to trash {}: {error}", path.display())),
        DeletionMode::Permanent if kind == EntryKind::Directory => fs::remove_dir_all(path)
            .with_context(|| format!("failed to permanently delete {}", path.display())),
        DeletionMode::Permanent => fs::remove_file(path)
            .with_context(|| format!("failed to permanently delete {}", path.display())),
    }
}

fn copy_path(source: &Path, target: &Path) -> Result<()> {
    match fs::symlink_metadata(target) {
        Ok(_) => bail!("target already exists: {}", target.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect copy target {}", target.display()));
        }
    }
    let temporary = temporary_copy_path(target)?;
    if let Err(error) = copy_entry(source, &temporary) {
        let cleanup = remove_copy(&temporary)
            .err()
            .map(|cleanup| cleanup.to_string());
        bail!("{}", combine_error(error, cleanup));
    }
    if let Err(error) = fs::rename(&temporary, target) {
        let cleanup = remove_copy(&temporary)
            .err()
            .map(|cleanup| cleanup.to_string());
        bail!("{}", combine_error(error, cleanup));
    }
    Ok(())
}

fn temporary_copy_path(target: &Path) -> Result<PathBuf> {
    let parent = target
        .parent()
        .with_context(|| format!("copy target has no parent: {}", target.display()))?;
    loop {
        let value = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".runyte-copy-{}-{value}", std::process::id()));
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to inspect temporary copy path {}",
                        candidate.display()
                    )
                });
            }
        }
    }
}

fn copy_entry(source: &Path, target: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("failed to inspect copy source {}", source.display()))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        let link = fs::read_link(source)
            .with_context(|| format!("failed to read link {}", source.display()))?;
        create_symlink(source, &link, target)?;
    } else if file_type.is_dir() {
        fs::create_dir(target)
            .with_context(|| format!("failed to create directory {}", target.display()))?;
        for entry in fs::read_dir(source)
            .with_context(|| format!("failed to read directory {}", source.display()))?
        {
            let entry =
                entry.with_context(|| format!("failed to read entry in {}", source.display()))?;
            copy_entry(&entry.path(), &target.join(entry.file_name()))?;
        }
        fs::set_permissions(target, metadata.permissions())
            .with_context(|| format!("failed to copy permissions to {}", target.display()))?;
    } else if file_type.is_file() {
        fs::copy(source, target).with_context(|| {
            format!(
                "failed to copy {} to {}",
                source.display(),
                target.display()
            )
        })?;
    } else {
        bail!("unsupported copy source: {}", source.display());
    }
    Ok(())
}

#[cfg(unix)]
fn create_symlink(_source: &Path, link: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(link, target)
        .with_context(|| format!("failed to copy symlink to {}", target.display()))
}

#[cfg(windows)]
fn create_symlink(source: &Path, link: &Path, target: &Path) -> Result<()> {
    let result = match fs::metadata(source) {
        Ok(metadata) if metadata.is_dir() => std::os::windows::fs::symlink_dir(link, target),
        Ok(_) => std::os::windows::fs::symlink_file(link, target),
        Err(error) => return Err(error).context("failed to inspect symlink target kind"),
    };
    result.with_context(|| format!("failed to copy symlink to {}", target.display()))
}

#[cfg(not(any(unix, windows)))]
fn create_symlink(_source: &Path, _link: &Path, target: &Path) -> Result<()> {
    bail!(
        "copying symlinks is unsupported on this platform: {}",
        target.display()
    )
}

fn remove_copy(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)
                .with_context(|| format!("failed to remove temporary copy {}", path.display()))
        }
        Ok(_) => fs::remove_file(path)
            .with_context(|| format!("failed to remove temporary copy {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect temporary copy {}", path.display())),
    }
}

/// One step of the interleaved create/finalize phase of [`FsPlan::apply`].
///
/// Both variants are ordered by the same rule — the parent directory of the
/// path they produce has to exist first — so they share one work list.
#[derive(Clone, Debug)]
enum PendingStep {
    Create(FsOperation),
    Finalize(FsOperation, PathBuf),
}

impl PendingStep {
    /// The path this step brings into existence.
    fn target(&self) -> &Path {
        match self {
            Self::Create(FsOperation::Create { path, .. }) => path,
            Self::Create(_) => unreachable!("only creates are queued as create steps"),
            Self::Finalize(operation, _) => operation.target().expect("move target"),
        }
    }
}

fn rollback_staged(root: &Path, staged: &[(FsOperation, PathBuf)]) -> Option<String> {
    let mut failures = Vec::new();
    for (operation, temporary) in staged.iter().rev() {
        let Some(source) = operation.staged_source() else {
            continue;
        };
        let temporary = root.join(temporary);
        match fs::symlink_metadata(&temporary) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Ok(_) => {}
            Err(error) => {
                failures.push(format!(
                    "could not inspect staged {} at {}: {error}",
                    source.display(),
                    temporary.display()
                ));
                continue;
            }
        }
        if let Err(error) = fs::rename(&temporary, root.join(source)) {
            failures.push(format!(
                "could not restore {} from {}: {error}",
                source.display(),
                temporary.display()
            ));
        }
    }
    (!failures.is_empty()).then(|| failures.join("; "))
}

fn combine_error(error: impl fmt::Display, cleanup: Option<String>) -> String {
    match cleanup {
        Some(cleanup) => format!("{error}; cleanup incomplete: {cleanup}"),
        None => error.to_string(),
    }
}
