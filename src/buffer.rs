// SPDX-License-Identifier: MPL-2.0

//! Rope-backed buffers with transactional editing and undo.
//!
//! A buffer owns a [`Text`] and an undo history of inverse [`Transaction`]s.
//! [`Buffer::apply`] is the only way to mutate buffer text, which is what makes
//! a multi-cursor edit a single undo step and keeps undo memory proportional to
//! edit size rather than document size.

use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail, ensure};

use crate::{
    content_alignment::{ContentAlignment, ContentLayout},
    directory_buffer::{DirectoryBuffer, DirectoryTransfer},
    fs_plan::{FsPlan, TransferMode},
    notification::{NOTIFICATIONS_BUFFER_NAME, NotificationDocument, NotificationRow},
    row_hints::{RowHints, display_cells},
    settings::{SETTINGS_BUFFER_NAME, SettingId},
    text::{Offset, Text, Transaction},
};

pub use crate::text::Position;

static NEXT_SAVE_TEMPORARY: AtomicU64 = AtomicU64::new(1);

/// A file whose complete bytes cannot be represented safely by a text buffer.
///
/// Kept typed so the application can route a file that changed after its
/// bounded binary probe to the external-program workflow instead of showing a
/// generic read failure.
#[derive(Debug)]
pub struct BinaryFileError;

impl std::fmt::Display for BinaryFileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("binary file cannot be loaded as editable text")
    }
}

impl std::error::Error for BinaryFileError {}

/// The display name of the changed-file list, which is also how the editor
/// finds the buffer again rather than opening a second one.
pub const GIT_STATUS_NAME: &str = "[git status]";

/// The display name of the local branch list.
pub const GIT_BRANCHES_NAME: &str = "[git branches]";

/// The display name of the repository worktree list.
pub const GIT_WORKTREES_NAME: &str = "[git worktrees]";
pub const GIT_LOG_NAME: &str = "[git log]";
pub const GIT_BLAME_NAME: &str = "[git blame]";
pub const GIT_STASH_NAME: &str = "[git stashes]";
pub const WORKSPACE_SEARCH_NAME: &str = "[workspace search]";

/// The display name of the commit message buffer, and how the editor finds it
/// again rather than opening a second one.
pub const COMMIT_MESSAGE_NAME: &str = "[git commit]";

/// The display name of contextual view help. The general manual owns `[help]`.
pub const HELP_NAME: &str = "[view help]";

/// The display name of the diagnostic log projection opened by `:log-open`.
pub const LOG_NAME: &str = "[log]";

/// How many undo steps a buffer retains.
const HISTORY_LIMIT: usize = 1000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BufferKind {
    File,
    Directory,
    Scratch,
    Virtual {
        identity: GeneratedViewIdentity,
        name: String,
        /// Where unified-diff content begins, in character offsets.
        ///
        /// `Some(0)` describes a buffer that is entirely a patch. A later
        /// offset lets a generated view keep prose or metadata ahead of its
        /// patch without asking frontends to infer the boundary from text.
        diff_start: Option<Offset>,
    },
    /// A commit message being written.
    ///
    /// Editable, unlike the other generated buffers, and saving it commits
    /// rather than writing a file — the same shape as the `COMMIT_EDITMSG` an
    /// external editor is handed.
    CommitMessage,
    /// The changed-file list, one file per line under its section heading.
    ///
    /// A kind of its own because its Tab menu acts on the files under the
    /// selection, and nothing else in the editor reads a row as a file.
    GitStatus,
    /// Local branches, one per line, with the current branch marked.
    GitBranches,
    /// Registered checkouts for the repository's common Git directory.
    GitWorktrees,
    /// Bounded commit summaries, one stable object identity per row.
    GitLog,
    /// Live-buffer attribution, aligned one row per requested source line.
    GitBlame,
    GitStash,
    /// Complete commit metadata and patch, keyed by the full object ID rather
    /// than the abbreviated display title.
    GitCommit {
        oid: String,
        name: String,
        diff_start: Offset,
    },
    /// A query-time workspace result set. Rendered rows remain ordinary
    /// searchable text while activation reads these stable typed targets.
    WorkspaceSearch {
        query: String,
        mode: String,
        rows: Vec<Option<WorkspaceSearchTarget>>,
        limited: bool,
    },
    /// Rendered help for one view.
    ///
    /// A buffer rather than an overlay so help is searchable, scrollable, and
    /// splittable with the same keys as everything else, and so no view has to
    /// grow its own scrolling. The text is generated at open time and never
    /// refreshed: it describes the view help was opened from, not itself.
    Help,
    /// The typed setting registry rendered as a searchable document.
    ///
    /// Every physical row keeps the setting identity it belongs to so wrapped
    /// columns remain activatable without parsing their rendered text.
    Settings {
        rows: Vec<Option<SettingId>>,
    },
    /// The retained in-memory notification history, newest first.
    ///
    /// Rows carry their notification identity and severity so frontends never
    /// need to parse the rendered document to provide semantic styling.
    Notifications {
        rows: Vec<NotificationRow>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GeneratedViewIdentity {
    /// Deliberately named internal projections and test/embedder documents.
    Named(String),
    About,
    Tutorial,
    Manual,
    Documentation,
    /// The diagnostic log owned by the process that owns `App`.
    Log,
    GitIndex,
    GitDiff {
        path: PathBuf,
        scope: String,
    },
    GitDiffSide {
        path: PathBuf,
        scope: String,
        previous: bool,
    },
    /// One immutable observation of an ordinary file's disk contents.
    DiskSnapshot {
        source_buffer: usize,
        revision: String,
    },
}

/// Why an ordinary file buffer no longer agrees with its accepted baseline.
///
/// This is presentation-neutral state. Frontends render every non-synchronized
/// value as `[STALE]` and leave the more specific explanation to notifications.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExternalFileStatus {
    #[default]
    Synchronized,
    Changed,
    Deleted,
    Binary,
    Unreadable,
}

impl ExternalFileStatus {
    pub const fn is_stale(self) -> bool {
        !matches!(self, Self::Synchronized)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceSearchTarget {
    pub path: PathBuf,
    pub row: usize,
    pub column: usize,
    pub length: usize,
}

/// Longest line, in bytes, a document may have and still be soft-wrapped.
///
/// Set from measurement rather than taste. Wrapping one line costs about
/// 12ns per character per frame, counting every pass a frame makes over it, so
/// the cost is linear and easy to place: a one-million-character line takes
/// about 10ms a frame, sixteen million about 160ms, and sixty-four million
/// about 750ms. The limit sits at the point where a frame approaches a whole
/// second on a slower machine, because that is where a document stops being
/// slow to scroll and starts being impossible to use.
///
/// It is deliberately far above anything a person would want wrapped. Below it
/// wrapping degrades gradually and stays the reader's choice; a minified file
/// of a few megabytes keeps it.
pub const SOFT_WRAP_LINE_LIMIT: usize = 64_000_000;

#[derive(Clone, Debug)]
pub struct Buffer {
    text: Text,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub kind: BufferKind,
    directory: Option<DirectoryBuffer>,
    /// Branch/tag decorations for `BufferKind::GitLog` rows, keyed by buffer
    /// line. Read separately from the row's text so a ref name is never part
    /// of what a line's text says, mirroring `directory`'s symlink targets.
    git_log_hints: HashMap<usize, String>,
    /// The last content accepted as clean, when this buffer has one.
    ///
    /// Scratch and generated buffers may have no saved baseline.
    saved_text: Option<Text>,
    undo: Vec<Vec<Transaction>>,
    redo: Vec<Vec<Transaction>>,
    /// Inverse transactions collected during one Insert-mode action.
    ///
    /// They are kept newest-last while the action is live, then reversed into
    /// the order in which undo must apply them when the checkpoint is closed.
    undo_group: Option<Vec<Transaction>>,
    /// What the file looked like when this buffer last agreed with it.
    ///
    /// `None` for a buffer with no file behind it, and for one whose file
    /// could not be inspected, which is treated as nothing to protect.
    disk_state: Option<DiskState>,
    /// Monotonic identity of the accepted disk baseline and path ownership.
    disk_generation: u64,
    external_status: ExternalFileStatus,
    external_observation: Option<FileObservation>,
    last_reported_observation: Option<FileObservation>,
    /// Bytes in the longest line this buffer's text had when it was built.
    ///
    /// Measured once, where the text arrives, because it exists to answer one
    /// question before anything is drawn: whether this document is cheap
    /// enough to soft-wrap. See [`Buffer::soft_wrap_viable`].
    longest_line: usize,
    /// Where a pane places this buffer's content, and the width it was
    /// measured at.
    ///
    /// Left and top for everything with a file behind it: alignment is
    /// something a generated page asks for, and moving a document the person
    /// is editing away from its own first column would put its text somewhere
    /// its coordinates do not say it is.
    layout: ContentLayout,
}

/// Enough of a file's metadata to notice that something else rewrote it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiskState {
    len: u64,
    modified: Option<std::time::SystemTime>,
    digest: String,
    identity: Option<FileIdentity>,
    access: Option<FileAccessState>,
}

/// One complete path observation. Text and disk state are always built from
/// the same open file handle, so a confirmation never combines two revisions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FileObservation {
    Text { text: Arc<str>, state: DiskState },
    Deleted,
    Binary { digest: String },
    Unreadable { message: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileObservationRequest {
    pub(crate) buffer: usize,
    pub(crate) path: PathBuf,
    pub(crate) generation: u64,
    pub(crate) baseline_metadata: Option<FileMetadataHint>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileObservationEvent {
    pub(crate) buffer: usize,
    pub(crate) path: PathBuf,
    pub(crate) generation: u64,
    pub(crate) observation: FileObservation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileMetadataHint {
    len: u64,
    modified: Option<std::time::SystemTime>,
    identity: Option<FileIdentity>,
    access: Option<FileAccessState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ObservationApply {
    Ignored,
    Synchronized,
    Converged,
    Stale { notify: bool },
}

impl DiskState {
    fn inspect(path: &Path) -> Result<Option<Self>> {
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
            }
        };
        let metadata = file
            .metadata()
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        Ok(Some(Self::from_contents(&file, &metadata, &contents)?))
    }

    fn from_contents(file: &File, metadata: &fs::Metadata, contents: &[u8]) -> Result<Self> {
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            digest: crate::hash::sha256_hex(contents),
            identity: file_identity(metadata),
            access: file_access_state(file, metadata)?,
        })
    }

    fn same_contents(&self, other: &Self) -> bool {
        self.len == other.len && self.digest == other.digest
    }

    fn metadata_hint(&self) -> FileMetadataHint {
        FileMetadataHint {
            len: self.len,
            modified: self.modified,
            identity: self.identity.clone(),
            access: self.access.clone(),
        }
    }

    fn revision_key(&self) -> String {
        format!("{}:{}", self.len, self.digest)
    }

    fn matches_displaced(&self, expected: &Self) -> bool {
        self.len == expected.len
            && self.modified == expected.modified
            && self.digest == expected.digest
            && self.identity == expected.identity
            && match (&self.access, &expected.access) {
                (Some(current), Some(expected)) => {
                    current.mode == expected.mode
                        && current.uid == expected.uid
                        && current.gid == expected.gid
                        && current.acl_digest == expected.acl_digest
                }
                (None, None) => true,
                _ => false,
            }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileAccessState {
    mode: u32,
    uid: u32,
    gid: u32,
    changed_seconds: i64,
    changed_nanoseconds: i64,
    acl_digest: Option<String>,
}

#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    Some(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
fn file_identity(_metadata: &fs::Metadata) -> Option<FileIdentity> {
    None
}

#[cfg(unix)]
fn file_access_state(file: &File, metadata: &fs::Metadata) -> Result<Option<FileAccessState>> {
    use std::os::unix::fs::MetadataExt;

    Ok(Some(FileAccessState {
        mode: metadata.mode(),
        uid: metadata.uid(),
        gid: metadata.gid(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
        acl_digest: file_acl_digest(file)?,
    }))
}

#[cfg(not(unix))]
fn file_access_state(_file: &File, _metadata: &fs::Metadata) -> Result<Option<FileAccessState>> {
    Ok(None)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn file_acl_digest(file: &File) -> Result<Option<String>> {
    use std::os::fd::AsRawFd;

    const ACL_NAME: &[u8] = b"system.posix_acl_access\0";
    // SAFETY: the descriptor is live and ACL_NAME is NUL terminated.
    let size = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            ACL_NAME.as_ptr().cast(),
            std::ptr::null_mut(),
            0,
        )
    };
    if size < 0 {
        let error = io::Error::last_os_error();
        if matches!(
            error.raw_os_error(),
            Some(libc::ENODATA) | Some(libc::EOPNOTSUPP)
        ) {
            return Ok(None);
        }
        return Err(error.into());
    }
    let mut acl = vec![0_u8; size as usize];
    // SAFETY: acl owns the reported number of writable bytes.
    if unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            ACL_NAME.as_ptr().cast(),
            acl.as_mut_ptr().cast(),
            acl.len(),
        )
    } < 0
    {
        return Err(io::Error::last_os_error().into());
    }
    Ok(Some(crate::hash::sha256_hex(&acl)))
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn file_acl_digest(file: &File) -> Result<Option<String>> {
    use std::{ffi::CStr, os::fd::AsRawFd};

    type Acl = *mut core::ffi::c_void;
    const ACL_TYPE_EXTENDED: i32 = 0x0000_0100;
    unsafe extern "C" {
        fn acl_get_fd_np(fd: libc::c_int, kind: i32) -> Acl;
        fn acl_to_text(acl: Acl, length: *mut libc::ssize_t) -> *mut libc::c_char;
        fn acl_free(object: *mut core::ffi::c_void) -> libc::c_int;
    }
    // SAFETY: the descriptor is live and the type is Darwin's extended ACL.
    let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        let error = io::Error::last_os_error();
        if matches!(
            error.raw_os_error(),
            Some(libc::ENOENT) | Some(libc::EINVAL)
        ) {
            return Ok(None);
        }
        return Err(error.into());
    }
    let mut length = 0;
    // SAFETY: acl is live and length points to writable storage.
    let text = unsafe { acl_to_text(acl, &mut length) };
    if text.is_null() {
        // SAFETY: acl_get_fd_np transferred ownership above.
        unsafe { acl_free(acl) };
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: acl_to_text returns a NUL-terminated allocation.
    let digest = crate::hash::sha256_hex(unsafe { CStr::from_ptr(text) }.to_bytes());
    // SAFETY: both objects were allocated by the ACL library.
    unsafe {
        acl_free(text.cast());
        acl_free(acl);
    }
    Ok(Some(digest))
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
)))]
fn file_acl_digest(_file: &File) -> Result<Option<String>> {
    Ok(None)
}

fn read_text_and_state(path: &Path, action: &str) -> Result<(String, DiskState)> {
    let mut file =
        File::open(path).with_context(|| format!("failed to {action} {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .with_context(|| format!("failed to {action} {}", path.display()))?;
    if crate::external_open::is_binary(&contents, true) {
        return Err(BinaryFileError)
            .with_context(|| format!("failed to {action} {}", path.display()));
    }
    let state = DiskState::from_contents(&file, &metadata, &contents)?;
    let contents =
        String::from_utf8(contents).expect("complete binary classification rejects invalid UTF-8");
    Ok((contents, state))
}

pub(crate) fn observe_file(path: &Path) -> FileObservation {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return FileObservation::Deleted;
        }
        Err(error) => {
            return FileObservation::Unreadable {
                message: error.to_string(),
            };
        }
    };
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            return FileObservation::Unreadable {
                message: error.to_string(),
            };
        }
    };
    let mut contents = Vec::new();
    if let Err(error) = file.read_to_end(&mut contents) {
        return FileObservation::Unreadable {
            message: error.to_string(),
        };
    }
    if crate::external_open::is_binary(&contents, true) {
        return FileObservation::Binary {
            digest: crate::hash::sha256_hex(&contents),
        };
    }
    let state = match DiskState::from_contents(&file, &metadata, &contents) {
        Ok(state) => state,
        Err(error) => {
            return FileObservation::Unreadable {
                message: error.to_string(),
            };
        }
    };
    let text = String::from_utf8(contents)
        .expect("complete binary classification rejects invalid UTF-8")
        .into();
    FileObservation::Text { text, state }
}

pub(crate) fn inspect_file_metadata(path: &Path) -> Option<FileMetadataHint> {
    let file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    Some(FileMetadataHint {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        identity: file_identity(&metadata),
        access: file_access_state(&file, &metadata).ok()?,
    })
}

struct SaveTemporary(Option<PathBuf>);

impl Drop for SaveTemporary {
    fn drop(&mut self) {
        if let Some(path) = self.0.as_ref() {
            let _ = fs::remove_file(path);
        }
    }
}

impl SaveTemporary {
    fn path(&self) -> &Path {
        self.0
            .as_deref()
            .expect("a retained save temporary is no longer used")
    }

    fn keep(mut self) -> PathBuf {
        self.0
            .take()
            .expect("a save temporary can only be retained once")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SaveOutcome {
    Durable,
    CommittedWithWarning(String),
}

#[derive(Debug)]
struct AtomicWriteStatus {
    durability_warning: Option<String>,
    installed_state: DiskState,
}

#[derive(Clone, Copy)]
enum ReplacePolicy<'a> {
    Expected(&'a DiskState),
    NoReplace,
    Force,
}

impl<'a> ReplacePolicy<'a> {
    fn expected(self) -> Option<&'a DiskState> {
        match self {
            Self::Expected(state) => Some(state),
            Self::NoReplace | Self::Force => None,
        }
    }
}

impl AtomicWriteStatus {
    fn durable(installed_state: DiskState) -> Self {
        Self {
            durability_warning: None,
            installed_state,
        }
    }

    fn committed_with_warning(warning: impl Into<String>, installed_state: DiskState) -> Self {
        Self {
            durability_warning: Some(warning.into()),
            installed_state,
        }
    }

    fn finish(self) -> SaveOutcome {
        match self.durability_warning {
            Some(warning) => SaveOutcome::CommittedWithWarning(warning),
            None => SaveOutcome::Durable,
        }
    }

    fn warn(&mut self, warning: impl Into<String>) {
        let warning = warning.into();
        self.durability_warning = Some(match self.durability_warning.take() {
            Some(existing) => format!("{existing}; {warning}"),
            None => warning,
        });
    }
}

fn atomic_write(
    path: &Path,
    contents: &[u8],
    policy: ReplacePolicy<'_>,
) -> Result<AtomicWriteStatus> {
    atomic_write_with(
        path,
        contents,
        policy,
        |file, contents| file.write_all(contents),
        sync_parent,
    )
}

fn atomic_write_checked(
    path: &Path,
    contents: &[u8],
    policy: ReplacePolicy<'_>,
    expected_identity: &Path,
) -> Result<AtomicWriteStatus> {
    atomic_write_with_identity(
        path,
        contents,
        policy,
        Some(expected_identity),
        |file, contents| file.write_all(contents),
        sync_parent,
    )
}

fn atomic_write_with(
    path: &Path,
    contents: &[u8],
    policy: ReplacePolicy<'_>,
    write_contents: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
    sync_directory: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<AtomicWriteStatus> {
    atomic_write_with_identity(path, contents, policy, None, write_contents, sync_directory)
}

fn atomic_write_with_identity(
    path: &Path,
    contents: &[u8],
    policy: ReplacePolicy<'_>,
    expected_identity: Option<&Path>,
    write_contents: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
    sync_directory: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<AtomicWriteStatus> {
    if let Some(expected) = expected_identity {
        ensure!(
            crate::path_safety::path_identity(path)?.as_path() == expected,
            "{} changed its resolved identity before it could be saved",
            path.display()
        );
    }
    let destination = resolve_write_target(path, 0)?;
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let metadata = fs::metadata(&destination).ok();
    if metadata.is_some() {
        // Atomic rename is authorized by the directory, unlike an in-place
        // write. Retain the old save boundary: a readable but non-writable
        // file must not become writable merely because its parent is.
        OpenOptions::new()
            .write(true)
            .open(&destination)
            .with_context(|| format!("failed to open {} for writing", path.display()))?;
    }
    let (temporary, mut file) = create_save_temporary(parent)?;
    let temporary = SaveTemporary(Some(temporary));
    write_contents(&mut file, contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    if let Some(metadata) = metadata.as_ref() {
        preserve_destination_metadata(&file, &destination, metadata).with_context(|| {
            format!(
                "failed to preserve access metadata while writing {}",
                path.display()
            )
        })?;
    }
    file.sync_all()
        .with_context(|| format!("failed to sync {}", path.display()))?;
    let installed_state = DiskState::from_contents(&file, &file.metadata()?, contents)?;
    drop(file);
    let latest_destination = resolve_write_target(path, 0)?;
    ensure!(
        latest_destination == destination,
        "{} changed its symbolic-link target while it was being saved; the replacement was not \
         installed",
        path.display()
    );
    if let Some(expected) = expected_identity {
        ensure!(
            crate::path_safety::path_identity(path)?.as_path() == expected,
            "{} changed its resolved identity while it was being saved; the replacement was not installed",
            path.display()
        );
    }
    let replacement_warning = match replace_file(temporary.path(), &destination, policy) {
        Ok(warning) => warning,
        Err(error) => {
            let retained = temporary.keep();
            return Err(error).with_context(|| {
                format!(
                    "failed to replace {}; recoverable contents were retained at {}",
                    path.display(),
                    retained.display()
                )
            });
        }
    };
    let mut warnings = Vec::new();
    if let Some(warning) = replacement_warning {
        warnings.push(warning);
    }
    if let Err(error) = sync_directory(parent) {
        warnings.push(format!(
            "{} was saved, but its directory entry could not be synced: {error}",
            path.display()
        ));
    }
    if warnings.is_empty() {
        Ok(AtomicWriteStatus::durable(installed_state))
    } else {
        Ok(AtomicWriteStatus::committed_with_warning(
            warnings.join("; "),
            installed_state,
        ))
    }
}

fn ensure_disk_unchanged(path: &Path, expected: Option<&DiskState>) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    // Deletion is deliberately not a conflict: there is no newer on-disk
    // content left to lose, and the atomic replacement recreates the file.
    if let Some(current) = DiskState::inspect(path)?
        && &current != expected
    {
        return Err(SaveConflict(format!(
            "{} changed on disk since it was read; use :write! to overwrite it or :reload to discard this buffer",
            path.display()
        ))
        .into());
    }
    Ok(())
}

#[derive(Debug)]
struct SaveConflict(String);

impl std::fmt::Display for SaveConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SaveConflict {}

pub(crate) fn is_save_conflict(error: &anyhow::Error) -> bool {
    error.downcast_ref::<SaveConflict>().is_some()
}

fn resolve_write_target(path: &Path, depth: usize) -> Result<PathBuf> {
    ensure!(depth < 40, "too many symbolic links in {}", path.display());
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let target = fs::read_link(path)
                .with_context(|| format!("failed to read symbolic link {}", path.display()))?;
            let target = if target.is_absolute() {
                target
            } else {
                path.parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .join(target)
            };
            resolve_write_target(&target, depth + 1)
        }
        Ok(_) => Ok(path.to_path_buf()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to inspect save target {}", path.display()))
        }
    }
}

fn create_save_temporary(parent: &Path) -> Result<(PathBuf, File)> {
    for _ in 0..100 {
        let number = NEXT_SAVE_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".runyte-save-{}-{number}.tmp", std::process::id()));
        match open_private_temporary(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to create temporary file in {}", parent.display())
                });
            }
        }
    }
    bail!(
        "failed to create a unique temporary file in {}",
        parent.display()
    )
}

#[cfg(unix)]
fn open_private_temporary(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(windows)]
fn open_private_temporary(path: &Path) -> io::Result<File> {
    use std::{
        mem::size_of,
        os::windows::{
            ffi::OsStrExt,
            io::{FromRawHandle, RawHandle},
        },
    };
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree},
        Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
        },
        Storage::FileSystem::{CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL},
    };

    let path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let descriptor_text = "D:P(A;;GA;;;OW)\0".encode_utf16().collect::<Vec<_>>();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: the SDDL and output pointer are valid for the call. Windows
    // allocates the returned self-relative descriptor with LocalAlloc.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor_text.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if converted == 0 {
        return Err(io::Error::last_os_error());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.cast(),
        bInheritHandle: 0,
    };
    // SAFETY: the path and security descriptor are live, NUL-terminated
    // buffers. CREATE_NEW provides the same collision protection as
    // OpenOptions::create_new.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    // SAFETY: ConvertStringSecurityDescriptorToSecurityDescriptorW allocated
    // this descriptor with LocalAlloc, and CreateFileW no longer uses it.
    unsafe {
        LocalFree(descriptor.cast());
    }
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: CreateFileW returned a uniquely owned file handle.
        Ok(unsafe { File::from_raw_handle(handle as RawHandle) })
    }
}

#[cfg(unix)]
fn preserve_destination_metadata(
    temporary: &File,
    destination: &Path,
    metadata: &fs::Metadata,
) -> io::Result<()> {
    use std::os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, PermissionsExt},
    };

    // SAFETY: the descriptor is live for the call. A failure occurs before
    // replacement, leaving the original file intact.
    if unsafe { libc::fchown(temporary.as_raw_fd(), metadata.uid(), metadata.gid()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    temporary.set_permissions(fs::Permissions::from_mode(metadata.mode()))?;
    preserve_posix_acl(temporary, destination)
}

#[cfg(not(unix))]
fn preserve_destination_metadata(
    _temporary: &File,
    _destination: &Path,
    _metadata: &fs::Metadata,
) -> io::Result<()> {
    // ReplaceFileW merges the existing file's attributes and DACL into the
    // replacement. New files retain the private descriptor used at creation.
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn preserve_posix_acl(temporary: &File, destination: &Path) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    const ACL_NAME: &[u8] = b"system.posix_acl_access\0";
    let source = File::open(destination)?;
    // SAFETY: both descriptors are live and ACL_NAME is NUL terminated.
    let size = unsafe {
        libc::fgetxattr(
            source.as_raw_fd(),
            ACL_NAME.as_ptr().cast(),
            std::ptr::null_mut(),
            0,
        )
    };
    if size < 0 {
        let error = io::Error::last_os_error();
        if matches!(
            error.raw_os_error(),
            Some(libc::ENODATA) | Some(libc::EOPNOTSUPP)
        ) {
            // A default directory ACL may have been inherited by the private
            // temporary. Remove it when the destination has no access ACL.
            // SAFETY: the descriptor and name are valid.
            let removed =
                unsafe { libc::fremovexattr(temporary.as_raw_fd(), ACL_NAME.as_ptr().cast()) };
            if removed != 0 {
                let remove_error = io::Error::last_os_error();
                if !matches!(
                    remove_error.raw_os_error(),
                    Some(libc::ENODATA) | Some(libc::EOPNOTSUPP)
                ) {
                    return Err(remove_error);
                }
            }
            return Ok(());
        }
        return Err(error);
    }
    let mut acl = vec![0_u8; size as usize];
    // SAFETY: acl owns size writable bytes and both descriptors remain live.
    if unsafe {
        libc::fgetxattr(
            source.as_raw_fd(),
            ACL_NAME.as_ptr().cast(),
            acl.as_mut_ptr().cast(),
            acl.len(),
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: acl contains the exact attribute bytes read above.
    if unsafe {
        libc::fsetxattr(
            temporary.as_raw_fd(),
            ACL_NAME.as_ptr().cast(),
            acl.as_ptr().cast(),
            acl.len(),
            0,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn preserve_posix_acl(temporary: &File, destination: &Path) -> io::Result<()> {
    use std::{
        ffi::CString,
        os::{fd::AsRawFd, unix::ffi::OsStrExt},
    };

    type Acl = *mut core::ffi::c_void;
    const ACL_TYPE_EXTENDED: i32 = 0x0000_0100;
    unsafe extern "C" {
        fn acl_get_file(path: *const libc::c_char, kind: i32) -> Acl;
        fn acl_set_fd(fd: libc::c_int, acl: Acl) -> libc::c_int;
        fn acl_free(object: *mut core::ffi::c_void) -> libc::c_int;
    }
    let path = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    // SAFETY: path is NUL terminated and ACL_TYPE_EXTENDED is the Darwin
    // access-control-list class.
    let acl = unsafe { acl_get_file(path.as_ptr(), ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        let error = io::Error::last_os_error();
        if matches!(
            error.raw_os_error(),
            Some(libc::ENOENT) | Some(libc::EOPNOTSUPP)
        ) {
            // Darwin reports ENOENT when a file has no extended ACL. The
            // temporary was created in the destination directory, so there
            // is no destination ACL to copy in this case. EOPNOTSUPP is the
            // equivalent result on filesystems without ACL support.
            return Ok(());
        }
        return Err(error);
    }
    // SAFETY: both the descriptor and ACL returned above remain live.
    let applied = unsafe { acl_set_fd(temporary.as_raw_fd(), acl) };
    let error = (applied != 0).then(io::Error::last_os_error);
    // SAFETY: acl_get_file transfers ownership of the ACL object.
    unsafe {
        acl_free(acl);
    }
    match error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
fn preserve_posix_acl(temporary: &File, destination: &Path) -> io::Result<()> {
    use std::{
        ffi::CString,
        os::{fd::AsRawFd, unix::ffi::OsStrExt},
    };

    type Acl = *mut core::ffi::c_void;
    const ACL_TYPE_ACCESS: i32 = 0x0000_0002;
    unsafe extern "C" {
        fn acl_get_file(path: *const libc::c_char, kind: i32) -> Acl;
        fn acl_set_fd(fd: libc::c_int, acl: Acl) -> libc::c_int;
        fn acl_free(object: *mut core::ffi::c_void) -> libc::c_int;
    }
    let path = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    // SAFETY: path is NUL terminated and ACL_TYPE_ACCESS requests the access
    // ACL rather than a directory's default ACL.
    let acl = unsafe { acl_get_file(path.as_ptr(), ACL_TYPE_ACCESS) };
    if acl.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both the descriptor and ACL returned above remain live.
    let applied = unsafe { acl_set_fd(temporary.as_raw_fd(), acl) };
    let error = (applied != 0).then(io::Error::last_os_error);
    // SAFETY: acl_get_file transfers ownership of the ACL object.
    unsafe {
        acl_free(acl);
    }
    match error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly"
    ))
))]
fn preserve_posix_acl(_temporary: &File, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic saves cannot preserve file ACLs on this Unix platform",
    ))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[allow(clippy::unnecessary_cast)] // Android exposes these flags as c_int.
fn replace_file(
    source: &Path,
    destination: &Path,
    policy: ReplacePolicy<'_>,
) -> io::Result<Option<String>> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let source_c = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let destination_c = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;
    if matches!(policy, ReplacePolicy::Force) {
        fs::rename(source, destination)?;
        return Ok(None);
    }
    if matches!(policy, ReplacePolicy::NoReplace) {
        // SAFETY: both paths are live NUL-terminated strings.
        return if linux_renameat2(
            source_c.as_ptr(),
            destination_c.as_ptr(),
            libc::RENAME_NOREPLACE as u32,
        ) == 0
        {
            Ok(None)
        } else {
            Err(io::Error::last_os_error())
        };
    }
    // SAFETY: both paths are live NUL-terminated strings. RENAME_EXCHANGE
    // atomically retains the exact displaced destination at `source`.
    let exchanged = linux_renameat2(
        source_c.as_ptr(),
        destination_c.as_ptr(),
        libc::RENAME_EXCHANGE as u32,
    );
    if exchanged == 0 {
        let displaced = DiskState::inspect(source)
            .map_err(|error| io::Error::other(format!("cannot inspect displaced file: {error}")))?;
        if displaced
            .as_ref()
            .zip(policy.expected())
            .is_some_and(|(displaced, expected)| displaced.matches_displaced(expected))
        {
            return match fs::remove_file(source) {
                Ok(()) => Ok(None),
                Err(error) => Ok(Some(format!(
                    "{} was saved, but its displaced prior contents at {} could not be removed: {error}",
                    destination.display(),
                    source.display()
                ))),
            };
        }
        // Put the exact displaced object back. The source path is deliberately
        // retained by the caller on every error, so even a concurrent change
        // to the installed destination survives this recovery exchange.
        // SAFETY: the same valid path buffers remain live.
        if linux_renameat2(
            source_c.as_ptr(),
            destination_c.as_ptr(),
            libc::RENAME_EXCHANGE as u32,
        ) != 0
        {
            return Err(io::Error::other(format!(
                "destination changed during save and recovery exchange failed: {}",
                io::Error::last_os_error()
            )));
        }
        return Err(io::Error::other(
            "destination changed during save; its displaced contents were restored",
        ));
    }
    let exchange_error = io::Error::last_os_error();
    if exchange_error.kind() != io::ErrorKind::NotFound {
        return Err(exchange_error);
    }
    // The file was deleted after it was read. Recreate it without overwriting
    // a new entry that may appear between the exchange and this call.
    // SAFETY: both path buffers remain valid for the call.
    if linux_renameat2(
        source_c.as_ptr(),
        destination_c.as_ptr(),
        libc::RENAME_NOREPLACE as u32,
    ) == 0
    {
        Ok(None)
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn linux_renameat2(
    source: *const libc::c_char,
    destination: *const libc::c_char,
    flags: u32,
) -> libc::c_int {
    // SAFETY: callers supply live NUL-terminated paths. Calling the kernel
    // directly avoids depending on the Android API level that added bionic's
    // renameat2 wrapper.
    unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source,
            libc::AT_FDCWD,
            destination,
            flags,
        ) as libc::c_int
    }
}

#[cfg(all(not(windows), not(any(target_os = "linux", target_os = "android"))))]
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn replace_file(
    source: &Path,
    destination: &Path,
    policy: ReplacePolicy<'_>,
) -> io::Result<Option<String>> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    if matches!(policy, ReplacePolicy::Force) {
        fs::rename(source, destination)?;
        return Ok(None);
    }
    let source_c = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let destination_c = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;
    if matches!(policy, ReplacePolicy::NoReplace) {
        // SAFETY: both paths are live NUL-terminated strings.
        return if unsafe {
            libc::renamex_np(source_c.as_ptr(), destination_c.as_ptr(), libc::RENAME_EXCL)
        } == 0
        {
            Ok(None)
        } else {
            Err(io::Error::last_os_error())
        };
    }
    // SAFETY: RENAME_SWAP atomically retains the displaced destination.
    let exchanged =
        unsafe { libc::renamex_np(source_c.as_ptr(), destination_c.as_ptr(), libc::RENAME_SWAP) };
    if exchanged == 0 {
        let displaced = DiskState::inspect(source)
            .map_err(|error| io::Error::other(format!("cannot inspect displaced file: {error}")))?;
        if displaced
            .as_ref()
            .zip(policy.expected())
            .is_some_and(|(displaced, expected)| displaced.matches_displaced(expected))
        {
            return match fs::remove_file(source) {
                Ok(()) => Ok(None),
                Err(error) => Ok(Some(format!(
                    "{} was saved, but its displaced prior contents at {} could not be removed: {error}",
                    destination.display(),
                    source.display()
                ))),
            };
        }
        // SAFETY: the same paths remain valid; the caller retains `source` on
        // error so a concurrent installed object is not discarded.
        if unsafe { libc::renamex_np(source_c.as_ptr(), destination_c.as_ptr(), libc::RENAME_SWAP) }
            != 0
        {
            return Err(io::Error::other(format!(
                "destination changed during save and recovery exchange failed: {}",
                io::Error::last_os_error()
            )));
        }
        return Err(io::Error::other(
            "destination changed during save; its displaced contents were restored",
        ));
    }
    let exchange_error = io::Error::last_os_error();
    if exchange_error.kind() != io::ErrorKind::NotFound {
        return Err(exchange_error);
    }
    // SAFETY: RENAME_EXCL installs only if the deleted path stayed absent.
    if unsafe { libc::renamex_np(source_c.as_ptr(), destination_c.as_ptr(), libc::RENAME_EXCL) }
        == 0
    {
        Ok(None)
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(all(
    not(windows),
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))
))]
fn replace_file(
    source: &Path,
    destination: &Path,
    policy: ReplacePolicy<'_>,
) -> io::Result<Option<String>> {
    if !matches!(policy, ReplacePolicy::Force) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this platform cannot atomically protect an ordinary save from concurrent changes; use :write! to force",
        ));
    }
    fs::rename(source, destination)?;
    Ok(None)
}

#[cfg(windows)]
fn replace_file(
    source: &Path,
    destination: &Path,
    policy: ReplacePolicy<'_>,
) -> io::Result<Option<String>> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW,
    };

    let destination_path = destination.to_path_buf();
    let destination_exists = fs::symlink_metadata(destination).is_ok();
    let backup = destination_exists
        .then(|| create_save_backup_path(destination))
        .transpose()?;
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let backup_wide = backup.as_ref().map(|backup| {
        backup
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    });
    // SAFETY: both paths are owned, NUL-terminated UTF-16 buffers that remain
    // alive for the duration of the call.
    let replaced = if destination_exists {
        // ReplaceFileW preserves the destination's attributes, DACL,
        // encryption, compression, and named streams.
        unsafe {
            ReplaceFileW(
                destination.as_ptr(),
                source.as_ptr(),
                backup_wide
                    .as_ref()
                    .map_or(std::ptr::null(), |backup| backup.as_ptr()),
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        }
    } else {
        // Do not request replacement here: if another process creates the
        // destination after our check, fail instead of dropping its DACL.
        unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        }
    };
    if replaced == 0 {
        let error = io::Error::last_os_error();
        let backup = backup
            .as_ref()
            .map_or_else(|| "[none]".into(), |path| path.display().to_string());
        return Err(io::Error::new(
            error.kind(),
            format!("{error}; the original backup path is {backup}"),
        ));
    }
    if let (ReplacePolicy::Expected(expected), Some(backup)) = (policy, backup.as_ref()) {
        let displaced = DiskState::inspect(backup).map_err(|error| {
            io::Error::other(format!(
                "cannot inspect displaced file retained at {}: {error}",
                backup.display()
            ))
        })?;
        if displaced
            .as_ref()
            .is_none_or(|displaced| !displaced.matches_displaced(expected))
        {
            let recovery = create_save_backup_path(&destination_path).map_err(|error| {
                io::Error::other(format!(
                    "cannot reserve a recovery path; displaced contents remain at {}: {error}",
                    backup.display()
                ))
            })?;
            let recovery_wide = recovery
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>();
            let backup_wide = backup
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>();
            // SAFETY: all three NUL-terminated path buffers remain live.
            // Restoring with a backup preserves any concurrently changed
            // installed destination at `recovery` rather than deleting it.
            let restored = unsafe {
                ReplaceFileW(
                    destination.as_ptr(),
                    backup_wide.as_ptr(),
                    recovery_wide.as_ptr(),
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
            if restored == 0 {
                return Err(io::Error::other(format!(
                    "destination changed during save; its displaced contents remain at {} and restoration failed: {}",
                    backup.display(),
                    io::Error::last_os_error()
                )));
            }
            return Err(io::Error::other(format!(
                "destination changed during save; its displaced contents were restored and recoverable replacement contents remain at {}",
                recovery.display()
            )));
        }
    }
    let mut warnings = Vec::new();
    if let Some(backup) = backup
        && let Err(error) = fs::remove_file(&backup)
    {
        warnings.push(format!(
            "{} was saved, but its backup {} could not be removed: {error}",
            Path::new(&std::ffi::OsString::from_wide(
                &destination[..destination.len() - 1]
            ))
            .display(),
            backup.display()
        ));
    }
    let destination_path = std::ffi::OsString::from_wide(&destination[..destination.len() - 1]);
    if let Err(error) = OpenOptions::new()
        .read(true)
        .write(true)
        .open(Path::new(&destination_path))
        .and_then(|file| file.sync_all())
    {
        warnings.push(format!(
            "{} was saved, but its replacement metadata could not be synced: {error}",
            Path::new(&destination_path).display()
        ));
    }
    if warnings.is_empty() {
        Ok(None)
    } else {
        Ok(Some(warnings.join("; ")))
    }
}

#[cfg(windows)]
fn create_save_backup_path(destination: &Path) -> io::Result<PathBuf> {
    use windows_sys::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };

    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for _ in 0..100 {
        let mut nonce = [0_u8; 16];
        // SAFETY: nonce is a live writable buffer and the system-preferred
        // provider requires a null algorithm handle.
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                nonce.as_mut_ptr(),
                nonce.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status < 0 {
            return Err(io::Error::other(format!(
                "BCryptGenRandom failed with NTSTATUS {status:#x}"
            )));
        }
        let nonce = nonce
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let candidate = parent.join(format!(".runyte-save-backup-{nonce}"));
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "save backup names exhausted",
    ))
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> io::Result<()> {
    Ok(())
}

impl Buffer {
    pub fn scratch() -> Self {
        Self {
            text: Text::new(),
            path: None,
            dirty: false,
            kind: BufferKind::Scratch,
            directory: None,
            git_log_hints: HashMap::new(),
            longest_line: 0,
            saved_text: Some(Text::new()),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    pub fn open(path: &Path) -> Result<Self> {
        let (contents, disk_state) = read_text_and_state(path, "open")?;
        let text = Text::from_str(&contents);
        Ok(Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: Some(path.to_path_buf()),
            dirty: false,
            kind: BufferKind::File,
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: Some(disk_state),
            disk_generation: 1,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        })
    }

    pub fn external_file_status(&self) -> ExternalFileStatus {
        self.external_status
    }

    pub(crate) fn file_observation_request(&self, buffer: usize) -> Option<FileObservationRequest> {
        (self.kind == BufferKind::File).then(|| FileObservationRequest {
            buffer,
            path: self.path.clone().expect("a file buffer has a path"),
            generation: self.disk_generation,
            baseline_metadata: self.disk_state.as_ref().map(DiskState::metadata_hint),
        })
    }

    pub(crate) fn observe_now(&self, buffer: usize) -> Option<FileObservationEvent> {
        let request = self.file_observation_request(buffer)?;
        Some(FileObservationEvent {
            observation: observe_file(&request.path),
            buffer: request.buffer,
            path: request.path,
            generation: request.generation,
        })
    }

    pub(crate) fn apply_file_observation(
        &mut self,
        event: &FileObservationEvent,
    ) -> ObservationApply {
        if self.kind != BufferKind::File
            || self.path.as_deref() != Some(event.path.as_path())
            || self.disk_generation != event.generation
        {
            return ObservationApply::Ignored;
        }

        if let FileObservation::Text { text, state } = &event.observation {
            if self.disk_state.as_ref() == Some(state) {
                self.clear_external_file_state();
                return ObservationApply::Synchronized;
            }
            if self.text.to_string() == text.as_ref() {
                self.disk_state = Some(state.clone());
                self.disk_generation = self.disk_generation.wrapping_add(1);
                // The text already agrees. Only advance the saved baseline;
                // retaining history lets undo recover the edit that converged.
                self.saved_text = Some(self.text.clone());
                self.dirty = false;
                self.clear_external_file_state();
                return ObservationApply::Converged;
            }
        }

        self.external_status = match &event.observation {
            FileObservation::Text { .. } => ExternalFileStatus::Changed,
            FileObservation::Deleted => ExternalFileStatus::Deleted,
            FileObservation::Binary { .. } => ExternalFileStatus::Binary,
            FileObservation::Unreadable { .. } => ExternalFileStatus::Unreadable,
        };
        let notify = self.last_reported_observation.as_ref() != Some(&event.observation);
        self.external_observation = Some(event.observation.clone());
        if notify {
            self.last_reported_observation = Some(event.observation.clone());
        }
        ObservationApply::Stale { notify }
    }

    pub(crate) fn reload_from_observation(&mut self, observation: &FileObservation) -> Result<()> {
        ensure!(self.kind == BufferKind::File, "buffer is not a file");
        let FileObservation::Text { text, state } = observation else {
            bail!("the current disk version is not editable text");
        };
        self.disk_state = Some(state.clone());
        self.text = Text::from_str(text);
        self.undo.clear();
        self.redo.clear();
        self.undo_group = None;
        self.accept_current_as_disk_baseline();
        Ok(())
    }

    pub(crate) fn observed_revision_key(observation: &FileObservation) -> String {
        match observation {
            FileObservation::Text { state, .. } => state.revision_key(),
            FileObservation::Deleted => "deleted".to_owned(),
            FileObservation::Binary { digest } => format!("binary:{digest}"),
            FileObservation::Unreadable { message } => format!("unreadable:{message}"),
        }
    }

    fn clear_external_file_state(&mut self) {
        self.external_status = ExternalFileStatus::Synchronized;
        self.external_observation = None;
        self.last_reported_observation = None;
    }

    fn accept_current_as_disk_baseline(&mut self) {
        self.mark_saved();
        self.disk_generation = self.disk_generation.wrapping_add(1);
        self.clear_external_file_state();
    }

    pub fn open_directory(path: &Path, show_hidden: bool) -> Result<Self> {
        let (directory, contents) = DirectoryBuffer::open(path.to_path_buf(), show_hidden)?;
        let text = Text::from_str(&contents);
        Ok(Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: Some(path.to_path_buf()),
            dirty: false,
            kind: BufferKind::Directory,
            directory: Some(directory),
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        })
    }

    pub fn virtual_text(name: impl Into<String>, text: &str) -> Self {
        let name = name.into();
        Self::virtual_content(GeneratedViewIdentity::Named(name.clone()), name, text, None)
    }

    pub fn virtual_text_identified(
        identity: GeneratedViewIdentity,
        name: impl Into<String>,
        text: &str,
    ) -> Self {
        Self::virtual_content(identity, name, text, None)
    }

    pub fn workspace_search(
        query: impl Into<String>,
        mode: impl Into<String>,
        text: &str,
        rows: Vec<Option<WorkspaceSearchTarget>>,
        limited: bool,
    ) -> Self {
        let text = Text::from_str(text);
        debug_assert_eq!(rows.len(), text.len_lines());
        Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: None,
            dirty: false,
            kind: BufferKind::WorkspaceSearch {
                query: query.into(),
                mode: mode.into(),
                rows,
                limited,
            },
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    pub fn git_commit(
        oid: impl Into<String>,
        name: impl Into<String>,
        text: &str,
        diff_start: Offset,
    ) -> Self {
        debug_assert!(diff_start <= text.chars().count());
        let text = Text::from_str(text);
        Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: None,
            dirty: false,
            kind: BufferKind::GitCommit {
                oid: oid.into(),
                name: name.into(),
                diff_start,
            },
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    /// Places this buffer's content in the pane rather than against its left
    /// edge.
    ///
    /// Only a read-only buffer can be aligned. Padding is drawn beside text
    /// the person cannot reach with a caret anyway, so a row and a column go
    /// on meaning what they say; in an editable buffer the two would disagree
    /// the moment anyone typed. A request on an editable buffer is refused
    /// here rather than at each call site, so no generated view can align
    /// itself by accident.
    #[must_use]
    pub fn aligned(mut self, alignment: ContentAlignment) -> Self {
        if self.is_read_only() {
            self.layout = ContentLayout::measured(alignment, &self.text.to_string());
        }
        self
    }

    /// Where a pane places this buffer's content.
    pub fn content_layout(&self) -> ContentLayout {
        self.layout
    }

    /// A read-only buffer holding a unified diff.
    pub fn virtual_diff(name: impl Into<String>, text: &str) -> Self {
        let name = name.into();
        Self::virtual_content(
            GeneratedViewIdentity::Named(name.clone()),
            name,
            text,
            Some(0),
        )
    }

    pub fn virtual_diff_identified(
        identity: GeneratedViewIdentity,
        name: impl Into<String>,
        text: &str,
    ) -> Self {
        Self::virtual_content(identity, name, text, Some(0))
    }

    /// A read-only generated view whose unified diff starts after a prefix.
    pub fn virtual_diff_from(name: impl Into<String>, text: &str, diff_start: Offset) -> Self {
        debug_assert!(diff_start <= text.chars().count());
        let name = name.into();
        Self::virtual_content(
            GeneratedViewIdentity::Named(name.clone()),
            name,
            text,
            Some(diff_start),
        )
    }

    /// A commit message, pre-filled with the template a reader edits.
    ///
    /// It starts clean rather than dirty: nothing has been written yet, so
    /// abandoning it should not have to be confirmed.
    pub fn commit_message(text: &str) -> Self {
        let text = Text::from_str(text);
        Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: None,
            dirty: false,
            kind: BufferKind::CommitMessage,
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    /// The changed-file list.
    pub fn git_status(text: &str) -> Self {
        let text = Text::from_str(text);
        Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: None,
            dirty: false,
            kind: BufferKind::GitStatus,
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    /// The local branch list.
    pub fn git_branches(text: &str) -> Self {
        let text = Text::from_str(text);
        Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: None,
            dirty: false,
            kind: BufferKind::GitBranches,
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    /// The registered worktree list.
    pub fn git_worktrees(text: &str) -> Self {
        let text = Text::from_str(text);
        Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: None,
            dirty: false,
            kind: BufferKind::GitWorktrees,
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    pub fn git_log(text: &str) -> Self {
        let text = Text::from_str(text);
        Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: None,
            dirty: false,
            kind: BufferKind::GitLog,
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    pub fn git_blame(text: &str) -> Self {
        let text = Text::from_str(text);
        Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: None,
            dirty: false,
            kind: BufferKind::GitBlame,
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    pub fn git_stash(text: &str) -> Self {
        let text = Text::from_str(text);
        Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: None,
            dirty: false,
            kind: BufferKind::GitStash,
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    fn virtual_content(
        identity: GeneratedViewIdentity,
        name: impl Into<String>,
        text: &str,
        diff_start: Option<Offset>,
    ) -> Self {
        let text = Text::from_str(text);
        Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: None,
            dirty: false,
            kind: BufferKind::Virtual {
                identity,
                name: name.into(),
                diff_start,
            },
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    /// Rendered help for one view.
    pub fn help(text: &str) -> Self {
        let text = Text::from_str(text);
        Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: None,
            dirty: false,
            kind: BufferKind::Help,
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    pub fn settings(text: &str, rows: Vec<Option<SettingId>>) -> Self {
        let text = Text::from_str(text);
        Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: None,
            dirty: false,
            kind: BufferKind::Settings { rows },
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    pub fn notifications(document: NotificationDocument) -> Self {
        let text = Text::from_str(&document.text);
        Self {
            longest_line: text.longest_line_bytes(),
            saved_text: Some(text.clone()),
            text,
            path: None,
            dirty: false,
            kind: BufferKind::Notifications {
                rows: document.rows,
            },
            directory: None,
            git_log_hints: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group: None,
            disk_state: None,
            disk_generation: 0,
            external_status: ExternalFileStatus::Synchronized,
            external_observation: None,
            last_reported_observation: None,
            layout: ContentLayout::default(),
        }
    }

    /// Why a mutation was refused, phrased as the view the reader is looking
    /// at rather than as the buffer kind behind it.
    ///
    /// Read-only-ness is defined here so a new generated buffer cannot become
    /// read-only without also saying what to call itself.
    pub fn read_only_reason(&self) -> Option<&'static str> {
        match &self.kind {
            BufferKind::Virtual {
                diff_start: Some(_),
                ..
            } => Some("this diff is read-only"),
            BufferKind::Virtual { .. } => Some("this buffer is read-only"),
            BufferKind::Help => Some("help is read-only"),
            BufferKind::Settings { .. } => Some("config is read-only"),
            BufferKind::Notifications { .. } => Some("notifications are read-only"),
            BufferKind::GitStatus => Some("the changed-file list is read-only"),
            BufferKind::GitBranches => Some("the branch list is read-only"),
            BufferKind::GitWorktrees => Some("the worktree list is read-only"),
            BufferKind::GitLog => Some("the Git log is read-only"),
            BufferKind::GitBlame => Some("the Git blame view is read-only"),
            BufferKind::GitStash => Some("the Git stash list is read-only"),
            BufferKind::GitCommit { .. } => Some("this commit detail is read-only"),
            BufferKind::WorkspaceSearch { .. } => Some("workspace search results are read-only"),
            _ => None,
        }
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only_reason().is_some()
    }

    /// Whether Runyte assembled this buffer instead of reading ordinary file
    /// text. Clean special buffers have bounded recent-view lifetime and may
    /// be retired once they are detached and fall outside the two most recent;
    /// scratch remains ordinary pathless text.
    pub fn is_special(&self) -> bool {
        !matches!(self.kind, BufferKind::File | BufferKind::Scratch)
    }

    /// Whether this scratch has no text or unsaved state worth retaining.
    pub fn is_empty_clean_scratch(&self) -> bool {
        self.kind == BufferKind::Scratch && self.len_chars() == 0 && !self.dirty
    }

    /// Whether this buffer holds edits that outliving it would lose, and that
    /// the person could keep by saving where they are.
    ///
    /// Narrower than [`Buffer::dirty`], which answers only whether the text has
    /// moved away from its baseline, and is what the `[+]` marker reads. A
    /// scratch buffer is excluded: it has no path, so nothing about it can be
    /// saved in place, and the empty baseline it starts from makes it dirty
    /// after a single keystroke. Counting it would mean a workspace whose only
    /// edited buffer is the scratchpad could never be described as clean, could
    /// never be stopped, and could never retire while idle — for text nobody
    /// asked to keep.
    pub fn holds_unsaved_work(&self) -> bool {
        self.dirty && self.kind != BufferKind::Scratch
    }

    pub fn is_directory(&self) -> bool {
        self.kind == BufferKind::Directory
    }

    pub fn is_help(&self) -> bool {
        self.kind == BufferKind::Help
    }

    pub fn is_manual(&self) -> bool {
        self.generated_view_identity() == Some(&GeneratedViewIdentity::Manual)
    }

    pub fn is_settings(&self) -> bool {
        matches!(self.kind, BufferKind::Settings { .. })
    }

    /// Bytes in the longest line this buffer's text had when it was built.
    pub fn longest_line(&self) -> usize {
        self.longest_line
    }

    /// Whether soft wrapping this buffer is cheap enough to redo every frame.
    ///
    /// Wrapping is computed per logical line and from that line's start, so
    /// its cost follows the longest line rather than the viewport. Under the
    /// limit that cost is invisible; a minified document, where one line is
    /// the whole file, would instead pay a pass over all of it for every frame
    /// it stays on screen, and is shown unwrapped.
    pub fn soft_wrap_viable(&self) -> bool {
        self.longest_line <= SOFT_WRAP_LINE_LIMIT
    }

    pub fn setting_at(&self, row: usize) -> Option<SettingId> {
        let BufferKind::Settings { rows } = &self.kind else {
            return None;
        };
        rows.get(row).copied().flatten()
    }

    pub fn is_notifications(&self) -> bool {
        matches!(self.kind, BufferKind::Notifications { .. })
    }

    pub fn notification_row_at(&self, row: usize) -> Option<NotificationRow> {
        let BufferKind::Notifications { rows } = &self.kind else {
            return None;
        };
        rows.get(row).copied()
    }

    pub fn is_git_branches(&self) -> bool {
        self.kind == BufferKind::GitBranches
    }

    pub fn is_git_worktrees(&self) -> bool {
        self.kind == BufferKind::GitWorktrees
    }

    pub fn is_git_log(&self) -> bool {
        self.kind == BufferKind::GitLog
    }

    pub fn is_git_blame(&self) -> bool {
        self.kind == BufferKind::GitBlame
    }

    pub fn is_git_stash(&self) -> bool {
        self.kind == BufferKind::GitStash
    }

    pub fn is_workspace_search(&self) -> bool {
        matches!(self.kind, BufferKind::WorkspaceSearch { .. })
    }

    pub fn generated_view_identity(&self) -> Option<&GeneratedViewIdentity> {
        let BufferKind::Virtual { identity, .. } = &self.kind else {
            return None;
        };
        Some(identity)
    }

    pub fn is_git_commit_oid(&self, oid: &str) -> bool {
        matches!(&self.kind, BufferKind::GitCommit { oid: current, .. } if current == oid)
    }

    pub fn workspace_search_target_at(&self, row: usize) -> Option<&WorkspaceSearchTarget> {
        let BufferKind::WorkspaceSearch { rows, .. } = &self.kind else {
            return None;
        };
        rows.get(row).and_then(Option::as_ref)
    }

    pub fn replace_workspace_search(
        &mut self,
        query: impl Into<String>,
        mode: impl Into<String>,
        text: &str,
        rows: Vec<Option<WorkspaceSearchTarget>>,
        limited: bool,
    ) -> bool {
        if !self.is_workspace_search() {
            return false;
        }
        self.text = Text::from_str(text);
        debug_assert_eq!(rows.len(), self.text.len_lines());
        self.kind = BufferKind::WorkspaceSearch {
            query: query.into(),
            mode: mode.into(),
            rows,
            limited,
        };
        self.undo.clear();
        self.redo.clear();
        self.undo_group = None;
        self.mark_saved();
        true
    }

    pub fn display_name(&self) -> String {
        match &self.kind {
            BufferKind::File => self
                .path
                .as_ref()
                .map_or_else(|| "[file]".to_owned(), |path| path.display().to_string()),
            BufferKind::Directory => self.path.as_ref().map_or_else(
                || "[directory]".to_owned(),
                |path| format!("{}/", path.display()),
            ),
            BufferKind::Scratch => "[scratch]".to_owned(),
            BufferKind::GitStatus => GIT_STATUS_NAME.to_owned(),
            BufferKind::GitBranches => GIT_BRANCHES_NAME.to_owned(),
            BufferKind::GitWorktrees => GIT_WORKTREES_NAME.to_owned(),
            BufferKind::GitLog => GIT_LOG_NAME.to_owned(),
            BufferKind::GitBlame => GIT_BLAME_NAME.to_owned(),
            BufferKind::GitStash => GIT_STASH_NAME.to_owned(),
            BufferKind::GitCommit { name, .. } => name.clone(),
            BufferKind::WorkspaceSearch { .. } => WORKSPACE_SEARCH_NAME.to_owned(),
            BufferKind::CommitMessage => COMMIT_MESSAGE_NAME.to_owned(),
            BufferKind::Virtual { name, .. } => name.clone(),
            BufferKind::Help => HELP_NAME.to_owned(),
            BufferKind::Settings { .. } => SETTINGS_BUFFER_NAME.to_owned(),
            BufferKind::Notifications { .. } => NOTIFICATIONS_BUFFER_NAME.to_owned(),
        }
    }

    /// The structural identity shown in a pane's top border.
    ///
    /// File and explorer names are paths, so their kind would otherwise be
    /// invisible. Virtual views already carry a bracketed structural name and
    /// retain it unchanged.
    pub fn pane_title(&self) -> String {
        match &self.kind {
            BufferKind::File => self.path.as_ref().map_or_else(
                || "[file]".to_owned(),
                |path| format!("[file] {}", path.display()),
            ),
            BufferKind::Directory => self.path.as_ref().map_or_else(
                || "[explorer]".to_owned(),
                |path| format!("[explorer] {}", path.display()),
            ),
            _ => self.display_name(),
        }
    }

    /// Whether this buffer is a commit message waiting to be written.
    pub fn is_commit_message(&self) -> bool {
        self.kind == BufferKind::CommitMessage
    }

    /// Whether this buffer is the changed-file list.
    pub fn is_git_status(&self) -> bool {
        self.kind == BufferKind::GitStatus
    }

    /// Whether this buffer holds a unified diff to be read as one.
    pub fn is_diff(&self) -> bool {
        self.diff_start().is_some()
    }

    /// The first character offset whose row should be interpreted as a patch.
    pub fn diff_start(&self) -> Option<Offset> {
        match self.kind {
            BufferKind::Virtual { diff_start, .. } => diff_start,
            BufferKind::GitCommit { diff_start, .. } => Some(diff_start),
            _ => None,
        }
    }

    /// Replaces unsaved editable text with its authoritative baseline.
    ///
    /// Discard is intentionally outside the transaction history: after the
    /// person confirms it, undo must not resurrect the changes they chose to
    /// throw away.
    pub fn discard_changes_to(&mut self, text: &str) -> Result<()> {
        if let Some(reason) = self.read_only_reason() {
            bail!("{reason}");
        }
        ensure!(
            !self.is_directory(),
            "directory buffers are managed separately"
        );
        self.text = Text::from_str(text);
        self.undo.clear();
        self.redo.clear();
        self.undo_group = None;
        self.mark_saved();
        Ok(())
    }

    // -- Reading -----------------------------------------------------------

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// A stamp that changes whenever this buffer's text does, including when
    /// it is replaced wholesale by a reload or a discard.
    pub fn revision(&self) -> u64 {
        self.text.revision()
    }

    pub fn len_lines(&self) -> usize {
        self.text.len_lines()
    }

    pub fn last_row(&self) -> usize {
        self.text.last_row()
    }

    pub fn len_chars(&self) -> usize {
        self.text.len_chars()
    }

    pub fn len_bytes(&self) -> usize {
        self.text.len_bytes()
    }

    pub fn line_len(&self, row: usize) -> usize {
        self.text.line_len(row)
    }

    pub fn line_string(&self, row: usize) -> String {
        self.text.line_string(row)
    }

    pub fn lines(&self) -> impl Iterator<Item = String> + '_ {
        self.text.lines()
    }

    pub fn offset_of(&self, position: Position) -> Offset {
        self.text.offset_of(position)
    }

    pub fn position_of(&self, offset: Offset) -> Position {
        self.text.position_of(offset)
    }

    pub fn line_to_offset(&self, row: usize) -> Offset {
        self.text.line_to_offset(row)
    }

    pub fn offset_to_row(&self, offset: Offset) -> usize {
        self.text.offset_to_row(offset)
    }

    pub fn char_at(&self, offset: Offset) -> Option<char> {
        self.text.char_at(offset)
    }

    pub fn slice(&self, from: Offset, to: Offset) -> String {
        self.text.slice_string(from, to)
    }

    pub fn clamp_offset(&self, offset: Offset, insert: bool) -> Offset {
        self.text.clamp_offset(offset, insert)
    }

    /// Clamps a view coordinate. Retained for callers that still think in rows
    /// and columns, such as rendering and the file picker.
    pub fn clamp(&self, position: Position, insert: bool) -> Position {
        let offset = self.text.offset_of(position);
        self.text
            .position_of(self.text.clamp_offset(offset, insert))
    }

    /// Offset of the last character position on a row, for Normal-mode motion.
    pub fn row_end_offset(&self, row: usize, insert: bool) -> Offset {
        let start = self.text.line_to_offset(row);
        let len = self.text.line_len(row);
        if insert {
            start + len
        } else {
            start + len.saturating_sub(1)
        }
    }

    // -- Writing -----------------------------------------------------------

    /// Applies a transaction, recording its inverse for undo.
    ///
    /// Returns `false` when the buffer is read-only or the transaction is a
    /// no-op, in which case no history entry is created.
    pub fn apply(&mut self, transaction: &Transaction) -> bool {
        if self.is_read_only() || transaction.is_empty() {
            return false;
        }
        let directory_before = self.is_directory().then(|| self.text.to_string());
        let revert = self.text.apply(transaction);
        if let (Some(before), Some(directory)) = (directory_before, self.directory.as_mut()) {
            directory.reconcile(&before, &self.text.to_string());
        }
        let inverse = revert.into_transaction();
        if let Some(group) = &mut self.undo_group {
            group.push(inverse);
        } else {
            self.undo.push(vec![inverse]);
            if self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.update_dirty();
        true
    }

    /// Starts collecting subsequent edits into one undo checkpoint.
    pub fn begin_undo_group(&mut self) {
        self.undo_group.get_or_insert_with(Vec::new);
    }

    /// Closes the current checkpoint, if it contains any edits.
    pub fn commit_undo_group(&mut self) {
        let Some(mut group) = self.undo_group.take() else {
            return;
        };
        if group.is_empty() {
            return;
        }
        group.reverse();
        self.undo.push(group);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
    }

    /// Characters of text retained by the undo and redo stacks.
    ///
    /// This is the observable behind the "undo is bounded by edit size, not
    /// document size" property: it must not grow with the length of the buffer.
    pub fn history_footprint(&self) -> usize {
        self.undo
            .iter()
            .chain(&self.redo)
            .flatten()
            .chain(self.undo_group.iter().flatten())
            .map(Transaction::footprint)
            .sum()
    }

    pub fn history_len(&self) -> usize {
        self.undo.len()
            + usize::from(
                self.undo_group
                    .as_ref()
                    .is_some_and(|group| !group.is_empty()),
            )
    }

    pub fn undo(&mut self) -> bool {
        self.undo_with_transactions().is_some()
    }

    /// Undoes one checkpoint and returns the inverse transactions in the
    /// order they were applied, so editor selections can follow the text.
    pub(crate) fn undo_with_transactions(&mut self) -> Option<Vec<Transaction>> {
        if self.is_read_only() {
            return None;
        }
        self.commit_undo_group();
        let group = self.undo.pop()?;
        let mut redo = Vec::with_capacity(group.len());
        for transaction in &group {
            let directory_before = self.is_directory().then(|| self.text.to_string());
            let revert = self.text.apply(transaction);
            if let (Some(before), Some(directory)) = (directory_before, self.directory.as_mut()) {
                directory.reconcile(&before, &self.text.to_string());
            }
            redo.push(revert.into_transaction());
        }
        redo.reverse();
        self.redo.push(redo);
        self.update_dirty();
        Some(group)
    }

    pub fn redo(&mut self) -> bool {
        self.redo_with_transactions().is_some()
    }

    /// Redoes one checkpoint and returns its applied transactions.
    pub(crate) fn redo_with_transactions(&mut self) -> Option<Vec<Transaction>> {
        if self.is_read_only() {
            return None;
        }
        self.commit_undo_group();
        let group = self.redo.pop()?;
        let mut undo = Vec::with_capacity(group.len());
        for transaction in &group {
            let directory_before = self.is_directory().then(|| self.text.to_string());
            let revert = self.text.apply(transaction);
            if let (Some(before), Some(directory)) = (directory_before, self.directory.as_mut()) {
                directory.reconcile(&before, &self.text.to_string());
            }
            undo.push(revert.into_transaction());
        }
        undo.reverse();
        self.undo.push(undo);
        self.update_dirty();
        Some(group)
    }

    /// Replaces the contents of a read-only virtual buffer.
    ///
    /// Virtual buffers are projections of durable state, so this bypasses the
    /// transaction history deliberately: there is nothing for a person to undo.
    pub fn replace_virtual_text(&mut self, text: &str) -> bool {
        if !self.is_read_only() {
            return false;
        }
        self.text = Text::from_str(text);
        // Alignment survives a reprojection, but the width it centres does
        // not: new text is a new block.
        self.layout = self.layout.remeasured(text);
        self.undo.clear();
        self.redo.clear();
        self.undo_group = None;
        self.mark_saved();
        true
    }

    /// Replaces a Git-log buffer's text and its per-row branch/tag hints
    /// together, so a hint can never point at a row whose text has moved on.
    pub fn replace_git_log_text(
        &mut self,
        text: &str,
        hints: impl IntoIterator<Item = (usize, String)>,
    ) -> bool {
        if !self.replace_virtual_text(text) {
            return false;
        }
        self.git_log_hints = hints.into_iter().collect();
        true
    }

    /// Writes the buffer back to its own path.
    ///
    /// Refuses when the file changed underneath the buffer unless `replace` is
    /// set. A `git checkout`, a rebase, or another editor can rewrite a file
    /// while it sits open here, and writing over that discards the newer
    /// contents with no record of them. `:write!` overwrites anyway, `:reload`
    /// takes the newer file instead.
    pub fn save(&mut self, replace: bool) -> Result<SaveOutcome> {
        self.save_with(replace, atomic_write)
    }

    pub(crate) fn save_checked(
        &mut self,
        replace: bool,
        expected_identity: PathBuf,
    ) -> Result<SaveOutcome> {
        self.save_with(replace, move |path, contents, policy| {
            atomic_write_checked(path, contents, policy, &expected_identity)
        })
    }

    fn save_with(
        &mut self,
        replace: bool,
        write: impl FnOnce(&Path, &[u8], ReplacePolicy<'_>) -> Result<AtomicWriteStatus>,
    ) -> Result<SaveOutcome> {
        if let Some(reason) = self.read_only_reason() {
            bail!("{reason}");
        }
        anyhow::ensure!(
            !self.is_directory(),
            "directory buffers require a confirmed filesystem plan"
        );
        let path = self
            .path
            .as_ref()
            .context("buffer has no path; use :write <path>")?;
        if !replace && let Some(recorded) = &self.disk_state {
            ensure_disk_unchanged(path, Some(recorded))?;
        }
        let policy = match (replace, self.disk_state.as_ref()) {
            (true, _) => ReplacePolicy::Force,
            (false, Some(state)) => ReplacePolicy::Expected(state),
            (false, None) => ReplacePolicy::NoReplace,
        };
        let mut write_status = write(path, self.text.to_string().as_bytes(), policy)?;
        match DiskState::inspect(path) {
            Ok(Some(current)) if current.same_contents(&write_status.installed_state) => {
                self.disk_state = Some(current);
                self.accept_current_as_disk_baseline();
            }
            Ok(_) => write_status.warn(format!(
                "{} was saved, but changed again before Runyte could verify it; the buffer remains modified",
                path.display()
            )),
            Err(error) => write_status.warn(format!(
                "{} was saved, but could not be verified ({error}); the buffer remains modified",
                path.display()
            )),
        }
        Ok(write_status.finish())
    }

    /// Writes the buffer to `path`, taking it over as the buffer's identity.
    ///
    /// Refuses to replace an unrelated existing file unless `replace` is set.
    /// `:write <path>` takes a path the person typed, so the target is as
    /// likely to be a typo as an intention, and the write is not recoverable:
    /// unlike an explorer plan it neither stages nor trashes what it destroys.
    /// `:write! <path>` is the way to mean it.
    pub fn save_as(&mut self, path: PathBuf, replace: bool) -> Result<SaveOutcome> {
        let same_path = self.has_path_identity(&path);
        let expected = self.disk_state.clone();
        self.save_as_with(path, replace, move |path, contents| {
            let policy = match (replace, same_path, expected.as_ref()) {
                (true, _, _) => ReplacePolicy::Force,
                (false, true, Some(state)) => ReplacePolicy::Expected(state),
                (false, _, _) => ReplacePolicy::NoReplace,
            };
            atomic_write(path, contents, policy)
        })
    }

    pub(crate) fn save_as_checked(
        &mut self,
        path: PathBuf,
        replace: bool,
        expected_identity: PathBuf,
    ) -> Result<SaveOutcome> {
        let same_path = self.has_path_identity(&path);
        let expected_state = self.disk_state.clone();
        self.save_as_with(path, replace, move |path, contents| {
            let policy = match (replace, same_path, expected_state.as_ref()) {
                (true, _, _) => ReplacePolicy::Force,
                (false, true, Some(state)) => ReplacePolicy::Expected(state),
                (false, _, _) => ReplacePolicy::NoReplace,
            };
            atomic_write_checked(path, contents, policy, &expected_identity)
        })
    }

    fn save_as_with(
        &mut self,
        path: PathBuf,
        replace: bool,
        write: impl FnOnce(&Path, &[u8]) -> Result<AtomicWriteStatus>,
    ) -> Result<SaveOutcome> {
        if let Some(reason) = self.read_only_reason() {
            bail!("{reason}");
        }
        anyhow::ensure!(
            !self.is_directory(),
            "directory buffers cannot be written to another path"
        );
        if !replace && !self.has_path_identity(&path) {
            // `symlink_metadata` so a dangling symlink still counts as taken;
            // writing through it would create the file it points at.
            anyhow::ensure!(
                fs::symlink_metadata(&path).is_err(),
                "{} already exists; use :write! to replace it",
                path.display()
            );
        }
        // Write before changing the buffer identity. A failed save-as must
        // remain attached to its original path and language-derived services.
        let mut write_status = write(&path, self.text.to_string().as_bytes())?;
        self.path = Some(path);
        self.kind = BufferKind::File;
        self.directory = None;
        let path = self
            .path
            .as_deref()
            .expect("save-as just assigned its path");
        match DiskState::inspect(path) {
            Ok(Some(current)) if current.same_contents(&write_status.installed_state) => {
                self.disk_state = Some(current);
                self.accept_current_as_disk_baseline();
            }
            Ok(_) => write_status.warn(format!(
                "{} was saved, but changed again before Runyte could verify it; the buffer remains modified",
                path.display()
            )),
            Err(error) => write_status.warn(format!(
                "{} was saved, but could not be verified ({error}); the buffer remains modified",
                path.display()
            )),
        }
        Ok(write_status.finish())
    }

    fn has_path_identity(&self, requested: &Path) -> bool {
        let Some(current) = self.path.as_deref() else {
            return false;
        };
        if current == requested {
            return true;
        }
        match (
            crate::path_safety::path_identity(current),
            crate::path_safety::path_identity(requested),
        ) {
            (Ok(current), Ok(requested)) => current == requested,
            _ => false,
        }
    }

    pub(crate) fn owns_path_identity(&self, requested: &Path) -> bool {
        self.has_path_identity(requested)
    }

    /// Replaces a file buffer with its current contents on disk.
    ///
    /// The read completes before any in-memory state changes, so a missing or
    /// unreadable file leaves the buffer and its undo history untouched.
    pub fn reload(&mut self) -> Result<()> {
        ensure!(self.kind == BufferKind::File, "buffer is not a file");
        let path = self.path.as_ref().context("buffer has no path")?;
        let (contents, disk_state) = read_text_and_state(path, "reload")?;
        self.disk_state = Some(disk_state);
        self.text = Text::from_str(&contents);
        self.undo.clear();
        self.redo.clear();
        self.undo_group = None;
        self.accept_current_as_disk_baseline();
        Ok(())
    }

    pub fn directory_plan(&self) -> Result<FsPlan> {
        self.directory
            .as_ref()
            .context("buffer is not a directory")?
            .plan(&self.text.to_string())
    }

    pub fn directory_root(&self) -> Option<&Path> {
        self.directory.as_ref().map(DirectoryBuffer::root)
    }

    pub fn directory_entry_path(&self, row: usize) -> Result<Option<PathBuf>> {
        self.directory
            .as_ref()
            .context("buffer is not a directory")?
            .entry_path(&self.text.to_string(), row)
    }

    pub fn directory_row_is_directory(&self, row: usize) -> bool {
        let line = self.line_string(row);
        self.directory_line_is_directory(row, &line)
    }

    pub fn directory_line_is_directory(&self, row: usize, line: &str) -> bool {
        self.directory
            .as_ref()
            .and_then(|directory| directory.entry_kind_at_line(line, row).ok())
            .flatten()
            == Some(crate::fs_plan::EntryKind::Directory)
    }

    /// The read-only annotations this buffer's rows carry.
    ///
    /// An explorer annotates each symlink with what it points at, and every
    /// annotated row shares one column since filenames stay close enough in
    /// length for that to read as a table. A Git log instead trails its
    /// paging keys and each commit's branch and tag refs one space past that
    /// row's own text with no shared column: commit subjects vary too widely
    /// in length for a shared column to work, and a page's usual short rows
    /// must not lose their hints off the right edge just because one commit
    /// elsewhere has an unusually long subject. Other buffer kinds have
    /// nothing to say yet.
    pub fn row_hints(&self) -> RowHints {
        if self.is_git_log() {
            let heading = self.line_string(0);
            let cells = display_cells(&heading);
            let hint = [
                "(Ctrl-n/p: next/prev page)",
                "(Ctrl-n/p: next/prev)",
                "(Ctrl-n/p: pages)",
            ]
            .into_iter()
            .find(|hint| cells + 1 + display_cells(hint) <= 80)
            .unwrap_or("(Ctrl-n/p: pages)");
            let mut entries = vec![(0, hint.to_owned())];
            entries.extend(
                self.git_log_hints
                    .iter()
                    .map(|(&line, text)| (line, text.clone())),
            );
            return RowHints::trailing(entries);
        }
        let Some(directory) = self.directory.as_ref() else {
            return RowHints::default();
        };
        RowHints::aligned(
            directory
                .symlink_targets()
                .into_iter()
                .map(|(row, target)| {
                    (
                        row,
                        display_cells(&self.line_string(row)),
                        format!("→ {}", target.display()),
                    )
                }),
        )
    }

    pub fn directory_transfer_at(&self, row: usize) -> Result<Option<DirectoryTransfer>> {
        self.directory
            .as_ref()
            .context("buffer is not a directory")?
            .transfer_at(&self.text.to_string(), row)
    }

    pub fn assign_directory_transfers(
        &mut self,
        start_row: usize,
        transfers: &[DirectoryTransfer],
        mode: TransferMode,
    ) -> Result<()> {
        self.directory
            .as_mut()
            .context("buffer is not a directory")?
            .assign_transfers(start_row, transfers, mode)
    }

    pub fn pending_directory_move_sources(&self) -> std::collections::HashSet<PathBuf> {
        self.directory
            .as_ref()
            .map(DirectoryBuffer::pending_move_sources)
            .unwrap_or_default()
    }

    pub fn reload_directory(&mut self, show_hidden: bool) -> Result<()> {
        let text = self
            .directory
            .as_mut()
            .context("buffer is not a directory")?
            .reload(show_hidden)?;
        self.text = Text::from_str(&text);
        self.undo.clear();
        self.redo.clear();
        self.undo_group = None;
        self.mark_saved();
        Ok(())
    }

    /// Refreshes a successfully applied directory plan without replacing the
    /// edited row order. The current projection becomes the clean saved text;
    /// a later reload re-renders the sorted filesystem snapshot.
    pub fn accept_directory_plan(&mut self, show_hidden: bool) -> Result<()> {
        let text = self.text.to_string();
        let text = self
            .directory
            .as_mut()
            .context("buffer is not a directory")?
            .refresh_baseline_preserving_order(&text, show_hidden)?;
        self.text = Text::from_str(&text);
        self.undo.clear();
        self.redo.clear();
        self.undo_group = None;
        self.mark_saved();
        Ok(())
    }

    pub fn rebase_directory_after_external_removals(
        &mut self,
        removed: &std::collections::HashSet<PathBuf>,
    ) -> Result<bool> {
        self.directory
            .as_mut()
            .context("buffer is not a directory")?
            .rebase_after_external_removals(removed)
    }

    pub fn retarget_path(&mut self, path: PathBuf) {
        // A rename changes Unix ctime even when the same inode and contents
        // arrived unchanged at the confirmed destination. Advance the saved
        // state only when every stable property still matches; a race or an
        // unrelated replacement then remains a conflict on the next save.
        if let Some(expected) = self.disk_state.as_ref()
            && let Ok(Some(current)) = DiskState::inspect(&path)
            && current.matches_displaced(expected)
        {
            self.disk_state = Some(current);
        }
        self.path = Some(path.clone());
        self.disk_generation = self.disk_generation.wrapping_add(1);
        self.clear_external_file_state();
        if let Some(directory) = &mut self.directory {
            directory.retarget_root(path);
        }
    }

    /// Points this directory buffer at another directory.
    ///
    /// Navigation retargets rather than opening a second buffer, so a pane
    /// keeps one explorer however far it walks. Undo history is dropped with
    /// the old listing: the two directories share no text, and an undo across
    /// the boundary would restore entries that were never in this one.
    pub fn retarget_directory(&mut self, path: &Path, show_hidden: bool) -> Result<()> {
        ensure!(self.is_directory(), "buffer is not a directory");
        let (directory, contents) = DirectoryBuffer::open(path.to_path_buf(), show_hidden)?;
        self.directory = Some(directory);
        self.path = Some(path.to_path_buf());
        self.text = Text::from_str(&contents);
        self.undo.clear();
        self.redo.clear();
        self.undo_group = None;
        self.mark_saved();
        Ok(())
    }

    pub fn mark_saved(&mut self) {
        self.saved_text = Some(self.text.clone());
        self.dirty = false;
    }

    fn update_dirty(&mut self) {
        self.dirty = self
            .saved_text
            .as_ref()
            .is_none_or(|saved| !self.text.same_content(saved));
    }

    #[cfg(test)]
    pub fn set_text(&mut self, value: &str) {
        self.text = Text::from_str(value);
        self.undo.clear();
        self.redo.clear();
        self.undo_group = None;
        self.mark_saved();
    }
}

impl std::fmt::Display for Buffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.text)
    }
}

pub fn ordered(a: Position, b: Position) -> (Position, Position) {
    if a <= b { (a, b) } else { (b, a) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Change;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn buffer_with(text: &str) -> Buffer {
        let mut buffer = Buffer::scratch();
        buffer.set_text(text);
        buffer
    }

    #[test]
    fn pane_titles_prefix_paths_with_their_structural_buffer_kind() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-pane-title-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("notes.txt");
        fs::write(&path, "notes").unwrap();

        assert_eq!(
            Buffer::open(&path).unwrap().pane_title(),
            format!("[file] {}", path.display())
        );
        assert_eq!(
            Buffer::open_directory(&directory, false)
                .unwrap()
                .pane_title(),
            format!("[explorer] {}", directory.display())
        );
        assert_eq!(Buffer::git_status("").pane_title(), "[git status]");
        assert_eq!(Buffer::git_branches("").pane_title(), "[git branches]");
        assert_eq!(Buffer::git_log("").pane_title(), "[git log]");
        assert_eq!(Buffer::git_blame("").pane_title(), "[git blame]");

        fs::remove_dir_all(directory).unwrap();
    }

    /// Every refusal names the view the reader is looking at. Reporting the
    /// theme list as a "virtual buffer" describes an implementation detail
    /// nobody reading it has seen.
    #[test]
    fn a_read_only_refusal_names_its_own_view() {
        for (buffer, reason) in [
            (Buffer::git_status(""), "the changed-file list is read-only"),
            (Buffer::git_branches(""), "the branch list is read-only"),
            (Buffer::virtual_diff("diff", ""), "this diff is read-only"),
            (Buffer::virtual_text("out", ""), "this buffer is read-only"),
        ] {
            assert_eq!(buffer.read_only_reason(), Some(reason));
            assert_eq!(
                buffer
                    .clone()
                    .discard_changes_to("")
                    .unwrap_err()
                    .to_string(),
                reason
            );
        }

        // An editable buffer refuses nothing, so it has no reason to give.
        assert_eq!(Buffer::scratch().read_only_reason(), None);
        assert_eq!(Buffer::commit_message("").read_only_reason(), None);
    }

    /// Word completion contributes candidates from every buffer that is not
    /// read-only, and shows its own popup only in the buffers among those
    /// where the text is authored prose rather than a filename being renamed.
    /// A kind that forgets to declare itself in `read_only_reason` would
    /// silently leak generated text into (or a real one out of) the index, so
    /// this is checked against every `BufferKind` rather than a sample.
    #[test]
    fn word_completion_eligibility_matches_read_only_and_directory_status() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-word-completion-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("notes.txt");
        fs::write(&path, "notes").unwrap();

        let buffers = [
            ("file", Buffer::open(&path).unwrap(), true, true),
            (
                "directory",
                Buffer::open_directory(&directory, false).unwrap(),
                true,
                false,
            ),
            ("scratch", Buffer::scratch(), true, true),
            ("commit message", Buffer::commit_message(""), true, true),
            (
                "virtual text",
                Buffer::virtual_text("out", ""),
                false,
                false,
            ),
            (
                "virtual diff",
                Buffer::virtual_diff("diff", ""),
                false,
                false,
            ),
            ("git status", Buffer::git_status(""), false, false),
            ("git branches", Buffer::git_branches(""), false, false),
            ("git worktrees", Buffer::git_worktrees(""), false, false),
            ("git log", Buffer::git_log(""), false, false),
            ("git blame", Buffer::git_blame(""), false, false),
            ("git stash", Buffer::git_stash(""), false, false),
            (
                "git commit",
                Buffer::git_commit("oid", "name", "", 0),
                false,
                false,
            ),
            (
                "workspace search",
                Buffer::workspace_search("q", "mode", "", vec![None], false),
                false,
                false,
            ),
            ("help", Buffer::help(""), false, false),
            ("settings", Buffer::settings("", Vec::new()), false, false),
            (
                "notifications",
                Buffer::notifications(NotificationDocument {
                    text: String::new(),
                    rows: Vec::new(),
                }),
                false,
                false,
            ),
        ];

        for (name, buffer, contributes, shows_popup) in buffers {
            assert_eq!(
                !buffer.is_read_only(),
                contributes,
                "{name}: contributes-to-index mismatch"
            );
            assert_eq!(
                !buffer.is_read_only() && !buffer.is_directory(),
                shows_popup,
                "{name}: shows-popup mismatch"
            );
        }

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn applying_a_transaction_marks_the_buffer_dirty() {
        let mut buffer = buffer_with("abc");
        assert!(buffer.apply(&Transaction::insert(0, "x")));
        assert_eq!(buffer.to_string(), "xabc");
        assert!(buffer.dirty);
    }

    #[test]
    fn an_edited_scratch_buffer_is_dirty_but_holds_no_unsaved_work() {
        let mut scratch = Buffer::scratch();
        assert!(!scratch.dirty);
        assert!(!scratch.holds_unsaved_work());

        assert!(scratch.apply(&Transaction::insert(0, "a note to self")));
        assert!(scratch.dirty);
        assert!(!scratch.holds_unsaved_work());

        // A buffer with a file behind it keeps counting, and emptying the
        // scratchpad back to its baseline clears its own marker.
        let mut file = buffer_with("saved");
        file.kind = BufferKind::File;
        file.path = Some(PathBuf::from("/nowhere/note.txt"));
        assert!(file.apply(&Transaction::insert(0, "x")));
        assert!(file.holds_unsaved_work());

        assert!(scratch.apply(&Transaction::delete(0, scratch.len_chars())));
        assert!(!scratch.dirty);
    }

    #[test]
    fn failed_save_as_preserves_the_original_buffer_identity() {
        let mut buffer = buffer_with("unsaved");
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let original = std::env::temp_dir().join(format!(
            "runyte-buffer-save-as-{}-{unique}-original.sh",
            std::process::id(),
        ));
        buffer.path = Some(original.clone());
        buffer.kind = BufferKind::File;
        buffer.dirty = true;
        let missing_parent = std::env::temp_dir().join(format!(
            "runyte-buffer-save-as-{}-{unique}-missing",
            std::process::id(),
        ));
        let requested = missing_parent.join("new.rs");

        assert!(buffer.save_as(requested, false).is_err());
        assert_eq!(buffer.path.as_deref(), Some(original.as_path()));
        assert_eq!(buffer.kind, BufferKind::File);
        assert!(buffer.dirty);
        assert_eq!(buffer.to_string(), "unsaved");
    }

    #[test]
    fn a_failed_temporary_write_leaves_the_destination_intact() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-atomic-failure-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("notes.txt");
        fs::write(&path, "the prior complete contents").unwrap();

        let error = atomic_write_with(
            &path,
            b"replacement contents",
            ReplacePolicy::Force,
            |file, contents| {
                file.write_all(&contents[..4])?;
                Err(io::Error::other("injected write failure"))
            },
            sync_parent,
        )
        .unwrap_err();

        assert!(error.to_string().contains("failed to write"), "{error:#}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "the prior complete contents"
        );
        assert_eq!(
            fs::read_dir(&directory).unwrap().count(),
            1,
            "the failed temporary file was not cleaned up"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_retained_replacement_survives_automatic_temporary_cleanup() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "runyte-buffer-retained-replacement-{}-{unique}",
            std::process::id()
        ));
        fs::write(&path, "complete replacement").unwrap();

        let retained = SaveTemporary(Some(path.clone())).keep();

        assert_eq!(retained, path);
        assert_eq!(
            fs::read_to_string(&retained).unwrap(),
            "complete replacement"
        );
        fs::remove_file(retained).unwrap();
    }

    #[test]
    fn a_post_commit_sync_error_keeps_save_state_aligned_with_disk() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-sync-failure-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("notes.txt");
        fs::write(&path, "original").unwrap();
        let mut buffer = Buffer::open(&path).unwrap();
        assert!(buffer.apply(&Transaction::insert(0, "edited ")));

        let outcome = buffer
            .save_with(false, |path, contents, expected| {
                atomic_write_with(
                    path,
                    contents,
                    expected,
                    |file, contents| file.write_all(contents),
                    |_| Err(io::Error::other("injected directory sync failure")),
                )
            })
            .unwrap();

        assert!(
            matches!(
                outcome,
                SaveOutcome::CommittedWithWarning(ref warning)
                    if warning.contains("injected directory sync failure")
            ),
            "{outcome:?}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "edited original");
        assert!(!buffer.dirty);
        assert_eq!(buffer.disk_state, DiskState::inspect(&path).unwrap());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_post_commit_sync_error_still_adopts_a_save_as_path() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-save-as-sync-failure-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("notes.txt");
        let mut buffer = buffer_with("contents");

        let outcome = buffer
            .save_as_with(path.clone(), false, |path, contents| {
                atomic_write_with(
                    path,
                    contents,
                    ReplacePolicy::NoReplace,
                    |file, contents| file.write_all(contents),
                    |_| Err(io::Error::other("injected directory sync failure")),
                )
            })
            .unwrap();

        assert!(
            matches!(
                outcome,
                SaveOutcome::CommittedWithWarning(ref warning)
                    if warning.contains("injected directory sync failure")
            ),
            "{outcome:?}"
        );
        assert_eq!(buffer.path.as_deref(), Some(path.as_path()));
        assert_eq!(buffer.kind, BufferKind::File);
        assert!(!buffer.dirty);
        assert_eq!(fs::read_to_string(&path).unwrap(), "contents");
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn saving_through_a_symlink_preserves_the_link_and_target_permissions() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-atomic-symlink-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let target = directory.join("target.txt");
        let link = directory.join("link.txt");
        fs::write(&target, "original").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        let original_metadata = fs::metadata(&target).unwrap();
        symlink("target.txt", &link).unwrap();
        let mut buffer = Buffer::open(&link).unwrap();
        assert!(buffer.apply(&Transaction::insert(0, "edited ")));

        buffer.save(false).unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "edited original");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let saved_metadata = fs::metadata(&target).unwrap();
        assert_eq!(saved_metadata.uid(), original_metadata.uid());
        assert_eq!(saved_metadata.gid(), original_metadata.gid());
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_new_save_temporary_is_private_before_contents_are_written() {
        use std::os::unix::fs::PermissionsExt;

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-private-temporary-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("new.txt");

        let status = atomic_write_with(
            &path,
            b"private contents",
            ReplacePolicy::NoReplace,
            |file, contents| {
                assert_eq!(file.metadata()?.permissions().mode() & 0o777, 0o600);
                file.write_all(contents)
            },
            sync_parent,
        )
        .unwrap();

        assert_eq!(status.finish(), SaveOutcome::Durable);
        assert_eq!(fs::read_to_string(&path).unwrap(), "private contents");
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_save_does_not_bypass_a_non_writable_destination() {
        use std::os::unix::fs::PermissionsExt;

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-readonly-destination-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("readonly.txt");
        fs::write(&path, "original").unwrap();
        let mut buffer = Buffer::open(&path).unwrap();
        assert!(buffer.apply(&Transaction::insert(0, "edited ")));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();

        let error = buffer.save(true).unwrap_err();

        assert!(error.to_string().contains("for writing"), "{error:#}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
        assert!(buffer.dirty);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_replacement_restores_special_permission_bits_after_writing() {
        use std::os::unix::fs::PermissionsExt;

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-atomic-mode-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("script");
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o4755)).unwrap();
        if fs::metadata(&path).unwrap().permissions().mode() & 0o7777 != 0o4755 {
            // Sandboxed Darwin processes may report a successful chmod while
            // the sandbox strips special mode bits. There is no preservation
            // behavior to test when the fixture cannot acquire them.
            fs::remove_dir_all(directory).unwrap();
            return;
        }

        assert_eq!(
            atomic_write(&path, b"#!/bin/sh\nexit 0\n", ReplacePolicy::Force)
                .unwrap()
                .finish(),
            SaveOutcome::Durable
        );

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o4755
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[test]
    fn atomic_replacement_saves_files_without_extended_acls() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-atomic-no-acl-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("plain.txt");
        fs::write(&path, "original").unwrap();

        assert_eq!(
            atomic_write(&path, b"replacement", ReplacePolicy::Force)
                .unwrap()
                .finish(),
            SaveOutcome::Durable
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "replacement");
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn atomic_replacement_preserves_a_posix_access_acl() {
        use std::os::fd::AsRawFd;

        fn entry(bytes: &mut Vec<u8>, tag: u16, permissions: u16, id: u32) {
            bytes.extend_from_slice(&tag.to_le_bytes());
            bytes.extend_from_slice(&permissions.to_le_bytes());
            bytes.extend_from_slice(&id.to_le_bytes());
        }

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-atomic-acl-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("shared.txt");
        fs::write(&path, "original").unwrap();
        let file = File::open(&path).unwrap();
        let mut acl = 2_u32.to_le_bytes().to_vec();
        entry(&mut acl, 0x01, 0o6, u32::MAX);
        // SAFETY: getuid has no preconditions.
        entry(&mut acl, 0x02, 0o4, unsafe { libc::getuid() });
        entry(&mut acl, 0x04, 0o0, u32::MAX);
        entry(&mut acl, 0x10, 0o4, u32::MAX);
        entry(&mut acl, 0x20, 0o0, u32::MAX);
        const ACL_NAME: &[u8] = b"system.posix_acl_access\0";
        // SAFETY: the descriptor and the NUL-terminated name are valid, and
        // acl owns the supplied bytes.
        let set = unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                ACL_NAME.as_ptr().cast(),
                acl.as_ptr().cast(),
                acl.len(),
                0,
            )
        };
        if set != 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EOPNOTSUPP) {
            fs::remove_dir_all(directory).unwrap();
            return;
        }
        assert_eq!(set, 0, "{}", io::Error::last_os_error());
        drop(file);

        assert_eq!(
            atomic_write(&path, b"replacement", ReplacePolicy::Force)
                .unwrap()
                .finish(),
            SaveOutcome::Durable
        );

        let saved = File::open(&path).unwrap();
        let mut actual = vec![0_u8; acl.len()];
        // SAFETY: actual owns enough writable bytes for the known ACL.
        let read = unsafe {
            libc::fgetxattr(
                saved.as_raw_fd(),
                ACL_NAME.as_ptr().cast(),
                actual.as_mut_ptr().cast(),
                actual.len(),
            )
        };
        assert_eq!(read, acl.len() as isize, "{}", io::Error::last_os_error());
        assert_eq!(actual, acl);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saving_refuses_a_file_that_changed_underneath_the_buffer() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "runyte-buffer-stale-{}-{unique}.txt",
            std::process::id(),
        ));
        fs::write(&path, "original\n").unwrap();
        let mut buffer = Buffer::open(&path).unwrap();
        buffer.apply(&Transaction::insert(0, "edited "));

        // Something else rewrites the file: a checkout, a rebase, another
        // editor. The length differs, so this does not depend on clock
        // granularity.
        fs::write(&path, "a different version entirely\n").unwrap();

        let refused = buffer.save(false).unwrap_err();

        assert!(
            refused.to_string().contains("changed on disk"),
            "unexpected message: {refused}"
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "a different version entirely\n",
            "the newer file was overwritten"
        );
        assert!(
            buffer.dirty,
            "a refused save must not mark the buffer clean"
        );

        // `:write!` means it, and the buffer agrees with the file again after.
        buffer.save(true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "edited original\n");
        assert!(!buffer.dirty);

        buffer.apply(&Transaction::insert(0, "more "));
        buffer.save(false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "more edited original\n");

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn reload_rejects_binary_replacement_without_changing_live_state() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "runyte-buffer-binary-reload-{}-{unique}.txt",
            std::process::id()
        ));
        fs::write(&path, "original\n").unwrap();
        let mut buffer = Buffer::open(&path).unwrap();
        assert!(buffer.apply(&Transaction::insert(0, "unsaved ")));
        let revision = buffer.revision();
        fs::write(&path, b"replacement\0binary").unwrap();

        let error = buffer.reload().unwrap_err();

        assert!(error.is::<BinaryFileError>(), "{error:#}");
        assert_eq!(buffer.to_string(), "unsaved original\n");
        assert_eq!(buffer.revision(), revision);
        assert!(buffer.dirty);
        assert!(buffer.undo(), "a refused reload must retain undo history");
        assert_eq!(buffer.to_string(), "original\n");
        assert!(!buffer.dirty);
        fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn save_as_accepts_an_alias_of_the_buffers_current_file() {
        use std::os::unix::fs::symlink;

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-save-as-alias-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("target.txt");
        let alias = directory.join("alias.txt");
        fs::write(&target, "original\n").unwrap();
        symlink("target.txt", &alias).unwrap();
        let mut buffer = Buffer::open(&target).unwrap();
        assert!(buffer.apply(&Transaction::insert(0, "edited ")));

        buffer.save_as(alias.clone(), false).unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "edited original\n");
        assert_eq!(buffer.path.as_deref(), Some(alias.as_path()));
        assert!(!buffer.dirty);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn retargeting_does_not_accept_an_unrelated_destination_state() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-retarget-conflict-{}-{unique}",
            std::process::id(),
        ));
        fs::create_dir(&directory).unwrap();
        let source = directory.join("source.txt");
        let destination = directory.join("destination.txt");
        fs::write(&source, "original\n").unwrap();
        let mut buffer = Buffer::open(&source).unwrap();
        fs::write(&destination, "unrelated destination\n").unwrap();

        buffer.retarget_path(destination.clone());
        assert!(buffer.apply(&Transaction::insert(0, "edited ")));
        let error = buffer.save(false).unwrap_err();

        assert!(error.to_string().contains("changed on disk"), "{error:#}");
        assert_eq!(
            fs::read_to_string(&destination).unwrap(),
            "unrelated destination\n"
        );
        assert!(buffer.dirty);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saving_detects_a_same_size_rewrite_with_a_preserved_timestamp() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "runyte-buffer-stale-digest-{}-{unique}.txt",
            std::process::id(),
        ));
        fs::write(&path, "aaaaaaaa\n").unwrap();
        let mut buffer = Buffer::open(&path).unwrap();
        let recorded = buffer.disk_state.clone().unwrap();
        assert!(buffer.apply(&Transaction::insert(0, "edited ")));

        fs::write(&path, "bbbbbbbb\n").unwrap();
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        file.set_times(fs::FileTimes::new().set_modified(recorded.modified.unwrap()))
            .unwrap();
        let current = DiskState::inspect(&path).unwrap().unwrap();
        assert_eq!(current.len, recorded.len);
        assert_eq!(current.modified, recorded.modified);
        assert_eq!(current.identity, recorded.identity);
        assert_ne!(current.digest, recorded.digest);

        let error = buffer.save(false).unwrap_err();
        assert!(error.to_string().contains("changed on disk"), "{error:#}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "bbbbbbbb\n");
        assert!(buffer.dirty);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn atomic_save_rechecks_the_destination_after_writing_its_temporary() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-save-race-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("notes.txt");
        fs::write(&path, "original\n").unwrap();
        let mut buffer = Buffer::open(&path).unwrap();
        assert!(buffer.apply(&Transaction::insert(0, "edited ")));

        let error = buffer
            .save_with(false, |path, contents, expected| {
                atomic_write_with(
                    path,
                    contents,
                    expected,
                    |file, contents| {
                        file.write_all(contents)?;
                        fs::write(path, "external\n")
                    },
                    sync_parent,
                )
            })
            .unwrap_err();

        assert!(
            format!("{error:#}").contains("destination changed"),
            "{error:#}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "external\n");
        assert!(buffer.dirty);
        let retained = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|candidate| candidate != &path)
            .expect("the complete replacement is retained for recovery");
        assert_eq!(fs::read_to_string(retained).unwrap(), "edited original\n");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_post_commit_change_does_not_mark_different_buffer_text_clean() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-post-save-race-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("notes.txt");
        fs::write(&path, "original\n").unwrap();
        let mut buffer = Buffer::open(&path).unwrap();
        assert!(buffer.apply(&Transaction::insert(0, "edited ")));

        let outcome = buffer
            .save_with(false, |path, contents, expected| {
                let status = atomic_write(path, contents, expected)?;
                fs::write(path, "external\n")?;
                Ok(status)
            })
            .unwrap();

        assert!(matches!(
            outcome,
            SaveOutcome::CommittedWithWarning(ref warning)
                if warning.contains("changed again")
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "external\n");
        assert!(buffer.dirty);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn save_as_does_not_replace_a_target_created_during_temporary_write() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-save-as-race-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("new.txt");
        let mut buffer = buffer_with("buffer contents");
        let dirty_before = buffer.dirty;

        let error = buffer
            .save_as_with(path.clone(), false, |path, contents| {
                fs::write(path, "external contents")?;
                atomic_write(path, contents, ReplacePolicy::NoReplace)
            })
            .unwrap_err();

        assert!(format!("{error:#}").contains("failed to replace"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "external contents");
        assert_ne!(buffer.path.as_deref(), Some(path.as_path()));
        assert_eq!(buffer.dirty, dirty_before);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn retargeting_a_symlink_during_save_changes_neither_target() {
        use std::os::unix::fs::symlink;

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-save-symlink-race-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let first = directory.join("first.txt");
        let second = directory.join("second.txt");
        let link = directory.join("link.txt");
        fs::write(&first, "first\n").unwrap();
        fs::write(&second, "second\n").unwrap();
        symlink(&first, &link).unwrap();
        let mut buffer = Buffer::open(&link).unwrap();
        assert!(buffer.apply(&Transaction::insert(0, "edited ")));

        let error = buffer
            .save_with(false, |path, contents, expected| {
                atomic_write_with(
                    path,
                    contents,
                    expected,
                    |file, contents| {
                        file.write_all(contents)?;
                        fs::remove_file(&link)?;
                        symlink(&second, &link)
                    },
                    sync_parent,
                )
            })
            .unwrap_err();

        assert!(
            error.to_string().contains("symbolic-link target"),
            "{error:#}"
        );
        assert_eq!(fs::read_to_string(first).unwrap(), "first\n");
        assert_eq!(fs::read_to_string(second).unwrap(), "second\n");
        assert!(buffer.dirty);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_identity_checked_force_save_rejects_a_retargeted_symlink() {
        use std::os::unix::fs::symlink;

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-checked-save-race-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let first = directory.join("first.txt");
        let second = directory.join("second.txt");
        let link = directory.join("link.txt");
        fs::write(&first, "first\n").unwrap();
        fs::write(&second, "second\n").unwrap();
        symlink(&first, &link).unwrap();
        let expected_identity = crate::path_safety::path_identity(&link).unwrap();

        let error = atomic_write_with_identity(
            &link,
            b"replacement\n",
            ReplacePolicy::Force,
            Some(&expected_identity),
            |file, contents| {
                file.write_all(contents)?;
                fs::remove_file(&link)?;
                symlink(&second, &link)
            },
            sync_parent,
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("symbolic-link target")
                || error.to_string().contains("resolved identity"),
            "{error:#}"
        );
        assert_eq!(fs::read_to_string(first).unwrap(), "first\n");
        assert_eq!(fs::read_to_string(second).unwrap(), "second\n");
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn tightening_permissions_during_save_is_not_overwritten() {
        use std::os::unix::fs::PermissionsExt;

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-save-mode-race-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("notes.txt");
        fs::write(&path, "original\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut buffer = Buffer::open(&path).unwrap();
        assert!(buffer.apply(&Transaction::insert(0, "edited ")));

        let error = buffer
            .save_with(false, |path, contents, policy| {
                atomic_write_with(
                    path,
                    contents,
                    policy,
                    |file, contents| {
                        file.write_all(contents)?;
                        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                    },
                    sync_parent,
                )
            })
            .unwrap_err();

        assert!(format!("{error:#}").contains("destination changed"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(buffer.dirty);
        fs::remove_dir_all(directory).unwrap();
    }

    /// A file deleted while the buffer was open is not a conflict: there is
    /// nothing left to lose by writing it again.
    #[test]
    fn saving_recreates_a_file_that_was_deleted_underneath_the_buffer() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "runyte-buffer-deleted-{}-{unique}.txt",
            std::process::id(),
        ));
        fs::write(&path, "original\n").unwrap();
        let mut buffer = Buffer::open(&path).unwrap();
        buffer.apply(&Transaction::insert(0, "kept "));
        fs::remove_file(&path).unwrap();

        buffer.save(false).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "kept original\n");
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn saving_to_another_path_refuses_to_replace_an_existing_file() {
        let mut buffer = buffer_with("replacement");
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let occupied = std::env::temp_dir().join(format!(
            "runyte-buffer-save-as-{}-{unique}-occupied.txt",
            std::process::id(),
        ));
        fs::write(&occupied, "existing contents").unwrap();
        buffer.kind = BufferKind::File;

        let refused = buffer.save_as(occupied.clone(), false).unwrap_err();

        assert!(
            refused.to_string().contains("use :write! to replace it"),
            "unexpected message: {refused}"
        );
        assert_eq!(fs::read_to_string(&occupied).unwrap(), "existing contents");
        assert_eq!(buffer.path, None);

        buffer.save_as(occupied.clone(), true).unwrap();

        assert_eq!(fs::read_to_string(&occupied).unwrap(), "replacement");
        assert_eq!(buffer.path.as_deref(), Some(occupied.as_path()));

        // Rewriting a buffer to the path it already owns is not a replacement.
        buffer.save_as(occupied.clone(), false).unwrap();

        fs::remove_file(&occupied).unwrap();
    }

    #[test]
    fn a_no_op_transaction_creates_no_history() {
        let mut buffer = buffer_with("abc");
        assert!(!buffer.apply(&Transaction::insert(0, "")));
        assert!(!buffer.dirty);
        assert!(!buffer.undo());
    }

    #[test]
    fn undo_and_redo_restore_text() {
        let mut buffer = buffer_with("");
        buffer.apply(&Transaction::insert(0, "x"));
        assert!(buffer.undo());
        assert_eq!(buffer.to_string(), "");
        assert!(buffer.redo());
        assert_eq!(buffer.to_string(), "x");
    }

    #[test]
    fn undo_to_saved_content_clears_dirty_and_redo_restores_it() {
        let mut buffer = buffer_with("original");
        buffer.apply(&Transaction::insert(8, " revised"));
        buffer.mark_saved();
        buffer.apply(&Transaction::insert(0, "new "));
        assert!(buffer.dirty);

        assert!(buffer.undo());
        assert_eq!(buffer.to_string(), "original revised");
        assert!(!buffer.dirty, "undo restored the saved content");

        assert!(buffer.redo());
        assert!(buffer.dirty, "redo leaves the saved content");
    }

    #[test]
    fn undoing_all_edits_to_the_original_content_clears_dirty() {
        let mut buffer = buffer_with("original");
        buffer.apply(&Transaction::insert(8, " one"));
        buffer.apply(&Transaction::insert(12, " two"));
        assert!(buffer.dirty);

        assert!(buffer.undo());
        assert!(buffer.dirty, "one remaining edit still differs from disk");
        assert!(buffer.undo());
        assert_eq!(buffer.to_string(), "original");
        assert!(!buffer.dirty, "all edits have been undone");
    }

    #[test]
    fn a_multi_range_edit_is_one_undo_step() {
        let mut buffer = buffer_with("a b c");
        let transaction = Transaction::new(vec![
            Change::new(0, 1, "X"),
            Change::new(2, 3, "Y"),
            Change::new(4, 5, "Z"),
        ]);
        buffer.apply(&transaction);
        assert_eq!(buffer.to_string(), "X Y Z");
        assert!(buffer.undo());
        assert_eq!(buffer.to_string(), "a b c");
        assert!(!buffer.undo(), "one edit produced exactly one step");
    }

    #[test]
    fn an_explicit_group_undoes_and_redoes_as_one_step() {
        let mut buffer = buffer_with("");
        buffer.begin_undo_group();
        buffer.apply(&Transaction::insert(0, "a"));
        buffer.apply(&Transaction::insert(1, "b"));
        buffer.apply(&Transaction::insert(2, "c"));
        buffer.commit_undo_group();

        assert_eq!(buffer.history_len(), 1);
        assert!(buffer.undo());
        assert_eq!(buffer.to_string(), "");
        assert!(!buffer.undo(), "the group consumes one history entry");
        assert!(buffer.redo());
        assert_eq!(buffer.to_string(), "abc");
    }

    #[test]
    fn read_only_buffers_reject_edits() {
        let mut buffer = Buffer::virtual_text("[projection]", "text");
        assert!(!buffer.apply(&Transaction::insert(0, "x")));
        assert_eq!(buffer.to_string(), "text");
        assert!(!buffer.undo());
    }

    #[test]
    fn virtual_replacement_clears_history() {
        let mut buffer = Buffer::virtual_text("[projection]", "one");
        assert!(buffer.replace_virtual_text("two"));
        assert_eq!(buffer.to_string(), "two");
        assert!(!buffer.dirty);
    }

    #[test]
    fn unicode_positions_round_trip() {
        let buffer = buffer_with("a🦀b\nçd");
        assert_eq!(buffer.line_len(0), 3);
        assert_eq!(buffer.line_len(1), 2);
        let position = Position::new(0, 2);
        assert_eq!(buffer.position_of(buffer.offset_of(position)), position);
    }

    #[test]
    fn clamping_keeps_the_caret_on_a_character_in_normal_mode() {
        let buffer = buffer_with("abc\n\ndef");
        assert_eq!(
            buffer.clamp(Position::new(0, 9), false),
            Position::new(0, 2)
        );
        assert_eq!(buffer.clamp(Position::new(0, 9), true), Position::new(0, 3));
        assert_eq!(
            buffer.clamp(Position::new(1, 4), false),
            Position::new(1, 0)
        );
    }

    #[test]
    fn history_is_bounded() {
        let mut buffer = buffer_with("");
        for index in 0..HISTORY_LIMIT + 50 {
            buffer.apply(&Transaction::insert(index, "x"));
        }
        assert_eq!(buffer.undo.len(), HISTORY_LIMIT);
    }

    #[test]
    fn external_observation_marks_stale_without_touching_text_or_history() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-external-observation-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("notes.txt");
        fs::write(&path, "baseline\n").unwrap();
        let mut buffer = Buffer::open(&path).unwrap();
        buffer.apply(&Transaction::insert(0, "local "));
        let before = buffer.to_string();
        let history = buffer.history_len();

        fs::write(&path, "external\n").unwrap();
        let event = buffer.observe_now(7).unwrap();
        assert_eq!(
            buffer.apply_file_observation(&event),
            ObservationApply::Stale { notify: true }
        );
        assert_eq!(buffer.external_file_status(), ExternalFileStatus::Changed);
        assert_eq!(buffer.to_string(), before);
        assert_eq!(buffer.history_len(), history);
        assert!(buffer.dirty);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn matching_external_text_converges_without_clearing_undo() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-external-convergence-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("notes.txt");
        fs::write(&path, "base").unwrap();
        let mut buffer = Buffer::open(&path).unwrap();
        buffer.apply(&Transaction::insert(4, "!"));
        fs::write(&path, "base!").unwrap();

        let event = buffer.observe_now(0).unwrap();
        assert_eq!(
            buffer.apply_file_observation(&event),
            ObservationApply::Converged
        );
        assert!(!buffer.dirty);
        assert_eq!(buffer.history_len(), 1);
        assert!(buffer.undo());
        assert_eq!(buffer.to_string(), "base");
        assert!(buffer.dirty);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn observation_from_before_a_save_is_ignored() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-buffer-old-observation-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("notes.txt");
        fs::write(&path, "before").unwrap();
        let mut buffer = Buffer::open(&path).unwrap();
        fs::write(&path, "external").unwrap();
        let old = buffer.observe_now(0).unwrap();

        buffer.apply(&Transaction::insert(0, "saved "));
        buffer.save(true).unwrap();
        assert_eq!(
            buffer.apply_file_observation(&old),
            ObservationApply::Ignored
        );
        assert_eq!(
            buffer.external_file_status(),
            ExternalFileStatus::Synchronized
        );

        fs::remove_dir_all(directory).unwrap();
    }
}
