// SPDX-License-Identifier: MPL-2.0

//! Host-owned monitoring of the paths buffers project.
//!
//! Native directory notifications are wake-up hints. The worker debounces
//! them, then constructs one complete observation from one file handle. A
//! periodic metadata pass covers lost events without re-reading every file.
//!
//! An explorer is registered the same way, with its directory listing as the
//! observation. A listing has no cheap metadata to compare first, so the
//! periodic pass reads it and the worker forwards it only when it differs
//! from the last listing it sent, which keeps an unchanged directory from
//! waking the editor every pass.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, SyncSender, sync_channel},
    thread,
    time::{Duration, Instant},
};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::buffer::{
    FileObservation, FileObservationEvent, FileObservationRequest, ObservationTarget,
    inspect_file_metadata, observe_directory, observe_file,
};

const COMMAND_CAPACITY: usize = 256;
const EVENT_CAPACITY: usize = 64;
const DEBOUNCE: Duration = Duration::from_millis(150);
const RECONCILE_INTERVAL: Duration = Duration::from_secs(2);

enum WorkerMessage {
    Sync(Vec<FileObservationRequest>),
    Observe(FileObservationRequest),
    Native(notify::Result<Event>),
    Stop,
}

pub struct FileMonitorHandle {
    commands: SyncSender<WorkerMessage>,
    synced_requests: Vec<FileObservationRequest>,
}

impl FileMonitorHandle {
    pub fn sync(&mut self, requests: Vec<FileObservationRequest>) {
        if self.synced_requests == requests {
            return;
        }
        if self
            .commands
            .try_send(WorkerMessage::Sync(requests.clone()))
            .is_ok()
        {
            self.synced_requests = requests;
        }
    }

    pub fn observe(&self, request: FileObservationRequest) {
        let _ = self.commands.try_send(WorkerMessage::Observe(request));
    }
}

impl Drop for FileMonitorHandle {
    fn drop(&mut self) {
        let _ = self.commands.try_send(WorkerMessage::Stop);
    }
}

pub fn spawn() -> (FileMonitorHandle, mpsc::Receiver<FileObservationEvent>) {
    let (commands, receiver) = sync_channel(COMMAND_CAPACITY);
    let native_commands = commands.clone();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = native_commands.try_send(WorkerMessage::Native(event));
    })
    .ok();
    let (events, event_receiver) = mpsc::channel(EVENT_CAPACITY);
    thread::Builder::new()
        .name("runyte-file-monitor".to_owned())
        .spawn(move || run_worker(watcher.as_mut(), receiver, events))
        .expect("file monitor thread must start");
    (
        FileMonitorHandle {
            commands,
            synced_requests: Vec::new(),
        },
        event_receiver,
    )
}

fn run_worker(
    mut watcher: Option<&mut RecommendedWatcher>,
    receiver: Receiver<WorkerMessage>,
    events: mpsc::Sender<FileObservationEvent>,
) {
    let mut registrations = HashMap::<usize, FileObservationRequest>::new();
    let mut watched = HashSet::<PathBuf>::new();
    let mut due = HashMap::<usize, Instant>::new();
    let mut unstable = HashMap::<usize, FileObservation>::new();
    // The last listing forwarded for each directory registration, so a pass
    // that re-reads an unchanged directory sends nothing.
    let mut forwarded = HashMap::<usize, FileObservation>::new();
    let mut next_reconcile = Instant::now() + RECONCILE_INTERVAL;

    loop {
        let now = Instant::now();
        let next_due = due.values().copied().min().unwrap_or(next_reconcile);
        let wake_at = next_reconcile.min(next_due);
        match receiver.recv_timeout(wake_at.saturating_duration_since(now)) {
            Ok(WorkerMessage::Sync(requests)) => {
                sync_registrations(
                    watcher.as_deref_mut(),
                    &mut registrations,
                    &mut watched,
                    &mut due,
                    &mut forwarded,
                    requests,
                );
            }
            Ok(WorkerMessage::Observe(request)) => {
                forwarded.remove(&request.buffer);
                registrations.insert(request.buffer, request.clone());
                due.insert(request.buffer, Instant::now());
            }
            Ok(WorkerMessage::Native(Ok(event))) => {
                let now = Instant::now() + DEBOUNCE;
                for request in registrations.values() {
                    if native_affects(&event, request) {
                        due.insert(request.buffer, now);
                    }
                }
            }
            Ok(WorkerMessage::Native(Err(_))) => {
                let now = Instant::now() + DEBOUNCE;
                due.extend(registrations.keys().map(|buffer| (*buffer, now)));
            }
            Ok(WorkerMessage::Stop) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }

        let now = Instant::now();
        if now >= next_reconcile {
            for request in registrations.values() {
                // An explorer's baseline is a listing rather than one file's
                // metadata, so there is nothing cheaper to compare ahead of
                // reading the directory. `forwarded` is what keeps that read
                // from becoming an event when nothing moved.
                if request.target.is_directory()
                    || inspect_file_metadata(&request.path) != request.baseline_metadata
                {
                    due.entry(request.buffer).or_insert(now);
                }
            }
            next_reconcile = now + RECONCILE_INTERVAL;
        }

        let ready = due
            .iter()
            .filter_map(|(buffer, deadline)| (*deadline <= now).then_some(*buffer))
            .collect::<Vec<_>>();
        for buffer in ready {
            due.remove(&buffer);
            let Some(request) = registrations.get(&buffer).cloned() else {
                continue;
            };
            let observation = match request.target {
                ObservationTarget::File => observe_file(&request.path),
                ObservationTarget::Directory { show_hidden } => {
                    observe_directory(&request.path, show_hidden)
                }
            };
            let needs_stability_retry = matches!(
                observation,
                FileObservation::Deleted | FileObservation::Unreadable { .. }
            );
            if needs_stability_retry && unstable.get(&buffer) != Some(&observation) {
                unstable.insert(buffer, observation);
                due.insert(buffer, now + DEBOUNCE);
                continue;
            }
            unstable.remove(&buffer);
            // A directory is re-read on every reconcile whether or not
            // anything moved, so an identical listing must not become an
            // event: it would redraw the editor twice a second for nothing.
            let repeatable = request.target.is_directory();
            if repeatable && forwarded.get(&buffer) == Some(&observation) {
                continue;
            }
            let record = repeatable.then(|| observation.clone());
            let event = FileObservationEvent {
                buffer,
                path: request.path,
                generation: request.generation,
                observation,
            };
            match events.try_send(event) {
                Ok(()) => {
                    if let Some(record) = record {
                        forwarded.insert(buffer, record);
                    }
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    // The periodic pass will recover a coalesced observation.
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => return,
            }
        }
    }
}

fn sync_registrations(
    mut watcher: Option<&mut RecommendedWatcher>,
    registrations: &mut HashMap<usize, FileObservationRequest>,
    watched: &mut HashSet<PathBuf>,
    due: &mut HashMap<usize, Instant>,
    forwarded: &mut HashMap<usize, FileObservation>,
    requests: Vec<FileObservationRequest>,
) {
    let next = requests
        .into_iter()
        .map(|request| (request.buffer, request))
        .collect::<HashMap<_, _>>();
    for (buffer, request) in &next {
        if registrations.get(buffer) != Some(request) {
            due.insert(*buffer, Instant::now());
            // A buffer that has accepted a new baseline must hear the next
            // observation even when the path itself did not move.
            forwarded.remove(buffer);
        }
    }
    *registrations = next;

    // A file is watched through the directory holding it, because that is
    // where a replacement or a rename appears. An explorer is watched
    // directly, because its own children are what it projects.
    let next_directories = registrations
        .values()
        .filter_map(|request| match request.target {
            ObservationTarget::File => request.path.parent().map(Path::to_path_buf),
            ObservationTarget::Directory { .. } => Some(request.path.clone()),
        })
        .collect::<HashSet<_>>();
    for directory in watched
        .difference(&next_directories)
        .cloned()
        .collect::<Vec<_>>()
    {
        if let Some(watcher) = watcher.as_deref_mut() {
            let _ = watcher.unwatch(&directory);
        }
        watched.remove(&directory);
    }
    for directory in next_directories
        .difference(watched)
        .cloned()
        .collect::<Vec<_>>()
    {
        if watcher.as_deref_mut().is_some_and(|watcher| {
            watcher
                .watch(&directory, RecursiveMode::NonRecursive)
                .is_ok()
        }) {
            watched.insert(directory);
        }
    }
    due.retain(|buffer, _| registrations.contains_key(buffer));
    forwarded.retain(|buffer, _| registrations.contains_key(buffer));
}

fn affects(event_path: &Path, file_path: &Path) -> bool {
    event_path == file_path
        || event_path == file_path.parent().unwrap_or(file_path)
        || event_path.parent() == file_path.parent()
}

/// Whether an event can have changed what `root` lists.
///
/// Only `root` itself and its immediate children: an explorer projects one
/// directory, and a write deeper in the tree changes a child directory's
/// contents rather than this listing's rows.
fn listing_affects(event_path: &Path, root: &Path) -> bool {
    event_path == root || event_path.parent() == Some(root)
}

fn native_affects(event: &Event, request: &FileObservationRequest) -> bool {
    !event.kind.is_access()
        && event.paths.iter().any(|path| match request.target {
            ObservationTarget::File => affects(path, &request.path),
            ObservationTarget::Directory { .. } => listing_affects(path, &request.path),
        })
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::*;
    use crate::{buffer::Buffer, directory_buffer::ListingView};

    #[tokio::test]
    async fn registration_produces_one_complete_generation_tagged_observation() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-file-monitor-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("notes.txt");
        fs::write(&path, "baseline").unwrap();
        let buffer = Buffer::open(&path).unwrap();
        let request = buffer.file_observation_request(9).unwrap();
        let generation = request.generation;
        fs::write(&path, "external").unwrap();

        let (mut monitor, mut events) = spawn();
        monitor.sync(vec![request]);
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("the worker is bounded")
            .expect("the worker is live");
        assert_eq!(event.buffer, 9);
        assert_eq!(event.generation, generation);
        assert!(matches!(
            event.observation,
            FileObservation::Text { ref text, .. } if text.as_ref() == "external"
        ));

        drop(monitor);
        fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn an_explorer_hears_its_listing_once_and_then_stays_quiet() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "runyte-file-monitor-listing-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("a.txt"), "a").unwrap();
        let buffer = Buffer::open_directory(&directory, ListingView::default()).unwrap();
        let request = buffer.file_observation_request(4).unwrap();
        assert!(request.target.is_directory());
        fs::write(directory.join("b.txt"), "b").unwrap();

        let (mut monitor, mut events) = spawn();
        monitor.sync(vec![request]);
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("the worker is bounded")
            .expect("the worker is live");
        assert_eq!(event.buffer, 4);
        let FileObservation::Directory { listing } = event.observation else {
            panic!("an explorer registration observes a listing");
        };
        assert_eq!(
            listing,
            crate::fs_plan::DirectoryListing::read(&directory, false).unwrap()
        );

        // A directory is re-read on every reconcile, so the quiet that
        // follows is what keeps an unchanged listing from redrawing the
        // editor twice a second.
        assert!(
            tokio::time::timeout(RECONCILE_INTERVAL + DEBOUNCE * 4, events.recv())
                .await
                .is_err(),
            "an unchanged listing must not be forwarded again"
        );

        drop(monitor);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_directory_event_wakes_every_file_in_that_directory() {
        let directory = Path::new("/tmp/project");
        assert!(affects(
            Path::new("/tmp/project/.notes.txt.save"),
            Path::new("/tmp/project/notes.txt")
        ));
        assert!(!affects(
            Path::new("/tmp/other/notes.txt"),
            Path::new("/tmp/project/notes.txt")
        ));
        assert!(affects(directory, Path::new("/tmp/project/notes.txt")));
    }

    fn request_for(path: &Path, target: ObservationTarget) -> FileObservationRequest {
        FileObservationRequest {
            buffer: 0,
            path: path.to_path_buf(),
            generation: 1,
            target,
            baseline_metadata: None,
        }
    }

    #[test]
    fn reading_a_monitored_file_does_not_observe_itself_forever() {
        let path = Path::new("/tmp/project/notes.txt");
        let request = request_for(path, ObservationTarget::File);
        let access = Event::new(notify::EventKind::Access(notify::event::AccessKind::Any))
            .add_path(path.to_path_buf());
        let modify = Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(path.to_path_buf());
        assert!(!native_affects(&access, &request));
        assert!(native_affects(&modify, &request));
    }

    #[test]
    fn an_explorer_wakes_for_its_own_children_and_not_for_deeper_writes() {
        let root = Path::new("/tmp/project");
        assert!(listing_affects(Path::new("/tmp/project/notes.txt"), root));
        assert!(listing_affects(root, root));
        assert!(!listing_affects(
            Path::new("/tmp/project/src/main.rs"),
            root
        ));
        assert!(!listing_affects(Path::new("/tmp/other/notes.txt"), root));

        let request = request_for(root, ObservationTarget::Directory { show_hidden: false });
        let modify = Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(root.join("src/main.rs"));
        assert!(!native_affects(&modify, &request));
    }

    #[test]
    fn unchanged_registrations_do_not_wake_the_worker_again() {
        let (commands, receiver) = sync_channel(2);
        let mut monitor = FileMonitorHandle {
            commands,
            synced_requests: Vec::new(),
        };
        let request = FileObservationRequest {
            buffer: 7,
            path: PathBuf::from("/tmp/notes.txt"),
            generation: 3,
            target: ObservationTarget::File,
            baseline_metadata: None,
        };

        monitor.sync(vec![request.clone()]);
        assert!(matches!(receiver.try_recv(), Ok(WorkerMessage::Sync(_))));
        monitor.sync(vec![request]);
        assert!(matches!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
    }
}
