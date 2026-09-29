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
        }
    }

    pub fn show(&mut self, show_hidden: bool) {
        self.visible = true;
        self.expand(self.root.clone(), show_hidden);
    }

    pub fn expand(&mut self, path: PathBuf, show_hidden: bool) {
        self.show_hidden = show_hidden;
        if path == self.root || self.kind(&path) == Some(EntryKind::Directory) {
            if !self.expanded.contains(&path) && self.expanded.len() >= MAX_CACHED_LISTINGS {
                self.errors
                    .insert(path, "too many expanded directories".into());
                return;
            }
            self.expanded.insert(path.clone());
            if !self.listings.contains_key(&path) {
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
            thread::Builder::new()
                .name("directory-tree-listing".into())
                .spawn(move || {
                    while let Ok(request) = receiver.recv() {
                        let entries =
                            read_directory(&request.path).map_err(|error| error.to_string());
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
                    self.listings.insert(result.path.clone(), entries);
                    self.errors.remove(&result.path);
                }
                Err(error) => {
                    self.errors.insert(result.path, error);
                }
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
        });
        if expanded && depth < 128 {
            let mut entries = self.listings.get(path).cloned().unwrap_or_default();
            entries.sort_by(|left, right| {
                (left.kind != EntryKind::Directory)
                    .cmp(&(right.kind != EntryKind::Directory))
                    .then_with(|| left.path.file_name().cmp(&right.path.file_name()))
            });
            for entry in entries {
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
                        entries.push(TreeEntry { path, kind });
                    }
                }
            }
        }
    }
}

fn read_directory(path: &Path) -> Result<Vec<TreeEntry>> {
    let mut entries = Vec::new();
    for result in fs::read_dir(path)? {
        let entry = result?;
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
