// SPDX-License-Identifier: MPL-2.0

//! Owned, serialized filesystem operations for a rename-capable native host.
//! The listener retains this guard until its connection owners and pending pipe
//! have closed. Request cancellation never transfers publication ownership.

use crate::workspace::windows_endpoint::{EndpointMetadata, NameStore, Publication};
use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
};
use tokio::sync::{oneshot, watch};

/// Dropping a queued ticket lets the worker skip it if it has not begun. Once
/// begun, the update finishes independently; cancellation then has an unknown
/// outcome. Inspect the current metadata rather than assuming rollback.
#[derive(Debug)]
pub struct RenameTicket {
    result: oneshot::Receiver<io::Result<EndpointMetadata>>,
}

impl RenameTicket {
    /// Cancellation of this consuming future drops its ticket. There is no
    /// filesystem wall-clock timeout or automatic retry of an uncertain update.
    pub async fn wait(self) -> io::Result<EndpointMetadata> {
        self.result
            .await
            .map_err(|_| closed("publication operation owner stopped"))?
    }
}

fn closed(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, message)
}

#[derive(Debug)]
struct Command {
    name: String,
    result: oneshot::Sender<io::Result<EndpointMetadata>>,
}

#[derive(Debug, Default)]
struct State {
    queued: Option<Command>,
    retiring: bool,
    failed: bool,
}

#[derive(Debug, Default)]
struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

#[derive(Clone, Debug)]
pub(crate) struct RenameSender(Arc<Shared>);

impl RenameSender {
    pub(crate) fn rename(&self, name: &str) -> io::Result<RenameTicket> {
        crate::workspace::session_name::validate_host_name(name)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let mut state = self.0.lock();
        if state.retiring || state.failed {
            return Err(closed("publication operation owner is unavailable"));
        }
        if state.queued.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "a publication operation is already queued",
            ));
        }
        let (result, receiver) = oneshot::channel();
        state.queued = Some(Command {
            name: name.to_owned(),
            result,
        });
        self.0.ready.notify_one();
        Ok(RenameTicket { result: receiver })
    }
}

impl Shared {
    // Poison cannot lose the retirement signal or strand the owned thread.
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn retire(&self) {
        self.lock().retiring = true;
        self.ready.notify_all();
    }
}

#[derive(Debug)]
pub(super) struct Worker {
    initial: EndpointMetadata,
    shared: Arc<Shared>,
    metadata: watch::Receiver<EndpointMetadata>,
    failed: watch::Receiver<bool>,
    completion: Option<oneshot::Receiver<io::Result<()>>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub(super) fn start(publication: Publication, names: NameStore) -> io::Result<Self> {
        Self::start_with(publication, names, Publication::rename)
    }

    pub(super) fn start_with(
        mut publication: Publication,
        names: NameStore,
        mut rename: impl FnMut(&mut Publication, &NameStore, &str) -> io::Result<()> + Send + 'static,
    ) -> io::Result<Self> {
        let initial = publication.metadata().clone();
        let (metadata_tx, metadata) = watch::channel(initial.clone());
        let (failed_tx, failed) = watch::channel(false);
        let (completed, completion) = oneshot::channel();
        let shared = Arc::new(Shared::default());
        let worker_shared = shared.clone();
        let thread = thread::Builder::new()
            .name("runyte-publication".into())
            .spawn(move || {
                // Catch around a borrow, not a closure owning Publication. A panic
                // must retain its handles/ledger until transport retirement.
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    loop {
                        let command = {
                            let mut state = worker_shared.lock();
                            while state.queued.is_none() && !state.retiring {
                                state = worker_shared
                                    .ready
                                    .wait(state)
                                    .unwrap_or_else(|error| error.into_inner());
                            }
                            if state.retiring {
                                break;
                            }
                            state
                                .queued
                                .take()
                                .expect("queued command observed under lock")
                        };
                        // This locked admission decision is the operation's start
                        // boundary. Retirement after it waits for this operation.
                        {
                            let state = worker_shared.lock();
                            if state.retiring || command.result.is_closed() {
                                continue;
                            }
                        }
                        let result = rename(&mut publication, &names, &command.name);
                        let current = publication.metadata().clone();
                        metadata_tx.send_replace(current.clone());
                        let _ = command.result.send(result.map(|()| current));
                    }
                }));
                if outcome.is_err() {
                    // Only verified Publication commits change this snapshot, even
                    // when an injected/late failure follows the commit itself.
                    metadata_tx.send_replace(publication.metadata().clone());
                    let mut state = worker_shared.lock();
                    state.failed = true;
                    if let Some(command) = state.queued.take() {
                        let _ = command.result.send(Err(closed(
                            "publication worker failed; update outcome is unknown",
                        )));
                    }
                    failed_tx.send_replace(true);
                    while !state.retiring {
                        state = worker_shared
                            .ready
                            .wait(state)
                            .unwrap_or_else(|error| error.into_inner());
                    }
                }
                if let Some(command) = worker_shared.lock().queued.take() {
                    let _ = command.result.send(Err(closed("publication is retiring")));
                }
                // This only runs after the listener's explicit ordered retirement.
                let cleanup = catch_unwind(AssertUnwindSafe(|| publication.cleanup()))
                    .unwrap_or_else(|_| Err(io::Error::other("publication cleanup panicked")));
                drop(publication);
                let result = if outcome.is_err() && cleanup.is_ok() {
                    Err(io::Error::other(
                        "publication worker panicked; update outcome is unknown",
                    ))
                } else {
                    cleanup
                };
                let _ = completed.send(result);
            })?;
        Ok(Self {
            initial,
            shared,
            metadata,
            failed,
            completion: Some(completion),
            thread: Some(thread),
        })
    }

    pub(super) fn initial(&self) -> &EndpointMetadata {
        &self.initial
    }
    pub(super) fn snapshot(&self) -> watch::Receiver<EndpointMetadata> {
        self.metadata.clone()
    }
    pub(super) fn failure(&self) -> watch::Receiver<bool> {
        self.failed.clone()
    }
    pub(super) fn requests(&self) -> RenameSender {
        RenameSender(self.shared.clone())
    }

    pub(super) async fn retire(&mut self) -> io::Result<()> {
        self.shared.retire();
        let result = match self.completion.as_mut() {
            Some(completion) => completion
                .await
                .unwrap_or_else(|_| Err(closed("publication worker stopped during retirement"))),
            None => return Ok(()),
        };
        self.completion.take();
        let joined = self
            .thread
            .take()
            .expect("retirement retains its worker")
            .join();
        if joined.is_err() {
            return Err(io::Error::other("publication worker thread panicked"));
        }
        result
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.retire();
        if let Some(thread) = self.thread.take() {
            // Native filesystem calls cannot safely be detached or forcibly
            // interrupted. Ordered teardown waits for their owned completion.
            let _ = thread.join();
        }
    }
}

#[derive(Debug)]
pub(super) enum Ownership {
    Inline(Publication),
    Managed(Worker),
}

impl Ownership {
    pub(super) fn initial(&self) -> &EndpointMetadata {
        match self {
            Self::Inline(value) => value.metadata(),
            Self::Managed(value) => value.initial(),
        }
    }
    pub(super) fn snapshot(&self) -> Option<watch::Receiver<EndpointMetadata>> {
        match self {
            Self::Inline(_) => None,
            Self::Managed(value) => Some(value.snapshot()),
        }
    }
    pub(super) fn failure(&self) -> Option<watch::Receiver<bool>> {
        match self {
            Self::Inline(_) => None,
            Self::Managed(value) => Some(value.failure()),
        }
    }
    pub(super) fn requests(&self) -> Option<RenameSender> {
        match self {
            Self::Inline(_) => None,
            Self::Managed(value) => Some(value.requests()),
        }
    }
    pub(super) async fn retire(&mut self) -> io::Result<()> {
        match self {
            Self::Inline(value) => value.cleanup(),
            Self::Managed(value) => value.retire().await,
        }
    }
}

#[cfg(test)]
mod tests;
