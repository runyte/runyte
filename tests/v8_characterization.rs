// SPDX-License-Identifier: MPL-2.0

use std::{
    path::Path,
    sync::{Arc, Condvar, Mutex, mpsc},
    time::Duration,
};

use runyte::{
    git::{
        BaseContent, Branch, DiffScope, Divergence, FileComparison, GitProvider, Head, Repository,
        RepositoryStatus,
    },
    headless::HeadlessEditor,
    text::Transaction,
};

/// A deliberately thread-safe fake kept separate from `MemoryGitProvider`.
///
/// The editor fake models synchronous provider semantics with `Rc`/`RefCell`
/// and must not acquire an unsafe or dishonest `Send` implementation merely
/// so V8 can test service scheduling.
#[derive(Clone)]
struct HeldGitProvider {
    started: mpsc::Sender<()>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl HeldGitProvider {
    fn wait_until_released(&self) {
        self.started.send(()).unwrap();
        let (released, wake) = &*self.release;
        let mut released = released.lock().unwrap();
        while !*released {
            released = wake.wait(released).unwrap();
        }
    }
}

impl GitProvider for HeldGitProvider {
    fn discover(&self, _start: &Path) -> runyte::git::Result<Option<Repository>> {
        unreachable!("the characterization holds a status request")
    }

    fn status(&self, _repository: &Repository) -> runyte::git::Result<RepositoryStatus> {
        self.wait_until_released();
        Ok(RepositoryStatus {
            head: Head::Branch("main".to_owned()),
            upstream: None,
            divergence: Divergence::default(),
            files: Vec::new(),
        })
    }

    fn branches(&self, _repository: &Repository) -> runyte::git::Result<Vec<Branch>> {
        unreachable!("the characterization holds a status request")
    }

    fn checkout_branch(&self, _repository: &Repository, _branch: &str) -> runyte::git::Result<()> {
        unreachable!("the characterization holds a status request")
    }

    fn create_branch(
        &self,
        _repository: &Repository,
        _branch: &str,
        _start_point: &str,
    ) -> runyte::git::Result<()> {
        unreachable!("the characterization holds a status request")
    }

    fn delete_branch(
        &self,
        _repository: &Repository,
        _branch: &str,
        _force: bool,
    ) -> runyte::git::Result<()> {
        unreachable!("the characterization holds a status request")
    }

    fn staged_content(
        &self,
        _repository: &Repository,
        _path: &Path,
    ) -> runyte::git::Result<BaseContent> {
        unreachable!("the characterization holds a status request")
    }

    fn diff(
        &self,
        _repository: &Repository,
        _scope: DiffScope,
        _path: Option<&Path>,
    ) -> runyte::git::Result<String> {
        unreachable!("the characterization holds a status request")
    }

    fn file_comparison(
        &self,
        _repository: &Repository,
        _scope: DiffScope,
        _path: &Path,
    ) -> runyte::git::Result<FileComparison> {
        unreachable!("the characterization holds a status request")
    }

    fn stage(&self, _repository: &Repository, _path: &Path) -> runyte::git::Result<()> {
        unreachable!("the characterization holds a status request")
    }

    fn unstage(&self, _repository: &Repository, _path: &Path) -> runyte::git::Result<()> {
        unreachable!("the characterization holds a status request")
    }

    fn discard(&self, _repository: &Repository, _path: &Path) -> runyte::git::Result<()> {
        unreachable!("the characterization holds a status request")
    }

    fn pull(&self, _repository: &Repository) -> runyte::git::Result<String> {
        unreachable!("the characterization holds a status request")
    }

    fn rebase_onto_upstream(&self, _repository: &Repository) -> runyte::git::Result<String> {
        unreachable!("the characterization holds a status request")
    }

    fn push(&self, _repository: &Repository, _branch: &str) -> runyte::git::Result<String> {
        unreachable!("the characterization holds a status request")
    }

    fn commit(&self, _repository: &Repository, _message: &str) -> runyte::git::Result<String> {
        unreachable!("the characterization holds a status request")
    }
}

#[test]
fn held_git_work_leaves_headless_input_and_snapshot_progress_available() {
    let (started_tx, started_rx) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let provider = HeldGitProvider {
        started: started_tx,
        release: release.clone(),
    };
    let worker = std::thread::spawn(move || provider.status(&Repository::new("/project")).unwrap());
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the fake Git request did not start");

    let mut editor = HeadlessEditor::with_text_in(std::env::temp_dir(), "abc").unwrap();
    assert!(
        editor
            .apply_transaction(Transaction::insert(3, "d"))
            .unwrap()
    );
    assert_eq!(editor.active_text(), "abcd");
    let first = editor.snapshot(80, 24);
    let second = editor.snapshot(80, 24);
    assert_eq!(
        first, second,
        "snapshot generation must remain deterministic"
    );

    let (released, wake) = &*release;
    *released.lock().unwrap() = true;
    wake.notify_all();
    let status = worker.join().unwrap();
    assert_eq!(status.head, Head::Branch("main".to_owned()));
}
