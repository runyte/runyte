// SPDX-License-Identifier: MPL-2.0

//! The Git command-line provider against real repositories.
//!
//! These tests create throwaway repositories under the system temporary
//! directory. Nothing here touches the repository Runyte is developed in, and
//! nothing writes to a person's configuration or cache.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use runyte::git::{
    BaseContent, BlameRequest, DeletionAuthorization, DiffScope, Divergence, FileComparison,
    FileState, GitCliProvider, GitError, GitMutation, GitOperation, GitProvider, GitResponse,
    GitService, GitServiceEvent, GitServiceHandle, Head, LineStats, LogRequest,
    MAX_BLAME_INPUT_BYTES, PartialStageSelection, RefreshSpec, Repository, StashMutation,
    StashScope, WorktreeCreate,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempRepository(PathBuf);

impl TempRepository {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "runyte-git-provider-{name}-{}-{nanos}-{count}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let repository = Self(root.canonicalize().unwrap());
        repository.git(&["init", "-q"]);
        repository.git(&["config", "user.name", "Runyte Test"]);
        repository.git(&["config", "user.email", "runyte@example.invalid"]);
        repository
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn repository(&self) -> Repository {
        Repository::new(&self.0)
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn git(&self, arguments: &[&str]) {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(&self.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", message]);
    }
}

fn git_output(repository: &TempRepository, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

impl Drop for TempRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A bare remote, a clone under test, and a second clone standing in for
/// whoever else pushes to it.
///
/// The remote is bare because a repository with a working tree refuses a push
/// to the branch it has checked out, and the peer exists because commits have
/// to reach a bare repository from somewhere. Together they are the least setup
/// that lets a test watch a branch fall behind, catch up, and be published.
struct TempClone {
    origin: PathBuf,
    peer: PathBuf,
    work: PathBuf,
}

impl TempClone {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "runyte-git-clone-{name}-{}-{nanos}-{count}",
            std::process::id()
        ));
        fs::create_dir_all(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let origin = base.join("origin.git");
        let peer = base.join("peer");
        let work = base.join("work");

        run(
            &base,
            &["init", "-q", "--bare", "--initial-branch=main"],
            &origin,
        );
        run(&base, &["init", "-q", "--initial-branch=main"], &peer);
        let clone = Self { origin, peer, work };
        clone.in_peer(&["config", "user.name", "Runyte Test"]);
        clone.in_peer(&["config", "user.email", "runyte@example.invalid"]);
        clone.in_peer(&["remote", "add", "origin", clone.origin.to_str().unwrap()]);
        fs::write(clone.peer.join("source.rs"), "base\n").unwrap();
        clone.in_peer(&["add", "-A"]);
        clone.in_peer(&["commit", "-qm", "base"]);
        clone.in_peer(&["push", "-q", "--set-upstream", "origin", "main"]);

        run(
            &base,
            &["clone", "-q", clone.origin.to_str().unwrap()],
            &clone.work,
        );
        clone.git(&["config", "user.name", "Runyte Test"]);
        clone.git(&["config", "user.email", "runyte@example.invalid"]);
        clone
    }

    fn repository(&self) -> Repository {
        Repository::new(&self.work)
    }

    fn path(&self) -> &Path {
        &self.work
    }

    fn git(&self, arguments: &[&str]) {
        run_in(&self.work, arguments);
    }

    fn in_peer(&self, arguments: &[&str]) {
        run_in(&self.peer, arguments);
    }

    fn write(&self, relative: &str, contents: &str) {
        fs::write(self.work.join(relative), contents).unwrap();
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", message]);
    }

    /// Puts a commit on the remote's `main` from somewhere that is not the
    /// clone under test, which is what leaves that clone behind.
    fn commit_upstream(&self, contents: &str) {
        self.in_peer(&["pull", "-q", "--ff-only"]);
        fs::write(self.peer.join("source.rs"), contents).unwrap();
        self.in_peer(&["commit", "-qam", "from elsewhere"]);
        self.in_peer(&["push", "-q", "origin", "main"]);
    }

    /// The same, on a file the clone under test is not editing, so the two
    /// sides drift apart without their changes overlapping.
    fn commit_upstream_file(&self, relative: &str, contents: &str) {
        self.in_peer(&["pull", "-q", "--ff-only"]);
        fs::write(self.peer.join(relative), contents).unwrap();
        self.in_peer(&["add", "-A"]);
        self.in_peer(&["commit", "-qm", "from elsewhere"]);
        self.in_peer(&["push", "-q", "origin", "main"]);
    }

    /// The subjects on the current branch, tip first, which is how a test says
    /// what a rebase did to the shape of the history rather than only to its
    /// contents.
    fn subjects(&self) -> Vec<String> {
        let output = Command::new("git")
            .args(["log", "--format=%s"])
            .current_dir(&self.work)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// What `git stash list` holds, which is where an autostash that could not
    /// be reapplied ends up.
    fn stashes(&self) -> Vec<String> {
        let output = Command::new("git")
            .args(["stash", "list"])
            .current_dir(&self.work)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Whether a rebase is stopped partway through this working tree, which is
    /// the state a refusal must never leave behind.
    fn rebase_in_progress(&self) -> bool {
        ["rebase-merge", "rebase-apply"]
            .iter()
            .any(|state| self.work.join(".git").join(state).exists())
    }

    /// What the remote holds for one branch, so a test can check that a push
    /// actually arrived rather than only that Git returned success.
    fn remote_head(&self, branch: &str) -> Option<String> {
        let output = Command::new("git")
            .args(["rev-parse", &format!("refs/heads/{branch}")])
            .current_dir(&self.origin)
            .output()
            .unwrap();
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    fn head(&self) -> String {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.work)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}

impl Drop for TempClone {
    fn drop(&mut self) {
        if let Some(base) = self.work.parent() {
            let _ = fs::remove_dir_all(base);
        }
    }
}

fn run_in(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} in {} failed: {}",
        directory.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Runs a Git command that creates `target`, from a directory that exists.
fn run(directory: &Path, arguments: &[&str], target: &Path) {
    let output = Command::new("git")
        .args(arguments)
        .arg(target)
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} {} failed: {}",
        target.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn provider() -> GitCliProvider {
    GitCliProvider::from_environment().expect("these tests need `git` on PATH")
}

fn line_selection(path: PathBuf, lines: (usize, usize)) -> PartialStageSelection {
    PartialStageSelection {
        path,
        scope: DiffScope::Unstaged,
        buffer: None,
        guard: None,
        hunk: None,
        lines: Some(lines),
    }
}

#[test]
fn discovery_finds_the_working_tree_from_any_directory_inside_it() {
    let repository = TempRepository::new("discover");
    repository.write("src/main.rs", "fn main() {}\n");
    repository.commit("base");
    let provider = provider();

    let found = provider
        .discover(&repository.path().join("src"))
        .unwrap()
        .expect("a repository above src");

    assert_eq!(
        found.workdir().canonicalize().unwrap(),
        repository.path().to_path_buf()
    );
    assert_eq!(found.common_dir(), repository.path().join(".git"));
}

#[test]
fn linked_worktrees_share_a_common_repository_identity() {
    let repository = TempRepository::new("linked-identity");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    let linked = repository.path().join("linked");
    repository.git(&["worktree", "add", "-q", "-b", "linked", "linked"]);
    let provider = provider();

    let main = provider.discover(repository.path()).unwrap().unwrap();
    let other = provider.discover(&linked).unwrap().unwrap();

    assert_ne!(main.workdir(), other.workdir());
    assert_eq!(main.common_dir(), other.common_dir());
}

#[test]
fn typed_worktree_discovery_and_creation_preserve_paths_and_common_identity() {
    let repository = TempRepository::new("worktree-list");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    let provider = provider();
    let git_repository = provider.discover(repository.path()).unwrap().unwrap();
    let destination = repository.path().join("linked checkout");
    provider
        .create_worktree(
            &git_repository,
            &WorktreeCreate {
                destination: destination.clone(),
                start: match provider.status(&git_repository).unwrap().head {
                    Head::Branch(name) => name,
                    head => panic!("expected branch, got {head:?}"),
                },
                new_branch: Some("linked-worktree".to_owned()),
            },
        )
        .unwrap();

    let worktrees = provider.worktrees(&git_repository).unwrap();
    let linked = worktrees
        .iter()
        .find(|worktree| worktree.path == destination)
        .expect("created worktree is listed");
    assert_eq!(linked.branch.as_deref(), Some("refs/heads/linked-worktree"));
    assert_eq!(linked.common_dir, git_repository.common_dir());
    assert!(
        worktrees
            .iter()
            .any(|worktree| worktree.path == repository.path())
    );

    let branches = provider.branches(&git_repository).unwrap();
    assert_eq!(
        branches
            .iter()
            .find(|branch| branch.name == "linked-worktree")
            .expect("created branch is listed")
            .checkouts,
        vec![destination]
    );
}

#[test]
fn removing_a_clean_worktree_keeps_its_branch_and_clears_checkout_annotation() {
    let repository = TempRepository::new("worktree-remove");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    let provider = provider();
    let git_repository = repository.repository();
    let destination = repository.path().join("removable");
    provider
        .create_worktree(
            &git_repository,
            &WorktreeCreate {
                destination: destination.clone(),
                start: "HEAD".to_owned(),
                new_branch: Some("kept-branch".to_owned()),
            },
        )
        .unwrap();

    provider
        .remove_worktree(&git_repository, &destination)
        .unwrap();

    assert!(!destination.exists());
    assert!(
        provider
            .worktrees(&git_repository)
            .unwrap()
            .iter()
            .all(|worktree| worktree.path != destination)
    );
    let branch = provider
        .branches(&git_repository)
        .unwrap()
        .into_iter()
        .find(|branch| branch.name == "kept-branch")
        .expect("removing a worktree must leave its branch");
    assert!(branch.checkouts.is_empty());
}

#[test]
fn removing_a_dirty_or_locked_worktree_never_forces_it() {
    let repository = TempRepository::new("worktree-remove-refuse");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    let provider = provider();
    let git_repository = repository.repository();
    let untracked = repository.path().join("untracked");
    provider
        .create_worktree(
            &git_repository,
            &WorktreeCreate {
                destination: untracked.clone(),
                start: "HEAD".to_owned(),
                new_branch: Some("untracked-branch".to_owned()),
            },
        )
        .unwrap();
    fs::write(untracked.join("untracked.txt"), "must survive\n").unwrap();
    let error = provider
        .remove_worktree(&git_repository, &untracked)
        .unwrap_err();
    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
    assert_eq!(
        fs::read_to_string(untracked.join("untracked.txt")).unwrap(),
        "must survive\n"
    );

    let dirty = repository.path().join("dirty");
    provider
        .create_worktree(
            &git_repository,
            &WorktreeCreate {
                destination: dirty.clone(),
                start: "HEAD".to_owned(),
                new_branch: Some("dirty-branch".to_owned()),
            },
        )
        .unwrap();
    fs::write(dirty.join("source.rs"), "modified and must survive\n").unwrap();
    let error = provider
        .remove_worktree(&git_repository, &dirty)
        .unwrap_err();
    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
    assert_eq!(
        fs::read_to_string(dirty.join("source.rs")).unwrap(),
        "modified and must survive\n"
    );

    let locked = repository.path().join("locked");
    provider
        .create_worktree(
            &git_repository,
            &WorktreeCreate {
                destination: locked.clone(),
                start: "HEAD".to_owned(),
                new_branch: Some("locked-branch".to_owned()),
            },
        )
        .unwrap();
    repository.git(&["worktree", "lock", locked.to_str().unwrap()]);
    let error = provider
        .remove_worktree(&git_repository, &locked)
        .unwrap_err();
    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
    assert!(locked.exists());
}

#[test]
fn worktree_preflight_refuses_dirty_files_and_requires_typing_for_unpushed_commits() {
    let clone = TempClone::new("worktree-delete-preflight");
    clone.git(&["checkout", "-q", "-b", "feature"]);
    clone.git(&["branch", "--set-upstream-to", "origin/main"]);
    clone.write("feature.rs", "local only\n");
    clone.commit("local feature");
    clone.git(&["checkout", "-q", "main"]);
    let provider = provider();
    let repository = clone.repository();
    let destination = clone.path().parent().unwrap().join("linked-feature");
    provider
        .create_worktree(
            &repository,
            &WorktreeCreate {
                destination: destination.clone(),
                start: "feature".to_owned(),
                new_branch: None,
            },
        )
        .unwrap();

    let plan = provider
        .prepare_worktree_removal(&repository, &destination)
        .unwrap();
    assert_eq!(plan.branch.as_deref(), Some("feature"));
    assert_eq!(plan.required_authorization, DeletionAuthorization::Typed);
    let error = provider
        .remove_worktree_guarded(&repository, &plan, DeletionAuthorization::Enter)
        .unwrap_err();
    assert!(error.to_string().contains("typed confirmation"), "{error}");

    fs::write(destination.join("untracked.txt"), "do not remove\n").unwrap();
    let error = provider
        .prepare_worktree_removal(&repository, &destination)
        .unwrap_err();
    assert!(error.to_string().contains("uncommitted changes"), "{error}");
    assert!(destination.join("untracked.txt").exists());

    fs::remove_file(destination.join("untracked.txt")).unwrap();
    fs::write(destination.join("source.rs"), "unstaged change\n").unwrap();
    let error = provider
        .prepare_worktree_removal(&repository, &destination)
        .unwrap_err();
    assert!(error.to_string().contains("uncommitted changes"), "{error}");
    run_in(&destination, &["add", "source.rs"]);
    let error = provider
        .prepare_worktree_removal(&repository, &destination)
        .unwrap_err();
    assert!(error.to_string().contains("uncommitted changes"), "{error}");

    run_in(
        &destination,
        &["commit", "-qm", "move reviewed worktree tip"],
    );
    let error = provider
        .remove_worktree_guarded(&repository, &plan, DeletionAuthorization::Typed)
        .unwrap_err();
    assert!(
        error.to_string().contains("changed after it was reviewed"),
        "{error}"
    );
    assert!(destination.exists());
}

#[test]
fn detached_worktree_preflight_distinguishes_retained_and_unretained_commits() {
    let repository = TempRepository::new("detached-worktree-retention");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    let current = git_output(&repository, &["branch", "--show-current"])
        .trim()
        .to_owned();
    let retained = repository.path().join("detached-retained");
    repository.git(&[
        "worktree",
        "add",
        "-q",
        "--detach",
        retained.to_str().unwrap(),
        "HEAD",
    ]);
    let provider = provider();
    let git_repository = repository.repository();
    let retained_plan = provider
        .prepare_worktree_removal(&git_repository, &retained)
        .unwrap();
    assert!(retained_plan.branch.is_none());
    assert!(retained_plan.detached_retained);
    assert_eq!(
        retained_plan.required_authorization,
        DeletionAuthorization::Enter
    );

    repository.git(&["checkout", "-q", "-b", "temporary-history"]);
    repository.write("unique.rs", "unretained\n");
    repository.commit("unretained detached tip");
    let unretained_tip = git_output(&repository, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    repository.git(&["checkout", "-q", &current]);
    repository.git(&["branch", "-D", "temporary-history"]);
    let unretained = repository.path().join("detached-unretained");
    repository.git(&[
        "worktree",
        "add",
        "-q",
        "--detach",
        unretained.to_str().unwrap(),
        &unretained_tip,
    ]);
    let unretained_plan = provider
        .prepare_worktree_removal(&git_repository, &unretained)
        .unwrap();
    assert!(unretained_plan.branch.is_none());
    assert!(!unretained_plan.detached_retained);
    assert_eq!(
        unretained_plan.required_authorization,
        DeletionAuthorization::Typed
    );
}

#[test]
fn worktree_preflight_fails_closed_when_its_attached_branch_cannot_be_inspected() {
    let repository = TempRepository::new("worktree-missing-attached-branch");
    repository.git(&["commit", "--allow-empty", "-qm", "base"]);
    repository.git(&["branch", "feature"]);
    let provider = provider();
    let git_repository = repository.repository();
    let destination = repository.path().join("linked-missing-branch");
    provider
        .create_worktree(
            &git_repository,
            &WorktreeCreate {
                destination: destination.clone(),
                start: "feature".to_owned(),
                new_branch: None,
            },
        )
        .unwrap();
    // `update-ref` can leave a linked worktree's symbolic HEAD naming a ref
    // which no longer exists. That must be an inspection failure, not an
    // untracked worktree eligible for the weaker Enter-only path.
    repository.git(&["update-ref", "-d", "refs/heads/feature"]);

    let error = provider
        .prepare_worktree_removal(&git_repository, &destination)
        .unwrap_err();
    assert!(
        error.to_string().contains("could not be inspected"),
        "{error}"
    );
    assert!(destination.exists());
}

#[test]
fn async_worktree_removal_reconciles_worktrees_and_branches() {
    let repository = TempRepository::new("worktree-remove-async");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    let provider = provider();
    let git_repository = repository.repository();
    let destination = repository.path().join("async-removable");
    provider
        .create_worktree(
            &git_repository,
            &WorktreeCreate {
                destination: destination.clone(),
                start: "HEAD".to_owned(),
                new_branch: Some("async-kept".to_owned()),
            },
        )
        .unwrap();
    let plan = provider
        .prepare_worktree_removal(&git_repository, &destination)
        .unwrap();
    let (service, mut events) = GitService::spawn(provider);
    let id = service
        .try_submit(GitOperation::Mutate {
            repository: git_repository,
            mutation: GitMutation::RemoveWorktree {
                plan: Box::new(plan),
                authorization: DeletionAuthorization::Enter,
            },
            refresh: RefreshSpec {
                branches: true,
                worktrees: true,
                ..RefreshSpec::default()
            },
        })
        .unwrap();

    loop {
        match events.blocking_recv().expect("Git service stopped") {
            GitServiceEvent::Completed {
                id: completed,
                result,
                ..
            } if completed == id => {
                let GitResponse::Mutation {
                    failure, snapshot, ..
                } = result.unwrap()
                else {
                    panic!("expected a mutation response");
                };
                assert!(failure.is_none(), "{failure:?}");
                let snapshot = snapshot.unwrap();
                let worktrees = snapshot.worktrees.expect("worktrees were reconciled");
                assert!(
                    worktrees
                        .iter()
                        .all(|worktree| worktree.path != destination)
                );
                let branch = snapshot
                    .branches
                    .expect("branches were reconciled")
                    .into_iter()
                    .find(|branch| branch.name == "async-kept")
                    .expect("the branch remains");
                assert!(branch.checkouts.is_empty());
                break;
            }
            _ => {}
        }
    }
}

#[cfg(unix)]
#[test]
fn worktree_discovery_keeps_a_non_utf8_destination_addressable() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let repository = TempRepository::new("worktree-non-utf8");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    let provider = provider();
    let git_repository = provider.discover(repository.path()).unwrap().unwrap();
    let destination = repository
        .path()
        .join(OsString::from_vec(b"linked-\xff".to_vec()));
    provider
        .create_worktree(
            &git_repository,
            &WorktreeCreate {
                destination: destination.clone(),
                start: "HEAD".to_owned(),
                new_branch: Some("encoded-worktree".to_owned()),
            },
        )
        .unwrap();
    assert!(
        provider
            .worktrees(&git_repository)
            .unwrap()
            .iter()
            .any(|worktree| worktree.path == destination)
    );
    assert_eq!(
        provider
            .branches(&git_repository)
            .unwrap()
            .into_iter()
            .find(|branch| branch.name == "encoded-worktree")
            .expect("non-UTF-8 checkout's branch is listed")
            .checkouts,
        vec![destination.clone()]
    );
    provider
        .remove_worktree(&git_repository, &destination)
        .unwrap();
    assert!(!destination.exists());
    assert!(
        provider
            .branches(&git_repository)
            .unwrap()
            .iter()
            .any(|branch| branch.name == "encoded-worktree")
    );
}

#[test]
fn failed_atomic_worktree_creation_leaves_no_destination_or_branch() {
    let repository = TempRepository::new("worktree-create-failure");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    let provider = provider();
    let git_repository = provider.discover(repository.path()).unwrap().unwrap();
    let destination = repository.path().join("failed-worktree");
    let error = provider
        .create_worktree(
            &git_repository,
            &WorktreeCreate {
                destination: destination.clone(),
                start: "missing-start-point".to_owned(),
                new_branch: Some("must-not-remain".to_owned()),
            },
        )
        .unwrap_err();
    assert!(matches!(error, GitError::Failed { .. }));
    assert!(!destination.exists());
    assert!(
        provider
            .branches(&git_repository)
            .unwrap()
            .iter()
            .all(|branch| branch.name != "must-not-remain")
    );
}

#[test]
fn a_directory_outside_any_repository_has_none() {
    let outside = std::env::temp_dir().join(format!(
        "runyte-git-provider-outside-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&outside).unwrap();

    // The system temporary directory is not itself in a repository on any
    // machine this runs on; if it were, this assertion would be meaningless
    // rather than wrong, so it is checked against Git's own answer.
    let inside = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&outside)
        .output()
        .unwrap()
        .status
        .success();
    if !inside {
        assert!(provider().discover(&outside).unwrap().is_none());
    }

    let missing = outside.join("nowhere");
    assert!(matches!(
        provider().discover(&missing),
        Err(GitError::Io { .. })
    ));
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn status_reports_the_branch_and_both_sides_of_each_change() {
    let repository = TempRepository::new("status");
    repository.write("kept.rs", "kept\n");
    repository.write("edited.rs", "one\n");
    repository.write("removed.rs", "gone\n");
    repository.commit("base");
    repository.write("edited.rs", "two\n");
    fs::remove_file(repository.path().join("removed.rs")).unwrap();
    repository.write("added.rs", "new\n");
    repository.git(&["add", "added.rs"]);
    repository.write("stray.rs", "untracked\n");

    let status = provider().status(&repository.repository()).unwrap();

    assert!(matches!(status.head, Head::Branch(_)));
    let by_path = |name: &str| {
        status
            .files
            .iter()
            .find(|file| file.path == PathBuf::from(name))
            .unwrap_or_else(|| panic!("{name} is missing from the status"))
    };
    assert_eq!(by_path("edited.rs").worktree, FileState::Modified);
    assert_eq!(by_path("removed.rs").worktree, FileState::Deleted);
    assert_eq!(by_path("added.rs").index, FileState::Added);
    assert!(by_path("stray.rs").is_untracked());
    assert!(
        !status
            .files
            .iter()
            .any(|file| file.path == PathBuf::from("kept.rs"))
    );

    let counts = status.counts();
    assert_eq!(counts.modified, 1);
    assert_eq!(counts.deleted, 1);
    assert_eq!(counts.added, 1);
    assert_eq!(counts.untracked, 1);
}

/// The changed-file list's numbers, from the trees Git compared to produce the
/// status they sit beside.
///
/// A file that is staged and then edited again is counted twice on purpose:
/// those are two changes with a row each. An untracked file has no diff at
/// all, so its lines are counted from the file itself, and one that is not
/// text is left uncounted exactly as `--numstat` leaves a binary one.
#[test]
fn line_counts_cover_both_sides_of_the_index_and_untracked_files() {
    let repository = TempRepository::new("numstat");
    repository.write("both.rs", "one\ntwo\nthree\n");
    repository.write("edited.rs", "one\ntwo\n");
    repository.write("image.bin", "placeholder\n");
    repository.commit("base");
    repository.write("both.rs", "one\ntwo\nthree\nfour\n");
    repository.git(&["add", "both.rs"]);
    repository.write("both.rs", "one\ntwo\nthree\nfour\nfive\n");
    repository.write("edited.rs", "one\n");
    fs::write(repository.path().join("image.bin"), [0u8, 1, 2, 3]).unwrap();
    repository.write("stray.rs", "new\nlines\nhere\n");
    fs::write(repository.path().join("stray.bin"), [0u8, 9]).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("stray.rs", repository.path().join("stray-link.rs")).unwrap();

    let status = provider().status(&repository.repository()).unwrap();
    let stats = provider()
        .status_stats(&repository.repository(), &status)
        .unwrap();
    let counted = |scope, name: &str| stats.get(scope, Path::new(name));

    assert_eq!(
        counted(DiffScope::Staged, "both.rs"),
        Some(LineStats::new(1, 0))
    );
    assert_eq!(
        counted(DiffScope::Unstaged, "both.rs"),
        Some(LineStats::new(1, 0))
    );
    assert_eq!(
        counted(DiffScope::Unstaged, "edited.rs"),
        Some(LineStats::new(0, 1))
    );
    assert_eq!(
        counted(DiffScope::Unstaged, "stray.rs"),
        Some(LineStats::new(3, 0)),
        "an untracked file is all additions"
    );
    assert_eq!(
        counted(DiffScope::Unstaged, "image.bin"),
        None,
        "a binary file has no lines to count"
    );
    assert_eq!(counted(DiffScope::Unstaged, "stray.bin"), None);
    #[cfg(unix)]
    assert_eq!(
        counted(DiffScope::Unstaged, "stray-link.rs"),
        None,
        "an untracked symlink is not counted by following its target"
    );
    assert_eq!(counted(DiffScope::Staged, "edited.rs"), None);
}

/// A rename is one row in the list, named by where the file ended up, so its
/// count has to arrive under that path rather than under the old one.
#[test]
fn a_renamed_file_is_counted_under_its_new_path() {
    let repository = TempRepository::new("numstat-rename");
    repository.write("old.rs", "one\ntwo\n");
    repository.commit("base");
    repository.git(&["mv", "old.rs", "new.rs"]);
    repository.write("new.rs", "one\ntwo\nthree\n");
    repository.git(&["add", "new.rs"]);

    let status = provider().status(&repository.repository()).unwrap();
    let stats = provider()
        .status_stats(&repository.repository(), &status)
        .unwrap();

    assert_eq!(
        stats.get(DiffScope::Staged, Path::new("new.rs")),
        Some(LineStats::new(1, 0))
    );
    assert_eq!(stats.get(DiffScope::Staged, Path::new("old.rs")), None);
}

/// A name Git would have had to quote in the default format, and a name that
/// would be read as an option if arguments went through anything but an
/// argument vector.
#[test]
fn awkward_paths_survive_status_and_staged_reads() {
    let repository = TempRepository::new("awkward");
    let awkward = "a dir/--not-an-option --;$(touch pwned).txt";
    repository.write(awkward, "one\n");
    repository.commit("base");
    repository.write(awkward, "two\n");

    let status = provider().status(&repository.repository()).unwrap();
    assert_eq!(status.files.len(), 1);
    assert_eq!(status.files[0].path, PathBuf::from(awkward));

    let staged = provider()
        .staged_content(&repository.repository(), &repository.path().join(awkward))
        .unwrap();
    assert_eq!(staged, BaseContent::Text("one\n".to_owned()));
    assert!(
        !repository.path().join("pwned").exists(),
        "no shell interpreted the path"
    );
}

#[test]
fn staged_content_is_the_index_rather_than_head() {
    let repository = TempRepository::new("staged");
    repository.write("source.rs", "committed\n");
    repository.commit("base");
    repository.write("source.rs", "staged\n");
    repository.git(&["add", "source.rs"]);
    repository.write("source.rs", "working\n");

    let staged = provider()
        .staged_content(
            &repository.repository(),
            &repository.path().join("source.rs"),
        )
        .unwrap();

    assert_eq!(staged, BaseContent::Text("staged\n".to_owned()));
}

#[test]
fn untracked_binary_and_conflicted_paths_have_no_staged_text() {
    let repository = TempRepository::new("absent");
    repository.write("tracked.rs", "a\n");
    fs::write(repository.path().join("image.bin"), [0u8, 1, 2, 3]).unwrap();
    repository.commit("base");
    repository.write("stray.rs", "new\n");

    let staged = |name: &str| {
        provider()
            .staged_content(&repository.repository(), &repository.path().join(name))
            .unwrap()
    };

    assert_eq!(staged("stray.rs"), BaseContent::Absent);
    assert_eq!(staged("image.bin"), BaseContent::Binary);
    assert_eq!(staged("missing.rs"), BaseContent::Absent);
}

/// A path in the middle of a merge has index entries at stages one to three
/// and none at stage zero, so there is no single text it used to be.
#[test]
fn a_conflicted_path_reports_no_base() {
    let repository = TempRepository::new("conflict");
    repository.write("clash.rs", "base\n");
    repository.commit("base");
    repository.git(&["checkout", "-q", "-b", "other"]);
    repository.write("clash.rs", "theirs\n");
    repository.commit("theirs");
    repository.git(&["checkout", "-q", "-"]);
    repository.write("clash.rs", "ours\n");
    repository.commit("ours");
    let merge = Command::new("git")
        .args(["merge", "other"])
        .current_dir(repository.path())
        .output()
        .unwrap();
    assert!(!merge.status.success(), "the merge should conflict");

    let staged = provider()
        .staged_content(
            &repository.repository(),
            &repository.path().join("clash.rs"),
        )
        .unwrap();
    let status = provider().status(&repository.repository()).unwrap();

    assert_eq!(staged, BaseContent::Absent);
    assert_eq!(status.counts().conflicted, 1);
}

#[test]
fn an_unborn_branch_still_reports_its_name_and_untracked_files() {
    let repository = TempRepository::new("unborn");
    repository.write("first.rs", "new\n");

    let status = provider().status(&repository.repository()).unwrap();

    assert!(matches!(status.head, Head::Unborn(_)), "{:?}", status.head);
    assert!(status.head.label().ends_with("(unborn)"));
    assert_eq!(status.counts().untracked, 1);
}

#[test]
fn a_detached_head_reports_the_commit_it_is_on() {
    let repository = TempRepository::new("detached");
    repository.write("source.rs", "a\n");
    repository.commit("base");
    repository.git(&["checkout", "-q", "--detach"]);

    let status = provider().status(&repository.repository()).unwrap();

    assert!(
        matches!(status.head, Head::Detached(_)),
        "{:?}",
        status.head
    );
    assert!(status.head.label().starts_with('@'));
}

#[test]
fn local_branches_are_listed_and_a_clean_checkout_switches_head() {
    let repository = TempRepository::new("branches");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    let provider = provider();
    let current = match provider.status(&repository.repository()).unwrap().head {
        Head::Branch(name) => name,
        head => panic!("expected an attached branch, got {head:?}"),
    };
    repository.git(&["branch", "feature"]);

    let branches = provider.branches(&repository.repository()).unwrap();
    assert_eq!(branches.len(), 2);
    assert!(
        branches
            .iter()
            .any(|branch| branch.name == current && branch.current)
    );
    assert!(
        branches
            .iter()
            .any(|branch| branch.name == "feature" && !branch.current)
    );

    provider
        .checkout_branch(&repository.repository(), "feature")
        .unwrap();
    assert_eq!(
        provider.status(&repository.repository()).unwrap().head,
        Head::Branch("feature".to_owned())
    );
}

#[test]
fn checkout_refuses_uncommitted_changes_before_git_can_switch() {
    let repository = TempRepository::new("dirty-checkout");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    repository.git(&["branch", "feature"]);
    repository.write("source.rs", "edited\n");
    let provider = provider();
    let before = provider.status(&repository.repository()).unwrap().head;

    let error = provider
        .checkout_branch(&repository.repository(), "feature")
        .unwrap_err();

    assert!(matches!(error, GitError::DirtyWorktree { files: 1 }));
    assert_eq!(
        provider.status(&repository.repository()).unwrap().head,
        before
    );
}

/// The two things a branch row says beyond its name come from Git rather than
/// from anything Runyte computes: how far it has drifted from the ref it
/// tracks, and whether its commits are already reachable from `HEAD`.
#[test]
fn branches_report_upstream_drift_and_whether_they_are_merged() {
    let clone = TempClone::new("drift");
    // One commit here that the remote has not seen, one there that this clone
    // has not, so the tracking branch has drifted in both directions.
    clone.write("source.rs", "local\n");
    clone.commit("local");
    clone.git(&["branch", "unmerged"]);
    clone.git(&["checkout", "-q", "unmerged"]);
    clone.write("source.rs", "unmerged\n");
    clone.commit("unmerged");
    clone.git(&["checkout", "-q", "-"]);
    clone.commit_upstream("remote\n");
    clone.git(&["fetch", "-q"]);
    let provider = provider();

    let branches = provider.branches(&clone.repository()).unwrap();

    let tracking = branches
        .iter()
        .find(|branch| branch.current)
        .expect("the checked-out branch");
    let upstream = tracking.upstream.as_ref().expect("a tracked upstream");
    assert_eq!(upstream.name, "origin/main");
    assert_eq!(upstream.remote, "origin");
    assert_eq!(upstream.reference, "refs/heads/main");
    assert_eq!(
        upstream.divergence,
        Some(Divergence {
            ahead: 1,
            behind: 1,
        })
    );
    assert!(tracking.merged, "HEAD is reachable from itself");
    let unmerged = branches
        .iter()
        .find(|branch| branch.name == "unmerged")
        .expect("the second branch");
    assert_eq!(unmerged.upstream, None);
    assert!(!unmerged.merged, "its commit is not on the current branch");
}

/// A pull takes what the remote has when nothing local competes with it, and
/// refuses rather than merging when something does.
#[test]
fn pulling_fast_forwards_and_refuses_a_branch_that_has_diverged() {
    let clone = TempClone::new("pull");
    clone.commit_upstream("remote\n");
    let provider = provider();
    let before = clone.head();

    let summary = provider.pull(&clone.repository()).unwrap();

    assert!(summary.contains("Fast-forward"), "{summary}");
    assert_ne!(clone.head(), before, "the clone took the remote's commit");
    assert_eq!(
        fs::read_to_string(clone.path().join("source.rs")).unwrap(),
        "remote\n"
    );
    assert_eq!(
        provider
            .branches(&clone.repository())
            .unwrap()
            .iter()
            .find(|branch| branch.current)
            .and_then(|branch| branch.upstream.as_ref())
            .and_then(|upstream| upstream.divergence),
        Some(Divergence::default()),
        "the branch is level with its upstream afterwards"
    );

    // Nothing to take is a success that says so rather than an error.
    let summary = provider.pull(&clone.repository()).unwrap();
    assert!(summary.contains("up to date"), "{summary}");

    // Commits on both sides mean a merge, which is exactly what `--ff-only`
    // refuses: the working tree is left alone rather than half-merged.
    clone.write("source.rs", "local\n");
    clone.commit("local");
    clone.commit_upstream("remote again\n");
    let before = clone.head();

    let error = provider.pull(&clone.repository()).unwrap_err();

    // Reported as drift with its counts rather than as whatever `--ff-only`
    // printed, because this is the one refusal a caller can offer a way out of.
    assert_eq!(
        error,
        GitError::Diverged {
            branch: "main".to_owned(),
            upstream: "origin/main".to_owned(),
            ahead: 1,
            behind: 1,
        },
        "{error:?}"
    );
    assert_eq!(clone.head(), before, "a refused pull moved nothing");
    assert_eq!(
        fs::read_to_string(clone.path().join("source.rs")).unwrap(),
        "local\n"
    );
}

/// Two people on one branch, editing different files: the local commits are
/// replayed on top of the remote ones, and both sets of work survive.
#[test]
fn rebasing_replays_local_commits_onto_the_upstream() {
    let clone = TempClone::new("rebase");
    clone.write("local.rs", "first\n");
    clone.commit("local one");
    clone.write("local.rs", "second\n");
    clone.commit("local two");
    clone.commit_upstream_file("theirs.rs", "from the other clone\n");
    let provider = provider();

    let diverged = provider.pull(&clone.repository()).unwrap_err();
    assert!(
        matches!(
            diverged,
            GitError::Diverged {
                ahead: 2,
                behind: 1,
                ..
            }
        ),
        "{diverged:?}"
    );

    provider.rebase_onto_upstream(&clone.repository()).unwrap();

    // Linear, with the two local commits replayed on top of theirs.
    assert_eq!(
        clone.subjects(),
        vec!["local two", "local one", "from elsewhere", "base"]
    );
    assert_eq!(
        fs::read_to_string(clone.path().join("local.rs")).unwrap(),
        "second\n"
    );
    assert_eq!(
        fs::read_to_string(clone.path().join("theirs.rs")).unwrap(),
        "from the other clone\n",
        "the other clone's file arrived"
    );
    assert_eq!(
        provider
            .branches(&clone.repository())
            .unwrap()
            .into_iter()
            .find(|branch| branch.current)
            .and_then(|branch| branch.upstream)
            .and_then(|upstream| upstream.divergence),
        Some(Divergence {
            ahead: 2,
            behind: 0
        }),
        "ahead of the upstream and no longer behind it"
    );

    // Which is what the push that was rejected before now goes through as.
    provider.push(&clone.repository(), "main").unwrap();
    assert_eq!(
        clone.remote_head("main").as_deref(),
        Some(clone.head().as_str())
    );
}

/// The invariant a fast-forward-only pull protects holds for the replay too: a
/// rebase that cannot finish undoes itself rather than leaving a working tree
/// Runyte has no surface to resolve.
#[test]
fn a_conflicting_rebase_is_undone_and_changes_nothing() {
    let clone = TempClone::new("rebase-conflict");
    // Both sides edited the same line of the same file, which is the one thing
    // a replay cannot decide on its own.
    clone.write("source.rs", "mine\n");
    clone.commit("local");
    clone.commit_upstream("theirs\n");
    let provider = provider();
    let before = clone.head();

    provider.pull(&clone.repository()).unwrap_err();
    let error = provider
        .rebase_onto_upstream(&clone.repository())
        .unwrap_err();

    let message = error.to_string();
    assert!(message.contains("conflict"), "{message}");
    assert!(message.contains("undone"), "{message}");
    assert_eq!(clone.head(), before, "the branch is where it was");
    assert!(
        !clone.rebase_in_progress(),
        "no half-finished rebase was left behind"
    );
    assert_eq!(
        fs::read_to_string(clone.path().join("source.rs")).unwrap(),
        "mine\n",
        "no conflict markers were written into the working tree"
    );
    assert!(
        provider
            .status(&clone.repository())
            .unwrap()
            .files
            .is_empty(),
        "the working tree is clean"
    );
    assert_eq!(clone.subjects(), vec!["local", "base"]);
}

/// Neither the pull nor the replay may inherit a configured autostash.
///
/// With `rebase.autoStash` or `merge.autoStash` set, Git stashes uncommitted
/// changes, does its work, and reapplies them. When that reapplication
/// conflicts it says so and *exits successfully*, leaving conflict markers in
/// the working tree and a stash to recover from. Nothing is mid-rebase
/// afterwards, so no rollback would run and the caller would report success
/// over a conflicted tree. Refusing the dirty tree up front is the only answer
/// that keeps that from happening.
#[test]
fn a_configured_autostash_cannot_turn_a_refusal_into_a_conflicted_success() {
    let clone = TempClone::new("autostash");
    clone.git(&["config", "rebase.autoStash", "true"]);
    clone.git(&["config", "merge.autoStash", "true"]);
    let provider = provider();

    // Behind only, with an uncommitted edit to the very file the remote
    // changed: the fast-forward would stash it and fail to put it back.
    clone.commit_upstream("from elsewhere\n");
    clone.write("source.rs", "uncommitted\n");
    let before = clone.head();

    let error = provider.pull(&clone.repository()).unwrap_err();

    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
    assert_eq!(clone.head(), before, "the refused pull moved nothing");
    assert_eq!(
        fs::read_to_string(clone.path().join("source.rs")).unwrap(),
        "uncommitted\n",
        "no conflict markers were written into the working tree"
    );
    assert!(clone.stashes().is_empty(), "no stash was left behind");

    // The same for the replay, on a branch that has drifted both ways.
    clone.git(&["checkout", "-q", "--", "source.rs"]);
    clone.write("local.rs", "mine\n");
    clone.commit("local");
    clone.commit_upstream("moved again\n");
    clone.write("source.rs", "uncommitted\n");
    let before = clone.head();

    let error = provider
        .rebase_onto_upstream(&clone.repository())
        .unwrap_err();

    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
    assert_eq!(clone.head(), before, "the refused replay moved nothing");
    assert_eq!(
        fs::read_to_string(clone.path().join("source.rs")).unwrap(),
        "uncommitted\n"
    );
    assert!(clone.stashes().is_empty(), "no stash was left behind");
    assert!(!clone.rebase_in_progress());
}

/// A pull that never reached the remote says so, rather than reporting drift
/// that only the last successful fetch knows about.
///
/// The remote-tracking refs outlive the connection that filled them, so a
/// branch that was already diverged still looks diverged when the remote is
/// unreachable. Reading the drift from those would turn "the remote did not
/// answer" into an offer to replay commits onto a tip nobody has seen.
#[test]
fn an_unreachable_remote_is_reported_as_itself_and_not_as_stale_drift() {
    let clone = TempClone::new("pull-unreachable");
    clone.write("source.rs", "local\n");
    clone.commit("local");
    clone.commit_upstream("remote\n");
    let provider = provider();

    // The drift is real and cached, so the only thing separating the two
    // outcomes is whether this pull's own fetch got through.
    let diverged = provider.pull(&clone.repository()).unwrap_err();
    assert!(
        matches!(diverged, GitError::Diverged { .. }),
        "{diverged:?}"
    );

    clone.git(&["remote", "set-url", "origin", "/nonexistent/gone.git"]);
    let error = provider.pull(&clone.repository()).unwrap_err();

    assert!(
        !matches!(error, GitError::Diverged { .. }),
        "an unreachable remote was reported as drift: {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("does not appear to be a git repository")
            || message.contains("Could not read from remote"),
        "{message}"
    );
}

/// The drift and the replay both survive the asynchronous service, which is
/// the path production takes: the editor never sees the provider directly.
#[test]
fn a_diverged_pull_and_its_replay_run_through_the_ordered_async_service() {
    let clone = TempClone::new("rebase-async-service");
    clone.write("local.rs", "mine\n");
    clone.commit("local");
    clone.commit_upstream_file("theirs.rs", "theirs\n");
    let (service, mut events) = GitService::spawn(provider());

    let failure = submit_mutation(&service, &mut events, &clone, GitMutation::Pull)
        .expect_err("a diverged branch has no fast-forward");
    assert!(
        matches!(
            failure,
            GitError::Diverged {
                ahead: 1,
                behind: 1,
                ..
            }
        ),
        "{failure:?}"
    );

    submit_mutation(
        &service,
        &mut events,
        &clone,
        GitMutation::RebaseOntoUpstream,
    )
    .expect("the replay went through");

    assert_eq!(clone.subjects(), vec!["local", "from elsewhere", "base"]);
}

/// Runs one mutation through the service and waits for its own completion,
/// returning the failure it reported if there was one.
fn submit_mutation(
    service: &GitServiceHandle,
    events: &mut tokio::sync::mpsc::Receiver<GitServiceEvent>,
    clone: &TempClone,
    mutation: GitMutation,
) -> Result<(), GitError> {
    let id = service
        .try_submit(GitOperation::Mutate {
            repository: clone.repository(),
            mutation,
            refresh: RefreshSpec::default(),
        })
        .unwrap();
    loop {
        match events.blocking_recv().expect("Git service stopped") {
            GitServiceEvent::Completed {
                id: completed,
                result,
                ..
            } if completed == id => match *result {
                Ok(GitResponse::Mutation { failure, .. }) => {
                    return failure.map_or(Ok(()), Err);
                }
                result => panic!("unexpected mutation result: {result:?}"),
            },
            _ => {}
        }
    }
}

/// A branch with nothing to replay is still reconciled, which is what a reader
/// who pressed the key after someone else pushed expects.
#[test]
fn rebasing_needs_a_branch_with_an_upstream() {
    let clone = TempClone::new("rebase-upstream");
    let provider = provider();

    // Behind only: there is nothing local to replay, and it fast-forwards.
    clone.commit_upstream("remote\n");
    provider.rebase_onto_upstream(&clone.repository()).unwrap();
    assert_eq!(
        fs::read_to_string(clone.path().join("source.rs")).unwrap(),
        "remote\n"
    );

    // A branch tracking nothing has nothing to replay onto, and a detached
    // HEAD is not a branch at all.
    clone.git(&["checkout", "-q", "-b", "solo"]);
    let error = provider
        .rebase_onto_upstream(&clone.repository())
        .unwrap_err();
    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");

    clone.git(&["checkout", "-q", "--detach"]);
    let error = provider
        .rebase_onto_upstream(&clone.repository())
        .unwrap_err();
    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
}

#[test]
fn pulling_needs_a_branch_with_an_upstream() {
    let clone = TempClone::new("pull-upstream");
    clone.git(&["checkout", "-q", "-b", "solo"]);
    let provider = provider();

    // A branch tracking nothing has nowhere to pull from.
    let error = provider.pull(&clone.repository()).unwrap_err();
    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");

    // Neither has a detached HEAD, which is not a branch at all.
    clone.git(&["checkout", "-q", "--detach"]);
    let error = provider.pull(&clone.repository()).unwrap_err();
    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
}

/// Pushing publishes to the ref the branch tracks, and creates that ref the
/// first time round.
#[test]
fn pushing_publishes_a_tracked_branch_and_adopts_an_untracked_one() {
    let clone = TempClone::new("push");
    clone.write("source.rs", "local\n");
    clone.commit("local");
    let provider = provider();

    let summary = provider.push(&clone.repository(), "main").unwrap();

    assert!(!summary.is_empty());
    assert_eq!(
        clone.remote_head("main").as_deref(),
        Some(clone.head().as_str()),
        "the remote holds what the clone does"
    );
    // Pushing again has nothing to send and says so.
    let summary = provider.push(&clone.repository(), "main").unwrap();
    assert!(summary.contains("up-to-date"), "{summary}");

    // A branch tracking nothing is published and adopted, so the row that had
    // no annotation before has one afterwards.
    clone.git(&["checkout", "-q", "-b", "feature"]);
    clone.write("source.rs", "on feature\n");
    clone.commit("feature work");
    assert!(clone.remote_head("feature").is_none());

    provider.push(&clone.repository(), "feature").unwrap();

    assert_eq!(
        clone.remote_head("feature").as_deref(),
        Some(clone.head().as_str())
    );
    let upstream = provider
        .branches(&clone.repository())
        .unwrap()
        .into_iter()
        .find(|branch| branch.name == "feature")
        .and_then(|branch| branch.upstream)
        .expect("the push set an upstream");
    assert_eq!(upstream.name, "origin/feature");
    assert_eq!(upstream.divergence, Some(Divergence::default()));
}

/// A branch that is not checked out can still be published, because pushing
/// touches no working tree.
#[test]
fn pushing_works_on_a_branch_that_is_not_checked_out() {
    let clone = TempClone::new("push-other");
    clone.git(&["checkout", "-q", "-b", "feature"]);
    clone.write("source.rs", "on feature\n");
    clone.commit("feature work");
    let published = clone.head();
    clone.git(&["checkout", "-q", "main"]);
    let provider = provider();

    provider.push(&clone.repository(), "feature").unwrap();

    assert_eq!(clone.remote_head("feature").as_deref(), Some(&*published));
    // The working tree stayed on the branch it was on.
    assert_eq!(
        provider.status(&clone.repository()).unwrap().head,
        Head::Branch("main".to_owned())
    );

    // A name that is not a local branch is refused before Git is asked.
    let error = provider
        .push(&clone.repository(), "no-such-branch")
        .unwrap_err();
    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
}

#[test]
fn pushing_refuses_an_option_shaped_default_remote() {
    let repository = TempRepository::new("push-option-default-remote");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    repository.git(&[
        "remote",
        "add",
        "--",
        "--receive-pack=runyte-test-helper",
        "/does/not/matter",
    ]);
    let provider = provider();
    let branch = match provider.status(&repository.repository()).unwrap().head {
        Head::Branch(branch) => branch,
        head => panic!("expected an attached branch, got {head:?}"),
    };

    let error = provider
        .push(&repository.repository(), &branch)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("remote names beginning with `-` are refused"),
        "{error}"
    );
}

#[test]
fn pushing_refuses_an_option_shaped_tracked_remote() {
    let clone = TempClone::new("push-option-tracked-remote");
    let remote = "--receive-pack=runyte-test-helper";
    clone.git(&[
        "remote",
        "add",
        "--",
        remote,
        clone.origin.to_str().unwrap(),
    ]);
    clone.git(&["push", "--", remote, "main"]);
    clone.git(&["config", "branch.main.remote", remote]);
    clone.git(&["config", "branch.main.merge", "refs/heads/main"]);
    let provider = provider();
    let branches = provider.branches(&clone.repository()).unwrap();
    let upstream = branches
        .iter()
        .find(|branch| branch.name == "main")
        .and_then(|branch| branch.upstream.as_ref())
        .expect("the maliciously named upstream remains visible");
    assert_eq!(upstream.remote, remote);

    let error = provider.push(&clone.repository(), "main").unwrap_err();

    assert!(
        error
            .to_string()
            .contains("remote names beginning with `-` are refused"),
        "{error}"
    );
}

/// A push that would overwrite work on the remote is a refusal to report, not
/// something to overrule: nothing here forces.
#[test]
fn pushing_refuses_rather_than_overwriting_the_remote() {
    let clone = TempClone::new("push-reject");
    clone.commit_upstream("remote\n");
    clone.write("source.rs", "local\n");
    clone.commit("local");
    let provider = provider();
    let remote_before = clone.remote_head("main");

    let error = provider.push(&clone.repository(), "main").unwrap_err();

    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
    assert_eq!(
        clone.remote_head("main"),
        remote_before,
        "the remote still holds what it did"
    );
    // Git puts what to do about this in a `hint:` line, which is filtered out
    // along with the rest of the noise, so the refusal says it itself. Without
    // that, a pull that refuses a diverged branch on the other side leaves the
    // two keys pointing at each other.
    //
    // Which of the two wordings Git uses depends on whether the clone has
    // fetched since the remote moved, so both are treated as the same refusal.
    let message = error.to_string();
    assert!(
        message.contains("(fetch first)") || message.contains("(non-fast-forward)"),
        "{message}"
    );
    assert!(
        message.contains("behind what the remote holds"),
        "{message}"
    );
    assert!(message.contains("Pull it first"), "{message}");
}

/// Creating a branch is one step, not two: what comes back is a working tree
/// on the new branch, holding what the branch it started from held.
#[test]
fn creating_a_branch_starts_it_where_asked_and_switches_to_it() {
    let repository = TempRepository::new("create-branch");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    repository.git(&["branch", "feature"]);
    repository.git(&["checkout", "-q", "feature"]);
    repository.write("source.rs", "on feature\n");
    repository.commit("feature work");
    let provider = provider();
    let base = match provider.status(&repository.repository()).unwrap().head {
        Head::Branch(name) => name,
        head => panic!("expected an attached branch, got {head:?}"),
    };
    assert_eq!(base, "feature");
    provider
        .checkout_branch(&repository.repository(), "main")
        .or_else(|_| provider.checkout_branch(&repository.repository(), "master"))
        .unwrap();

    provider
        .create_branch(&repository.repository(), "spike", "feature")
        .unwrap();

    assert_eq!(
        provider.status(&repository.repository()).unwrap().head,
        Head::Branch("spike".to_owned())
    );
    assert_eq!(
        fs::read_to_string(repository.path().join("source.rs")).unwrap(),
        "on feature\n"
    );

    // A name already taken fails, and fails before `HEAD` has moved anywhere.
    let error = provider
        .create_branch(&repository.repository(), "feature", "spike")
        .unwrap_err();
    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
    assert_eq!(
        provider.status(&repository.repository()).unwrap().head,
        Head::Branch("spike".to_owned())
    );
    // So does a name Git will not accept, and one that could be read as an
    // option rather than as a name.
    for name in ["--force", "bad name"] {
        let error = provider
            .create_branch(&repository.repository(), name, "spike")
            .unwrap_err();
        assert!(
            matches!(error, GitError::Failed { .. }),
            "{name}: {error:?}"
        );
    }
}

/// A dirty working tree stops a branch from being created, not just from being
/// switched to: the refusal happens before anything exists to clean up.
#[test]
fn creating_a_branch_refuses_uncommitted_changes_before_making_a_ref() {
    let repository = TempRepository::new("create-branch-dirty");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    let provider = provider();
    let before = provider.status(&repository.repository()).unwrap().head;
    let start = match &before {
        Head::Branch(name) => name.clone(),
        head => panic!("expected an attached branch, got {head:?}"),
    };
    repository.write("source.rs", "edited\n");

    let error = provider
        .create_branch(&repository.repository(), "spike", &start)
        .unwrap_err();

    assert!(matches!(error, GitError::DirtyWorktree { files: 1 }));
    assert_eq!(
        provider.status(&repository.repository()).unwrap().head,
        before
    );
    assert!(
        provider
            .branches(&repository.repository())
            .unwrap()
            .iter()
            .all(|branch| branch.name != "spike"),
        "the refused branch must not have been created"
    );
}

#[test]
fn deleting_a_branch_needs_force_only_when_its_commits_are_not_merged() {
    let repository = TempRepository::new("delete-branch");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    let provider = provider();
    let current = match provider.status(&repository.repository()).unwrap().head {
        Head::Branch(name) => name,
        head => panic!("expected an attached branch, got {head:?}"),
    };
    repository.git(&["branch", "merged"]);
    repository.git(&["checkout", "-q", "-b", "unmerged"]);
    repository.write("source.rs", "unmerged\n");
    repository.commit("unmerged work");
    repository.git(&["checkout", "-q", &current]);

    // The branch this working tree is on is refused before Git is asked.
    let error = provider
        .delete_branch(&repository.repository(), &current, true)
        .unwrap_err();
    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");

    provider
        .delete_branch(&repository.repository(), "merged", false)
        .unwrap();
    // An unmerged branch is exactly what an unforced delete refuses.
    let error = provider
        .delete_branch(&repository.repository(), "unmerged", false)
        .unwrap_err();
    assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
    provider
        .delete_branch(&repository.repository(), "unmerged", true)
        .unwrap();

    let names = provider
        .branches(&repository.repository())
        .unwrap()
        .into_iter()
        .map(|branch| branch.name)
        .collect::<Vec<_>>();
    assert_eq!(names, vec![current]);
}

#[test]
fn guarded_branch_deletion_distinguishes_local_retention_and_rejects_a_moved_tip() {
    let repository = TempRepository::new("guarded-delete-branch");
    repository.write("source.rs", "base\n");
    repository.commit("base");
    let current = git_output(&repository, &["branch", "--show-current"])
        .trim()
        .to_owned();
    repository.git(&["checkout", "-q", "-b", "feature"]);
    repository.write("feature.rs", "unique\n");
    repository.commit("feature");
    repository.git(&["branch", "keeper"]);
    repository.git(&["checkout", "-q", &current]);
    let provider = provider();
    let git_repository = repository.repository();

    let retained = provider
        .prepare_branch_deletion(&git_repository, "feature")
        .unwrap();
    assert_eq!(retained.retaining_branches, vec!["keeper"]);
    assert_eq!(
        retained.required_authorization,
        DeletionAuthorization::Enter
    );

    repository.git(&["branch", "-D", "keeper"]);
    let unpublished = provider
        .prepare_branch_deletion(&git_repository, "feature")
        .unwrap();
    assert!(unpublished.retaining_branches.is_empty());
    assert_eq!(
        unpublished.required_authorization,
        DeletionAuthorization::Typed
    );
    let error = provider
        .delete_branch_guarded(&git_repository, &unpublished, DeletionAuthorization::Enter)
        .unwrap_err();
    assert!(error.to_string().contains("typed confirmation"), "{error}");

    repository.git(&["branch", "-f", "feature", &current]);
    let error = provider
        .delete_branch_guarded(&git_repository, &unpublished, DeletionAuthorization::Typed)
        .unwrap_err();
    assert!(
        error.to_string().contains("changed after it was reviewed"),
        "{error}"
    );
}

/// The provider must refuse output it cannot hold rather than grow to fit it.
#[test]
fn output_past_the_bound_is_refused_and_the_child_is_stopped() {
    let repository = TempRepository::new("bound");
    repository.write("large.txt", &"x".repeat(64 * 1024));
    repository.commit("base");

    let error = provider()
        .with_max_output_bytes(1024)
        .staged_content(
            &repository.repository(),
            &repository.path().join("large.txt"),
        )
        .unwrap_err();

    assert!(
        matches!(error, GitError::TooLarge { limit: 1024, .. }),
        "{error}"
    );
}

#[test]
fn an_absent_git_leaves_every_answer_unavailable() {
    let repository = TempRepository::new("absent-git");
    let provider = GitCliProvider::new("runyte-git-that-does-not-exist");

    let error = provider.discover(repository.path()).unwrap_err();
    assert!(error.is_unavailable(), "{error}");
    assert!(
        provider
            .status(&repository.repository())
            .unwrap_err()
            .is_unavailable()
    );
}

/// A path that belongs to some other repository is not answered about.
#[test]
fn a_path_outside_the_repository_is_refused() {
    let repository = TempRepository::new("outside");
    repository.write("source.rs", "a\n");
    repository.commit("base");

    let error = provider()
        .staged_content(&repository.repository(), Path::new("/elsewhere/source.rs"))
        .unwrap_err();

    assert!(matches!(error, GitError::NotARepository { .. }), "{error}");
}

#[test]
fn the_two_diff_scopes_are_different_comparisons() {
    let repository = TempRepository::new("diff-scopes");
    repository.write("source.rs", "committed\n");
    repository.commit("base");
    repository.write("source.rs", "staged\n");
    repository.git(&["add", "source.rs"]);
    repository.write("source.rs", "working\n");
    let provider = provider();
    let path = repository.path().join("source.rs");

    let unstaged = provider
        .diff(&repository.repository(), DiffScope::Unstaged, Some(&path))
        .unwrap();
    let staged = provider
        .diff(&repository.repository(), DiffScope::Staged, Some(&path))
        .unwrap();

    // Unstaged is the index against the working tree.
    assert!(unstaged.contains("-staged"), "{unstaged}");
    assert!(unstaged.contains("+working"), "{unstaged}");
    // Staged is HEAD against the index: what a commit would take.
    assert!(staged.contains("-committed"), "{staged}");
    assert!(staged.contains("+staged"), "{staged}");
}

#[test]
fn complete_file_comparisons_follow_each_diff_scope_and_preserve_absence() {
    let repository = TempRepository::new("file-comparisons");
    repository.write("source.rs", "committed\n");
    repository.write("tracked.rs", "committed\n");
    repository.commit("base");
    repository.write("source.rs", "staged\n");
    repository.write("tracked.rs", "staged\n");
    repository.git(&["add", "source.rs", "tracked.rs"]);
    repository.write("source.rs", "working\n");
    repository.write("tracked.rs", "working\n");
    repository.write("new.rs", "new\n");
    repository.git(&["rm", "-q", "-f", "--cached", "source.rs"]);
    let provider = provider();

    assert_eq!(
        provider
            .file_comparison(
                &repository.repository(),
                DiffScope::Staged,
                &repository.path().join("tracked.rs"),
            )
            .unwrap(),
        FileComparison {
            previous: BaseContent::Text("committed\n".to_owned()),
            current: BaseContent::Text("staged\n".to_owned()),
        }
    );
    assert_eq!(
        provider
            .file_comparison(
                &repository.repository(),
                DiffScope::Unstaged,
                &repository.path().join("tracked.rs"),
            )
            .unwrap(),
        FileComparison {
            previous: BaseContent::Text("staged\n".to_owned()),
            current: BaseContent::Text("working\n".to_owned()),
        }
    );

    assert_eq!(
        provider
            .file_comparison(
                &repository.repository(),
                DiffScope::Staged,
                &repository.path().join("source.rs"),
            )
            .unwrap(),
        FileComparison {
            previous: BaseContent::Text("committed\n".to_owned()),
            current: BaseContent::Absent,
        }
    );
    assert_eq!(
        provider
            .file_comparison(
                &repository.repository(),
                DiffScope::Unstaged,
                &repository.path().join("new.rs"),
            )
            .unwrap(),
        FileComparison {
            previous: BaseContent::Absent,
            current: BaseContent::Text("new\n".to_owned()),
        }
    );
}

#[test]
fn a_path_with_no_changes_diffs_to_nothing() {
    let repository = TempRepository::new("diff-clean");
    repository.write("source.rs", "a\n");
    repository.commit("base");

    let diff = provider()
        .diff(
            &repository.repository(),
            DiffScope::Unstaged,
            Some(&repository.path().join("source.rs")),
        )
        .unwrap();

    assert!(diff.is_empty(), "{diff}");
}

/// Staging is what makes gutter marks go away, so the base has to move with it.
#[test]
fn staging_moves_the_base_and_unstaging_moves_it_back() {
    let repository = TempRepository::new("stage");
    repository.write("source.rs", "committed\n");
    repository.commit("base");
    repository.write("source.rs", "working\n");
    let provider = provider();
    let path = repository.path().join("source.rs");
    let staged = || {
        provider
            .staged_content(&repository.repository(), &path)
            .unwrap()
    };

    assert_eq!(staged(), BaseContent::Text("committed\n".to_owned()));

    provider.stage(&repository.repository(), &path).unwrap();
    assert_eq!(staged(), BaseContent::Text("working\n".to_owned()));

    provider.unstage(&repository.repository(), &path).unwrap();
    assert_eq!(staged(), BaseContent::Text("committed\n".to_owned()));
}

/// Before the first commit `HEAD` resolves to nothing, which is where
/// `restore --staged` would fail; unstaging has to drop the entry instead.
#[test]
fn staging_and_unstaging_work_before_the_first_commit() {
    let repository = TempRepository::new("stage-unborn");
    repository.write("first.rs", "new\n");
    let provider = provider();
    let path = repository.path().join("first.rs");

    provider.stage(&repository.repository(), &path).unwrap();
    assert_eq!(
        provider
            .staged_content(&repository.repository(), &path)
            .unwrap(),
        BaseContent::Text("new\n".to_owned())
    );
    let status = provider.status(&repository.repository()).unwrap();
    assert_eq!(status.counts().added, 1);

    provider.unstage(&repository.repository(), &path).unwrap();

    assert_eq!(
        provider
            .staged_content(&repository.repository(), &path)
            .unwrap(),
        BaseContent::Absent
    );
    assert_eq!(
        provider
            .status(&repository.repository())
            .unwrap()
            .counts()
            .untracked,
        1
    );
}

#[test]
fn staging_records_a_deletion_as_a_deletion() {
    let repository = TempRepository::new("stage-delete");
    repository.write("gone.rs", "a\n");
    repository.commit("base");
    fs::remove_file(repository.path().join("gone.rs")).unwrap();
    let provider = provider();

    provider
        .stage(&repository.repository(), &repository.path().join("gone.rs"))
        .unwrap();

    let status = provider.status(&repository.repository()).unwrap();
    assert_eq!(status.files[0].index, FileState::Deleted);
    assert_eq!(status.counts().deleted, 1);
}

#[test]
fn staging_a_path_outside_the_repository_is_refused() {
    let repository = TempRepository::new("stage-outside");
    repository.write("source.rs", "a\n");
    repository.commit("base");

    let error = provider()
        .stage(&repository.repository(), Path::new("/elsewhere/source.rs"))
        .unwrap_err();

    assert!(matches!(error, GitError::NotARepository { .. }), "{error}");
}

#[test]
fn committing_records_the_index_and_nothing_else() {
    let repository = TempRepository::new("commit");
    repository.write("staged.rs", "one\n");
    repository.write("untouched.rs", "one\n");
    repository.commit("base");
    repository.write("staged.rs", "two\n");
    repository.write("unstaged.rs", "new\n");
    repository.git(&["add", "staged.rs"]);
    let provider = provider();

    let summary = provider
        .commit(
            &repository.repository(),
            "Subject line\n\nA body paragraph.",
        )
        .unwrap();

    assert!(summary.contains("Subject line"), "{summary}");
    let status = provider.status(&repository.repository()).unwrap();
    assert_eq!(
        status.counts().untracked,
        1,
        "the untracked file stayed out"
    );
    assert!(!status.files.iter().any(|file| file.is_staged()));

    // The whole message, body included, reached the commit.
    let logged = Command::new("git")
        .args(["log", "-1", "--format=%B"])
        .current_dir(repository.path())
        .output()
        .unwrap();
    let logged = String::from_utf8_lossy(&logged.stdout);
    assert!(logged.contains("Subject line"), "{logged}");
    assert!(logged.contains("A body paragraph."), "{logged}");
}

#[test]
fn commit_search_returns_full_messages_in_newest_first_order() {
    let repository = TempRepository::new("commit-search");
    repository.write("a.rs", "one\n");
    repository.commit("Initial subject");
    repository.write("a.rs", "two\n");
    repository.commit("Visible subject\n\nA distinctive body phrase.");

    let result = provider().search_commits(&repository.repository()).unwrap();

    assert!(!result.limited);
    assert_eq!(result.commits.len(), 2);
    assert_eq!(result.commits[0].summary.subject, "Visible subject");
    assert!(
        result.commits[0]
            .message
            .contains("A distinctive body phrase."),
        "{:?}",
        result.commits[0]
    );
}

/// A message is one argument vector element, so nothing in it is syntax.
#[test]
fn a_message_full_of_shell_characters_is_recorded_verbatim() {
    let repository = TempRepository::new("commit-quoting");
    repository.write("a.rs", "one\n");
    repository.git(&["add", "-A"]);
    let awkward = "Fix $(touch pwned) && `rm -rf /` -- \"quoted\" 'single'";

    provider()
        .commit(&repository.repository(), awkward)
        .unwrap();

    let logged = Command::new("git")
        .args(["log", "-1", "--format=%s"])
        .current_dir(repository.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&logged.stdout).trim(), awkward);
    assert!(!repository.path().join("pwned").exists());
}

#[test]
fn committing_with_nothing_staged_is_refused_by_git() {
    let repository = TempRepository::new("commit-empty");
    repository.write("a.rs", "one\n");
    repository.commit("base");

    let error = provider()
        .commit(&repository.repository(), "nothing to record")
        .unwrap_err();

    assert!(matches!(error, GitError::Failed { .. }), "{error}");
}

/// Discarding takes both sides: what was staged and what was not.
#[test]
fn discarding_restores_a_path_to_head() {
    let repository = TempRepository::new("discard");
    repository.write("both.rs", "committed\n");
    repository.commit("base");
    repository.write("both.rs", "staged\n");
    repository.git(&["add", "both.rs"]);
    repository.write("both.rs", "working\n");
    let provider = provider();
    let path = repository.path().join("both.rs");

    provider.discard(&repository.repository(), &path).unwrap();

    assert_eq!(fs::read_to_string(&path).unwrap(), "committed\n");
    assert_eq!(
        provider
            .staged_content(&repository.repository(), &path)
            .unwrap(),
        BaseContent::Text("committed\n".to_owned())
    );
    assert!(
        provider
            .status(&repository.repository())
            .unwrap()
            .counts()
            .is_empty(),
        "the path should be clean on both sides"
    );
}

/// Only the named path, however awkward its name.
#[test]
fn discarding_leaves_every_other_path_alone() {
    let repository = TempRepository::new("discard-scope");
    let awkward = "a dir/--not-an-option.rs";
    repository.write(awkward, "committed\n");
    repository.write("neighbour.rs", "committed\n");
    repository.commit("base");
    repository.write(awkward, "changed\n");
    repository.write("neighbour.rs", "changed\n");

    provider()
        .discard(&repository.repository(), &repository.path().join(awkward))
        .unwrap();

    assert_eq!(
        fs::read_to_string(repository.path().join(awkward)).unwrap(),
        "committed\n"
    );
    assert_eq!(
        fs::read_to_string(repository.path().join("neighbour.rs")).unwrap(),
        "changed\n",
        "a neighbouring change was thrown away too"
    );
}

#[test]
fn discarding_an_untracked_path_is_refused_by_git() {
    let repository = TempRepository::new("discard-untracked");
    repository.write("a.rs", "one\n");
    repository.commit("base");
    repository.write("stray.rs", "never tracked\n");

    let error = provider()
        .discard(
            &repository.repository(),
            &repository.path().join("stray.rs"),
        )
        .unwrap_err();

    assert!(matches!(error, GitError::Failed { .. }), "{error}");
    assert!(
        repository.path().join("stray.rs").exists(),
        "an untracked file was removed"
    );
}

#[test]
fn history_pages_continue_by_object_identity_and_details_are_bounded_values() {
    let repository = TempRepository::new("history-pages");
    for (index, subject) in ["first", "second λ", "third\tfield"]
        .into_iter()
        .enumerate()
    {
        repository.write("history.txt", &format!("{index}\n"));
        repository.git(&["add", "history.txt"]);
        repository.git(&["commit", "--quiet", "-m", subject]);
    }
    let provider = provider();
    let first = provider
        .log_page(
            &repository.repository(),
            &LogRequest {
                cursor: None,
                limit: 2,
            },
        )
        .unwrap();
    assert_eq!(first.commits.len(), 2);
    assert_eq!(first.total_pages, 2);
    assert_eq!(first.commits[0].subject, "third\tfield");
    let cursor = first.next.expect("large history did not expose a cursor");
    assert_eq!(cursor.boundary, first.commits[1].oid);
    repository.write("history.txt", "new head\n");
    repository.git(&["add", "history.txt"]);
    repository.git(&["commit", "--quiet", "-m", "new head after cursor"]);
    let second = provider
        .log_page(
            &repository.repository(),
            &LogRequest {
                cursor: Some(cursor),
                limit: 2,
            },
        )
        .unwrap();
    assert_eq!(second.commits.len(), 1);
    assert_eq!(second.total_pages, 2);
    assert_eq!(second.commits[0].subject, "first");
    assert!(
        first
            .commits
            .iter()
            .all(|commit| !second.commits.iter().any(|later| later.oid == commit.oid))
    );

    let detail = provider
        .commit_detail(&repository.repository(), &first.commits[0].oid)
        .unwrap();
    assert_eq!(detail.summary.oid, first.commits[0].oid);
    assert!(detail.body.contains("third\tfield"));
    assert!(detail.patch.contains("history.txt"));
    assert!(matches!(
        provider.log_page(
            &repository.repository(),
            &LogRequest {
                cursor: None,
                limit: 10_001,
            },
        ),
        Err(GitError::Failed { .. })
    ));
    assert!(matches!(
        GitCliProvider::new("git")
            .with_max_output_bytes(16)
            .commit_detail(&repository.repository(), &first.commits[0].oid),
        Err(GitError::TooLarge { .. })
    ));
}

#[test]
fn commit_detail_reads_a_patch_past_the_default_output_bound() {
    let repository = TempRepository::new("large-commit-detail");
    // One line per byte of margin over `DEFAULT_MAX_OUTPUT_BYTES` (16 MiB):
    // large enough that the unified diff Git prints for it exceeds that
    // bound, small enough to stay well under the patch-specific bound this
    // call is given.
    let mut large_file = String::with_capacity(20 * 1024 * 1024);
    for line in 0..1_000_000 {
        large_file.push_str(&format!("line {line} of a very large tracked file\n"));
    }
    repository.write("large.txt", &large_file);
    repository.git(&["add", "large.txt"]);
    repository.git(&["commit", "--quiet", "-m", "add a large file"]);

    let provider = provider();
    let head = provider
        .log_page(
            &repository.repository(),
            &LogRequest {
                cursor: None,
                limit: 1,
            },
        )
        .unwrap();
    let detail = provider
        .commit_detail(&repository.repository(), &head.commits[0].oid)
        .unwrap();
    assert!(detail.patch.len() > 16 * 1024 * 1024);
    assert!(detail.patch.contains("large.txt"));
}

#[test]
fn commit_detail_honors_a_patch_limit_lowered_below_the_default() {
    let repository = TempRepository::new("lowered-commit-detail-bound");
    let mut file = String::new();
    for line in 0..200 {
        file.push_str(&format!("line {line} of a moderately sized file\n"));
    }
    repository.write("moderate.txt", &file);
    repository.git(&["add", "moderate.txt"]);
    repository.git(&["commit", "--quiet", "-m", "add a moderate file"]);

    let head = provider()
        .log_page(
            &repository.repository(),
            &LogRequest {
                cursor: None,
                limit: 1,
            },
        )
        .unwrap();
    let error = GitCliProvider::new("git")
        .with_max_output_bytes(512)
        .commit_detail(&repository.repository(), &head.commits[0].oid)
        .unwrap_err();
    assert!(
        matches!(error, GitError::TooLarge { limit: 512, .. }),
        "an explicitly lowered bound must not be raised back up for the patch: {error}"
    );
}

#[test]
fn blame_uses_live_unsaved_text_and_marks_new_lines_uncommitted() {
    let repository = TempRepository::new("live-blame");
    repository.write("note.txt", "one\ntwo\nthree\n");
    repository.commit("base");
    let committed = provider()
        .blame(
            &repository.repository(),
            &BlameRequest {
                path: repository.path().join("note.txt"),
                content: "one\ntwo\nthree\n".to_owned(),
                lines: None,
            },
        )
        .unwrap();
    assert_eq!(committed.len(), 3);
    assert!(committed.iter().all(|line| line.oid.is_some()));
    let request = BlameRequest {
        path: repository.path().join("note.txt"),
        content: "one\nunsaved λ\nthree\n".to_owned(),
        lines: None,
    };
    let lines = provider()
        .blame(&repository.repository(), &request)
        .unwrap();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].oid.is_some());
    assert_eq!(lines[1].oid, None);
    assert_eq!(lines[1].text, "unsaved λ");
    let current = provider()
        .blame(
            &repository.repository(),
            &BlameRequest {
                path: repository.path().join("note.txt"),
                content: "one\nunsaved λ\nthree\n".to_owned(),
                lines: Some((2, 2)),
            },
        )
        .unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].source_line, 2);
    assert_eq!(current[0].oid, None);
}

#[test]
fn blame_follows_an_uncommitted_rename_and_refuses_untracked_or_oversized_input() {
    let repository = TempRepository::new("blame-edges");
    repository.write("old name.txt", "tracked\n");
    repository.commit("base");
    repository.git(&["mv", "old name.txt", "new name.txt"]);
    let provider = provider();
    let renamed = provider
        .blame(
            &repository.repository(),
            &BlameRequest {
                path: repository.path().join("new name.txt"),
                content: "tracked\n".to_owned(),
                lines: None,
            },
        )
        .unwrap();
    assert_eq!(renamed.len(), 1);
    assert!(renamed[0].oid.is_some());

    repository.write("untracked.txt", "new\n");
    let untracked = provider.blame(
        &repository.repository(),
        &BlameRequest {
            path: repository.path().join("untracked.txt"),
            content: "new\n".to_owned(),
            lines: None,
        },
    );
    assert!(matches!(untracked, Err(GitError::Failed { .. })));

    let oversized = provider.blame(
        &repository.repository(),
        &BlameRequest {
            path: repository.path().join("new name.txt"),
            content: "x".repeat(MAX_BLAME_INPUT_BYTES + 1),
            lines: None,
        },
    );
    assert!(matches!(oversized, Err(GitError::TooLarge { .. })));

    let binary = provider.blame(
        &repository.repository(),
        &BlameRequest {
            path: repository.path().join("new name.txt"),
            content: "tracked\0binary\n".to_owned(),
            lines: None,
        },
    );
    assert!(matches!(binary, Err(GitError::Failed { .. })));
}

#[test]
fn log_preserves_both_parents_of_a_merge_commit() {
    let repository = TempRepository::new("history-merge");
    repository.write("base.txt", "base\n");
    repository.commit("base");
    repository.git(&["checkout", "-q", "-b", "side"]);
    repository.write("side.txt", "side\n");
    repository.commit("side");
    repository.git(&["checkout", "-q", "master"]);
    repository.write("main.txt", "main\n");
    repository.commit("main");
    repository.git(&["merge", "--quiet", "--no-ff", "-m", "merge", "side"]);
    let page = provider()
        .log_page(&repository.repository(), &LogRequest::default())
        .unwrap();
    assert_eq!(page.commits[0].subject, "merge");
    assert_eq!(page.commits[0].parents.len(), 2);
}

#[cfg(unix)]
#[test]
fn blame_keeps_a_non_utf8_path_addressable() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let repository = TempRepository::new("blame-non-utf8");
    let name = OsString::from_vec(b"odd-\xff.txt".to_vec());
    let path = repository.path().join(&name);
    fs::write(&path, "encoded path\n").unwrap();
    assert!(
        Command::new("git")
            .arg("add")
            .arg("--")
            .arg(&name)
            .current_dir(repository.path())
            .status()
            .unwrap()
            .success()
    );
    repository.git(&["commit", "--quiet", "-m", "encoded"]);
    let lines = provider()
        .blame(
            &repository.repository(),
            &BlameRequest {
                path,
                content: "encoded path\n".to_owned(),
                lines: None,
            },
        )
        .unwrap();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].oid.is_some());
}

#[test]
fn blame_can_attribute_live_text_for_a_path_deleted_from_disk() {
    let repository = TempRepository::new("blame-deleted-path");
    repository.write("deleted.txt", "committed\n");
    repository.commit("base");
    fs::remove_file(repository.path().join("deleted.txt")).unwrap();
    let lines = provider()
        .blame(
            &repository.repository(),
            &BlameRequest {
                path: repository.path().join("deleted.txt"),
                content: "committed\nlive after deletion\n".to_owned(),
                lines: None,
            },
        )
        .unwrap();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].oid.is_some());
    assert_eq!(lines[1].oid, None);
}

#[test]
fn named_stash_scopes_are_explicit_and_apply_keeps_the_entry_until_confirmed_drop() {
    let repository = TempRepository::new("stash-scopes");
    repository.write("tracked.txt", "base\n");
    repository.commit("base");
    repository.write("tracked.txt", "staged\n");
    repository.git(&["add", "tracked.txt"]);
    repository.write("tracked.txt", "unstaged\n");
    repository.write("untracked.txt", "new\n");
    let provider = provider();
    provider
        .mutate_stash(
            &repository.repository(),
            &StashMutation::Create {
                name: "worktree only".to_owned(),
                scope: StashScope::TrackedWorktree,
            },
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(repository.path().join("tracked.txt")).unwrap(),
        "staged\n"
    );
    assert_eq!(
        git_output(&repository, &["show", ":tracked.txt"]),
        "staged\n"
    );
    assert!(repository.path().join("untracked.txt").exists());
    let stash = provider
        .stashes(&repository.repository())
        .unwrap()
        .remove(0);
    repository.git(&["reset", "--hard", "-q", "HEAD"]);
    provider
        .mutate_stash(
            &repository.repository(),
            &StashMutation::Apply {
                oid: stash.oid.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        provider.stashes(&repository.repository()).unwrap()[0].oid,
        stash.oid
    );
    provider
        .mutate_stash(
            &repository.repository(),
            &StashMutation::Drop {
                oid: stash.oid.clone(),
            },
        )
        .unwrap();
    assert!(
        provider
            .stashes(&repository.repository())
            .unwrap()
            .is_empty()
    );
    let stale = provider
        .mutate_stash(
            &repository.repository(),
            &StashMutation::Apply { oid: stash.oid },
        )
        .unwrap_err()
        .to_string();
    assert!(stale.contains("no longer exists"), "{stale}");
}

#[test]
fn stash_apply_conflict_retains_the_recovery_object() {
    let repository = TempRepository::new("stash-conflict");
    repository.write("source.txt", "base\n");
    repository.commit("base");
    repository.write("source.txt", "stashed\n");
    let provider = provider();
    provider
        .mutate_stash(
            &repository.repository(),
            &StashMutation::Create {
                name: "conflict".to_owned(),
                scope: StashScope::TrackedWorktreeAndIndex,
            },
        )
        .unwrap();
    let stash = provider.stashes(&repository.repository()).unwrap()[0].clone();
    repository.write("source.txt", "other\n");
    repository.commit("other");
    let error = provider
        .mutate_stash(
            &repository.repository(),
            &StashMutation::Apply {
                oid: stash.oid.clone(),
            },
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("stash was retained"), "{error}");
    assert!(error.contains("external Git tool"), "{error}");
    assert_eq!(
        provider.stashes(&repository.repository()).unwrap()[0].oid,
        stash.oid
    );
}

#[test]
fn stash_all_and_untracked_scopes_do_not_choose_for_the_user() {
    let repository = TempRepository::new("stash-explicit-broad-scopes");
    repository.write("tracked.txt", "base\n");
    repository.commit("base");
    let provider = provider();

    repository.write("tracked.txt", "staged\n");
    repository.git(&["add", "tracked.txt"]);
    repository.write("tracked.txt", "worktree\n");
    repository.write("untracked.txt", "outside\n");
    provider
        .mutate_stash(
            &repository.repository(),
            &StashMutation::Create {
                name: "tracked index and worktree".to_owned(),
                scope: StashScope::TrackedWorktreeAndIndex,
            },
        )
        .unwrap();
    assert_eq!(git_output(&repository, &["show", ":tracked.txt"]), "base\n");
    assert_eq!(
        fs::read_to_string(repository.path().join("tracked.txt")).unwrap(),
        "base\n"
    );
    assert!(repository.path().join("untracked.txt").exists());

    repository.write("tracked.txt", "everything\n");
    provider
        .mutate_stash(
            &repository.repository(),
            &StashMutation::Create {
                name: "including untracked".to_owned(),
                scope: StashScope::TrackedAndUntracked,
            },
        )
        .unwrap();
    assert!(!repository.path().join("untracked.txt").exists());
    let names = provider
        .stashes(&repository.repository())
        .unwrap()
        .into_iter()
        .map(|entry| entry.subject)
        .collect::<Vec<_>>();
    assert!(
        names
            .iter()
            .any(|name| name.contains("including untracked"))
    );
    assert!(
        names
            .iter()
            .any(|name| name.contains("tracked index and worktree"))
    );
}

#[test]
fn exact_hunk_stage_unstage_and_selected_line_safe_slice_preserve_index_contents() {
    let repository = TempRepository::new("partial-stage");
    repository.write(
        "source.txt",
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    repository.commit("base");
    repository.write(
        "source.txt",
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
    );
    let provider = provider();
    let path = repository.path().join("source.txt");
    let diff = provider
        .diff(&repository.repository(), DiffScope::Unstaged, Some(&path))
        .unwrap();
    let hunks = runyte::git::parse_hunks(diff.as_bytes()).unwrap();
    assert_eq!(hunks.len(), 2);
    let request = provider
        .prepare_partial(
            &repository.repository(),
            &PartialStageSelection {
                path: path.clone(),
                scope: DiffScope::Unstaged,
                buffer: None,
                guard: None,
                hunk: Some(hunks[0].identity.clone()),
                lines: None,
            },
        )
        .unwrap();
    provider
        .apply_partial(&repository.repository(), &request)
        .unwrap();
    assert_eq!(
        git_output(&repository, &["show", ":source.txt"]),
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n"
    );

    let staged = provider
        .diff(&repository.repository(), DiffScope::Staged, Some(&path))
        .unwrap();
    let staged_hunk = runyte::git::parse_hunks(staged.as_bytes())
        .unwrap()
        .remove(0);
    let unstage = provider
        .prepare_partial(
            &repository.repository(),
            &PartialStageSelection {
                path: path.clone(),
                scope: DiffScope::Staged,
                buffer: None,
                guard: None,
                hunk: Some(staged_hunk.identity),
                lines: None,
            },
        )
        .unwrap();
    provider
        .apply_partial(&repository.repository(), &unstage)
        .unwrap();
    assert_eq!(
        git_output(&repository, &["show", ":source.txt"]),
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n"
    );

    let selected = provider
        .prepare_partial(&repository.repository(), &line_selection(path, (10, 10)))
        .unwrap();
    provider
        .apply_partial(&repository.repository(), &selected)
        .unwrap();
    assert_eq!(
        git_output(&repository, &["show", ":source.txt"]),
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n"
    );
}

#[test]
fn partial_stage_prepare_and_apply_run_through_the_ordered_async_service() {
    let repository = TempRepository::new("partial-async-service");
    repository.write("source.txt", "one\ntwo\n");
    repository.commit("base");
    repository.write("source.txt", "ONE\ntwo\n");
    let path = repository.path().join("source.txt");
    let (service, mut events) = GitService::spawn(provider());
    let id = service
        .try_submit(GitOperation::PreparePartial {
            repository: repository.repository(),
            selection: Box::new(line_selection(path, (1, 1))),
        })
        .unwrap();
    let prepared = loop {
        match events.blocking_recv().expect("Git service stopped") {
            GitServiceEvent::Completed {
                id: completed,
                result,
                ..
            } if completed == id => match *result {
                Ok(GitResponse::PreparedPartial(request)) => break request,
                result => panic!("unexpected prepare result: {result:?}"),
            },
            _ => {}
        }
    };
    let id = service
        .try_submit(GitOperation::Mutate {
            repository: repository.repository(),
            mutation: GitMutation::PartialStage(prepared),
            refresh: RefreshSpec::default(),
        })
        .unwrap();
    loop {
        match events.blocking_recv().expect("Git service stopped") {
            GitServiceEvent::Completed {
                id: completed,
                result,
                ..
            } if completed == id => match *result {
                Ok(GitResponse::Mutation { failure: None, .. }) => break,
                result => panic!("unexpected mutation result: {result:?}"),
            },
            _ => {}
        }
    }
    assert_eq!(
        git_output(&repository, &["show", ":source.txt"]),
        "ONE\ntwo\n"
    );
}

#[test]
fn stale_partial_requests_and_unsupported_shapes_change_nothing() {
    let repository = TempRepository::new("partial-stale");
    repository.write("source.txt", "one\ntwo\n");
    repository.commit("base");
    repository.write("source.txt", "ONE\ntwo\n");
    let provider = provider();
    let path = repository.path().join("source.txt");
    let selection = line_selection(path.clone(), (1, 1));
    let request = provider
        .prepare_partial(&repository.repository(), &selection)
        .unwrap();
    repository.write("source.txt", "external\ntwo\n");
    assert!(
        provider
            .apply_partial(&repository.repository(), &request)
            .is_err()
    );
    assert_eq!(
        git_output(&repository, &["show", ":source.txt"]),
        "one\ntwo\n"
    );

    repository.write("source.txt", "ONE\ntwo\n");
    let request = provider
        .prepare_partial(&repository.repository(), &selection)
        .unwrap();
    repository.write("other.txt", "index change\n");
    repository.git(&["add", "other.txt"]);
    assert!(
        provider
            .apply_partial(&repository.repository(), &request)
            .is_err()
    );
    assert_eq!(
        git_output(&repository, &["show", ":source.txt"]),
        "one\ntwo\n"
    );

    repository.git(&["reset", "-q"]);
    repository.git(&["mv", "source.txt", "renamed.txt"]);
    let renamed = provider.prepare_partial(
        &repository.repository(),
        &line_selection(repository.path().join("renamed.txt"), (1, 1)),
    );
    assert!(renamed.is_err());
}

#[test]
fn head_fingerprint_makes_partial_requests_terminally_stale() {
    let repository = TempRepository::new("partial-buffer-head-stale");
    repository.write("source.txt", "one\ntwo\n");
    repository.commit("base");
    repository.write("source.txt", "ONE\ntwo\n");
    let provider = provider();
    let path = repository.path().join("source.txt");
    let head_stale = provider
        .prepare_partial(&repository.repository(), &line_selection(path, (1, 1)))
        .unwrap();
    repository.git(&["commit", "--allow-empty", "-qm", "head moved"]);
    assert!(
        provider
            .apply_partial(&repository.repository(), &head_stale)
            .is_err()
    );
    assert_eq!(
        git_output(&repository, &["show", ":source.txt"]),
        "one\ntwo\n"
    );
}

#[test]
fn partial_patch_identity_and_single_hunk_are_enforced_at_apply() {
    let repository = TempRepository::new("partial-request-integrity");
    repository.write(
        "source.txt",
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    repository.commit("base");
    repository.write(
        "source.txt",
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
    );
    let provider = provider();
    let path = repository.path().join("source.txt");
    let hunks = runyte::git::parse_hunks(
        provider
            .diff(&repository.repository(), DiffScope::Unstaged, Some(&path))
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let mut request = provider
        .prepare_partial(
            &repository.repository(),
            &PartialStageSelection {
                path,
                scope: DiffScope::Unstaged,
                buffer: None,
                guard: None,
                hunk: Some(hunks[0].identity.clone()),
                lines: None,
            },
        )
        .unwrap();
    request.hunk = "f".repeat(64);
    assert!(
        provider
            .apply_partial(&repository.repository(), &request)
            .is_err()
    );
    assert_eq!(
        git_output(&repository, &["show", ":source.txt"]),
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n"
    );
}

#[test]
fn selected_line_additions_with_context_work_and_crossing_a_deletion_is_refused() {
    let repository = TempRepository::new("partial-addition-deletion-range");
    repository.write(
        "source.txt",
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    repository.commit("base");
    repository.write(
        "source.txt",
        "one\nADDED\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\n",
    );
    let provider = provider();
    let path = repository.path().join("source.txt");
    let refused = provider.prepare_partial(
        &repository.repository(),
        &line_selection(path.clone(), (1, 10)),
    );
    assert!(refused.is_err());
    assert_eq!(
        git_output(&repository, &["show", ":source.txt"]),
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n"
    );

    // Limit the selection to context plus the pure addition and apply only
    // that exact hunk; the distant deletion remains outside the index.
    let request = provider
        .prepare_partial(&repository.repository(), &line_selection(path, (1, 3)))
        .unwrap();
    provider
        .apply_partial(&repository.repository(), &request)
        .unwrap();
    assert_eq!(
        git_output(&repository, &["show", ":source.txt"]),
        "one\nADDED\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n"
    );
}

#[test]
fn selected_replacement_does_not_carry_an_unselected_deletion_in_its_hunk() {
    let repository = TempRepository::new("partial-independent-deletion");
    repository.write("source.txt", "old-a\nkeep\ndelete-me\nkeep\n");
    repository.commit("base");
    repository.write("source.txt", "new-a\nkeep\nkeep\n");
    let result = provider().prepare_partial(
        &repository.repository(),
        &line_selection(repository.path().join("source.txt"), (1, 1)),
    );
    assert!(result.is_err());
    assert_eq!(
        git_output(&repository, &["show", ":source.txt"]),
        "old-a\nkeep\ndelete-me\nkeep\n"
    );
}

#[cfg(unix)]
#[test]
fn partial_staging_refuses_file_mode_metadata() {
    use std::os::unix::fs::PermissionsExt;

    let repository = TempRepository::new("partial-mode");
    repository.write("source.sh", "old\n");
    repository.commit("base");
    repository.write("source.sh", "new\n");
    fs::set_permissions(
        repository.path().join("source.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let result = provider().prepare_partial(
        &repository.repository(),
        &line_selection(repository.path().join("source.sh"), (1, 1)),
    );
    assert!(result.is_err());
    assert_eq!(git_output(&repository, &["show", ":source.sh"]), "old\n");
}

#[cfg(unix)]
#[test]
fn patch_prepare_refuses_when_disk_changes_during_diff_capture() {
    use std::os::unix::fs::PermissionsExt;

    let repository = TempRepository::new("partial-capture-race");
    repository.write("source.txt", "old\n");
    repository.commit("base");
    repository.write("source.txt", "first\n");
    let real_git =
        String::from_utf8(Command::new("which").arg("git").output().unwrap().stdout).unwrap();
    let real_git = real_git.trim();
    let wrapper = repository.path().join("git-racing-diff");
    let source = repository.path().join("source.txt");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nfor arg in \"$@\"; do\n  if [ \"$arg\" = diff ]; then\n    '{real_git}' \"$@\"\n    printf 'second\\n' > '{}'\n    exit 0\n  fi\ndone\nexec '{real_git}' \"$@\"\n",
            source.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();

    let result = GitCliProvider::new(wrapper)
        .prepare_partial(&repository.repository(), &line_selection(source, (1, 1)));

    assert!(result.is_err());
    assert_eq!(git_output(&repository, &["show", ":source.txt"]), "old\n");
}

#[test]
fn no_newline_addition_is_exact_and_deletion_conflict_and_binary_are_refused() {
    let repository = TempRepository::new("partial-shapes");
    repository.write("source.txt", "old");
    repository.write("delete.txt", "remove\n");
    fs::write(repository.path().join("binary.dat"), [0_u8, 1, 2, 3]).unwrap();
    repository.commit("base");
    repository.write("source.txt", "new");
    fs::remove_file(repository.path().join("delete.txt")).unwrap();
    fs::write(repository.path().join("binary.dat"), [0_u8, 9, 2, 3]).unwrap();
    let provider = provider();
    let source = repository.path().join("source.txt");
    let diff = provider
        .diff(&repository.repository(), DiffScope::Unstaged, Some(&source))
        .unwrap();
    let hunk = runyte::git::parse_hunks(diff.as_bytes()).unwrap().remove(0);
    assert!(diff.contains("No newline at end of file"));
    assert_eq!(
        runyte::git::parse_hunks(diff.as_bytes()).unwrap()[0].identity,
        hunk.identity,
        "unchanged diff bytes keep one hunk identity across refresh"
    );
    let request = provider
        .prepare_partial(
            &repository.repository(),
            &PartialStageSelection {
                path: source,
                scope: DiffScope::Unstaged,
                buffer: None,
                guard: None,
                hunk: Some(hunk.identity),
                lines: None,
            },
        )
        .unwrap();
    provider
        .apply_partial(&repository.repository(), &request)
        .unwrap();
    assert_eq!(git_output(&repository, &["show", ":source.txt"]), "new");
    assert_eq!(
        git_output(&repository, &["show", ":delete.txt"]),
        "remove\n"
    );

    for path in ["delete.txt", "binary.dat"] {
        let result = provider.prepare_partial(
            &repository.repository(),
            &line_selection(repository.path().join(path), (1, 1)),
        );
        assert!(result.is_err(), "{path} unexpectedly became stageable");
    }

    repository.git(&["reset", "--hard", "-q", "HEAD"]);
    repository.git(&["checkout", "-q", "-b", "side"]);
    repository.write("source.txt", "side\n");
    repository.commit("side");
    repository.git(&["checkout", "-q", "master"]);
    repository.write("source.txt", "main\n");
    repository.commit("main");
    let output = Command::new("git")
        .args(["merge", "side"])
        .current_dir(repository.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let conflict = provider
        .prepare_partial(
            &repository.repository(),
            &line_selection(repository.path().join("source.txt"), (1, 1)),
        )
        .unwrap_err()
        .to_string();
    assert!(conflict.contains("conflicts"), "{conflict}");
}
