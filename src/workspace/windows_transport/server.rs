// SPDX-License-Identifier: MPL-2.0

//! One application owner for native accepted streams and endpoint publication.

use std::{future::Future, io, pin::Pin, time::Duration};

use anyhow::{Context, Result};
use futures_util::{StreamExt, stream::FuturesUnordered};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::{Instant, sleep_until},
};

use super::ServerEvent;
use crate::workspace::{
    transport_shared::serve_connection_with_peer,
    windows_endpoint::{EndpointMetadata, NameStore, PreparedEndpoint},
    windows_pipe::{Listener, RenameSender, RenameTicket},
};

const EVENT_CAPACITY: usize = 64;
const ACCEPT_BUDGET: Duration = Duration::from_secs(2);
const REFUSAL_BACKOFF: Duration = Duration::from_millis(10);
const FINAL_RESPONSE_DRAIN: Duration = Duration::from_secs(3);

type ConnectionFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

pub struct LocalServer {
    events: mpsc::Receiver<ServerEvent>,
    task: Option<JoinHandle<Result<()>>>,
    shutdown: Option<oneshot::Sender<()>>,
    admission: tokio::sync::watch::Sender<bool>,
    metadata: EndpointMetadata,
    metadata_updates: Option<tokio::sync::watch::Receiver<EndpointMetadata>>,
    rename: Option<RenameSender>,
}

impl LocalServer {
    /// Requires an active Tokio I/O runtime. Endpoint ownership remains held
    /// while the first pipe instance is created and readiness is published.
    pub fn bind(prepared: PreparedEndpoint) -> Result<Self> {
        Self::with_listener(Listener::bind(prepared)?)
    }

    /// Owns one filesystem worker for serialized live-name operations. The
    /// supplied name store is fixed for this host, never taken from metadata.
    pub fn bind_with_names(prepared: PreparedEndpoint, names: NameStore) -> Result<Self> {
        Self::with_listener(Listener::bind_with_names(prepared, names)?)
    }

    fn with_listener(listener: Listener) -> Result<Self> {
        Ok(Self::from_owner(Owner {
            connections: FuturesUnordered::new(),
            #[cfg(test)]
            _before_listener_drop: None,
            #[cfg(test)]
            observer: None,
            listener,
            drain_budget: FINAL_RESPONSE_DRAIN,
        }))
    }

    fn from_owner(owner: Owner) -> Self {
        let metadata = owner.listener.metadata().clone();
        let metadata_updates = owner.listener.metadata_updates();
        let rename = owner.listener.rename_sender();
        let (events_tx, events) = mpsc::channel(EVENT_CAPACITY);
        let (shutdown, shutdown_rx) = oneshot::channel();
        let (admission, admission_rx) = tokio::sync::watch::channel(true);
        let task = tokio::spawn(owner.run(events_tx, shutdown_rx, admission_rx));
        Self {
            events,
            task: Some(task),
            shutdown: Some(shutdown),
            admission,
            metadata,
            metadata_updates,
            rename,
        }
    }

    /// Bind-time metadata. Identity/address/project remain stable; its name
    /// may be stale after a rename. Use metadata_snapshot for live name state.
    pub fn metadata(&self) -> &EndpointMetadata {
        &self.metadata
    }

    /// Latest verified publication commit, including updates whose requester
    /// canceled or disconnected before observing the result.
    pub fn metadata_snapshot(&self) -> EndpointMetadata {
        self.metadata_updates
            .as_ref()
            .map_or_else(|| self.metadata.clone(), |updates| updates.borrow().clone())
    }

    pub fn try_rename(&self, name: &str) -> io::Result<RenameTicket> {
        self.rename
            .as_ref()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "this server has no configured name store",
                )
            })?
            .rename(name)
    }

    pub async fn recv(&mut self) -> Option<ServerEvent> {
        self.events.recv().await
    }

    /// Stop accepting new peers while existing responses/services finish. This
    /// does not begin final draining or retire the publication; shutdown does.
    pub fn stop_admission(&mut self) {
        self.admission.send_replace(false);
    }

    /// Stops acceptance, permits a fixed final-response drain, then awaits
    /// application teardown and attempted owned publication cleanup. Kernel
    /// handle release still needs subsequent IOCP completion dispatch.
    /// Cancelling this future retains the owner task here; call again to await
    /// it, or drop the server to abort. Repeated completed shutdown is harmless.
    pub async fn shutdown(&mut self) -> Result<()> {
        self.stop_admission();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.as_mut() {
            let mut events_open = true;
            let result = loop {
                tokio::select! {
                    biased;
                    result = &mut *task => break result,
                    event = self.events.recv(), if events_open => {
                        events_open = event.is_some();
                    }
                }
            };
            self.task.take();
            self.events.close();
            while self.events.try_recv().is_ok() {}
            result.context("native transport owner task failed")??;
        }
        Ok(())
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.as_ref() {
            task.abort();
        }
    }
}

struct Owner {
    // Declaration order is the cancellation/unwind boundary. Keep this owner
    // intact: every future/stream drops BEFORE Listener retires its publication.
    connections: FuturesUnordered<ConnectionFuture>,
    #[cfg(test)]
    _before_listener_drop: Option<DropAudit>,
    listener: Listener,
    drain_budget: Duration,
    #[cfg(test)]
    observer: Option<tokio::sync::watch::Sender<usize>>,
}

impl Owner {
    async fn run(
        mut self,
        events: mpsc::Sender<ServerEvent>,
        mut shutdown: oneshot::Receiver<()>,
        mut admission: tokio::sync::watch::Receiver<bool>,
    ) -> Result<()> {
        let project = self.listener.metadata().project_root_bytes.clone();
        let mut publication_failure = self.listener.publication_failure();
        let mut next_id = 1_u64;
        let mut retry_at = None;
        let mut admitting = *admission.borrow_and_update();
        loop {
            tokio::select! {
                biased;
                _ = &mut shutdown => {
                    self.drain().await;
                    #[cfg(test)]
                    self._before_listener_drop.take();
                    self.listener.retire().await.context("native publication retirement failed")?;
                    return Ok(());
                }
                _ = publication_failed(&mut publication_failure) => {
                    anyhow::bail!("native publication worker failed; update outcome is unknown");
                }
                changed = admission.changed(), if admitting => {
                    admitting = changed.is_ok() && *admission.borrow_and_update();
                }
                _ = self.connections.next(), if !self.connections.is_empty() => {
                    self.observe();
                    retry_at = None;
                }
                _ = sleep_until(retry_at.unwrap_or_else(Instant::now)), if retry_at.is_some() => {
                    retry_at = None;
                }
                accepted = self.listener.accept(Instant::now() + ACCEPT_BUDGET), if admitting && retry_at.is_none() => {
                    match accepted {
                        Ok(stream) => {
                            let id = next_id;
                            next_id = next_id.checked_add(1).context("native transport connection identity exhausted")?;
                            let peer = stream.peer().clone();
                            let events = events.clone();
                            let project = project.clone();
                            // Insert before another await. Events, including a
                            // blocked Connected send, belong to this future.
                            self.connections.push(Box::pin(async move {
                                let _ = serve_connection_with_peer(id, stream, events, project, peer).await;
                            }));
                            self.observe();
                        }
                        Err(error) if !self.listener.is_usable() => {
                            return Err(error).context("native transport listener became unusable");
                        }
                        Err(_) => {
                            // Backoff is selectable: existing peers, shutdown
                            // and bounded event sends continue progressing.
                            retry_at = Some(Instant::now() + REFUSAL_BACKOFF);
                        }
                    }
                }
            }
        }
    }

    async fn drain(&mut self) {
        let deadline = Instant::now() + self.drain_budget;
        while !self.connections.is_empty() {
            tokio::select! {
                biased;
                _ = sleep_until(deadline) => break,
                _ = self.connections.next() => {}
            }
        }
        self.connections.clear();
        self.observe();
    }

    fn observe(&self) {
        #[cfg(test)]
        if let Some(observer) = &self.observer {
            observer.send_replace(self.connections.len());
        }
    }
}

async fn publication_failed(receiver: &mut Option<tokio::sync::watch::Receiver<bool>>) {
    let Some(receiver) = receiver else {
        std::future::pending::<()>().await;
        return;
    };
    loop {
        if *receiver.borrow_and_update() {
            return;
        }
        if receiver.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
struct DropAudit(Option<Box<dyn FnOnce() + Send>>);

#[cfg(test)]
impl Drop for DropAudit {
    fn drop(&mut self) {
        if let Some(audit) = self.0.take() {
            audit();
        }
    }
}

#[cfg(test)]
mod tests;
