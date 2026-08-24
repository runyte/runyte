// SPDX-License-Identifier: MPL-2.0

//! What the editor remembers about Git between calls to it.
//!
//! Two facts are cached, and they refresh on different clocks. Repository
//! status and the staged text of each open file come from Git, so they change
//! only when something outside the buffer does: opening a file, saving one, or
//! asking for a refresh after committing elsewhere. The per-row marks are
//! derived from that staged text and the buffer's current text, so they change
//! on every edit and are recomputed here, in memory, without a subprocess.
//!
//! The tracker holds paths and strings. It has no idea what a buffer or a pane
//! is, which is what lets the editor call it from wherever it happens to know
//! that a file's text has moved on.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use super::{
    BaseContent, GitProvider, LineChange, Repository, RepositoryStatus, Result, RowChange,
    StatusStats, changed_rows,
};

/// The largest staged text that will be diffed.
///
/// Past this size the comparison stops being something worth doing between
/// keystrokes, and a file this large is not one whose gutter anybody is
/// reading. It is not tracked at all rather than tracked wrongly.
const MAX_BASE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct GitTracker {
    repository: Option<Repository>,
    status: Option<RepositoryStatus>,
    /// The line counts for that status, read only while something shows them,
    /// so an empty one means "not asked for" as often as "nothing to count".
    stats: StatusStats,
    files: HashMap<PathBuf, TrackedFile>,
}

#[derive(Debug)]
struct TrackedFile {
    /// The staged text, kept so edits can be diffed without asking Git again.
    base: String,
    rows: Vec<RowChange>,
    /// The buffer revision `rows` was computed from. `None` until the buffer
    /// text has been seen once, so the first update always computes.
    revision: Option<u64>,
}

impl GitTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Notes the repository the editor is working in, if there is one.
    pub fn attach(&mut self, repository: Option<Repository>) {
        if self.repository != repository {
            self.files.clear();
            self.status = None;
            self.stats = StatusStats::default();
        }
        self.repository = repository;
    }

    pub fn repository(&self) -> Option<&Repository> {
        self.repository.as_ref()
    }

    pub fn status(&self) -> Option<&RepositoryStatus> {
        self.status.as_ref()
    }

    /// How many lines each changed file adds and removes, where that was read.
    pub fn stats(&self) -> &StatusStats {
        &self.stats
    }

    /// Atomically installs one service-owned repository snapshot.
    ///
    /// Counts that were not read this time are dropped rather than kept: they
    /// described the previous status, and numbers beside a file that has since
    /// moved are worse than no numbers at all.
    pub fn apply_snapshot(
        &mut self,
        repository: Repository,
        status: RepositoryStatus,
        stats: StatusStats,
        staged: Vec<(PathBuf, BaseContent)>,
    ) {
        self.repository = Some(repository);
        self.status = Some(status);
        self.stats = stats;
        self.files = staged
            .into_iter()
            .filter_map(|(path, content)| match content {
                BaseContent::Text(base) if base.len() <= MAX_BASE_BYTES => Some((
                    path,
                    TrackedFile {
                        base,
                        rows: Vec::new(),
                        revision: None,
                    },
                )),
                BaseContent::Absent | BaseContent::Binary | BaseContent::Text(_) => None,
            })
            .collect();
    }

    /// Installs a status read on its own, without the counts that describe it.
    ///
    /// The previous counts are dropped for the same reason
    /// [`GitTracker::apply_snapshot`] drops uncounted ones: they were measured
    /// against a status this one replaces.
    pub fn apply_status(&mut self, status: RepositoryStatus) {
        self.status = Some(status);
        self.stats = StatusStats::default();
    }

    pub fn apply_staged_content(&mut self, path: PathBuf, content: BaseContent) {
        match content {
            BaseContent::Text(base) if base.len() <= MAX_BASE_BYTES => {
                self.files.insert(
                    path,
                    TrackedFile {
                        base,
                        rows: Vec::new(),
                        revision: None,
                    },
                );
            }
            BaseContent::Absent | BaseContent::Binary | BaseContent::Text(_) => {
                self.files.remove(&path);
            }
        }
    }

    /// The one-line label for a status surface: branch, drift, and counts.
    pub fn summary(&self) -> Option<String> {
        Some(self.status.as_ref()?.summary())
    }

    /// Whether this path has a staged text to compare a buffer against.
    ///
    /// A file that is untracked, conflicted, or binary has none, and shows no
    /// gutter at all rather than a column of marks claiming every line is new.
    pub fn tracks(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    pub fn rows(&self, path: &Path) -> &[RowChange] {
        self.files
            .get(path)
            .map_or(&[][..], |file| file.rows.as_slice())
    }

    pub fn change_at(&self, path: &Path, row: usize) -> Option<LineChange> {
        let rows = &self.files.get(path)?.rows;
        rows.binary_search_by_key(&row, |change| change.row)
            .ok()
            .map(|index| rows[index].change)
    }

    /// Re-reads which files Git considers changed.
    pub fn refresh_status(&mut self, provider: &dyn GitProvider) -> Result<()> {
        let Some(repository) = &self.repository else {
            return Ok(());
        };
        let status = provider.status(repository)?;
        // Counting is best-effort where the status itself is not: a provider
        // that cannot count leaves the list without numbers, which is not a
        // reason to leave the reader without the list.
        self.stats = provider
            .status_stats(repository, &status)
            .unwrap_or_default();
        self.status = Some(status);
        Ok(())
    }

    /// Reads the staged text of one path, replacing whatever was held for it.
    ///
    /// Called when a file is opened and whenever the index may have moved
    /// under it. A path outside the repository, or one Git has no unconflicted
    /// entry for, is dropped rather than remembered as unchanged.
    pub fn track(&mut self, provider: &dyn GitProvider, path: &Path) -> Result<()> {
        let Some(repository) = &self.repository else {
            return Ok(());
        };
        if !repository.contains(path) {
            self.files.remove(path);
            return Ok(());
        }
        match provider.staged_content(repository, path)? {
            BaseContent::Text(base) if base.len() <= MAX_BASE_BYTES => {
                self.files.insert(
                    path.to_path_buf(),
                    TrackedFile {
                        base,
                        rows: Vec::new(),
                        revision: None,
                    },
                );
            }
            _ => {
                self.files.remove(path);
            }
        }
        Ok(())
    }

    /// Re-reads the staged text of every tracked path, and the status with it.
    ///
    /// This is the call behind an explicit refresh: committing, staging, or
    /// switching branches outside the editor moves the base that every mark is
    /// measured against, and nothing about a buffer would reveal that.
    pub fn refresh(&mut self, provider: &dyn GitProvider) -> Result<()> {
        self.refresh_status(provider)?;
        let paths = self.files.keys().cloned().collect::<Vec<_>>();
        for path in paths {
            self.track(provider, &path)?;
        }
        Ok(())
    }

    pub fn forget(&mut self, path: &Path) {
        self.files.remove(path);
    }

    /// Recomputes the marks for a path when its buffer has moved on.
    ///
    /// `revision` is what makes this cheap enough to call before every frame:
    /// unchanged text is recognised without looking at it.
    pub fn update(&mut self, path: &Path, revision: u64, text: impl FnOnce() -> String) {
        let Some(file) = self.files.get_mut(path) else {
            return;
        };
        if file.revision == Some(revision) {
            return;
        }
        file.rows = changed_rows(&file.base, &text());
        file.revision = Some(revision);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{Divergence, FileState, FileStatus, GitError, Head, MemoryGitProvider};

    fn repository() -> Repository {
        Repository::new("/project")
    }

    fn provider() -> MemoryGitProvider {
        MemoryGitProvider::new(repository())
    }

    #[test]
    fn marks_come_from_the_staged_text_and_follow_the_buffer() {
        let provider = provider().with_staged("src/main.rs", "a\nb\nc\n");
        let mut tracker = GitTracker::new();
        tracker.attach(Some(repository()));
        let path = Path::new("/project/src/main.rs");
        tracker.track(&provider, path).unwrap();

        // Text identical to the index has no marks.
        tracker.update(path, 1, || "a\nb\nc\n".to_owned());
        assert!(tracker.rows(path).is_empty());

        tracker.update(path, 2, || "a\nB\nc\n".to_owned());
        assert_eq!(tracker.change_at(path, 1), Some(LineChange::Modified));
        assert_eq!(tracker.change_at(path, 0), None);
        assert_eq!(provider.calls(), 1, "editing must not run Git again");
    }

    /// An unchanged revision is the common case before every frame, so it has
    /// to be recognised without reading the buffer at all.
    #[test]
    fn an_unchanged_revision_does_not_look_at_the_text() {
        let provider = provider().with_staged("src/main.rs", "a\n");
        let mut tracker = GitTracker::new();
        tracker.attach(Some(repository()));
        let path = Path::new("/project/src/main.rs");
        tracker.track(&provider, path).unwrap();
        tracker.update(path, 7, || "a\nb\n".to_owned());

        tracker.update(path, 7, || panic!("the text must not be read again"));
        assert_eq!(tracker.change_at(path, 1), Some(LineChange::Added));
    }

    #[test]
    fn untracked_conflicted_and_binary_paths_have_no_gutter() {
        let provider = provider()
            .with_staged("tracked.rs", "a\n")
            .with_binary("image.png");
        let mut tracker = GitTracker::new();
        tracker.attach(Some(repository()));
        for name in ["tracked.rs", "image.png", "untracked.rs"] {
            tracker
                .track(&provider, &Path::new("/project").join(name))
                .unwrap();
        }

        assert!(tracker.tracks(Path::new("/project/tracked.rs")));
        assert!(!tracker.tracks(Path::new("/project/image.png")));
        assert!(!tracker.tracks(Path::new("/project/untracked.rs")));
    }

    #[test]
    fn a_path_outside_the_repository_is_never_tracked() {
        let provider = provider().with_staged("inside.rs", "a\n");
        let mut tracker = GitTracker::new();
        tracker.attach(Some(repository()));
        tracker
            .track(&provider, Path::new("/elsewhere/outside.rs"))
            .unwrap();

        assert!(!tracker.tracks(Path::new("/elsewhere/outside.rs")));
    }

    /// Committing outside the editor moves the base every mark is measured
    /// against, and only an explicit refresh can notice.
    #[test]
    fn a_refresh_rereads_the_base_of_every_open_file() {
        let mut provider = provider().with_staged("src/main.rs", "old\n");
        let mut tracker = GitTracker::new();
        tracker.attach(Some(repository()));
        let path = Path::new("/project/src/main.rs");
        tracker.track(&provider, path).unwrap();
        tracker.update(path, 1, || "new\n".to_owned());
        assert_eq!(tracker.change_at(path, 0), Some(LineChange::Modified));

        provider.set_staged("src/main.rs", "new\n");
        tracker.refresh(&provider).unwrap();
        tracker.update(path, 1, || "new\n".to_owned());

        assert_eq!(tracker.change_at(path, 0), None);
    }

    #[test]
    fn the_summary_reports_the_branch_and_what_is_outstanding() {
        let provider = provider().with_status(RepositoryStatus {
            head: Head::Branch("feature".to_owned()),
            upstream: Some("origin/feature".to_owned()),
            divergence: Divergence {
                ahead: 1,
                behind: 0,
            },
            files: vec![FileStatus {
                path: PathBuf::from("src/main.rs"),
                original_path: None,
                index: FileState::Unmodified,
                worktree: FileState::Modified,
            }],
        });
        let mut tracker = GitTracker::new();
        tracker.attach(Some(repository()));

        assert_eq!(tracker.summary(), None);
        tracker.refresh_status(&provider).unwrap();
        assert_eq!(tracker.summary().as_deref(), Some("feature ↑1 ~1"));
    }

    /// With no repository there is nothing to ask, and asking must not fail.
    #[test]
    fn a_project_without_a_repository_tracks_nothing_quietly() {
        let provider = provider().with_staged("src/main.rs", "a\n");
        let mut tracker = GitTracker::new();

        tracker.refresh(&provider).unwrap();
        tracker
            .track(&provider, Path::new("/project/src/main.rs"))
            .unwrap();

        assert!(tracker.summary().is_none());
        assert!(!tracker.tracks(Path::new("/project/src/main.rs")));
    }

    #[test]
    fn a_failing_provider_leaves_the_previous_marks_alone() {
        let provider = provider().with_staged("src/main.rs", "a\n").failing();
        let mut tracker = GitTracker::new();
        tracker.attach(Some(repository()));

        let error = tracker
            .track(&provider, Path::new("/project/src/main.rs"))
            .unwrap_err();

        assert!(matches!(error, GitError::Failed { .. }));
        assert!(!tracker.tracks(Path::new("/project/src/main.rs")));
    }

    /// Reattaching to a different repository invalidates everything measured
    /// against the old one.
    #[test]
    fn attaching_a_different_repository_drops_what_was_cached() {
        let provider = provider().with_staged("src/main.rs", "a\n");
        let mut tracker = GitTracker::new();
        tracker.attach(Some(repository()));
        let path = Path::new("/project/src/main.rs");
        tracker.track(&provider, path).unwrap();
        assert!(tracker.tracks(path));

        tracker.attach(Some(Repository::new("/other")));
        assert!(!tracker.tracks(path));
    }
}
