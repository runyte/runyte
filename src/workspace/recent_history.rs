// SPDX-License-Identifier: MPL-2.0

//! Bounded recent-workspace history and shared naming/numbering semantics.
//! Catalog discovery and host publication remain separate owners.

use super::session_name::{MAX_HOST_NAME_BYTES, normalize_session_name, validate_host_name};
use crate::{
    external_open,
    native_path::{decode_path, encode_path},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
#[path = "recent_history/unix.rs"]
mod storage;
#[cfg(windows)]
#[path = "recent_history/windows.rs"]
mod storage;

#[cfg(all(test, unix))]
pub(super) use storage::RecentFileLock;

pub(super) const RECENT_LIMIT: usize = 256;
pub(super) const MAX_RECENTS_BYTES: usize = 8 * 1024 * 1024;
pub(super) const MAX_PERSISTED_PATH_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct RecentWorkspace {
    pub(super) project_root_bytes: Vec<u8>,
    #[serde(default)]
    pub(super) name: Option<String>,
    /// Absent in catalogs written before workspaces were numbered, which is
    /// why it defaults rather than failing the whole file: an older history
    /// stays readable and is numbered on the next listing.
    #[serde(default)]
    pub(super) number: Option<u8>,
    /// Absent in catalogs written before the session manager showed activity.
    #[serde(default)]
    pub(super) last_active_unix_seconds: Option<u64>,
    /// Absent in catalogs written before a digit could be declined, which is
    /// the same as never having declined one.
    #[serde(default)]
    pub(super) number_declined: bool,
    /// Whether `number` was chosen explicitly through Renumber.
    ///
    /// Older catalogs omit this and therefore treat their assignments as
    /// automatic, which lets the first refresh close any gaps left by stopped
    /// sessions.
    #[serde(default)]
    pub(super) number_pinned: bool,
}

/// The largest number a workspace can carry.
///
/// A number is a shortcut pressed as one key in the session manager, so the
/// range is exactly the digits `1`-`9`. A tenth remembered workspace is
/// reached by name or path instead of by number.
pub const MAX_WORKSPACE_NUMBER: u8 = 9;

/// One remembered workspace: where it is, what it is called, and the digit
/// metadata used to order automatic assignments while it is running.
///
/// The recents file is ordered most-recently-visited first, so an entry's
/// position is deliberately not its automatic order. Explicit Renumber pins
/// the current digit while the session runs; a refresh clears every numbering
/// field once it observes that session stopped.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecentEntry {
    pub project_root: PathBuf,
    pub name: Option<String>,
    /// `1` through [`MAX_WORKSPACE_NUMBER`], or `None` when every number was
    /// already taken as this workspace was first recorded.
    pub number: Option<u8>,
    /// Whole Unix seconds make the regenerable catalog portable across
    /// processes while keeping elapsed-time presentation out of persistence.
    pub last_active_unix_seconds: Option<u64>,
    /// Somebody took this workspace's digit away.
    ///
    /// Clearing a number is an answer rather than an absence, so it is
    /// remembered: a running session is otherwise given the lowest free digit
    /// again on the next listing, and the manager would undo the decision
    /// before it could be seen. Numbering the workspace again ends it.
    pub number_declined: bool,
    /// The current number was explicitly chosen and must not be compacted
    /// while this session remains running.
    pub number_pinned: bool,
}

impl RecentEntry {
    /// A workspace that has not declined a digit, which every producer of a
    /// record but the reader means.
    pub(super) fn new(
        project_root: PathBuf,
        name: Option<String>,
        number: Option<u8>,
        last_active_unix_seconds: Option<u64>,
    ) -> Self {
        Self {
            project_root,
            name,
            number,
            last_active_unix_seconds,
            number_declined: false,
            number_pinned: false,
        }
    }
}

pub(super) fn recent_file() -> Option<PathBuf> {
    recent_file_in(external_open::cache_root())
}

/// Finds usable optional storage for stopped-workspace history.
///
/// Running-host discovery has its own runtime registry fallback and must not
/// become unavailable merely because the regenerable cache root cannot be
/// created. Once a cache directory is usable, errors from the recents file
/// itself remain observable so malformed history is never silently erased.
pub(super) fn recent_file_in(cache_root: Option<PathBuf>) -> Option<PathBuf> {
    let root = cache_root?;
    storage::prepare_parent(&root).ok()?;
    Some(root.join("workspaces.json"))
}

/// Remembers a workspace after startup so stopped hosts remain discoverable.
pub fn record_recent_workspace(project_root: &Path) -> Result<Option<RecordedWorkspace>> {
    let Some(path) = recent_file() else {
        return Ok(None);
    };
    record_recent_workspace_name_in(&path, project_root)
}

/// Ensures lifecycle and host-startup metadata exists without claiming a
/// workspace was visited or changing an existing entry's recency.
pub fn ensure_recent_workspace(project_root: &Path) -> Result<Option<RecordedWorkspace>> {
    let Some(path) = recent_file() else {
        return Ok(None);
    };
    ensure_recent_workspace_in(&path, project_root).map(Some)
}

/// Records the beginning or end of a successful interactive attachment.
///
/// Catalog discovery and lifecycle commands also remember workspace names and
/// ordering, but they must not claim the person entered a session. Keeping the
/// activity write separate makes the successful attachment handshake the only
/// producer of this timestamp.
pub fn record_workspace_activity(project_root: &Path) -> Result<()> {
    let Some(path) = recent_file() else {
        return Ok(());
    };
    record_workspace_activity_in(&path, project_root)
}

/// The number the catalog currently records for one workspace, if any.
///
/// A direct read rather than a listing: the status line needs this before any
/// refresh has run, and asking for the whole inventory to learn one digit
/// would make drawing the first frame wait on scanning every workspace.
pub fn recorded_workspace_number(project_root: &Path) -> Option<u8> {
    let path = recent_file()?;
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    read_recents(Some(&path))
        .ok()?
        .into_iter()
        .find(|entry| entry.project_root == canonical)
        .and_then(|entry| entry.number)
}

/// Gives one workspace a number shortcut, or takes its number away.
///
/// A number identifies exactly one workspace, so assigning one that another
/// workspace already holds swaps the pair rather than leaving a duplicate or
/// quietly unnumbering the other. Both keep a shortcut, and the returned path
/// lets the caller say where the old one went instead of leaving somebody to
/// discover it by pressing the key.
///
/// Taking a number away is remembered as a decision, so the workspace stays
/// unnumbered instead of being handed the lowest free digit by the next
/// listing. Giving it one again ends that.
pub(super) fn set_recent_workspace_number_in(
    path: Option<&Path>,
    project_root: &Path,
    number: Option<u8>,
) -> Result<Option<PathBuf>> {
    if let Some(number) = number {
        anyhow::ensure!(
            (1..=MAX_WORKSPACE_NUMBER).contains(&number),
            "a session number must be between 1 and {MAX_WORKSPACE_NUMBER}"
        );
    }
    let Some(path) = path else {
        return Ok(None);
    };
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut displaced = None;
    update_recents_result(path, |paths| {
        assign_missing_default_workspace_names(paths);
        assign_missing_default_workspace_numbers(paths);
        let Some(index) = paths
            .iter()
            .position(|entry| entry.project_root == canonical)
        else {
            anyhow::bail!("that workspace is not in the visited history")
        };
        let vacated = paths[index].number;
        if let Some(number) = number
            && let Some(holder) = paths
                .iter()
                .position(|entry| entry.number == Some(number) && entry.project_root != canonical)
        {
            // The swap hands the asking workspace's old number over, which is
            // why an unnumbered one leaves the other without a number rather
            // than duplicating the one it just gave away.
            paths[holder].number = vacated;
            if vacated.is_none() {
                paths[holder].number_pinned = false;
            }
            // The displaced workspace did not ask to lose its digit, so it is
            // left open to being given another one; only the workspace that
            // was cleared on purpose declines.
            displaced = Some(paths[holder].project_root.clone());
        }
        paths[index].number = number;
        paths[index].number_declined = number.is_none();
        paths[index].number_pinned = number.is_some();
        Ok(())
    })?;
    Ok(displaced)
}

#[cfg(test)]
pub(super) fn record_recent_workspace_in(path: &Path, project_root: &Path) -> Result<()> {
    record_recent_workspace_name_in(path, project_root).map(drop)
}

pub(super) fn record_recent_workspace_name_in(
    path: &Path,
    project_root: &Path,
) -> Result<Option<RecordedWorkspace>> {
    let canonical = project_root.canonicalize()?;
    let mut recorded = None;
    update_recents(path, |paths| {
        // Older catalogs predate automatic names and numbers. Claim both for
        // those rows before inserting a new workspace so an established
        // directory keeps the unsuffixed form and the low number, and the
        // newcomer receives `-2` and the next number up.
        assign_missing_default_workspace_names(paths);
        assign_missing_default_workspace_numbers(paths);
        let previous = paths
            .iter()
            .find(|entry| entry.project_root == canonical)
            .map(|entry| {
                (
                    entry.name.clone(),
                    entry.number,
                    entry.last_active_unix_seconds,
                    entry.number_declined,
                    entry.number_pinned,
                )
            });
        paths.retain(|entry| entry.project_root != canonical);
        let declined = previous
            .as_ref()
            .is_some_and(|(_, _, _, declined, _)| *declined);
        let (previous_name, previous_number, last_active_unix_seconds, number_pinned) = previous
            .map_or(
                (None, None, None, false),
                |(name, number, active, _, pinned)| (name, Some(number), active, pinned),
            );
        let name = previous_name
            .unwrap_or_else(|| unique_default_workspace_name(&canonical, paths.as_slice()));
        // Revisiting keeps the number this workspace already answered to.
        // Only a genuinely new record claims one, which is what makes the
        // default assignment order the order workspaces were created in, and a
        // workspace whose digit was taken away keeps none.
        let number = if declined {
            None
        } else {
            previous_number
                .flatten()
                .or_else(|| lowest_free_workspace_number(paths))
        };
        recorded = Some(RecordedWorkspace {
            name: name.clone(),
            number,
        });
        let mut entry = RecentEntry::new(canonical, Some(name), number, last_active_unix_seconds);
        entry.number_declined = declined;
        entry.number_pinned = number_pinned;
        paths.insert(0, entry);
        // Truncation drops the least recently visited tail, which can free a
        // number. The next new workspace claims it; the survivors keep theirs.
        paths.truncate(RECENT_LIMIT);
    })?;
    Ok(recorded)
}

pub(super) fn ensure_recent_workspace_in(
    path: &Path,
    project_root: &Path,
) -> Result<RecordedWorkspace> {
    let canonical = project_root.canonicalize()?;
    let mut recorded = None;
    update_recents(path, |entries| {
        assign_missing_default_workspace_names(entries);
        assign_missing_default_workspace_numbers(entries);
        if let Some(entry) = entries.iter().find(|entry| entry.project_root == canonical) {
            recorded = Some(RecordedWorkspace {
                name: entry.name.clone().unwrap_or_else(|| {
                    unique_default_workspace_name(&canonical, entries.as_slice())
                }),
                number: entry.number,
            });
            return;
        }
        let name = unique_default_workspace_name(&canonical, entries.as_slice());
        let number = lowest_free_workspace_number(entries);
        recorded = Some(RecordedWorkspace {
            name: name.clone(),
            number,
        });
        entries.insert(0, RecentEntry::new(canonical, Some(name), number, None));
        entries.truncate(RECENT_LIMIT);
    })?;
    recorded.context("workspace metadata was not recorded")
}

pub(super) fn record_workspace_activity_in(path: &Path, project_root: &Path) -> Result<()> {
    // Successful attachment is also a genuine visit for recency ordering. It
    // normally already has an entry from startup or session discovery, but
    // recreating a concurrently removed cache must not lose the activity.
    record_recent_workspace_name_in(path, project_root)?;
    let canonical = project_root.canonicalize()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs());
    update_recents(path, |entries| {
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.project_root == canonical)
        {
            entry.last_active_unix_seconds = now;
        }
    })
}

/// What recording a visit settled about a workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedWorkspace {
    pub name: String,
    pub number: Option<u8>,
}

pub(super) fn assign_missing_default_workspace_names(paths: &mut [RecentEntry]) {
    for index in 0..paths.len() {
        if paths[index].name.is_some() {
            continue;
        }
        let project_root = paths[index].project_root.clone();
        paths[index].name = Some(unique_default_workspace_name(&project_root, paths));
    }
}

/// Claims the lowest free number for every remembered workspace without one.
///
/// Older catalogs predate numbering entirely, and a workspace recorded while
/// all nine were taken carries none. Both are answered here, on the way to a
/// listing, so numbering never depends on having been present for a
/// particular release.
///
/// Numbers are meant to follow the order workspaces were created, and a new
/// record claims its number at exactly that moment. A catalog written before
/// numbering existed has no creation order to recover -- it is ordered by
/// recency and nothing else -- so this one-time backfill numbers those rows
/// most-recently-visited first and says so rather than inventing a history.
///
/// A workspace whose digit was taken away on purpose is not missing one.
pub(super) fn assign_missing_default_workspace_numbers(paths: &mut [RecentEntry]) {
    for index in 0..paths.len() {
        if paths[index].number.is_some() || paths[index].number_declined {
            continue;
        }
        paths[index].number = lowest_free_workspace_number(paths);
    }
}

/// The smallest number no remembered workspace holds, if any is left.
pub(super) fn lowest_free_workspace_number(paths: &[RecentEntry]) -> Option<u8> {
    (1..=MAX_WORKSPACE_NUMBER)
        .find(|candidate| paths.iter().all(|entry| entry.number != Some(*candidate)))
}

/// Derives a stable catalog name from the workspace directory and adds the
/// first free numeric suffix when another recorded workspace already owns it.
pub(super) fn unique_default_workspace_name(project_root: &Path, paths: &[RecentEntry]) -> String {
    let raw_base = project_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| project_root.display().to_string());
    let sanitized = raw_base
        .trim()
        .chars()
        .map(|character| {
            if character.is_control() {
                '-'
            } else {
                character
            }
        })
        .collect::<String>();
    let sanitized = normalize_session_name(&sanitized);
    let sanitized = if sanitized.is_empty() {
        "workspace"
    } else {
        sanitized.as_str()
    };
    let base = truncate_utf8(sanitized, MAX_HOST_NAME_BYTES).to_owned();
    let available = |candidate: &str| {
        paths
            .iter()
            .all(|entry| entry.name.as_deref() != Some(candidate))
    };
    if available(&base) {
        return base;
    }
    (2_u64..)
        .map(|suffix| {
            let suffix = format!("-{suffix}");
            let prefix = truncate_utf8(&base, MAX_HOST_NAME_BYTES - suffix.len());
            format!("{prefix}{suffix}")
        })
        .find(|candidate| available(candidate))
        .expect("an unbounded numeric suffix has an available value")
}

pub(super) fn truncate_utf8(value: &str, maximum_bytes: usize) -> &str {
    let mut end = value.len().min(maximum_bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Drops a workspace from the visited history, so a stopped one stops being
/// listed. Answers whether history held it at all.
///
/// Only the per-user recents record is removed. Nothing under the project's own
/// state root is touched, so a workspace cleared here is exactly as reachable as
/// one that was never opened: naming it starts a host there again.
pub(super) fn forget_recent_workspace_in(path: Option<&Path>, project_root: &Path) -> Result<bool> {
    let Some(path) = path else {
        return Ok(false);
    };
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut removed = false;
    update_recents(path, |paths| {
        let before = paths.len();
        paths.retain(|entry| entry.project_root != canonical);
        removed = paths.len() != before;
    })?;
    Ok(removed)
}

/// Removes exactly the named stopped rows from recent history in one locked
/// update. A workspace recorded concurrently at another path is preserved.
pub(super) fn clear_recent_workspaces_in(
    path: Option<&Path>,
    stopped: &[PathBuf],
) -> Result<usize> {
    let Some(path) = path else {
        return Ok(0);
    };
    let stopped = stopped
        .iter()
        .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
        .collect::<Vec<_>>();
    let mut removed = 0;
    update_recents(path, |paths| {
        let before = paths.len();
        paths.retain(|entry| !stopped.contains(&entry.project_root));
        removed = before - paths.len();
    })?;
    Ok(removed)
}

pub(super) fn rename_recent_workspace_in(
    path: &Path,
    project_root: &Path,
    name: &str,
) -> Result<()> {
    validate_host_name(name)?;
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    update_recents_result(path, |paths| {
        anyhow::ensure!(
            paths.iter().all(|entry| entry.project_root == canonical
                || entry.name.as_deref() != Some(name)),
            "session name {name:?} is already in use"
        );
        let entry = paths
            .iter_mut()
            .find(|entry| entry.project_root == canonical)
            .with_context(|| {
                format!(
                    "workspace {} is not in recent history",
                    project_root.display()
                )
            })?;
        entry.name = Some(name.to_owned());
        Ok(())
    })
}

pub(super) fn update_recents(
    path: &Path,
    update: impl FnOnce(&mut Vec<RecentEntry>),
) -> Result<()> {
    update_recents_result(path, |paths| {
        update(paths);
        Ok(())
    })
}

pub(super) fn update_recents_result(
    path: &Path,
    update: impl FnOnce(&mut Vec<RecentEntry>) -> Result<()>,
) -> Result<()> {
    let mut storage = storage::LockedHistory::acquire(path)?;
    let mut paths = decode_recents(&storage.read()?)?;
    update(&mut paths)?;
    storage.write(&encode_recents(&paths)?)
}

/// Native adapters retain captured identity and choose their own guarded
/// operation. Do not rewrite the history when a compare guard made no change.
#[cfg(windows)]
pub(super) fn update_recents_if_changed<T>(
    path: &Path,
    update: impl FnOnce(&mut Vec<RecentEntry>) -> Result<T>,
) -> Result<T> {
    let mut storage = storage::LockedHistory::acquire(path)?;
    let bytes = storage.read()?;
    // The compatibility reader repairs duplicate digits. A native guarded
    // transaction must not silently serialize that repair into unrelated rows.
    anyhow::ensure!(
        bytes.len() <= MAX_RECENTS_BYTES,
        "workspace recents exceed {MAX_RECENTS_BYTES} bytes"
    );
    {
        let raw: Vec<RecentWorkspace> = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            raw.len() <= RECENT_LIMIT,
            "workspace recents contain more than {RECENT_LIMIT} entries"
        );
        let mut claimed = std::collections::BTreeSet::new();
        anyhow::ensure!(
            raw.iter()
                .filter_map(|entry| entry.number)
                .all(|number| claimed.insert(number)),
            "native history transaction refuses duplicate session numbers"
        );
    }
    let mut paths = decode_recents(&bytes)?;
    let before = paths.clone();
    let result = update(&mut paths)?;
    if paths != before {
        storage.write(&encode_recents(&paths)?)?;
    }
    Ok(result)
}

pub(super) fn encode_recents(paths: &[RecentEntry]) -> Result<Vec<u8>> {
    anyhow::ensure!(
        paths.len() <= RECENT_LIMIT,
        "workspace recents contain more than {RECENT_LIMIT} entries"
    );
    for entry in paths {
        validate_recent_entry(entry)?;
    }
    let entries = paths
        .iter()
        .map(|entry| RecentWorkspace {
            project_root_bytes: encode_path(&entry.project_root),
            name: entry.name.clone(),
            number: entry.number,
            last_active_unix_seconds: entry.last_active_unix_seconds,
            number_declined: entry.number_declined,
            number_pinned: entry.number_pinned,
        })
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec_pretty(&entries)?;
    anyhow::ensure!(
        bytes.len() <= MAX_RECENTS_BYTES,
        "workspace recents exceed {MAX_RECENTS_BYTES} bytes"
    );
    Ok(bytes)
}

/// The remembered workspaces worth showing: those whose directory is still
/// there, plus any a running host is using.
///
/// A stopped workspace whose directory is gone has nothing left to open, so it
/// stays out of the listing. Its record survives in the file, because the
/// directory may come back — an unmounted volume or a detached external disk.
pub(super) fn listable_recents(entries: Vec<RecentEntry>) -> Vec<RecentEntry> {
    entries
        .into_iter()
        .filter(|entry| entry.project_root.is_dir())
        .collect()
}

/// Reads the remembered workspaces exactly as the file holds them.
///
/// A directory that has gone from disk is deliberately still returned. Every
/// write goes back through this reader, so filtering here would erase the
/// record the first time anything touched the file after the directory
/// disappeared, including for a host still running in it.
/// [`listable_recents`] drops those rows on the way to a listing instead,
/// which is the only place the distinction matters.
pub(super) fn read_recents(path: Option<&Path>) -> Result<Vec<RecentEntry>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let bytes = match storage::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    decode_recents(&bytes)
}

pub(super) fn decode_recents(bytes: &[u8]) -> Result<Vec<RecentEntry>> {
    anyhow::ensure!(
        bytes.len() <= MAX_RECENTS_BYTES,
        "workspace recents exceed {MAX_RECENTS_BYTES} bytes"
    );
    let entries: Vec<RecentWorkspace> = serde_json::from_slice(bytes)?;
    anyhow::ensure!(
        entries.len() <= RECENT_LIMIT,
        "workspace recents contain more than {RECENT_LIMIT} entries"
    );
    for entry in &entries {
        validate_recent_workspace(entry)?;
    }
    let mut entries = entries
        .into_iter()
        .map(|entry| {
            Ok(RecentEntry {
                project_root: decode_path(entry.project_root_bytes)?,
                name: entry.name,
                number: entry.number,
                last_active_unix_seconds: entry.last_active_unix_seconds,
                number_declined: entry.number_declined,
                number_pinned: entry.number_pinned,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    // A number identifies one workspace, so a file hand-edited into holding a
    // duplicate is repaired on the way in rather than reaching a listing where
    // one digit would select whichever row happened to be first.
    let mut claimed = Vec::new();
    for entry in &mut entries {
        match entry.number {
            Some(number) if claimed.contains(&number) => {
                entry.number = None;
                entry.number_pinned = false;
            }
            Some(number) => claimed.push(number),
            None => {}
        }
    }
    Ok(entries)
}

pub(super) fn validate_recent_workspace(entry: &RecentWorkspace) -> Result<()> {
    validate_persisted_path(
        &entry.project_root_bytes,
        "recent workspace project directory",
    )?;
    let project_root = decode_path(entry.project_root_bytes.clone())?;
    anyhow::ensure!(
        project_root.is_absolute(),
        "recent workspace project directory is not absolute"
    );
    if let Some(name) = entry.name.as_deref() {
        validate_host_name(name)?;
    }
    if let Some(number) = entry.number {
        anyhow::ensure!(
            (1..=MAX_WORKSPACE_NUMBER).contains(&number),
            "recent workspace number must be between 1 and {MAX_WORKSPACE_NUMBER}"
        );
    }
    Ok(())
}

pub(super) fn validate_recent_entry(entry: &RecentEntry) -> Result<()> {
    validate_persisted_path(
        &encode_path(&entry.project_root),
        "recent workspace project directory",
    )?;
    anyhow::ensure!(
        entry.project_root.is_absolute(),
        "recent workspace project directory is not absolute"
    );
    if let Some(name) = entry.name.as_deref() {
        validate_host_name(name)?;
    }
    if let Some(number) = entry.number {
        anyhow::ensure!(
            (1..=MAX_WORKSPACE_NUMBER).contains(&number),
            "recent workspace number must be between 1 and {MAX_WORKSPACE_NUMBER}"
        );
    }
    Ok(())
}

pub(super) fn validate_persisted_path(bytes: &[u8], description: &str) -> Result<()> {
    anyhow::ensure!(!bytes.is_empty(), "{description} is empty");
    anyhow::ensure!(
        bytes.len() <= MAX_PERSISTED_PATH_BYTES,
        "{description} exceeds {MAX_PERSISTED_PATH_BYTES} bytes"
    );
    let path = decode_path(bytes.to_vec())?;
    #[cfg(unix)]
    let contains_nul = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().contains(&0)
    };
    #[cfg(windows)]
    let contains_nul = {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().any(|unit| unit == 0)
    };
    anyhow::ensure!(!contains_nul, "{description} contains a null byte");
    Ok(())
}

#[cfg(test)]
mod tests;
