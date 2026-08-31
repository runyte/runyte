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
    #[cfg(test)]
    Barrier(SyncSender<()>),
    Stop,
}

pub struct GitMonitorHandle {
    commands: SyncSender<WorkerMessage>,
    overflowed: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    synced_repository: Option<Option<Repository>>,
}

impl GitMonitorHandle {
    /// Replaces the one repository observed by this workspace. `None`
    /// unregisters every path, which is also how a zero refresh interval
    /// disables watcher-triggered work.
    pub fn sync(&mut self, repository: Option<Repository>) {
        if self.synced_repository.as_ref() == Some(&repository) {
            return;
        }
        if self
            .commands
            .try_send(WorkerMessage::Sync(repository.clone()))
            .is_ok()
        {
            self.synced_repository = Some(repository);
        } else {
            self.overflowed.store(true, Ordering::Release);
        }
    }

    #[cfg(test)]
    fn sync_and_wait(&self, repository: Option<Repository>) {
        let (registered, receiver) = sync_channel(1);
        self.commands
            .send(WorkerMessage::Sync(repository))
            .expect("the Git monitor accepts test registration");
        self.commands
            .send(WorkerMessage::Barrier(registered))
            .expect("the Git monitor accepts a test barrier");
        receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("the Git monitor completed test registration");
    }

    #[cfg(test)]
    fn inject_native(&self, event: Event) {
        self.commands
            .send(WorkerMessage::Native {
                observed_at: Instant::now(),
                event: Ok(event),
            })
            .expect("the Git monitor accepts a native test event");
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
            synced_repository: None,
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
        let received = if let Some(deadline) = deadline {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(message) => Some(Some(message)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Some(None),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => None,
            }
        } else {
            receiver.recv().ok().map(Some)
        };
        let Some(message) = received else {
            break;
        };
        match message {
            None => {}
            Some(WorkerMessage::Sync(next)) => {
                if repository != next {
                    let complete =
                        sync_registration(watcher.as_deref_mut(), &mut watched, next.as_ref());
                    repository = next;
                    deadline = None;
                    last_observed = None;
                    full = false;
                    ready = None;
                    if !complete && repository.is_some() {
                        note_observation(&mut last_observed, &mut deadline, Instant::now());
                        full = true;
                    }
                }
            }
            Some(WorkerMessage::Native {
                observed_at,
                event: Ok(event),
            }) => {
                if repository
                    .as_ref()
                    .is_some_and(|repository| affects(&event, repository))
                {
                    note_observation(&mut last_observed, &mut deadline, observed_at);
                }
            }
            Some(WorkerMessage::Native {
                observed_at,
                event: Err(_),
            }) => {
                if repository.is_some() {
                    note_observation(&mut last_observed, &mut deadline, observed_at);
                    full = true;
                }
            }
            #[cfg(test)]
            Some(WorkerMessage::Barrier(registered)) => {
                let _ = registered.send(());
            }
            Some(WorkerMessage::Stop) => break,
        }

        if overflowed.swap(false, Ordering::AcqRel) && repository.is_some() {
            note_observation(&mut last_observed, &mut deadline, Instant::now());
            full = true;
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
                    // This dedicated worker may wait for the bounded output
                    // slot. Draining that slot does not wake the command
                    // channel, so retaining the event and returning to
                    // `recv` could otherwise strand it forever. Mark the
                    // coalesced event full before waiting: observations that
                    // arrive behind this wait are reconciled on the next loop.
                    let event = GitInvalidation {
                        overflowed: true,
                        ..event
                    };
                    if events.blocking_send(event).is_err() {
                        break;
                    }
                }
                Err(mpsc::error::TrySendError::Closed(_)) => break,
            }
        }
    }
}

fn note_observation(
    last_observed: &mut Option<Instant>,
    deadline: &mut Option<Instant>,
    observed_at: Instant,
) {
    *last_observed = Some(last_observed.map_or(observed_at, |last| last.max(observed_at)));
    // Callback time orders an observation against a snapshot. Quiet time is
    // local to the worker: queued observations may already be older than the
    // debounce interval by the time they can be processed.
    *deadline = Some(Instant::now() + DEBOUNCE);
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
            synced_repository: None,
        };

        drop(handle);

        assert!(stopped.load(Ordering::Acquire));
        let (events, _) = mpsc::channel(1);
        run_worker(None, receiver, events, overflowed, stopped);
    }

    #[test]
    fn unchanged_repository_registration_does_not_wake_the_worker_again() {
        let (commands, receiver) = sync_channel(2);
        let mut handle = GitMonitorHandle {
            commands,
            overflowed: Arc::new(AtomicBool::new(false)),
            stopped: Arc::new(AtomicBool::new(false)),
            synced_repository: None,
        };

        handle.sync(None);
        assert!(matches!(receiver.try_recv(), Ok(WorkerMessage::Sync(None))));
        handle.sync(None);
        assert!(matches!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn queued_observations_never_regress_freshness_or_expire_the_debounce() {
        let now = Instant::now();
        let older = now.checked_sub(Duration::from_secs(1)).unwrap();
        let newer = now.checked_sub(Duration::from_millis(500)).unwrap();
        let mut last_observed = None;
        let mut deadline = None;

        note_observation(&mut last_observed, &mut deadline, newer);
        let before_delayed_message = Instant::now();
        note_observation(&mut last_observed, &mut deadline, older);

        assert_eq!(last_observed, Some(newer));
        assert!(deadline.unwrap() >= before_delayed_message + DEBOUNCE);
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
        let root = root.canonicalize().unwrap();
        let git_dir = root.join("metadata/worktree");
        let common_dir = root.join("metadata/common");
        let workdir = root.join("work");
        let repository = Repository::with_git_dirs(&workdir, &git_dir, &common_dir);
        let (monitor, mut events) = spawn();
        monitor.sync_and_wait(Some(repository));

        for path in [
            workdir.join("source.rs"),
            git_dir.join("index"),
            common_dir.join("packed-refs"),
        ] {
            monitor.inject_native(Event::new(notify::EventKind::Any).add_path(path));
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

    #[tokio::test]
    async fn draining_a_full_output_delivers_the_retained_invalidation() {
        let repository = Repository::with_git_dirs("/work", "/metadata/worktree", "/metadata");
        let workdir = repository.workdir().to_path_buf();
        let (commands, receiver) = sync_channel(COMMAND_CAPACITY);
        let (events, mut event_receiver) = mpsc::channel(1);
        let overflowed = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        events
            .try_send(GitInvalidation {
                repository: PathBuf::from("/already-queued"),
                observed_at: Instant::now(),
                overflowed: false,
            })
            .unwrap();
        let worker_overflowed = Arc::clone(&overflowed);
        let worker_stopped = Arc::clone(&stopped);
        let worker = thread::spawn(move || {
            run_worker(None, receiver, events, worker_overflowed, worker_stopped);
        });
        commands
            .send(WorkerMessage::Sync(Some(repository)))
            .unwrap();

        tokio::time::sleep(DEBOUNCE + Duration::from_millis(50)).await;
        assert_eq!(
            event_receiver.recv().await.unwrap().repository,
            PathBuf::from("/already-queued")
        );
        let retained = tokio::time::timeout(Duration::from_secs(2), event_receiver.recv())
            .await
            .expect("draining output woke retained delivery")
            .expect("the Git monitor stayed live");
        assert_eq!(retained.repository, workdir);
        assert!(retained.overflowed);

        stopped.store(true, Ordering::Release);
        commands.send(WorkerMessage::Stop).unwrap();
        worker.join().unwrap();
    }
}
