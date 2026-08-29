// SPDX-License-Identifier: MPL-2.0

//! Bounded, debounced filesystem invalidation for ambient Git state.
//!
//! Native events are only hints that a repository snapshot may be stale. The
//! host decides whether a visible consumer needs a refresh, and the Git
//! service remains the only code that interprets repository state.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::git::Repository;

const COMMAND_CAPACITY: usize = 64;
const EVENT_CAPACITY: usize = 8;
const DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitInvalidation {
    pub repository: PathBuf,
    /// Latest native observation represented by this debounced event.
    pub observed_at: Instant,
    /// A watcher error or bounded-queue overflow loses path precision. The
    /// host treats both forms as the same full repository invalidation.
    pub overflowed: bool,
}

enum WorkerMessage {
    Sync(Option<Repository>),
    Native {
        observed_at: Instant,
        event: notify::Result<Event>,
    },
    Stop,
}

pub struct GitMonitorHandle {
    commands: SyncSender<WorkerMessage>,
    overflowed: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

impl GitMonitorHandle {
    /// Replaces the one repository observed by this workspace. `None`
    /// unregisters every path, which is also how a zero refresh interval
    /// disables watcher-triggered work.
    pub fn sync(&self, repository: Option<Repository>) {
        if self
            .commands
            .try_send(WorkerMessage::Sync(repository))
            .is_err()
        {
            self.overflowed.store(true, Ordering::Release);
        }
    }
}

impl Drop for GitMonitorHandle {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.commands.try_send(WorkerMessage::Stop);
    }
}

pub fn spawn() -> (GitMonitorHandle, mpsc::Receiver<GitInvalidation>) {
    let (commands, receiver) = sync_channel(COMMAND_CAPACITY);
    let overflowed = Arc::new(AtomicBool::new(false));
    let native_commands = commands.clone();
    let native_overflowed = Arc::clone(&overflowed);
    let mut watcher = notify::recommended_watcher(move |event| {
        if native_commands
            .try_send(WorkerMessage::Native {
                observed_at: Instant::now(),
                event,
            })
            .is_err()
        {
            native_overflowed.store(true, Ordering::Release);
        }
    })
    .ok();
    let (events, event_receiver) = mpsc::channel(EVENT_CAPACITY);
    let worker_overflowed = Arc::clone(&overflowed);
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_stopped = Arc::clone(&stopped);
    thread::Builder::new()
        .name("runyte-git-monitor".to_owned())
        .spawn(move || {
            run_worker(
                watcher.as_mut(),
                receiver,
                events,
                worker_overflowed,
                worker_stopped,
            );
        })
        .expect("Git monitor thread must start");
    (
        GitMonitorHandle {
            commands,
            overflowed,
            stopped,
        },
        event_receiver,
    )
}

fn run_worker(
    mut watcher: Option<&mut RecommendedWatcher>,
    receiver: Receiver<WorkerMessage>,
    events: mpsc::Sender<GitInvalidation>,
    overflowed: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
) {
    let mut repository = None::<Repository>;
    let mut watched = HashSet::<PathBuf>::new();
    let mut deadline = None::<Instant>;
    let mut last_observed = None::<Instant>;
    let mut full = false;
    let mut ready = None::<GitInvalidation>;

    while !stopped.load(Ordering::Acquire) {
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(WorkerMessage::Sync(next)) => {
                if repository != next {
                    let complete =
                        sync_registration(watcher.as_deref_mut(), &mut watched, next.as_ref());
                    repository = next;
                    deadline = None;
                    last_observed = None;
                    full = false;
                    ready = None;
                    if !complete && repository.is_some() {
                        last_observed = Some(Instant::now());
                        full = true;
                        deadline = last_observed.map(|observed| observed + DEBOUNCE);
                    }
                }
            }
            Ok(WorkerMessage::Native {
                observed_at,
                event: Ok(event),
            }) => {
                if repository
                    .as_ref()
                    .is_some_and(|repository| affects(&event, repository))
                {
                    last_observed = Some(observed_at);
                    deadline = Some(observed_at + DEBOUNCE);
                }
            }
            Ok(WorkerMessage::Native {
                observed_at,
                event: Err(_),
            }) => {
                if repository.is_some() {
                    last_observed = Some(observed_at);
                    full = true;
                    deadline = Some(observed_at + DEBOUNCE);
                }
            }
            Ok(WorkerMessage::Stop) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }

        if overflowed.swap(false, Ordering::AcqRel) && repository.is_some() {
            let observed = Instant::now();
            last_observed = Some(observed);
            full = true;
            deadline = Some(observed + DEBOUNCE);
        }

        let now = Instant::now();
        if deadline.is_some_and(|due| due <= now) {
            deadline = None;
            if let Some(repository) = repository.as_ref() {
                ready = Some(GitInvalidation {
                    repository: repository.workdir().to_path_buf(),
                    observed_at: last_observed.take().unwrap_or(now),
                    overflowed: full,
                });
                full = false;
            }
        }

        if let Some(event) = ready.take() {
            match events.try_send(event) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(event)) => {
                    // One retained full invalidation bounds the output while
                    // guaranteeing that a slow host eventually reconciles.
                    ready = Some(GitInvalidation {
                        overflowed: true,
                        ..event
                    });
                }
                Err(mpsc::error::TrySendError::Closed(_)) => break,
            }
        }
    }
}

fn sync_registration(
    mut watcher: Option<&mut RecommendedWatcher>,
    watched: &mut HashSet<PathBuf>,
    repository: Option<&Repository>,
) -> bool {
    for path in watched.drain() {
        if let Some(watcher) = watcher.as_deref_mut() {
            let _ = watcher.unwatch(&path);
        }
    }
    let Some(repository) = repository else {
        return true;
    };
    let roots = watched_roots(repository);
    for path in roots {
        if watcher
            .as_deref_mut()
            .is_some_and(|watcher| watcher.watch(&path, RecursiveMode::Recursive).is_ok())
        {
            watched.insert(path);
        }
    }
    watched.len() == watched_roots(repository).len()
}

fn watched_roots(repository: &Repository) -> Vec<PathBuf> {
    let mut roots = [
        repository.workdir().to_path_buf(),
        repository.git_dir().to_path_buf(),
        repository.common_dir().to_path_buf(),
    ]
    .into_iter()
    .collect::<HashSet<_>>()
    .into_iter()
    .collect::<Vec<_>>();
    roots.sort();
    roots
}

fn affects(event: &Event, repository: &Repository) -> bool {
    !event.kind.is_access()
        && event.paths.iter().any(|path| {
            under(path, repository.workdir())
                || under(path, repository.git_dir())
                || under(path, repository.common_dir())
        })
}

fn under(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::*;

    #[test]
    fn linked_worktree_watches_checkout_private_and_shared_metadata() {
        let repository =
            Repository::with_git_dirs("/work/linked", "/repo/.git/worktrees/linked", "/repo/.git");
        assert_eq!(
            watched_roots(&repository),
            vec![
                PathBuf::from("/repo/.git"),
                PathBuf::from("/repo/.git/worktrees/linked"),
                PathBuf::from("/work/linked"),
            ]
        );
    }

    #[test]
    fn worktree_index_head_refs_and_packed_refs_are_relevant() {
        let repository =
            Repository::with_git_dirs("/work/linked", "/repo/.git/worktrees/linked", "/repo/.git");
        for path in [
            "/work/linked/src/main.rs",
            "/repo/.git/worktrees/linked/index",
            "/repo/.git/worktrees/linked/HEAD",
            "/repo/.git/refs/heads/main",
            "/repo/.git/packed-refs",
            "/repo/.git/logs/refs/stash",
        ] {
            assert!(
                affects(
                    &Event::new(notify::EventKind::Any).add_path(path.into()),
                    &repository
                ),
                "{path} was ignored"
            );
        }
        assert!(!affects(
            &Event::new(notify::EventKind::Any).add_path("/other/file".into()),
            &repository
        ));
        assert!(!affects(
            &Event::new(notify::EventKind::Access(notify::event::AccessKind::Any))
                .add_path("/repo/.git/index".into()),
            &repository
        ));
    }

    #[test]
    fn dropping_the_handle_requests_shutdown_even_when_the_queue_is_full() {
        let (commands, receiver) = sync_channel(1);
        commands.try_send(WorkerMessage::Sync(None)).unwrap();
        let overflowed = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let handle = GitMonitorHandle {
            commands,
            overflowed: Arc::clone(&overflowed),
            stopped: Arc::clone(&stopped),
        };

        drop(handle);

        assert!(stopped.load(Ordering::Acquire));
        let (events, _) = mpsc::channel(1);
        run_worker(None, receiver, events, overflowed, stopped);
    }

    #[tokio::test]
    async fn a_native_burst_produces_one_debounced_invalidation() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-git-monitor-{}-{unique}",
            std::process::id()
        ));
        let git_dir = root.join("metadata/worktree");
        let common_dir = root.join("metadata/common");
        let workdir = root.join("work");
        fs::create_dir_all(&git_dir).unwrap();
        fs::create_dir_all(&common_dir).unwrap();
        fs::create_dir_all(&workdir).unwrap();
        let repository = Repository::with_git_dirs(&workdir, &git_dir, &common_dir);
        let (monitor, mut events) = spawn();
        monitor.sync(Some(repository));
        tokio::time::sleep(Duration::from_millis(75)).await;

        for _ in 0..3 {
            fs::write(workdir.join("source.rs"), "changed\n").unwrap();
            fs::write(git_dir.join("index"), "index\n").unwrap();
            fs::write(common_dir.join("packed-refs"), "refs\n").unwrap();
        }

        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("the debounced event arrived")
            .expect("the monitor stayed live");
        assert_eq!(event.repository, workdir);
        assert!(!event.overflowed);
        assert!(
            tokio::time::timeout(Duration::from_millis(300), events.recv())
                .await
                .is_err(),
            "one filesystem burst produced multiple invalidations"
        );

        drop(monitor);
        fs::remove_dir_all(root).unwrap();
    }
}
