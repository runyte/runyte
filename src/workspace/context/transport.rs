// SPDX-License-Identifier: MPL-2.0

//! Owner-private, bounded external context transport. Admission and semantic
//! dispatch stay on the workspace thread; socket tasks never inspect App.

use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::{Notify, Semaphore, mpsc, oneshot},
    task::JoinHandle,
};

pub const FRAME_BYTES: usize = super::wire::MAX_FRAME_BYTES;
pub const CONNECTIONS: usize = 8;
static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

#[derive(Default, Debug)]
pub struct Lease {
    cancelled: AtomicBool,
    wake: Notify,
}

impl Lease {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.wake.notify_waiters();
    }
    pub fn active(&self) -> bool {
        !self.cancelled.load(Ordering::Acquire)
    }
    pub async fn cancelled(&self) {
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.active() {
                return;
            }
            notified.await;
        }
    }
}

pub struct Reply {
    pub value: Value,
    pub lease: Option<Arc<Lease>>,
    pub close: bool,
}

pub enum Event {
    Frame {
        connection: u64,
        bytes: Vec<u8>,
        reply: oneshot::Sender<Reply>,
    },
    Closed(u64),
}

pub struct Server {
    task: JoinHandle<()>,
    socket: PathBuf,
}

impl Server {
    pub fn bind(socket: PathBuf, events: mpsc::Sender<Event>) -> std::io::Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        let listener = UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        let task = tokio::spawn(async move {
            let permits = Arc::new(Semaphore::new(CONNECTIONS));
            let mut peers = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break; };
                        let Ok(permit) = permits.clone().try_acquire_owned() else { continue; };
                        // SAFETY: geteuid has no preconditions.
                        if stream.peer_cred().map_or(true, |peer| peer.uid() != unsafe { libc::geteuid() }) { continue; }
                        let Ok(id) = NEXT_CONNECTION.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1)) else { break; };
                        let events = events.clone();
                        peers.spawn(async move {
                            let _permit = permit;
                            connection(stream, id, &events).await;
                            let _ = events.send(Event::Closed(id)).await;
                        });
                    }
                    _ = peers.join_next(), if !peers.is_empty() => {}
                }
            }
        });
        Ok(Self { task, socket })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.socket);
    }
}

async fn connection(stream: UnixStream, id: u64, events: &mpsc::Sender<Event>) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut lease: Option<Arc<Lease>> = None;
    loop {
        let mut bytes = Vec::new();
        let read = async {
            (&mut reader)
                .take((FRAME_BYTES + 1) as u64)
                .read_until(b'\n', &mut bytes)
                .await
        };
        let read = tokio::select! {
            biased;
            _ = revoked(&lease) => return,
            read = tokio::time::timeout(Duration::from_secs(if lease.is_some() { 300 } else { 10 }), read) => read,
        };
        if !matches!(read, Ok(Ok(1..))) || bytes.len() > FRAME_BYTES || bytes.last() != Some(&b'\n')
        {
            return;
        }
        let (reply, response) = oneshot::channel();
        if events
            .send(Event::Frame {
                connection: id,
                bytes,
                reply,
            })
            .await
            .is_err()
        {
            return;
        }
        let reply = tokio::select! {
            biased;
            _ = revoked(&lease) => return,
            reply = tokio::time::timeout(Duration::from_secs(10), response) => match reply { Ok(Ok(reply)) => reply, _ => return },
        };
        if reply.lease.is_some() {
            lease = reply.lease;
        }
        if lease.as_ref().is_some_and(|lease| !lease.active()) {
            return;
        }
        let Ok(mut bytes) = serde_json::to_vec(&reply.value) else {
            return;
        };
        if bytes.len() >= FRAME_BYTES {
            return;
        }
        bytes.push(b'\n');
        let written = tokio::select! {
            biased;
            _ = revoked(&lease) => return,
            written = tokio::time::timeout(Duration::from_secs(2), writer.write_all(&bytes)) => written,
        };
        if !matches!(written, Ok(Ok(()))) || reply.close {
            return;
        }
    }
}

async fn revoked(lease: &Option<Arc<Lease>>) {
    match lease {
        Some(lease) => lease.cancelled().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
#[path = "tests/transport.rs"]
mod tests;
