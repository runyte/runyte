// SPDX-License-Identifier: MPL-2.0

//! Lazy, path-identified navigation over one workspace directory.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
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
const RECONCILE_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Monitor {
    targets: HashSet<PathBuf>,
    changed: HashSet<PathBuf>,
}

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
}

struct ListingRequest {
    path: PathBuf,
    generation: u64,
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
    pub legend_visible: bool,
    /// Runtime override paired with the configured width when it was set.
    pub width_override: Option<(usize, u16)>,
    expanded: HashSet<PathBuf>,
    revealed_hidden: HashSet<PathBuf>,
    listings: HashMap<PathBuf, Vec<TreeEntry>>,
    errors: HashMap<PathBuf, String>,
    outstanding: HashMap<PathBuf, u64>,
    queued_refresh: HashSet<PathBuf>,
    show_hidden: bool,
    hidden_override: Option<bool>,
    search: Option<regex::Regex>,
    generation: u64,
    requests: Option<Sender<ListingRequest>>,
    results: Receiver<ListingResult>,
    result_sender: Option<Sender<ListingResult>>,
    wake: Arc<Notify>,
    monitor: Arc<Mutex<Monitor>>,
}

impl DirectoryTree {
    pub fn new(root: PathBuf) -> Self {
        let (result_sender, results) = mpsc::channel();
        let wake = Arc::new(Notify::new());
        Self {
            selected: root.clone(),
            legend_visible: true,
            width_override: None,
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
            queued_refresh: HashSet::new(),
            show_hidden: false,
            hidden_override: None,
            search: None,
            generation: 0,
            requests: None,
            results,
            result_sender: Some(result_sender),
            wake,
            monitor: Arc::default(),
        }
    }

    pub fn show(&mut self, show_hidden: bool) {
        self.visible = true;
        self.expand(self.root.clone(), show_hidden);
        self.sync_monitor();
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.sync_monitor();
    }

    fn sync_monitor(&self) {
        let targets = if self.visible {
            self.expanded
                .iter()
                .filter(|path| {
                    path.ancestors()
                        .take_while(|path| *path != self.root)
                        .all(|path| {
                            self.kind(path) == Some(EntryKind::Directory)
                                && path.parent().is_some_and(|parent| {
                                    self.expanded.contains(parent)
                                        && (self.show_hidden
                                            || self.revealed_hidden.contains(parent)
                                            || !path.file_name().is_some_and(|name| {
                                                name.to_string_lossy().starts_with('.')
                                            }))
                                })
                        })
                })
                .cloned()
                .collect()
        } else {
            HashSet::new()
        };
        let mut monitor = self.monitor.lock().unwrap();
        monitor.changed.retain(|path| targets.contains(path));
        monitor.targets = targets;
    }

    pub fn expand(&mut self, path: PathBuf, show_hidden: bool) {
        self.show_hidden = show_hidden;
        if path == self.root || self.kind(&path) == Some(EntryKind::Directory) {
            if !self.expanded.contains(&path) && self.expanded.len() >= MAX_CACHED_LISTINGS {
                self.errors
                    .insert(path, "too many expanded directories".into());
                return;
            }
            let newly_expanded = self.expanded.insert(path.clone());
            self.sync_monitor();
            if newly_expanded || !self.listings.contains_key(&path) {
                self.refresh(path);
            }
        }
    }

    pub fn refresh(&mut self, path: PathBuf) {
        if !self.expanded.contains(&path) {
            return;
        }
        if self.outstanding.contains_key(&path) {
            self.queued_refresh.insert(path);
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
            let monitor = Arc::clone(&self.monitor);
            thread::Builder::new()
                .name("directory-tree-listing".into())
                .spawn(move || run_worker(receiver, result_sender, wake, monitor))
                .expect("tree listing worker starts");
            self.requests = Some(requests);
        }
        if self.requests.as_ref().is_none_or(|requests| {
            requests
                .send(ListingRequest {
                    path: path.clone(),
                    generation,
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
            if !self.expanded.contains(&result.path) {
                self.queued_refresh.remove(&result.path);
                continue;
            }
            // A queued refresh supersedes this observation, including any
            // rows published immediately after an applied filesystem change.
            if self.queued_refresh.remove(&result.path) {
                self.refresh(result.path);
                continue;
            }
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
                    // Removed directories must not leave cached descendants that
                    // reappear if a different directory later takes the same name.
                    let directories = entries
                        .iter()
                        .filter(|entry| entry.kind == EntryKind::Directory)
                        .map(|entry| &entry.path)
                        .collect::<HashSet<_>>();
                    let removed = self
                        .listings
                        .get(&result.path)
                        .into_iter()
                        .flatten()
                        .filter(|old| {
                            old.kind == EntryKind::Directory && !directories.contains(&old.path)
                        })
                        .map(|entry| entry.path.clone())
                        .collect::<Vec<_>>();
                    for path in removed {
                        self.listings.retain(|child, _| !child.starts_with(&path));
                        self.expanded.retain(|child| !child.starts_with(&path));
                        self.revealed_hidden
                            .retain(|child| !child.starts_with(&path));
                        self.errors.retain(|child, _| !child.starts_with(&path));
                        self.queued_refresh
                            .retain(|child| !child.starts_with(&path));
                    }
                    self.listings.insert(result.path.clone(), entries);
                    self.errors.remove(&result.path);
                }
                Err(error) => {
                    self.errors.insert(result.path, error);
                }
            }
            changed = true;
        }
        if changed {
            self.sync_monitor();
        }
        let pending = self.monitor.lock().unwrap().changed.clone();
        for path in pending {
            // Keep excess invalidations until a listing completion frees a slot.
            if self.outstanding.len() >= MAX_OUTSTANDING {
                break;
            }
            self.monitor.lock().unwrap().changed.remove(&path);
            self.refresh(path);
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
        });
        if expanded && depth < 128 {
            for entry in self.listings.get(path).into_iter().flatten() {
                if rows.len() >= MAX_VISIBLE_ROWS {
                    break;
                }
                if !self.show_hidden
                    && !self.revealed_hidden.contains(path)
                    && entry
                        .path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with('.'))
                {
                    continue;
                }
                self.append_rows(&entry.path, entry.kind, depth + 1, rows);
            }
        }
    }

    pub fn kind(&self, path: &Path) -> Option<EntryKind> {
        if path == self.root {
            return Some(EntryKind::Directory);
        }
        self.listings
            .get(path.parent()?)
            .and_then(|entries| entries.iter().find(|entry| entry.path == path))
            .map(|entry| entry.kind)
    }

    pub fn show_hidden(&self, configured: bool) -> bool {
        self.hidden_override.unwrap_or(configured)
    }

    /// Listings retain dotfiles so toggling visibility also covers collapsed
    /// cached directories and results still in flight without another scan.
    pub fn toggle_hidden(&mut self, configured: bool) {
        self.show_hidden = !self.show_hidden(configured);
        self.hidden_override = Some(self.show_hidden);
        self.revealed_hidden.clear();
        self.sync_monitor();
        let rows = self.rows();
        while !rows.iter().any(|row| row.path == self.selected) {
            self.selected = self.selected.parent().unwrap_or(&self.root).to_path_buf();
        }
    }

    pub fn select_row(&mut self, row: usize) {
        let rows = self.rows();
        self.selected = rows[row.min(rows.len() - 1)].path.clone();
    }

    /// Search only displayed names; collapsed subtrees stay lazy.
    pub fn search(&mut self, pattern: &str) -> Result<bool> {
        if !pattern.is_empty() {
            self.search = Some(
                regex::RegexBuilder::new(pattern)
                    .case_insensitive(true)
                    .build()?,
            );
        }
        self.search_next(true)
    }

    pub fn search_next(&mut self, forward: bool) -> Result<bool> {
        let Some(pattern) = self.search.as_ref() else {
            anyhow::bail!("tree search pattern is empty");
        };
        let rows = self.rows();
        let current = rows
            .iter()
            .position(|row| row.path == self.selected)
            .unwrap_or(0);
        for step in 1..=rows.len() {
            let index = if forward {
                (current + step) % rows.len()
            } else {
                (current + rows.len() - step) % rows.len()
            };
            if rows[index]
                .path
                .file_name()
                .is_some_and(|name| pattern.is_match(&name.to_string_lossy()))
            {
                self.selected = rows[index].path.clone();
                return Ok(true);
            }
        }
        Ok(false)
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
            self.sync_monitor();
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
        self.sync_monitor();
    }

    pub fn reveal(&mut self, path: &Path, show_hidden: bool) -> Result<()> {
        // Buffer paths can retain `..` or a symlinked parent. Match the
        // spelling produced by tree listings, keeping a final symlink as the
        // entry itself rather than selecting its target.
        let path = match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => parent.canonicalize()?.join(name),
            _ => path.canonicalize()?,
        };
        self.show_hidden = show_hidden;
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
            }
            self.expanded.insert(parent.clone());
            // An explicit reveal must observe files and ancestors created or
            // moved since an earlier expansion cached this directory.
            self.refresh(parent);
        }
        self.selected = path.to_path_buf();
        self.sync_monitor();
        Ok(())
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
            !target
                .components()
                .any(|part| matches!(part, Component::ParentDir)),
            "parent traversal is not allowed"
        );
        Ok(())
    }

    /// Capture one operation's preconditions without changing navigation state.
    pub fn prepare_create(&self, parent: &Path, name: &str, show_hidden: bool) -> Result<FsPlan> {
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
            fs::symlink_metadata(&target)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
            "target already exists: {}",
            target.display()
        );
        let parent = target.parent().unwrap_or(parent);
        let baseline = DirectorySnapshot::read_bounded(parent, show_hidden, MAX_DIRECTORY_ENTRIES)?;
        FsPlan::build_explicit(
            self.root.clone(),
            vec![FsOperation::Create {
                path: relative_from_root(&self.root, &target)?,
                kind: if directory {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
            }],
            vec![(relative_from_root(&self.root, parent)?, baseline)],
            vec![],
        )
    }

    fn prepare_existing(
        &self,
        source: &Path,
        destination: Option<PathBuf>,
        show_hidden: bool,
    ) -> Result<FsPlan> {
        self.validate_target(source)?;
        let fingerprint = SourceFingerprint::capture_limited(
            source,
            Some(OperationLimits {
                entries: 50_000,
                depth: 128,
                bytes: u64::MAX,
                metadata_bytes: 64 * 1024 * 1024,
            }),
        )?;
        let kind = fingerprint.kind();
        let from = relative_from_root(&self.root, source)?;
        let parent = source.parent().unwrap_or(&self.root);
        let mut baselines = vec![(
            relative_from_root(&self.root, parent)?,
            DirectorySnapshot::read_bounded(parent, show_hidden, MAX_DIRECTORY_ENTRIES)?,
        )];
        let operation = if let Some(target) = destination {
            ensure!(target.is_absolute(), "move destination must be absolute");
            ensure!(source != target, "source and destination are the same");
            ensure!(
                !target.starts_with(source) || kind != EntryKind::Directory,
                "cannot move a directory inside itself"
            );
            let to = relative_from_root(&self.root, &target)?;
            if target.parent() == Some(parent) {
                FsOperation::Rename {
                    from: from.clone(),
                    to,
                    kind,
                }
            } else {
                let target_parent = target.parent().unwrap_or(&self.root);
                baselines.push((
                    relative_from_root(&self.root, target_parent)?,
                    DirectorySnapshot::read_bounded(
                        target_parent,
                        show_hidden,
                        MAX_DIRECTORY_ENTRIES,
                    )?,
                ));
                FsOperation::Move {
                    from: from.clone(),
                    to,
                    kind,
                }
            }
        } else {
            FsOperation::Delete {
                path: from.clone(),
                kind,
            }
        };
        FsPlan::build_explicit(
            self.root.clone(),
            vec![operation],
            baselines,
            vec![(from, fingerprint)],
        )
    }

    pub fn prepare_rename(&self, source: &Path, name: &str, show_hidden: bool) -> Result<FsPlan> {
        let name = Path::new(name);
        ensure!(
            name.components().count() == 1
                && matches!(name.components().next(), Some(Component::Normal(_))),
            "rename accepts one name"
        );
        self.prepare_existing(
            source,
            Some(source.parent().unwrap_or(&self.root).join(name)),
            show_hidden,
        )
    }

    pub fn prepare_move(&self, source: &Path, path: &str, show_hidden: bool) -> Result<FsPlan> {
        ensure!(!path.trim().is_empty(), "move needs a destination");
        let input = Path::new(path);
        let mut target = lexical_normalize(&if input.is_absolute() {
            input.to_path_buf()
        } else {
            source.parent().unwrap_or(&self.root).join(input)
        });
        if target.is_dir() || path.ends_with('/') || path.ends_with(std::path::MAIN_SEPARATOR) {
            target.push(
                source
                    .file_name()
                    .ok_or_else(|| anyhow::anyhow!("source has no name"))?,
            );
        }
        self.prepare_existing(source, Some(target), show_hidden)
    }

    pub fn prepare_delete(&self, source: &Path, show_hidden: bool) -> Result<FsPlan> {
        self.prepare_existing(source, None, show_hidden)
    }

    /// Publish known operation results immediately, without stat calls. Queued
    /// listings from before the operation cannot replace this newer observation.
    pub fn note_applied(&mut self, root: &Path, report: &crate::fs_plan::ApplyReport) {
        for operation in &report.applied {
            let (source, destination) = match operation {
                FsOperation::Create { path, kind } => (None, Some((path, *kind))),
                FsOperation::Delete { path, .. } => (Some(path), None),
                FsOperation::Rename { from, to, kind } | FsOperation::Move { from, to, kind } => {
                    (Some(from), Some((to, *kind)))
                }
                FsOperation::Copy { to, kind, .. } => (None, Some((to, *kind))),
            };
            if let Some(source) = source {
                let source = lexical_normalize(&root.join(source));
                if let Some(entries) = source
                    .parent()
                    .and_then(|parent| self.listings.get_mut(parent))
                {
                    entries.retain(|entry| entry.path != source);
                }
                self.listings.retain(|path, _| !path.starts_with(&source));
                self.expanded.retain(|path| !path.starts_with(&source));
                self.revealed_hidden
                    .retain(|path| !path.starts_with(&source));
                self.errors.retain(|path, _| !path.starts_with(&source));
                // Discard results for removed/renamed directories without
                // scheduling new work at the old path.
                self.queued_refresh
                    .retain(|path| !path.starts_with(&source));
                if self.selected.starts_with(&source) {
                    self.selected = source.parent().unwrap_or(&self.root).to_path_buf();
                }
            }
            if let Some((path, kind)) = destination {
                let path = lexical_normalize(&root.join(path));
                if !path.starts_with(&self.root) {
                    continue;
                }
                if let Some(parent) = path.parent()
                    && (self.listings.contains_key(parent)
                        || self.listings.len() < MAX_CACHED_LISTINGS)
                {
                    let entries = self.listings.entry(parent.to_path_buf()).or_default();
                    entries.retain(|entry| entry.path != path);
                    if entries.len() < MAX_DIRECTORY_ENTRIES {
                        let entry = TreeEntry { path, kind };
                        let index = entries
                            .binary_search_by(|existing| compare_entries(existing, &entry))
                            .unwrap_or_else(|index| index);
                        entries.insert(index, entry);
                    }
                }
            }
        }
        self.sync_monitor();
    }
}

/// One lazy worker serves explicit reads and periodic reconciliation. Both
/// inputs and invalidations are bounded by the tree's existing listing limits;
/// unchanged observations never wake the editor. No filesystem IO runs in poll.
fn run_worker(
    receiver: Receiver<ListingRequest>,
    results: Sender<ListingResult>,
    wake: Arc<Notify>,
    monitor: Arc<Mutex<Monitor>>,
) {
    let mut observed = HashMap::new();
    let mut next_check = Instant::now() + RECONCILE_INTERVAL;
    loop {
        match receiver.recv_timeout(next_check.saturating_duration_since(Instant::now())) {
            Ok(request) => {
                let entries = read_directory(&request.path).map_err(|error| error.to_string());
                let targets = monitor.lock().unwrap().targets.clone();
                observed.retain(|path, _| targets.contains(path));
                if targets.contains(&request.path) {
                    observed.insert(request.path.clone(), entries.clone());
                }
                if results
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
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() >= next_check {
            let targets = monitor.lock().unwrap().targets.clone();
            observed.retain(|path, _| targets.contains(path));
            for path in targets {
                if !monitor.lock().unwrap().targets.contains(&path) {
                    continue;
                }
                let entries = read_directory(&path).map_err(|error| error.to_string());
                if observed.get(&path) != Some(&entries) {
                    observed.insert(path.clone(), entries);
                    let mut monitor = monitor.lock().unwrap();
                    if monitor.targets.contains(&path) && monitor.changed.insert(path) {
                        wake.notify_one();
                    }
                }
            }
            next_check = Instant::now() + RECONCILE_INTERVAL;
        }
    }
}

fn read_directory(path: &Path) -> Result<Vec<TreeEntry>> {
    ensure!(
        fs::symlink_metadata(path)?.is_dir(),
        "path is not a directory"
    );
    let mut entries = Vec::new();
    for result in fs::read_dir(path)? {
        let entry = result?;
        ensure!(
            entries.len() < MAX_DIRECTORY_ENTRIES,
            "directory has too many entries"
        );
        let kind = match entry.file_type()? {
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
    entries.sort_by(compare_entries);
    Ok(entries)
}

fn compare_entries(left: &TreeEntry, right: &TreeEntry) -> std::cmp::Ordering {
    (left.kind != EntryKind::Directory)
        .cmp(&(right.kind != EntryKind::Directory))
        .then_with(|| left.path.file_name().cmp(&right.path.file_name()))
}

#[cfg(test)]
#[path = "directory_tree/tests/mod.rs"]
mod regression_tests;

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

    async fn until(tree: &mut DirectoryTree, ready: impl Fn(&DirectoryTree) -> bool) {
        let wake = tree.wake();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                tree.poll();
                if tree.outstanding.is_empty() && ready(tree) {
                    break;
                }
                wake.notified().await;
            }
        })
        .await
        .expect("background reconciliation should wake the editor");
    }

    #[tokio::test]
    async fn external_creates_renames_deletes_and_type_changes_reconcile_without_input() {
        let root = Temp::new("automatic");
        let folder = root.0.join("folder");
        fs::create_dir(&folder).unwrap();
        let selected = root.0.join("selected");
        fs::write(&selected, "keep selection").unwrap();
        let mut tree = DirectoryTree::new(root.0.clone());
        tree.show(false);
        until(&mut tree, |tree| tree.kind(&folder).is_some()).await;
        tree.expand(folder.clone(), false);
        until(&mut tree, |tree| tree.listings.contains_key(&folder)).await;
        tree.selected = selected.clone();
        let created = folder.join("created");
        fs::write(&created, "new").unwrap();
        until(&mut tree, |tree| {
            tree.kind(&created) == Some(EntryKind::File)
        })
        .await;
        assert_eq!(tree.selected, selected);
        let renamed = folder.join("renamed");
        fs::rename(&created, &renamed).unwrap();
        until(&mut tree, |tree| {
            tree.kind(&created).is_none() && tree.kind(&renamed).is_some()
        })
        .await;
        tree.selected = renamed.clone();
        fs::remove_file(&renamed).unwrap();
        until(&mut tree, |tree| tree.kind(&renamed).is_none()).await;
        assert_eq!(tree.selected, folder);
        fs::remove_dir(&folder).unwrap();
        fs::write(&folder, "directory replaced by a file").unwrap();
        until(&mut tree, |tree| {
            tree.kind(&folder) == Some(EntryKind::File)
        })
        .await;
        assert!(!tree.expanded.contains(&folder));
        assert!(!tree.listings.contains_key(&folder));
        assert!(!tree.monitor.lock().unwrap().targets.contains(&folder));
    }

    #[tokio::test]
    async fn collapsed_and_hidden_trees_stop_monitoring_and_refresh_on_return() {
        let root = Temp::new("automatic-visibility");
        let folder = root.0.join("folder");
        fs::create_dir_all(folder.join("nested")).unwrap();
        let mut tree = DirectoryTree::new(root.0.clone());
        assert!(tree.requests.is_none(), "startup creates no worker");
        tree.show(false);
        until(&mut tree, |tree| tree.kind(&folder).is_some()).await;
        tree.expand(folder.clone(), false);
        until(&mut tree, |tree| {
            tree.kind(&folder.join("nested")).is_some()
        })
        .await;
        tree.expand(folder.join("nested"), false);
        until(&mut tree, |_| true).await;
        tree.selected = folder.clone();
        tree.collapse_or_parent();
        assert_eq!(
            tree.monitor.lock().unwrap().targets,
            HashSet::from([root.0.clone()])
        );
        let file = folder.join("new");
        fs::write(&file, "new").unwrap();
        tree.expand_or_child(false);
        until(&mut tree, |tree| tree.kind(&file).is_some()).await;
        tree.hide();
        assert!(tree.monitor.lock().unwrap().targets.is_empty());
        let hidden_change = root.0.join("while-hidden");
        fs::write(&hidden_change, "new").unwrap();
        tree.show(false);
        until(&mut tree, |tree| tree.kind(&hidden_change).is_some()).await;
        // Drain the last completion permit, then ensure unchanged periodic
        // reads do not cause redraws or idle event-loop work.
        let wake = tree.wake();
        while tokio::time::timeout(Duration::from_millis(10), wake.notified())
            .await
            .is_ok()
        {}
        assert!(
            tokio::time::timeout(RECONCILE_INTERVAL * 2, wake.notified())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn hidden_directory_monitoring_follows_dotfile_visibility() {
        let root = Temp::new("automatic-dotfiles");
        let hidden = root.0.join(".hidden");
        fs::create_dir(&hidden).unwrap();
        let mut tree = DirectoryTree::new(root.0.clone());
        tree.show(true);
        until(&mut tree, |tree| tree.kind(&hidden).is_some()).await;
        tree.expand(hidden.clone(), true);
        until(&mut tree, |_| true).await;
        assert!(tree.monitor.lock().unwrap().targets.contains(&hidden));
        tree.toggle_hidden(true);
        assert!(!tree.monitor.lock().unwrap().targets.contains(&hidden));
        let file = hidden.join("new");
        fs::write(&file, "new").unwrap();
        tree.toggle_hidden(true);
        until(&mut tree, |tree| tree.kind(&file).is_some()).await;
        tree.selected = root.0.clone();
        tree.toggle_selected(true);
        assert!(tree.monitor.lock().unwrap().targets.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_replaced_by_a_symlink_is_not_scanned() {
        let root = Temp::new("automatic-symlink");
        let target = Temp::new("automatic-symlink-target");
        fs::write(target.0.join("outside"), "outside").unwrap();
        let link = root.0.join("link");
        std::os::unix::fs::symlink(&target.0, &link).unwrap();
        assert!(read_directory(&link).is_err());
        assert_eq!(read_directory(&root.0).unwrap()[0].kind, EntryKind::Symlink);
    }

    #[test]
    fn automatic_refresh_overflow_is_retained_until_completions_free_slots() {
        let root = Temp::new("automatic-overflow");
        let mut tree = DirectoryTree::new(root.0.clone());
        tree.visible = true;
        tree.expanded.insert(root.0.clone());
        tree.listings.insert(
            root.0.clone(),
            (0..MAX_OUTSTANDING)
                .map(|index| {
                    let path = root.0.join(index.to_string());
                    tree.expanded.insert(path.clone());
                    tree.outstanding.insert(path.clone(), 0);
                    TreeEntry {
                        path,
                        kind: EntryKind::Directory,
                    }
                })
                .collect(),
        );
        tree.sync_monitor();
        tree.monitor.lock().unwrap().changed.insert(root.0.clone());
        tree.poll();
        assert!(tree.monitor.lock().unwrap().changed.contains(&root.0));
        let path = root.0.join("0");
        tree.result_sender
            .as_ref()
            .unwrap()
            .send(ListingResult {
                path,
                generation: 0,
                entries: Ok(vec![]),
            })
            .unwrap();
        tree.poll();
        assert!(!tree.monitor.lock().unwrap().changed.contains(&root.0));
        assert!(tree.outstanding.contains_key(&root.0));
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
            tree.refresh(root.0.clone());
        }
        assert_eq!(tree.outstanding.len(), 1);
        assert_eq!(tree.outstanding[&root.0], generation);
        assert_eq!(tree.queued_refresh.len(), 1);
    }

    #[test]
    fn immediate_plans_validate_paths_collisions_and_changed_sources() {
        let root = Temp::new("checked-plans");
        let tree = DirectoryTree::new(root.0.clone());
        for name in ["", "../escape", "/absolute"] {
            assert!(tree.prepare_create(&root.0, name, false).is_err());
        }
        assert!(tree.prepare_delete(&root.0, false).is_err());
        tree.prepare_create(&root.0, "folder/", false)
            .unwrap()
            .apply(DeletionMode::Permanent)
            .unwrap();
        let file = root.0.join("file");
        fs::write(&file, "original").unwrap();
        assert!(tree.prepare_create(&root.0, "file", false).is_err());
        assert!(tree.prepare_rename(&file, "a/b", false).is_err());
        assert!(
            tree.prepare_move(&root.0.join("folder"), "folder/child", false)
                .is_err()
        );
        let plan = tree.prepare_delete(&file, false).unwrap();
        fs::write(&file, "changed contents").unwrap();
        assert!(plan.apply(DeletionMode::Permanent).is_err());
        assert!(file.exists());
        tree.prepare_move(&file, "folder/", false)
            .unwrap()
            .apply(DeletionMode::Permanent)
            .unwrap();
        assert!(root.0.join("folder/file").exists());
        assert!(!file.exists());
    }

    #[test]
    fn applied_entries_are_known_without_disk_fallback_and_deleted_subtrees_disappear() {
        let root = Temp::new("known-entries");
        let mut tree = DirectoryTree::new(root.0.clone());
        let folder = root.0.join("folder");
        let report = tree
            .prepare_create(&root.0, "folder/", false)
            .unwrap()
            .apply(DeletionMode::Permanent)
            .unwrap();
        tree.note_applied(&root.0, &report);
        assert_eq!(tree.kind(&folder), Some(EntryKind::Directory));
        fs::create_dir(root.0.join(".hidden")).unwrap();
        assert_eq!(tree.kind(&root.0.join(".hidden")), None);
        tree.expanded.insert(folder.clone());
        tree.listings.insert(folder.clone(), vec![]);
        tree.selected = folder.join("child");
        let report = tree
            .prepare_rename(&folder, "renamed", false)
            .unwrap()
            .apply(DeletionMode::Permanent)
            .unwrap();
        tree.note_applied(&root.0, &report);
        assert_eq!(tree.kind(&folder), None);
        assert!(!tree.expanded.contains(&folder));
        assert!(!tree.listings.contains_key(&folder));
        assert_eq!(tree.selected, root.0);
        let renamed = root.0.join("renamed");
        assert_eq!(tree.kind(&renamed), Some(EntryKind::Directory));
        let report = tree
            .prepare_delete(&renamed, false)
            .unwrap()
            .apply(DeletionMode::Permanent)
            .unwrap();
        tree.note_applied(&root.0, &report);
        assert_eq!(tree.kind(&renamed), None);
    }
    #[test]
    fn queued_refresh_discards_results_from_before_an_applied_change() {
        let root = Temp::new("superseded-listing");
        let mut tree = DirectoryTree::new(root.0.clone());
        tree.expanded.insert(root.0.clone());
        tree.outstanding.insert(root.0.clone(), 0);
        tree.queued_refresh.insert(root.0.clone());
        tree.result_sender
            .as_ref()
            .unwrap()
            .send(ListingResult {
                path: root.0.clone(),
                generation: 0,
                entries: Ok(vec![]),
            })
            .unwrap();
        let report = tree
            .prepare_create(&root.0, "folder/", false)
            .unwrap()
            .apply(DeletionMode::Permanent)
            .unwrap();
        tree.note_applied(&root.0, &report);
        tree.poll();
        assert_eq!(
            tree.kind(&root.0.join("folder")),
            Some(EntryKind::Directory)
        );
        await_rows(&mut tree, 2);
    }
    #[test]
    fn applied_move_outside_workspace_does_not_publish_outside_rows() {
        let root = Temp::new("move-outside-source");
        let destination = Temp::new("move-outside-destination");
        let mut tree = DirectoryTree::new(root.0.clone());
        let report = tree
            .prepare_create(&root.0, "file", false)
            .unwrap()
            .apply(DeletionMode::Permanent)
            .unwrap();
        tree.note_applied(&root.0, &report);
        tree.selected = root.0.join("file");
        let report = tree
            .prepare_move(&tree.selected, destination.0.to_str().unwrap(), false)
            .unwrap()
            .apply(DeletionMode::Permanent)
            .unwrap();
        tree.note_applied(&root.0, &report);
        assert!(destination.0.join("file").exists());
        assert_eq!(tree.kind(&root.0.join("file")), None);
        assert_eq!(tree.kind(&destination.0.join("file")), None);
        assert!(tree.listings.keys().all(|path| path.starts_with(&root.0)));
        assert_eq!(tree.selected, root.0);
    }
}
