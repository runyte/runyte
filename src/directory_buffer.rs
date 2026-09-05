// SPDX-License-Identifier: MPL-2.0

//! Editable text projections of directories.
//!
//! The visible buffer contains one relative path per line. A trailing `/`
//! creates a directory; existing entry kinds are retained. Stable entry
//! identities live beside the text so reordering rows does not become a
//! delete-and-create plan and in-place edits remain renames.

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

#[cfg(unix)]
use std::ffi::CStr;

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Datelike, Local};

use crate::{
    config::ExplorerSort,
    fs_plan::{
        DesiredEntry, DirectorySnapshot, EntryDetailFields, EntryId, EntryKind, FsPlan,
        SnapshotEntry, SourceFingerprint, TransferMode,
    },
    row_hints::{RowHints, display_cells},
};

/// How an explorer is asked to project a directory.
///
/// These three travel together because every entry point that reads a
/// directory needs all of them, and because they are the whole of what the
/// configuration says about a listing. Nothing here changes what the listing
/// is responsible for except `show_hidden`, which decides which entries the
/// baseline holds and therefore what a plan may act on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ListingView {
    pub show_hidden: bool,
    pub sort: ExplorerSort,
    pub details: bool,
}

impl ListingView {
    /// The view the configuration asks for.
    pub fn from_config(editor: &crate::config::EditorConfig) -> Self {
        Self {
            show_hidden: editor.show_hidden_files,
            sort: editor.explorer_sort,
            details: editor.explorer_details,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryTransfer {
    pub source: PathBuf,
    pub label: String,
    pub kind: EntryKind,
    pub expected: SourceFingerprint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RowOrigin {
    Snapshot(EntryId),
    Transfer {
        source: PathBuf,
        kind: EntryKind,
        mode: TransferMode,
        expected: SourceFingerprint,
    },
}

#[derive(Clone, Debug)]
pub struct DirectoryBuffer {
    root: PathBuf,
    baseline: DirectorySnapshot,
    row_origins: Vec<Option<RowOrigin>>,
    detached_origins: HashMap<String, Vec<RowOrigin>>,
    details: Option<DirectoryDetails>,
    hints: RowHints,
}

#[derive(Clone, Debug, Default)]
struct DirectoryDetails {
    snapshots: HashMap<EntryId, String>,
    transfers: HashMap<PathBuf, String>,
}

#[derive(Clone, Debug)]
struct RawDetails {
    kind: EntryKind,
    len: u64,
    modified_nanos: Option<u128>,
    mode: Option<u32>,
    owner: String,
    group: String,
}

impl DirectoryBuffer {
    /// Projects `root` as text under `view`.
    ///
    /// Hidden entries are left out of the baseline as well as the text, so a
    /// row missing from the listing is not read as a deletion when the plan
    /// is built. The sort order is applied here rather than to the baseline
    /// read: entry identities are assigned in the snapshot's own name order
    /// and compared against a second read in that same order, so reordering
    /// belongs to the projection, where `row_origins` already carries identity
    /// independently of where a row sits.
    pub fn open(root: PathBuf, view: ListingView) -> Result<(Self, String)> {
        let root = fs::canonicalize(&root)
            .with_context(|| format!("failed to resolve directory {}", root.display()))?;
        let baseline = DirectorySnapshot::read_with(&root, view.show_hidden)?;
        let order = sorted_rows(baseline.entries(), view.sort);
        let text = render_snapshot(&baseline, &order)?;
        let mut row_origins = order
            .iter()
            .map(|row| Some(RowOrigin::Snapshot(baseline.entries()[*row].id)))
            .collect::<Vec<_>>();
        if !row_origins.is_empty() {
            row_origins.push(None);
        }
        let mut directory = Self {
            root,
            baseline,
            row_origins,
            detached_origins: HashMap::new(),
            details: None,
            hints: RowHints::default(),
        };
        if view.details {
            directory.details = Some(directory.build_details());
        }
        directory.refresh_hints(&text);
        Ok((directory, text))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn retarget_root(&mut self, root: PathBuf) {
        self.root = root;
    }

    pub fn plan(&self, text: &str) -> Result<FsPlan> {
        let lines = split_lines(text);
        let mut desired = Vec::new();
        for (row, line) in lines.iter().enumerate() {
            if line.trim_end().is_empty() {
                continue;
            }
            let origin = self.row_origins.get(row).cloned().flatten();
            let (path, requested_kind) = parse_line(line)?;
            let entry = match origin {
                Some(RowOrigin::Snapshot(id)) => {
                    let kind = self
                        .baseline
                        .entry(id)
                        .map_or(requested_kind, |entry| entry.kind);
                    DesiredEntry::identified(id, path, kind)
                }
                Some(RowOrigin::Transfer {
                    source,
                    kind,
                    mode,
                    expected,
                }) => DesiredEntry::transfer(source, path, kind, mode, expected),
                None => DesiredEntry::create(path, requested_kind),
            };
            desired.push(entry);
        }
        FsPlan::build(self.root.clone(), self.baseline.clone(), desired)
    }

    pub fn transfer_at(&self, text: &str, row: usize) -> Result<Option<DirectoryTransfer>> {
        let Some(line) = split_lines(text).get(row).copied() else {
            return Ok(None);
        };
        if line.is_empty() {
            return Ok(None);
        }
        let Some(origin) = self.row_origins.get(row).cloned().flatten() else {
            anyhow::bail!("{} is new; write it before copying it", line);
        };
        let (visible, _) = parse_line(line)?;
        match origin {
            RowOrigin::Snapshot(id) => {
                let entry = self
                    .baseline
                    .entry(id)
                    .context("directory entry identity is stale")?;
                ensure!(
                    visible == entry.path,
                    "{} has pending edits; write them before copying it",
                    visible.display()
                );
                Ok(Some(DirectoryTransfer {
                    source: self.root.join(&entry.path),
                    label: entry.display_name()?,
                    kind: entry.kind,
                    expected: SourceFingerprint::capture(&self.root.join(&entry.path))?,
                }))
            }
            RowOrigin::Transfer {
                source,
                kind,
                expected,
                ..
            } => Ok(Some(DirectoryTransfer {
                source,
                label: format!("{}{}", visible.display(), kind.marker()),
                kind,
                expected,
            })),
        }
    }

    pub fn assign_transfers(
        &mut self,
        text: &str,
        start_row: usize,
        transfers: &[DirectoryTransfer],
        mode: TransferMode,
    ) -> Result<()> {
        let required_rows = start_row + transfers.len();
        if self.row_origins.len() < required_rows {
            self.row_origins.resize(required_rows, None);
        }
        for (offset, transfer) in transfers.iter().enumerate() {
            let row = start_row + offset;
            let visible = split_lines(&transfer.label)
                .first()
                .copied()
                .context("directory transfer has no label")?;
            let (path, _) = parse_line(visible)?;
            let restored = self
                .baseline
                .entries()
                .iter()
                .find(|entry| self.root.join(&entry.path) == transfer.source && entry.path == path)
                .map(|entry| RowOrigin::Snapshot(entry.id));
            self.row_origins[row] = restored.or_else(|| {
                Some(RowOrigin::Transfer {
                    source: transfer.source.clone(),
                    kind: transfer.kind,
                    mode,
                    expected: transfer.expected.clone(),
                })
            });
        }
        self.refresh_details();
        self.refresh_hints(text);
        Ok(())
    }

    /// Absolute sources of cut entries pasted into this listing but not yet
    /// applied.
    ///
    /// The source explorer uses this to avoid saving its removed row as an
    /// independent deletion. A pasted cut is one move owned by the
    /// destination plan; deleting its source first would leave that plan with
    /// a transfer identity which can no longer be inspected.
    pub fn pending_move_sources(&self) -> HashSet<PathBuf> {
        self.row_origins
            .iter()
            .flatten()
            .filter_map(|origin| match origin {
                RowOrigin::Transfer {
                    source,
                    mode: TransferMode::Move,
                    ..
                } => Some(source.clone()),
                RowOrigin::Snapshot(_) | RowOrigin::Transfer { .. } => None,
            })
            .collect()
    }

    /// Resolves the visible entry on `row` without applying any text edits.
    ///
    /// The edited label is deliberately authoritative: a renamed or newly
    /// inserted row must be saved before it can be navigated.
    ///
    /// A symlink resolves to what it points at. Opening the link itself would
    /// hand the editor a path whose staged Git text is the link body rather
    /// than the file's, so every line of the file it opened would read as
    /// changed. Renaming and deleting stay with the link, since those go
    /// through the plan and never through this path.
    pub fn entry_path(&self, text: &str, row: usize) -> Result<Option<PathBuf>> {
        let Some(line) = split_lines(text).get(row).copied() else {
            return Ok(None);
        };
        if line.is_empty() {
            return Ok(None);
        }
        let (relative, _) = parse_line(line)?;
        ensure!(
            relative
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
            "directory entry must be a relative path inside the explorer root"
        );
        let target = self.root.join(&relative);
        let link =
            fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink());
        if link {
            return fs::canonicalize(&target)
                .map(Some)
                .with_context(|| format!("{} is a broken symlink", relative.display()));
        }
        fs::metadata(&target).with_context(|| {
            format!(
                "{} does not exist; save directory edits before opening it",
                relative.display()
            )
        })?;
        Ok(Some(target))
    }

    /// The link target each row's entry points at, for rows that are symlinks.
    ///
    /// Read from the identity the row carries rather than from its text, so a
    /// link keeps saying what it points at while its name is being edited. A
    /// row that has become a new entry has no identity and no target.
    pub fn symlink_targets(&self) -> Vec<(usize, PathBuf)> {
        self.row_origins
            .iter()
            .enumerate()
            .filter_map(|(row, origin)| {
                let target = match origin.as_ref()? {
                    RowOrigin::Snapshot(id) => self.baseline.entry(*id)?.symlink_target()?,
                    RowOrigin::Transfer { expected, .. } => expected.symlink_target()?,
                };
                Some((row, target.to_path_buf()))
            })
            .collect()
    }

    /// Shows or hides presentation-only `ls -l` style fields before each
    /// filename, reporting whether anything changed.
    ///
    /// Whether they are shown is configuration rather than something this
    /// listing decides, so this sets a state rather than inverting one: two
    /// explorers asked for the same thing must end up alike however they
    /// started.
    pub fn set_details(&mut self, shown: bool, text: &str) -> bool {
        if shown == self.details.is_some() {
            return false;
        }
        self.details = shown.then(|| self.build_details());
        self.refresh_hints(text);
        true
    }

    pub fn details_shown(&self) -> bool {
        self.details.is_some()
    }

    /// The metadata prefixes carried by the rows' hidden identities.
    ///
    /// Edited names therefore remain the only editable part of the listing,
    /// while moved and renamed rows keep describing the same filesystem entry.
    pub fn detail_prefixes(&self) -> Vec<(usize, String)> {
        let Some(details) = self.details.as_ref() else {
            return Vec::new();
        };
        self.row_origins
            .iter()
            .enumerate()
            .filter_map(|(row, origin)| {
                let prefix = match origin.as_ref()? {
                    RowOrigin::Snapshot(id) => details.snapshots.get(id),
                    RowOrigin::Transfer { source, .. } => details.transfers.get(source),
                }?;
                Some((row, prefix.clone()))
            })
            .collect()
    }

    /// Cached annotations for this projection.
    ///
    /// They are rebuilt when identities, text, or metadata change. Cloning
    /// the value for a frame only clones shared map handles, so redraw cost is
    /// independent of the number of entries outside the viewport.
    pub fn row_hints(&self) -> RowHints {
        self.hints.clone()
    }

    pub fn detail_prefix_width(&self) -> usize {
        self.hints.prefix_width()
    }

    fn refresh_hints(&mut self, text: &str) {
        let lines = split_lines(text);
        let suffixes = self
            .row_origins
            .iter()
            .enumerate()
            .filter_map(|(row, origin)| {
                let target = match origin.as_ref()? {
                    RowOrigin::Snapshot(id) => self.baseline.entry(*id)?.symlink_target()?,
                    RowOrigin::Transfer { expected, .. } => expected.symlink_target()?,
                };
                Some((
                    row,
                    lines.get(row).map_or(0, |line| display_cells(line)),
                    format!("→ {}", target.display()),
                ))
            });
        self.hints = RowHints::aligned(suffixes).with_prefixes(self.detail_prefixes());
    }

    fn build_details(&self) -> DirectoryDetails {
        let mut users = HashMap::new();
        let mut groups = HashMap::new();
        let snapshots = self
            .baseline
            .entries()
            .iter()
            .map(|entry| {
                (
                    entry.id,
                    RawDetails::from_fields(entry.detail_fields(), &mut users, &mut groups),
                )
            })
            .collect::<Vec<_>>();
        let mut transfers = HashMap::new();
        for origin in self.row_origins.iter().flatten() {
            if let RowOrigin::Transfer {
                source, expected, ..
            } = origin
            {
                transfers.entry(source.clone()).or_insert_with(|| {
                    RawDetails::from_fields(expected.detail_fields(), &mut users, &mut groups)
                });
            }
        }
        let owner_width = snapshots
            .iter()
            .map(|(_, details)| display_cells(&details.owner))
            .chain(
                transfers
                    .values()
                    .map(|details| display_cells(&details.owner)),
            )
            .max()
            .unwrap_or(1);
        let group_width = snapshots
            .iter()
            .map(|(_, details)| display_cells(&details.group))
            .chain(
                transfers
                    .values()
                    .map(|details| display_cells(&details.group)),
            )
            .max()
            .unwrap_or(1);
        let size_width = snapshots
            .iter()
            .map(|(_, details)| human_size(details.len).len())
            .chain(
                transfers
                    .values()
                    .map(|details| human_size(details.len).len()),
            )
            .max()
            .unwrap_or(1);
        DirectoryDetails {
            snapshots: snapshots
                .into_iter()
                .map(|(id, details)| (id, details.format(owner_width, group_width, size_width)))
                .collect(),
            transfers: transfers
                .into_iter()
                .map(|(path, details)| (path, details.format(owner_width, group_width, size_width)))
                .collect(),
        }
    }

    fn refresh_details(&mut self) {
        if self.details.is_some() {
            self.details = Some(self.build_details());
        }
    }

    /// Resolves the semantic kind carried by one editable projection row.
    /// Existing and transferred entries retain their hidden identity while a
    /// new row derives its kind from the visible trailing marker.
    pub fn entry_kind_at_line(&self, line: &str, row: usize) -> Result<Option<EntryKind>> {
        if line.trim_end().is_empty() {
            return Ok(None);
        }
        let (_, requested_kind) = parse_line(line)?;
        let kind = match self.row_origins.get(row).and_then(Option::as_ref) {
            Some(RowOrigin::Snapshot(id)) => {
                self.baseline
                    .entry(*id)
                    .context("directory entry identity is stale")?
                    .kind
            }
            Some(RowOrigin::Transfer { kind, .. }) => *kind,
            None => requested_kind,
        };
        Ok(Some(kind))
    }

    /// Keeps hidden identities aligned with the editable rows.
    ///
    /// A same-sized edit is an in-place edit, so identities stay with their
    /// rows. When lines are inserted or removed, exact labels recover moved
    /// rows first; remaining changed rows inherit remaining identities in
    /// order. This covers modal cut/paste reordering without exposing IDs in
    /// the text.
    pub fn reconcile(&mut self, before: &str, after: &str) {
        let before_lines = split_lines(before);
        let after_lines = split_lines(after);
        self.row_origins.resize(before_lines.len(), None);
        let previous = self.row_origins.clone();
        let mut assigned = vec![None; after_lines.len()];
        let mut resolved = vec![false; after_lines.len()];
        let mut matched = vec![false; previous.len()];
        let mut by_label: HashMap<&str, Vec<(usize, Option<RowOrigin>)>> = HashMap::new();
        for (row, label) in before_lines.iter().enumerate() {
            if !label.is_empty() {
                by_label
                    .entry(label)
                    .or_default()
                    .push((row, previous.get(row).cloned().flatten()));
            }
        }
        for (row, label) in after_lines.iter().enumerate() {
            if !label.is_empty()
                && let Some(instances) = by_label.get(label)
            {
                if let Some((source_row, origin)) = instances
                    .iter()
                    .find(|(source_row, _)| !matched[*source_row])
                {
                    matched[*source_row] = true;
                    assigned[row] = origin.clone();
                    resolved[row] = true;
                } else if let Some(origin) = instances[0].1.as_ref()
                    && instances
                        .iter()
                        .all(|(_, candidate)| candidate.as_ref() == Some(origin))
                {
                    // A pasted copy initially has the same label as its source.
                    // Retain that source identity on the extra row; a later edit
                    // can then be planned as a copy instead of an empty create.
                    assigned[row] = Some(origin.clone());
                    resolved[row] = true;
                }
            }
        }

        let baseline_labels = self
            .baseline
            .entries()
            .iter()
            .filter_map(|entry| entry.display_name().ok().map(|label| (label, entry.id)))
            .collect::<HashMap<_, _>>();
        for (row, label) in after_lines.iter().enumerate() {
            if !resolved[row]
                && let Some(id) = baseline_labels.get(*label)
            {
                assigned[row] = Some(RowOrigin::Snapshot(*id));
                resolved[row] = true;
            }
        }

        for (row, label) in after_lines.iter().enumerate() {
            if !resolved[row]
                && !label.is_empty()
                && let Some(origins) = self.detached_origins.get_mut(*label)
            {
                assigned[row] = origins.pop();
                resolved[row] = true;
            }
        }
        self.detached_origins
            .retain(|_, origins| !origins.is_empty());

        // Editing commands such as `change` delete a row's contents and then
        // insert its replacement as separate transactions. Exact and restored
        // labels above get first claim so reorders follow their entries;
        // anything still unresolved at the same row is an in-place edit and
        // keeps its origin, including during the intermediate empty state.
        if before_lines.len() == after_lines.len() {
            for row in 0..after_lines.len() {
                if !resolved[row] && !matched[row] {
                    assigned[row] = previous[row].clone();
                    matched[row] = true;
                    resolved[row] = true;
                }
            }
        }

        let remaining_origins = previous
            .into_iter()
            .enumerate()
            .filter(|(row, _)| !matched[*row])
            .filter_map(|(row, origin)| origin.map(|origin| (row, origin)))
            .collect::<Vec<_>>();
        let mut remaining = remaining_origins.into_iter();
        for (row, identity) in assigned.iter_mut().enumerate() {
            if !resolved[row] && !after_lines[row].is_empty() {
                *identity = remaining.next().map(|(_, origin)| origin);
            }
        }
        for (row, origin) in remaining {
            if matches!(origin, RowOrigin::Transfer { .. })
                && let Some(label) = before_lines.get(row).filter(|label| !label.is_empty())
            {
                self.detached_origins
                    .entry((*label).to_owned())
                    .or_default()
                    .push(origin);
            }
        }
        self.row_origins = assigned;
        self.refresh_hints(after);
    }

    pub fn reload(&mut self, view: ListingView) -> Result<String> {
        let (fresh, text) = Self::open(self.root.clone(), view)?;
        *self = fresh;
        Ok(text)
    }

    /// Accepts the current editable order as the saved projection while
    /// refreshing its filesystem identities from disk.
    ///
    /// A successful filesystem plan has already made every visible path
    /// authoritative. Re-rendering the freshly sorted snapshot here would
    /// move renamed and created rows just as the person finishes editing
    /// them. Instead, map the current rows onto the fresh snapshot and keep
    /// their relative positions in the returned projection; the next explicit
    /// refresh or clean re-entry can render the canonical order.
    pub fn refresh_baseline_preserving_order(
        &mut self,
        text: &str,
        show_hidden: bool,
    ) -> Result<String> {
        let fresh = DirectorySnapshot::read_with(&self.root, show_hidden)?;
        let entries_by_path = fresh
            .entries()
            .iter()
            .map(|entry| (entry.path.as_path(), entry))
            .collect::<HashMap<_, _>>();
        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        for line in split_lines(text) {
            if line.trim_end().is_empty() {
                continue;
            }
            let (path, _) = parse_line(line)?;
            let Some(entry) = entries_by_path.get(path.as_path()) else {
                // A row may deliberately move or create an entry below a
                // child directory or above this root. It no longer belongs in
                // this immediate-child projection after the plan is applied.
                continue;
            };
            if seen.insert(entry.id) {
                entries.push(*entry);
            }
        }
        // The plan normally accounts for every new immediate child. Keep this
        // safe under unusual filesystem shapes by including anything the
        // fresh snapshot has that the edited text did not name.
        entries.extend(fresh.entries().iter().filter(|entry| seen.insert(entry.id)));
        let mut lines = entries
            .iter()
            .map(|entry| entry.display_name())
            .collect::<Result<Vec<_>>>()?;
        let row_origins = entries
            .iter()
            .map(|entry| Some(RowOrigin::Snapshot(entry.id)))
            .chain((!lines.is_empty()).then_some(None))
            .collect();
        if !lines.is_empty() {
            lines.push(String::new());
        }
        let projection = lines.join("\n");
        self.baseline = fresh;
        self.row_origins = row_origins;
        self.detached_origins.clear();
        self.refresh_details();
        self.refresh_hints(&projection);
        Ok(projection)
    }

    /// Advances a dirty listing past removals already completed by another
    /// explorer without discarding its unrelated edits.
    ///
    /// This is deliberately narrow: every on-disk difference must be one of
    /// `removed`, and no surviving row may still carry the removed identity.
    /// Additions and renames need a textual merge and therefore keep the old
    /// conflict behavior.
    pub fn rebase_after_external_removals(
        &mut self,
        text: &str,
        removed: &HashSet<PathBuf>,
    ) -> Result<bool> {
        let removed = self
            .baseline
            .entries()
            .iter()
            .filter(|entry| removed.contains(&self.root.join(&entry.path)))
            .map(|entry| entry.id)
            .collect::<HashSet<_>>();
        if removed.is_empty() {
            return Ok(false);
        }
        ensure!(
            self.row_origins.iter().all(|origin| !matches!(
                origin,
                Some(RowOrigin::Snapshot(id)) if removed.contains(id)
            )),
            "an externally removed entry still has pending directory edits"
        );

        let fresh = DirectorySnapshot::read_with(&self.root, self.baseline.show_hidden())?;
        let expected = self
            .baseline
            .entries()
            .iter()
            .filter(|entry| !removed.contains(&entry.id))
            .map(|entry| (&entry.path, entry.kind))
            .collect::<Vec<_>>();
        let current = fresh
            .entries()
            .iter()
            .map(|entry| (&entry.path, entry.kind))
            .collect::<Vec<_>>();
        ensure!(
            expected == current,
            "directory has other changes besides the completed move"
        );

        let old_paths = self
            .baseline
            .entries()
            .iter()
            .map(|entry| (entry.id, entry.path.clone()))
            .collect::<HashMap<_, _>>();
        let fresh_ids = fresh
            .entries()
            .iter()
            .map(|entry| (entry.path.clone(), entry.id))
            .collect::<HashMap<_, _>>();
        for origin in self.row_origins.iter_mut().flatten() {
            let RowOrigin::Snapshot(id) = origin else {
                continue;
            };
            let path = old_paths
                .get(id)
                .context("directory entry identity is stale")?;
            *id = *fresh_ids
                .get(path)
                .context("surviving directory entry disappeared")?;
        }
        self.baseline = fresh;
        self.refresh_details();
        self.refresh_hints(text);
        Ok(true)
    }

    pub fn baseline(&self) -> &DirectorySnapshot {
        &self.baseline
    }
}

impl RawDetails {
    fn from_fields(
        fields: EntryDetailFields,
        users: &mut HashMap<u32, String>,
        groups: &mut HashMap<u32, String>,
    ) -> Self {
        let EntryDetailFields {
            kind,
            len,
            modified_nanos,
            unix,
        } = fields;
        let (mode, owner, group) = unix.map_or_else(
            || (None, "-".to_owned(), "-".to_owned()),
            |(mode, uid, gid)| {
                (
                    Some(mode),
                    users
                        .entry(uid)
                        .or_insert_with(|| user_name(uid).unwrap_or_else(|| uid.to_string()))
                        .clone(),
                    groups
                        .entry(gid)
                        .or_insert_with(|| group_name(gid).unwrap_or_else(|| gid.to_string()))
                        .clone(),
                )
            },
        );
        Self {
            kind,
            len,
            modified_nanos,
            mode,
            owner,
            group,
        }
    }

    fn format(&self, owner_width: usize, group_width: usize, size_width: usize) -> String {
        let owner = pad_right(&self.owner, owner_width);
        let group = pad_right(&self.group, group_width);
        let size = pad_left(&human_size(self.len), size_width);
        format!(
            "{} {owner} {group} {size} {} ",
            permissions(self.kind, self.mode),
            modified_time(self.modified_nanos),
        )
    }
}

fn pad_right(text: &str, width: usize) -> String {
    format!(
        "{text}{}",
        " ".repeat(width.saturating_sub(display_cells(text)))
    )
}

fn pad_left(text: &str, width: usize) -> String {
    format!(
        "{}{text}",
        " ".repeat(width.saturating_sub(display_cells(text)))
    )
}

fn permissions(kind: EntryKind, mode: Option<u32>) -> String {
    let kind = match kind {
        EntryKind::File => '-',
        EntryKind::Directory => 'd',
        EntryKind::Symlink => 'l',
        EntryKind::Other => '?',
    };
    let Some(mode) = mode else {
        return format!("{kind}---------");
    };
    let mut text = String::with_capacity(10);
    text.push(kind);
    for (read, write, execute, special, special_execute, special_no_execute) in [
        (0o400, 0o200, 0o100, 0o4000, 's', 'S'),
        (0o040, 0o020, 0o010, 0o2000, 's', 'S'),
        (0o004, 0o002, 0o001, 0o1000, 't', 'T'),
    ] {
        text.push(if mode & read != 0 { 'r' } else { '-' });
        text.push(if mode & write != 0 { 'w' } else { '-' });
        text.push(match (mode & execute != 0, mode & special != 0) {
            (true, true) => special_execute,
            (false, true) => special_no_execute,
            (true, false) => 'x',
            (false, false) => '-',
        });
    }
    text
}

fn human_size(bytes: u64) -> String {
    const UNITS: &[char] = &['B', 'K', 'M', 'G', 'T', 'P', 'E'];
    if bytes < 1024 {
        return bytes.to_string();
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1}{}", UNITS[unit])
    } else {
        format!("{value:.0}{}", UNITS[unit])
    }
}

fn modified_time(nanos: Option<u128>) -> String {
    modified_time_for_year(nanos, Local::now().year())
}

fn modified_time_for_year(nanos: Option<u128>, current_year: i32) -> String {
    let Some(nanos) = nanos else {
        return "--- -- -----".to_owned();
    };
    let seconds = u64::try_from(nanos / 1_000_000_000).unwrap_or(u64::MAX);
    let subsecond = u32::try_from(nanos % 1_000_000_000).unwrap_or(0);
    let time = UNIX_EPOCH
        .checked_add(Duration::new(seconds, subsecond))
        .unwrap_or(UNIX_EPOCH);
    let local: DateTime<Local> = time.into();
    if local.year() == current_year {
        local.format("%b %e %H:%M").to_string()
    } else {
        local.format("%b %e  %Y").to_string()
    }
}

#[cfg(unix)]
fn user_name(uid: u32) -> Option<String> {
    unsafe {
        let mut record = std::mem::zeroed();
        let mut result = std::ptr::null_mut();
        let mut buffer = vec![0_u8; 16 * 1024];
        let status = libc::getpwuid_r(
            uid,
            &mut record,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        );
        (status == 0 && !result.is_null() && !record.pw_name.is_null()).then(|| {
            CStr::from_ptr(record.pw_name)
                .to_string_lossy()
                .into_owned()
        })
    }
}

#[cfg(not(unix))]
fn user_name(_uid: u32) -> Option<String> {
    None
}

#[cfg(unix)]
fn group_name(gid: u32) -> Option<String> {
    unsafe {
        let mut record = std::mem::zeroed();
        let mut result = std::ptr::null_mut();
        let mut buffer = vec![0_u8; 16 * 1024];
        let status = libc::getgrgid_r(
            gid,
            &mut record,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        );
        (status == 0 && !result.is_null() && !record.gr_name.is_null()).then(|| {
            CStr::from_ptr(record.gr_name)
                .to_string_lossy()
                .into_owned()
        })
    }
}

#[cfg(not(unix))]
fn group_name(_gid: u32) -> Option<String> {
    None
}

/// The rows of `entries`, in the order `sort` asks for.
///
/// The snapshot arrives in name order, so a row's index is its name key and an
/// ascending name sort has nothing of its own to compare. Every order sorts
/// stably and falls back to that index, which makes the name the tiebreak
/// between entries of equal size or equal modification time without a second
/// key having to say so.
fn sorted_rows(entries: &[SnapshotEntry], sort: ExplorerSort) -> Vec<usize> {
    let mut rows = (0..entries.len()).collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        let (first, second) = (
            entries[*left].detail_fields(),
            entries[*right].detail_fields(),
        );
        let directory = |fields: &EntryDetailFields| matches!(fields.kind, EntryKind::Directory);
        // `true` orders after `false`, so comparing the right against the left
        // is what puts directories first.
        directory(&second)
            .cmp(&directory(&first))
            .then_with(|| compare_by(sort, (*left, &first), (*right, &second)))
            .then_with(|| left.cmp(right))
    });
    rows
}

fn compare_by(
    sort: ExplorerSort,
    left: (usize, &EntryDetailFields),
    right: (usize, &EntryDetailFields),
) -> Ordering {
    let (left_row, left_fields) = left;
    let (right_row, right_fields) = right;
    match sort {
        ExplorerSort::Name => Ordering::Equal,
        ExplorerSort::NameDescending => right_row.cmp(&left_row),
        ExplorerSort::Modified => left_fields.modified_nanos.cmp(&right_fields.modified_nanos),
        ExplorerSort::ModifiedDescending => {
            right_fields.modified_nanos.cmp(&left_fields.modified_nanos)
        }
        // A directory's own length says how its entries are stored rather than
        // how much they hold, so it is not the number this order is about.
        // Directories keep their name order under either direction.
        ExplorerSort::Size | ExplorerSort::SizeDescending
            if matches!(left_fields.kind, EntryKind::Directory) =>
        {
            Ordering::Equal
        }
        ExplorerSort::Size => left_fields.len.cmp(&right_fields.len),
        ExplorerSort::SizeDescending => right_fields.len.cmp(&left_fields.len),
    }
}

fn render_snapshot(snapshot: &DirectorySnapshot, order: &[usize]) -> Result<String> {
    let mut lines = order
        .iter()
        .map(|row| snapshot.entries()[*row].display_name())
        .collect::<Result<Vec<_>>>()?;
    if lines.is_empty() {
        return Ok(String::new());
    }
    lines.push(String::new());
    Ok(lines.join("\n"))
}

fn split_lines(text: &str) -> Vec<&str> {
    text.split('\n').collect()
}

fn parse_line(line: &str) -> Result<(PathBuf, EntryKind)> {
    let line = line.trim_end();
    ensure!(
        !line.contains('\0'),
        "directory entries cannot contain NUL bytes"
    );
    let (name, kind) = if let Some(name) = line.strip_suffix('/') {
        (name, EntryKind::Directory)
    } else {
        (line, EntryKind::File)
    };
    ensure!(!name.is_empty(), "directory entries cannot be empty");
    ensure!(
        !name.chars().any(char::is_control),
        "directory entries cannot contain control characters"
    );
    let path = PathBuf::from(name);
    let text = path
        .to_str()
        .with_context(|| format!("{} is not valid UTF-8", path.display()))?;
    ensure!(
        !text.contains('\n'),
        "directory entries cannot contain newlines"
    );
    Ok((path, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_columns_pad_unicode_names_by_terminal_cells() {
        let details = RawDetails {
            kind: EntryKind::Other,
            len: 9,
            modified_nanos: None,
            mode: None,
            owner: "用".to_owned(),
            group: "e\u{301}".to_owned(),
        };

        assert_eq!(
            details.format(4, 3, 2),
            "?--------- 用   e\u{301}    9 --- -- ----- "
        );
    }

    #[test]
    fn details_use_a_year_for_modification_times_outside_the_current_year() {
        let july_2020 = 1_593_561_600_u128 * 1_000_000_000;

        assert!(modified_time_for_year(Some(july_2020), 2026).ends_with("2020"));
        assert!(!modified_time_for_year(Some(july_2020), 2020).ends_with("2020"));
        assert!(modified_time_for_year(Some(july_2020), 2020).contains(':'));
    }

    #[test]
    fn permission_details_include_special_execute_bits() {
        assert_eq!(permissions(EntryKind::File, Some(0o7744)), "-rwsr-Sr-T");
    }
}
