// SPDX-License-Identifier: MPL-2.0

//! Editable text projections of directories.
//!
//! The visible buffer contains one relative path per line. A trailing `/`
//! creates a directory; existing entry kinds are retained. Stable entry
//! identities live beside the text so reordering rows does not become a
//! delete-and-create plan and in-place edits remain renames.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, ensure};

use crate::fs_plan::{
    DesiredEntry, DirectorySnapshot, EntryId, EntryKind, FsPlan, SnapshotEntry, SourceFingerprint,
    TransferMode,
};

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
}

impl DirectoryBuffer {
    /// Projects `root` as text, listing dotfiles only when `show_hidden`.
    ///
    /// Hidden entries are left out of the baseline as well as the text, so a
    /// row missing from the listing is not read as a deletion when the plan
    /// is built.
    pub fn open(root: PathBuf, show_hidden: bool) -> Result<(Self, String)> {
        let root = fs::canonicalize(&root)
            .with_context(|| format!("failed to resolve directory {}", root.display()))?;
        let baseline = DirectorySnapshot::read_with(&root, show_hidden)?;
        let text = render_snapshot(&baseline)?;
        let mut row_origins = baseline
            .entries()
            .iter()
            .map(|entry| Some(RowOrigin::Snapshot(entry.id)))
            .collect::<Vec<_>>();
        if !row_origins.is_empty() {
            row_origins.push(None);
        }
        Ok((
            Self {
                root,
                baseline,
                row_origins,
                detached_origins: HashMap::new(),
            },
            text,
        ))
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
    }

    pub fn reload(&mut self, show_hidden: bool) -> Result<String> {
        let (fresh, text) = Self::open(self.root.clone(), show_hidden)?;
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
        Ok(projection)
    }

    /// Advances a dirty listing past removals already completed by another
    /// explorer without discarding its unrelated edits.
    ///
    /// This is deliberately narrow: every on-disk difference must be one of
    /// `removed`, and no surviving row may still carry the removed identity.
    /// Additions and renames need a textual merge and therefore keep the old
    /// conflict behavior.
    pub fn rebase_after_external_removals(&mut self, removed: &HashSet<PathBuf>) -> Result<bool> {
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
        Ok(true)
    }

    pub fn baseline(&self) -> &DirectorySnapshot {
        &self.baseline
    }
}

fn render_snapshot(snapshot: &DirectorySnapshot) -> Result<String> {
    let mut lines = snapshot
        .entries()
        .iter()
        .map(SnapshotEntry::display_name)
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
