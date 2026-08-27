// SPDX-License-Identifier: MPL-2.0

//! Host-owned monitoring of ordinary file buffers.
//!
//! Native directory notifications are wake-up hints. The worker debounces
//! them, then constructs one complete observation from one file handle. A
//! periodic metadata pass covers lost events without re-reading every file.

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
    FileObservation, FileObservationEvent, FileObservationRequest, inspect_file_metadata,
    observe_file,
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
}

impl FileMonitorHandle {
    pub fn sync(&self, requests: Vec<FileObservationRequest>) {
        let _ = self.commands.try_send(WorkerMessage::Sync(requests));
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
    (FileMonitorHandle { commands }, event_receiver)
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
    let mut next_reconcile = Instant::now() + RECONCILE_INTERVAL;

    loop {
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(WorkerMessage::Sync(requests)) => {
                sync_registrations(
                    watcher.as_deref_mut(),
                    &mut registrations,
                    &mut watched,
                    &mut due,
                    requests,
                );
            }
            Ok(WorkerMessage::Observe(request)) => {
                registrations.insert(request.buffer, request.clone());
                due.insert(request.buffer, Instant::now());
            }
            Ok(WorkerMessage::Native(Ok(event))) => {
                let now = Instant::now() + DEBOUNCE;
                for request in registrations.values() {
                    if event.paths.iter().any(|path| affects(path, &request.path)) {
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
                if inspect_file_metadata(&request.path) != request.baseline_metadata {
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
            let observation = observe_file(&request.path);
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
            let event = FileObservationEvent {
                buffer,
                path: request.path,
                generation: request.generation,
                observation,
            };
            match events.try_send(event) {
                Ok(()) => {}
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
    requests: Vec<FileObservationRequest>,
) {
    let next = requests
        .into_iter()
        .map(|request| (request.buffer, request))
        .collect::<HashMap<_, _>>();
    for (buffer, request) in &next {
        if registrations.get(buffer) != Some(request) {
            due.insert(*buffer, Instant::now());
        }
    }
    *registrations = next;

    let next_directories = registrations
        .values()
        .filter_map(|request| request.path.parent().map(Path::to_path_buf))
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
}

fn affects(event_path: &Path, file_path: &Path) -> bool {
    event_path == file_path
        || event_path == file_path.parent().unwrap_or(file_path)
        || event_path.parent() == file_path.parent()
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::*;
    use crate::buffer::Buffer;

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

        let (monitor, mut events) = spawn();
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
}
