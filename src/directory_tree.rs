// SPDX-License-Identifier: MPL-2.0

//! Lazy, path-identified navigation over one workspace directory.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use anyhow::{Result, ensure};
use tokio::sync::Notify;

use crate::fs_plan::{
    DirectorySnapshot, EntryKind, FsOperation, FsPlan, OperationLimits, SourceFingerprint,
    lexical_normalize, relative_from_root,
};

const MAX_DIRECTORY_ENTRIES: usize = 4096;
const MAX_OUTSTANDING: usize = 64;
const MAX_CACHED_LISTINGS: usize = 512;
const MAX_VISIBLE_ROWS: usize = 100_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeEntry {
    pub path: PathBuf,
    pub kind: EntryKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeRow {
    pub path: PathBuf,
    pub kind: EntryKind,
    pub depth: usize,
    pub expanded: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub pending: Option<String>,
}

#[derive(Clone, Debug)]
pub struct StagedIntention {
    pub source: Option<PathBuf>,
    pub destination: Option<PathBuf>,
    pub kind: EntryKind,
    pub fingerprint: Option<SourceFingerprint>,
}

#[derive(Clone, Debug)]
struct PendingState {
    intentions: Vec<StagedIntention>,
    baselines: HashMap<PathBuf, DirectorySnapshot>,
}

struct ListingRequest {
    path: PathBuf,
    generation: u64,
    show_hidden: bool,
}

struct ListingResult {
    path: PathBuf,
    generation: u64,
    entries: Result<Vec<TreeEntry>, String>,
}

/// Tree navigation state is retained when the sidebar is hidden. The worker
/// starts only after the first request and never recursively walks a directory.
pub struct DirectoryTree {
    pub root: PathBuf,
    pub visible: bool,
    pub focused: bool,
    pub selected: PathBuf,
    pub scroll: usize,
    pub viewport_rows: usize,
    expanded: HashSet<PathBuf>,
    revealed_hidden: HashSet<PathBuf>,
    listings: HashMap<PathBuf, Vec<TreeEntry>>,
    errors: HashMap<PathBuf, String>,
    outstanding: HashMap<PathBuf, u64>,
    queued_refresh: HashMap<PathBuf, bool>,
    generation: u64,
    requests: Option<Sender<ListingRequest>>,
    results: Receiver<ListingResult>,
    result_sender: Option<Sender<ListingResult>>,
    wake: Arc<Notify>,
    pending: PendingState,
    history: Vec<PendingState>,
    pub revision: u64,
}

impl DirectoryTree {
    pub fn new(root: PathBuf) -> Self {
        let (result_sender, results) = mpsc::channel();
        let wake = Arc::new(Notify::new());
        Self {
            selected: root.clone(),
            root,
            visible: false,
            focused: false,
            scroll: 0,
            viewport_rows: 15,
            expanded: HashSet::new(),
            revealed_hidden: HashSet::new(),
            listings: HashMap::new(),
            errors: HashMap::new(),
            outstanding: HashMap::new(),
            queued_refresh: HashMap::new(),
            generation: 0,
            requests: None,
            results,
            result_sender: Some(result_sender),
            wake,
            pending: PendingState {
                intentions: Vec::new(),
                baselines: HashMap::new(),
            },
            history: Vec::new(),
            revision: 0,
        }
    }

    pub fn show(&mut self, show_hidden: bool) {
        self.visible = true;
        self.expand(self.root.clone(), show_hidden);
    }

    pub fn expand(&mut self, path: PathBuf, show_hidden: bool) {
        if path == self.root || self.kind(&path) == Some(EntryKind::Directory) {
            if !self.expanded.contains(&path) && self.expanded.len() >= MAX_CACHED_LISTINGS {
                self.errors
                    .insert(path, "too many expanded directories".into());
                return;
            }
            self.expanded.insert(path.clone());
            if !self.listings.contains_key(&path)
                && !self.pending.intentions.iter().any(|intent| {
                    intent.source.is_none() && intent.destination.as_deref() == Some(&path)
                })
            {
                let include_hidden = show_hidden || self.revealed_hidden.contains(&path);
                self.refresh(path, include_hidden);
            }
        }
    }

    pub fn refresh(&mut self, path: PathBuf, show_hidden: bool) {
        if !self.expanded.contains(&path) {
            return;
        }
        let show_hidden = show_hidden || self.revealed_hidden.contains(&path);
        if self.outstanding.contains_key(&path) {
            self.queued_refresh
                .entry(path)
                .and_modify(|hidden| *hidden |= show_hidden)
                .or_insert(show_hidden);
            return;
        }
        if self.outstanding.len() >= MAX_OUTSTANDING {
            self.errors
                .insert(path, "too many pending directory listings".into());
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.outstanding.insert(path.clone(), generation);
        self.errors.remove(&path);
        if self.requests.is_none() {
            let (requests, receiver) = mpsc::channel::<ListingRequest>();
            let result_sender = self.result_sender.take().expect("tree worker starts once");
            let wake = Arc::clone(&self.wake);
            thread::Builder::new()
                .name("directory-tree-listing".into())
                .spawn(move || {
                    while let Ok(request) = receiver.recv() {
                        let entries = read_directory(&request.path, request.show_hidden)
                            .map_err(|error| error.to_string());
                        if result_sender
                            .send(ListingResult {
                                path: request.path,
                                generation: request.generation,
                                entries,
                            })
                            .is_err()
                        {
                            break;
                        }
                        wake.notify_one();
                    }
                })
                .expect("tree listing worker starts");
            self.requests = Some(requests);
        }
        if self.requests.as_ref().is_none_or(|requests| {
            requests
                .send(ListingRequest {
                    path: path.clone(),
                    generation,
                    show_hidden,
                })
                .is_err()
        }) {
            self.outstanding.remove(&path);
            self.errors
                .insert(path, "directory listing worker stopped".into());
        }
    }

    pub fn wake(&self) -> Arc<Notify> {
        Arc::clone(&self.wake)
    }

    /// Applies only the latest request for each path. Call from the mutable
    /// frame lifecycle or event loop, never from snapshot conversion.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(result) = self.results.try_recv() {
            if self.outstanding.get(&result.path) != Some(&result.generation) {
                continue;
            }
            self.outstanding.remove(&result.path);
            let refreshed_path = result.path.clone();
            match result.entries {
                Ok(entries) => {
                    if !self.listings.contains_key(&result.path)
                        && self.listings.len() >= MAX_CACHED_LISTINGS
                    {
                        if let Some(stale) = self
                            .listings
                            .keys()
                            .find(|path| **path != self.root && !self.expanded.contains(*path))
                            .cloned()
                        {
                            self.listings.remove(&stale);
                            self.errors.remove(&stale);
                        } else {
                            self.errors
                                .insert(result.path, "too many expanded directories".into());
                            changed = true;
                            continue;
                        }
                    }
                    self.listings.insert(result.path.clone(), entries);
                    self.errors.remove(&result.path);
                }
                Err(error) => {
                    self.errors.insert(result.path, error);
                }
            }
            if let Some(show_hidden) = self.queued_refresh.remove(&refreshed_path) {
                self.refresh(refreshed_path, show_hidden);
            }
            changed = true;
        }
        if changed
            && self.outstanding.is_empty()
            && !self.rows().iter().any(|row| row.path == self.selected)
        {
            let mut ancestor = self.selected.as_path();
            while ancestor != self.root && !self.rows().iter().any(|row| row.path == ancestor) {
                ancestor = ancestor.parent().unwrap_or(&self.root);
            }
            self.selected = ancestor.to_path_buf();
        }
        changed
    }

    pub fn rows(&self) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        self.append_rows(&self.root, EntryKind::Directory, 0, &mut rows);
        rows
    }

    fn append_rows(&self, path: &Path, kind: EntryKind, depth: usize, rows: &mut Vec<TreeRow>) {
        if rows.len() >= MAX_VISIBLE_ROWS {
            return;
        }
        let expanded = self.expanded.contains(path);
        rows.push(TreeRow {
            path: path.to_path_buf(),
            kind,
            depth,
            expanded,
            loading: self.outstanding.contains_key(path),
            error: self.errors.get(path).cloned(),
            pending: self.pending_for(path).map(|intent| {
                match (&intent.source, &intent.destination) {
                    (None, Some(_)) => "new".to_owned(),
                    (Some(_), None) => "delete".to_owned(),
                    (Some(_), Some(to)) => {
                        format!("→ {}", to.strip_prefix(&self.root).unwrap_or(to).display())
                    }
                    (None, None) => String::new(),
                }
            }),
        });
        if expanded && depth < 128 {
            let mut entries = self.listings.get(path).cloned().unwrap_or_default();
            for intent in &self.pending.intentions {
                if intent.source.is_none()
                    && let Some(target) = &intent.destination
                    && target.parent() == Some(path)
                    && !entries.iter().any(|entry| entry.path == *target)
                {
                    entries.push(TreeEntry {
                        path: target.clone(),
                        kind: intent.kind,
                    });
                }
            }
            entries.sort_by(|left, right| {
                (left.kind != EntryKind::Directory)
                    .cmp(&(right.kind != EntryKind::Directory))
                    .then_with(|| left.path.file_name().cmp(&right.path.file_name()))
            });
            for entry in entries {
                self.append_rows(&entry.path, entry.kind, depth + 1, rows);
            }
        }
    }

    pub fn kind(&self, path: &Path) -> Option<EntryKind> {
        if path == self.root {
            return Some(EntryKind::Directory);
        }
        if let Some(intent) =
            self.pending.intentions.iter().find(|intent| {
                intent.source.is_none() && intent.destination.as_deref() == Some(path)
            })
        {
            return Some(intent.kind);
        }
        self.listings
            .get(path.parent()?)?
            .iter()
            .find(|entry| entry.path == path)
            .map(|entry| entry.kind)
    }

    pub fn select_relative(&mut self, amount: isize) {
        let rows = self.rows();
        let current = rows
            .iter()
            .position(|row| row.path == self.selected)
            .unwrap_or(0);
        let last = rows.len().saturating_sub(1);
        self.selected = rows[current.saturating_add_signed(amount).min(last)]
            .path
            .clone();
    }

    pub fn select_first(&mut self) {
        self.selected = self.root.clone();
    }

    pub fn select_last(&mut self) {
        if let Some(last) = self.rows().last() {
            self.selected = last.path.clone();
        }
    }

    pub fn collapse_or_parent(&mut self) {
        if self.selected != self.root && self.expanded.remove(&self.selected) {
            return;
        }
        if let Some(parent) = self
            .selected
            .parent()
            .filter(|path| path.starts_with(&self.root))
        {
            self.selected = parent.to_path_buf();
        }
    }

    pub fn expand_or_child(&mut self, show_hidden: bool) {
        if self.kind(&self.selected) != Some(EntryKind::Directory) {
            return;
        }
        if self.expanded.contains(&self.selected) {
            let rows = self.rows();
            if let Some(index) = rows.iter().position(|row| row.path == self.selected)
                && let Some(child) = rows.get(index + 1)
                && child.depth == rows[index].depth + 1
            {
                self.selected = child.path.clone();
            }
        } else {
            self.expand(self.selected.clone(), show_hidden);
        }
    }

    pub fn toggle_selected(&mut self, show_hidden: bool) {
        if self.kind(&self.selected) == Some(EntryKind::Directory)
            && !self.expanded.remove(&self.selected)
        {
            self.expand(self.selected.clone(), show_hidden);
        }
    }

    pub fn reveal(&mut self, path: &Path, show_hidden: bool) -> Result<()> {
        ensure!(
            path.starts_with(&self.root),
            "path is outside this workspace"
        );
        let mut ancestors = Vec::new();
        let mut next = path.parent();
        while let Some(parent) = next {
            if !parent.starts_with(&self.root) {
                break;
            }
            ancestors.push(parent.to_path_buf());
            next = parent.parent();
        }
        ancestors.reverse();
        for parent in ancestors {
            // Reveal is an explicit file action, so it can verify each
            // ancestor before its parent's background listing has arrived.
            let kind = fs::symlink_metadata(&parent)?.file_type();
            ensure!(kind.is_dir(), "reveal path has a non-directory ancestor");
            if !show_hidden
                && path.strip_prefix(&parent).is_ok_and(|rest| {
                    rest.components()
                        .next()
                        .is_some_and(|part| part.as_os_str().to_string_lossy().starts_with('.'))
                })
            {
                self.revealed_hidden.insert(parent.clone());
                self.expanded.insert(parent.clone());
                self.refresh(parent, true);
            } else {
                self.expanded.insert(parent.clone());
                if !self.listings.contains_key(&parent) {
                    self.refresh(parent, show_hidden);
                }
            }
        }
        self.selected = path.to_path_buf();
        Ok(())
    }

    pub fn pending_count(&self) -> usize {
        self.pending.intentions.len()
    }

    pub fn pending_for(&self, path: &Path) -> Option<&StagedIntention> {
        self.pending.intentions.iter().find(|intent| {
            intent.source.as_deref() == Some(path)
                || intent.source.is_none() && intent.destination.as_deref() == Some(path)
        })
    }

    pub fn planned_destination(&self, source: &Path) -> Option<&Path> {
        self.pending
            .intentions
            .iter()
            .find(|intent| intent.source.as_deref() == Some(source))
            .and_then(|intent| intent.destination.as_deref())
    }

    fn remember_state(&mut self) {
        self.history.push(self.pending.clone());
        if self.history.len() > 128 {
            self.history.remove(0);
        }
        self.revision = self.revision.wrapping_add(1);
    }

    fn capture_parent(
        &self,
        parent: &Path,
        show_hidden: bool,
    ) -> Result<Option<(PathBuf, DirectorySnapshot)>> {
        if self.pending.baselines.contains_key(parent)
            || self.pending.intentions.iter().any(|intent| {
                intent.source.is_none() && intent.destination.as_deref() == Some(parent)
            })
        {
            return Ok(None);
        }
        let snapshot = DirectorySnapshot::read_bounded(parent, show_hidden, MAX_DIRECTORY_ENTRIES)?;
        Ok(Some((parent.to_path_buf(), snapshot)))
    }

    fn validate_target(&self, target: &Path) -> Result<()> {
        ensure!(
            target.is_absolute(),
            "tree operation target must be absolute"
        );
        ensure!(target != self.root, "the workspace root cannot be changed");
        ensure!(
            target.starts_with(&self.root),
            "tree operation target is outside the workspace"
        );
        ensure!(
            target
                .components()
                .all(|component| !matches!(component, Component::ParentDir)),
            "parent traversal is not allowed"
        );
        Ok(())
    }

    pub fn stage_create(
        &mut self,
        parent: &Path,
        name: &str,
        show_hidden: bool,
    ) -> Result<PathBuf> {
        ensure!(
            self.pending.intentions.len() < 4096,
            "too many staged operations"
        );
        ensure!(!name.trim().is_empty(), "new entry needs a name");
        let directory = name.ends_with(std::path::MAIN_SEPARATOR) || name.ends_with('/');
        let name = name.trim_end_matches(['/', '\\']);
        let relative = Path::new(name);
        ensure!(
            !relative.is_absolute()
                && relative
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "new entry needs a relative path without parent traversal"
        );
        let target = parent.join(relative);
        self.validate_target(&target)?;
        ensure!(
            !self
                .pending
                .intentions
                .iter()
                .any(|intent| intent.kind == EntryKind::Directory
                    && intent
                        .source
                        .as_ref()
                        .is_some_and(|source| target.starts_with(source))),
            "apply the parent directory change separately"
        );
        ensure!(
            fs::symlink_metadata(&target)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                && !self
                    .pending
                    .intentions
                    .iter()
                    .any(|intent| intent.destination.as_deref() == Some(&target)),
            "target already exists: {}",
            target.display()
        );
        let baseline = self.capture_parent(target.parent().unwrap_or(parent), show_hidden)?;
        self.remember_state();
        if let Some((parent, snapshot)) = baseline {
            self.pending.baselines.insert(parent, snapshot);
        }
        self.pending.intentions.push(StagedIntention {
            source: None,
            destination: Some(target.clone()),
            kind: if directory {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            fingerprint: None,
        });
        Ok(target)
    }

    fn stage_existing(
        &mut self,
        source: &Path,
        destination: Option<PathBuf>,
        show_hidden: bool,
    ) -> Result<()> {
        ensure!(
            self.pending.intentions.len() < 4096,
            "too many staged operations"
        );
        self.validate_target(source)?;
        if let Some(destination) = &destination {
            ensure!(
                destination.is_absolute(),
                "move destination must be absolute"
            );
            ensure!(source != destination, "source and destination are the same");
            ensure!(
                !destination.starts_with(source) || self.kind(source) != Some(EntryKind::Directory),
                "cannot move a directory inside itself"
            );
            ensure!(
                !self
                    .pending
                    .intentions
                    .iter()
                    .any(|intent| intent.destination.as_deref() == Some(destination)
                        && intent.source.as_deref() != Some(source)),
                "duplicate final target: {}",
                destination.display()
            );
        }
        if let Some(index) = self.pending.intentions.iter().position(|intent| {
            intent.source.is_none() && intent.destination.as_deref() == Some(source)
        }) {
            if destination.is_none() {
                self.remember_state();
                self.pending.intentions.remove(index);
                self.pending.intentions.retain(|intent| {
                    !intent
                        .destination
                        .as_ref()
                        .is_some_and(|target| target.starts_with(source))
                });
                return Ok(());
            }
            anyhow::bail!("apply the new entry before moving it");
        }
        let index = self
            .pending
            .intentions
            .iter()
            .position(|intent| intent.source.as_deref() == Some(source));
        let (kind, fingerprint) = if let Some(index) = index {
            let intent = &self.pending.intentions[index];
            (intent.kind, intent.fingerprint.clone())
        } else {
            let fingerprint = SourceFingerprint::capture_limited(
                source,
                Some(OperationLimits {
                    entries: 50_000,
                    depth: 128,
                    bytes: u64::MAX,
                    metadata_bytes: 64 * 1024 * 1024,
                }),
            )?;
            (fingerprint.kind(), Some(fingerprint))
        };
        if kind == EntryKind::Directory {
            ensure!(
                !self.pending.intentions.iter().any(|intent| intent
                    .source
                    .as_ref()
                    .is_some_and(|other| other != source && other.starts_with(source))
                    || intent
                        .destination
                        .as_ref()
                        .is_some_and(|other| other != source && other.starts_with(source))),
                "apply directory and descendant changes separately"
            );
        }
        let source_baseline =
            self.capture_parent(source.parent().unwrap_or(&self.root), show_hidden)?;
        let destination_baseline =
            if let Some(parent) = destination.as_ref().and_then(|path| path.parent()) {
                self.capture_parent(parent, show_hidden)?
            } else {
                None
            };
        self.remember_state();
        if let Some((parent, snapshot)) = source_baseline {
            self.pending.baselines.insert(parent, snapshot);
        }
        if let Some((parent, snapshot)) = destination_baseline {
            self.pending.baselines.entry(parent).or_insert(snapshot);
        }
        let intent = StagedIntention {
            source: Some(source.to_path_buf()),
            destination,
            kind,
            fingerprint,
        };
        if let Some(index) = index {
            self.pending.intentions[index] = intent;
        } else {
            self.pending.intentions.push(intent);
        }
        Ok(())
    }

    pub fn stage_rename(
        &mut self,
        source: &Path,
        name: &str,
        show_hidden: bool,
    ) -> Result<PathBuf> {
        let name = Path::new(name);
        ensure!(
            name.components().count() == 1
                && matches!(name.components().next(), Some(Component::Normal(_))),
            "rename accepts one name"
        );
        let target = self
            .planned_destination(source)
            .unwrap_or(source)
            .parent()
            .unwrap_or(&self.root)
            .join(name);
        self.stage_existing(source, Some(target.clone()), show_hidden)?;
        Ok(target)
    }

    pub fn stage_move(&mut self, source: &Path, path: &str, show_hidden: bool) -> Result<PathBuf> {
        ensure!(!path.trim().is_empty(), "move needs a destination");
        let input = Path::new(path);
        let mut target = lexical_normalize(&if input.is_absolute() {
            input.to_path_buf()
        } else {
            source.parent().unwrap_or(&self.root).join(input)
        });
        if target.is_dir() || path.ends_with('/') || path.ends_with(std::path::MAIN_SEPARATOR) {
            let current_name = self
                .pending
                .intentions
                .iter()
                .find(|intent| intent.source.as_deref() == Some(source))
                .and_then(|intent| intent.destination.as_ref())
                .and_then(|destination| destination.file_name())
                .or_else(|| source.file_name())
                .ok_or_else(|| anyhow::anyhow!("source has no name"))?
                .to_os_string();
            target.push(current_name);
        }
        self.stage_existing(source, Some(target.clone()), show_hidden)?;
        Ok(target)
    }

    pub fn stage_delete(&mut self, source: &Path, show_hidden: bool) -> Result<()> {
        self.stage_existing(source, None, show_hidden)
    }

    pub fn undo(&mut self) -> bool {
        if let Some(previous) = self.history.pop() {
            self.pending = previous;
            self.revision = self.revision.wrapping_add(1);
            true
        } else {
            false
        }
    }

    pub fn clear_pending(&mut self) {
        self.pending.intentions.clear();
        self.pending.baselines.clear();
        self.history.clear();
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn build_plan(&self) -> Result<FsPlan> {
        let mut operations = Vec::new();
        let mut sources = Vec::new();
        for intent in &self.pending.intentions {
            let relative = |path: &Path| relative_from_root(&self.root, path);
            let operation = match (&intent.source, &intent.destination) {
                (None, Some(to)) => FsOperation::Create {
                    path: relative(to)?,
                    kind: intent.kind,
                },
                (Some(from), None) => FsOperation::Delete {
                    path: relative(from)?,
                    kind: intent.kind,
                },
                (Some(from), Some(to)) => {
                    let from_relative = relative(from)?;
                    let to_relative = relative(to)?;
                    if from.parent() == to.parent() {
                        FsOperation::Rename {
                            from: from_relative,
                            to: to_relative,
                            kind: intent.kind,
                        }
                    } else {
                        FsOperation::Move {
                            from: from_relative,
                            to: to_relative,
                            kind: intent.kind,
                        }
                    }
                }
                (None, None) => unreachable!(),
            };
            if let (Some(from), Some(fingerprint)) = (&intent.source, &intent.fingerprint) {
                sources.push((relative(from)?, fingerprint.clone()));
            }
            operations.push(operation);
        }
        let baselines = self
            .pending
            .baselines
            .iter()
            .map(|(path, snapshot)| Ok((relative_from_root(&self.root, path)?, snapshot.clone())))
            .collect::<Result<Vec<_>>>()?;
        FsPlan::build_explicit(self.root.clone(), operations, baselines, sources)
    }
}

fn read_directory(path: &Path, show_hidden: bool) -> Result<Vec<TreeEntry>> {
    let mut entries = Vec::new();
    for result in fs::read_dir(path)? {
        let entry = result?;
        if !show_hidden && entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        ensure!(
            entries.len() < MAX_DIRECTORY_ENTRIES,
            "directory has too many entries"
        );
        let kind = match fs::symlink_metadata(entry.path())?.file_type() {
            kind if kind.is_dir() => EntryKind::Directory,
            kind if kind.is_file() => EntryKind::File,
            kind if kind.is_symlink() => EntryKind::Symlink,
            _ => EntryKind::Other,
        };
        entries.push(TreeEntry {
            path: entry.path(),
            kind,
        });
    }
    entries.sort_by(|left, right| {
        (left.kind != EntryKind::Directory)
            .cmp(&(right.kind != EntryKind::Directory))
            .then_with(|| left.path.file_name().cmp(&right.path.file_name()))
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_plan::DeletionMode;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);

    struct Temp(PathBuf);
    impl Temp {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "runyte-tree-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn await_rows(tree: &mut DirectoryTree, minimum: usize) {
        for _ in 0..10_000 {
            tree.poll();
            if tree.rows().len() >= minimum {
                return;
            }
            thread::yield_now();
        }
        panic!("tree listing did not arrive");
    }

    #[test]
    fn lazy_listing_navigation_and_hidden_reveal() {
        let root = Temp::new("listing");
        fs::create_dir(root.0.join("a")).unwrap();
        fs::write(root.0.join("b.txt"), "b").unwrap();
        fs::write(root.0.join(".hidden"), "h").unwrap();
        let mut tree = DirectoryTree::new(root.0.clone());
        assert_eq!(tree.rows().len(), 1);
        tree.show(false);
        await_rows(&mut tree, 3);
        assert_eq!(
            tree.rows()
                .iter()
                .map(|row| row.path.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![
                root.0.file_name().unwrap().to_string_lossy(),
                "a".into(),
                "b.txt".into()
            ]
        );
        tree.select_relative(1);
        assert_eq!(tree.selected, root.0.join("a"));
        tree.expand_or_child(false);
        tree.collapse_or_parent();
        assert_eq!(tree.selected, root.0.join("a"));
        tree.reveal(&root.0.join(".hidden"), false).unwrap();
        await_rows(&mut tree, 4);
        assert!(
            tree.rows()
                .iter()
                .any(|row| row.path == root.0.join(".hidden"))
        );
        assert_eq!(tree.selected, root.0.join(".hidden"));
    }

    #[tokio::test]
    async fn fresh_nested_reveal_lists_every_ancestor_and_wakes_the_ui() {
        let root = Temp::new("nested-reveal");
        fs::create_dir_all(root.0.join("a/b")).unwrap();
        let file = root.0.join("a/b/file.txt");
        fs::write(&file, "content").unwrap();
        let mut tree = DirectoryTree::new(root.0.clone());
        tree.show(false);
        tree.reveal(&file, false).unwrap();
        let wake = tree.wake();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !tree.rows().iter().any(|row| row.path == file) {
                wake.notified().await;
                tree.poll();
            }
        })
        .await
        .unwrap();
        assert_eq!(tree.selected, file);
    }

    #[test]
    fn repeated_refreshes_coalesce_while_one_listing_is_outstanding() {
        let root = Temp::new("coalesced-refresh");
        let mut tree = DirectoryTree::new(root.0.clone());
        tree.show(false);
        let generation = tree.outstanding[&root.0];
        for _ in 0..10_000 {
            tree.refresh(root.0.clone(), false);
        }
        assert_eq!(tree.outstanding.len(), 1);
        assert_eq!(tree.outstanding[&root.0], generation);
        assert_eq!(tree.queued_refresh.len(), 1);
    }

    #[test]
    fn staged_cross_directory_plan_is_reviewed_together() {
        let root = Temp::new("staging");
        fs::create_dir(root.0.join("left")).unwrap();
        fs::create_dir(root.0.join("right")).unwrap();
        fs::write(root.0.join("left/a.txt"), "a").unwrap();
        fs::write(root.0.join("right/b.txt"), "b").unwrap();
        let mut tree = DirectoryTree::new(root.0.clone());
        tree.stage_move(&root.0.join("left/a.txt"), "../right/a.txt", true)
            .unwrap();
        tree.stage_rename(&root.0.join("right/b.txt"), "c.txt", true)
            .unwrap();
        assert!(root.0.join("left/a.txt").exists());
        assert!(root.0.join("right/b.txt").exists());
        assert_eq!(tree.pending_count(), 2);
        let plan = tree.build_plan().unwrap();
        assert_eq!(plan.operations().len(), 2);
        plan.apply(DeletionMode::Permanent).unwrap();
        assert_eq!(fs::read(root.0.join("right/a.txt")).unwrap(), b"a");
        assert_eq!(fs::read(root.0.join("right/c.txt")).unwrap(), b"b");
        assert!(!root.0.join("left/a.txt").exists());
    }

    #[test]
    fn staged_source_and_directory_changes_refuse_every_operation() {
        let root = Temp::new("stale");
        fs::create_dir(root.0.join("left")).unwrap();
        fs::create_dir(root.0.join("right")).unwrap();
        fs::write(root.0.join("left/a.txt"), "a").unwrap();
        let mut tree = DirectoryTree::new(root.0.clone());
        tree.stage_move(&root.0.join("left/a.txt"), "../right/a.txt", true)
            .unwrap();
        tree.stage_create(&root.0.join("right"), "new.txt", true)
            .unwrap();
        let plan = tree.build_plan().unwrap();
        fs::write(root.0.join("left/a.txt"), "external").unwrap();
        assert!(plan.apply(DeletionMode::Permanent).is_err());
        assert!(!root.0.join("right/a.txt").exists());
        assert!(!root.0.join("right/new.txt").exists());
        fs::write(root.0.join("left/a.txt"), "a").unwrap();
        // The captured source identity is still stale even if text is restored.
        assert!(plan.apply(DeletionMode::Permanent).is_err());
    }

    #[test]
    fn staged_creation_undo_and_synthetic_parent() {
        let root = Temp::new("undo");
        let mut tree = DirectoryTree::new(root.0.clone());
        tree.show(true);
        tree.stage_create(&root.0, "new/", true).unwrap();
        tree.expand(root.0.join("new"), true);
        tree.stage_create(&root.0.join("new"), "child.txt", true)
            .unwrap();
        assert_eq!(tree.pending_count(), 2);
        assert!(
            tree.rows()
                .iter()
                .any(|row| row.path == root.0.join("new/child.txt"))
        );
        assert!(tree.undo());
        assert_eq!(tree.pending_count(), 1);
        tree.stage_delete(&root.0.join("new"), true).unwrap();
        assert_eq!(tree.pending_count(), 0);
        assert!(!root.0.join("new").exists());
    }

    #[test]
    fn staged_nested_creation_applies_in_parent_order() {
        let root = Temp::new("nested-create");
        let mut tree = DirectoryTree::new(root.0.clone());
        tree.stage_create(&root.0, "new/", true).unwrap();
        tree.stage_create(&root.0.join("new"), "child.txt", true)
            .unwrap();
        let plan = tree.build_plan().unwrap();
        assert!(!root.0.join("new").exists());
        plan.apply(DeletionMode::Permanent).unwrap();
        assert!(root.0.join("new/child.txt").is_file());
    }

    #[test]
    fn all_tree_actions_stage_without_changing_disk_and_apply_together() {
        let root = Temp::new("four-actions");
        fs::create_dir(root.0.join("other")).unwrap();
        for name in ["rename.txt", "move.txt", "delete.txt"] {
            fs::write(root.0.join(name), name).unwrap();
        }
        let mut tree = DirectoryTree::new(root.0.clone());
        tree.stage_create(&root.0, "new.txt", true).unwrap();
        tree.stage_rename(&root.0.join("rename.txt"), "renamed.txt", true)
            .unwrap();
        tree.stage_move(&root.0.join("move.txt"), "other/moved.txt", true)
            .unwrap();
        tree.stage_delete(&root.0.join("delete.txt"), true).unwrap();
        assert_eq!(tree.pending_count(), 4);
        assert!(!root.0.join("new.txt").exists());
        for name in ["rename.txt", "move.txt", "delete.txt"] {
            assert!(root.0.join(name).exists());
        }
        let plan = tree.build_plan().unwrap();
        plan.apply(DeletionMode::Permanent).unwrap();
        assert!(root.0.join("new.txt").exists());
        assert!(root.0.join("renamed.txt").exists());
        assert!(root.0.join("other/moved.txt").exists());
        assert!(!root.0.join("delete.txt").exists());
    }

    #[test]
    fn moving_a_staged_rename_into_a_directory_keeps_the_new_basename() {
        let root = Temp::new("rename-then-move");
        fs::create_dir(root.0.join("dest")).unwrap();
        let source = root.0.join("old.txt");
        fs::write(&source, "content").unwrap();
        let mut tree = DirectoryTree::new(root.0.clone());
        tree.stage_rename(&source, "new.txt", true).unwrap();
        assert_eq!(
            tree.stage_move(&source, "dest/", true).unwrap(),
            root.0.join("dest/new.txt")
        );
        assert_eq!(tree.pending_count(), 1);
        tree.build_plan()
            .unwrap()
            .apply(DeletionMode::Permanent)
            .unwrap();
        assert_eq!(fs::read(root.0.join("dest/new.txt")).unwrap(), b"content");
    }

    #[test]
    fn renaming_a_staged_move_keeps_the_destination_directory() {
        let root = Temp::new("move-then-rename");
        fs::create_dir(root.0.join("dest")).unwrap();
        let source = root.0.join("old.txt");
        fs::write(&source, "content").unwrap();
        let mut tree = DirectoryTree::new(root.0.clone());
        tree.stage_move(&source, "dest/", true).unwrap();
        assert_eq!(
            tree.planned_destination(&source),
            Some(root.0.join("dest/old.txt").as_path())
        );
        assert_eq!(
            tree.stage_rename(&source, "new.txt", true).unwrap(),
            root.0.join("dest/new.txt")
        );
        assert_eq!(tree.pending_count(), 1);
        tree.build_plan()
            .unwrap()
            .apply(DeletionMode::Permanent)
            .unwrap();
        assert_eq!(fs::read(root.0.join("dest/new.txt")).unwrap(), b"content");
    }
}
