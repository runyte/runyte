// SPDX-License-Identifier: MPL-2.0

use super::{CONNECTIONS, Event, FRAME_BYTES, Lease, NEXT_CONNECTION, revoked};
use crate::workspace::{
    context::storage::{Publication, Registration, Storage},
    windows_process_identity::{PinnedProcess, ProcessIdentity},
};
use std::{
    io,
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, atomic::Ordering},
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf},
    net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, PipeMode, ServerOptions,
    },
    sync::{Semaphore, mpsc, oneshot, watch},
    task::{JoinHandle, JoinSet},
    time::{Instant, timeout_at},
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, HANDLE},
    Storage::FileSystem::SECURITY_IDENTIFICATION,
    System::{
        IO::CancelIoEx,
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
    },
};

const PREFIX: &str = r"\\.\pipe\runyte-context-v1-";
const PIPE_INSTANCES: usize = CONNECTIONS + 1;
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;
const ACCEPT_BUDGET: Duration = Duration::from_secs(2);
const CONNECT_RETRY: Duration = Duration::from_millis(10);

pub struct Server {
    shutdown: watch::Sender<bool>,
    task: Option<JoinHandle<io::Result<()>>>,
    #[cfg(test)]
    permits: Arc<Semaphore>,
}

impl Server {
    pub fn bind(
        storage: Arc<Storage>,
        registration: Registration,
        events: mpsc::Sender<Event>,
    ) -> io::Result<Self> {
        validate_endpoint(&registration.endpoint)?;
        let expected = ProcessIdentity {
            pid: registration.pid,
            creation_time: registration.creation_time,
        };
        if expected != ProcessIdentity::current()? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "context registration does not name this process",
            ));
        }
        let pending = instance(&registration.endpoint, true)?;
        let publication = storage.register_owned(&registration)?;
        let (shutdown, receiver) = watch::channel(false);
        let permits = Arc::new(Semaphore::new(CONNECTIONS));
        let task = tokio::spawn(owner(
            pending,
            registration.endpoint,
            publication,
            events,
            permits.clone(),
            shutdown.clone(),
            receiver,
        ));
        Ok(Self {
            shutdown,
            task: Some(task),
            #[cfg(test)]
            permits,
        })
    }

    #[cfg(test)]
    pub(crate) fn available_permits(&self) -> usize {
        self.permits.available_permits()
    }

    pub(crate) fn stop(&mut self) {
        self.shutdown.send_replace(true);
    }

    pub async fn shutdown(&mut self) -> io::Result<()> {
        self.stop();
        let Some(task) = self.task.as_mut() else {
            return Ok(());
        };
        let joined = (&mut *task).await;
        self.task.take();
        joined
            .map_err(|error| io::Error::other(format!("context transport task failed: {error}")))?
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn owner(
    pending: NamedPipeServer,
    address: PathBuf,
    publication: Publication,
    events: mpsc::Sender<Event>,
    permits: Arc<Semaphore>,
    shutdown_sender: watch::Sender<bool>,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    let mut peers = JoinSet::new();
    let mut pending = Some(pending);
    let primary = loop {
        if *shutdown.borrow() {
            break Ok(());
        }
        let permit = tokio::select! {
            biased;
            _ = stopped(&mut shutdown) => break Ok(()),
            result = peers.join_next(), if !peers.is_empty() => {
                if let Some(Err(error)) = result {
                    break Err(io::Error::other(format!("context connection task failed: {error}")));
                }
                continue;
            }
            permit = permits.clone().acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => break Err(io::Error::other("context connection capacity closed")),
            },
        };
        let connected = tokio::select! {
            biased;
            _ = stopped(&mut shutdown) => break Ok(()),
            connected = timeout_at(
                Instant::now() + ACCEPT_BUDGET,
                pending.as_mut().expect("owner retains pending pipe").connect()
            ) => connected,
        };
        if connected.is_err() {
            drop(permit);
            continue;
        }
        if !matches!(connected, Ok(Ok(()))) {
            drop(permit);
            drop(pending.take());
            pending = match replacement(&address, &mut shutdown).await {
                Ok(pending) => Some(pending),
                Err(_) if *shutdown.borrow() => break Ok(()),
                Err(error) => break Err(error),
            };
            continue;
        }
        let peer = match pipe_peer(pending.as_ref().expect("owner retains pending pipe"), true) {
            Ok(peer) => peer,
            Err(_) => {
                drop(permit);
                drop(pending.take());
                pending = match replacement(&address, &mut shutdown).await {
                    Ok(pending) => Some(pending),
                    Err(_) if *shutdown.borrow() => break Ok(()),
                    Err(error) => break Err(error),
                };
                continue;
            }
        };
        let replacement = match replacement(&address, &mut shutdown).await {
            Ok(pending) => pending,
            Err(_) if *shutdown.borrow() => break Ok(()),
            Err(error) => break Err(error),
        };
        let stream = Pipe::new(
            pending
                .replace(replacement)
                .expect("owner retains pending pipe"),
            peer,
        );
        let id = match NEXT_CONNECTION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        {
            Ok(id) => id,
            Err(_) => break Err(io::Error::other("context connection identity exhausted")),
        };
        let sender = events.clone();
        let connection_shutdown = shutdown.clone();
        peers.spawn(async move {
            let mut connection_shutdown = connection_shutdown;
            connection(stream, id, sender.clone(), connection_shutdown.clone()).await;
            let _ = tokio::select! {
                biased;
                _ = stopped(&mut connection_shutdown) => Err(()),
                sent = sender.send(Event::Closed(id)) => sent.map_err(|_| ()),
            };
            drop(permit);
        });
    };
    shutdown_sender.send_replace(true);
    drop(pending.take());
    let mut joined = Ok(());
    while let Some(result) = peers.join_next().await {
        if let Err(error) = result
            && joined.is_ok()
        {
            joined = Err(io::Error::other(format!(
                "context connection task failed: {error}"
            )));
        }
    }
    drop(events);
    let retired = publication.retire();
    combine_owner_results(primary, joined, retired)
}

fn combine_owner_results(
    primary: io::Result<()>,
    joined: io::Result<()>,
    retired: io::Result<()>,
) -> io::Result<()> {
    let mut failure = primary.err().or_else(|| joined.err());
    if let Err(retirement) = retired {
        failure = Some(match failure {
            Some(primary) => io::Error::new(
                primary.kind(),
                format!("{primary}; context publication retirement failed: {retirement}"),
            ),
            None => retirement,
        });
    }
    failure.map_or(Ok(()), Err)
}

async fn replacement(
    address: &Path,
    shutdown: &mut watch::Receiver<bool>,
) -> io::Result<NamedPipeServer> {
    loop {
        match instance(address, false) {
            Ok(pipe) => return Ok(pipe),
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                tokio::select! {
                    biased;
                    _ = stopped(shutdown) => return Err(io::Error::new(io::ErrorKind::Interrupted, "context transport stopped")),
                    _ = tokio::time::sleep(CONNECT_RETRY) => {}
                }
            }
            Err(error) => return Err(error),
        }
    }
}

async fn connection(
    stream: Pipe<NamedPipeServer>,
    id: u64,
    events: mpsc::Sender<Event>,
    mut shutdown: watch::Receiver<bool>,
) {
    let (reader, mut writer) = tokio::io::split(stream);
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
            _ = stopped(&mut shutdown) => return,
            _ = revoked(&lease) => return,
            read = tokio::time::timeout(Duration::from_secs(if lease.is_some() { 300 } else { 10 }), read) => read,
        };
        if !matches!(read, Ok(Ok(1..))) || bytes.len() > FRAME_BYTES || bytes.last() != Some(&b'\n')
        {
            return;
        }
        let (reply, response) = oneshot::channel();
        let event = Event::Frame {
            connection: id,
            bytes,
            reply,
        };
        if !tokio::select! { biased; _ = stopped(&mut shutdown) => false, _ = revoked(&lease) => false, sent = events.send(event) => sent.is_ok() }
        {
            return;
        }
        let reply = tokio::select! {
            biased;
            _ = stopped(&mut shutdown) => return,
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
            _ = stopped(&mut shutdown) => return,
            _ = revoked(&lease) => return,
            written = tokio::time::timeout(Duration::from_secs(2), writer.write_all(&bytes)) => written,
        };
        if !matches!(written, Ok(Ok(()))) || reply.close {
            return;
        }
    }
}

async fn stopped(shutdown: &mut watch::Receiver<bool>) {
    if *shutdown.borrow_and_update() {
        return;
    }
    while shutdown.changed().await.is_ok() {
        if *shutdown.borrow_and_update() {
            return;
        }
    }
}

pub(crate) struct NativeConnection(Pipe<NamedPipeClient>);
impl NativeConnection {
    pub(crate) fn peer(&self) -> &Arc<PinnedProcess> {
        &self.0.peer
    }
}
impl AsyncRead for NativeConnection {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buffer)
    }
}
impl AsyncWrite for NativeConnection {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

pub(crate) async fn connect(
    endpoint: &Path,
    expected: ProcessIdentity,
    deadline: Instant,
) -> io::Result<NativeConnection> {
    let address = validate_endpoint(endpoint)?;
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "context pipe connection timed out",
            ));
        }
        match ClientOptions::new()
            .pipe_mode(PipeMode::Byte)
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(address)
        {
            Ok(stream) => {
                let peer = pipe_peer(&stream, false)?;
                if peer.identity() != expected {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "context pipe server identity changed",
                    ));
                }
                return Ok(NativeConnection(Pipe::new(stream, peer)));
            }
            Err(error) if matches!(error.raw_os_error(), Some(code) if code == ERROR_FILE_NOT_FOUND as i32 || code == ERROR_PIPE_BUSY as i32) =>
            {
                tokio::time::sleep_until(deadline.min(Instant::now() + CONNECT_RETRY)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn validate_endpoint(endpoint: &Path) -> io::Result<&str> {
    let value = endpoint.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "context pipe endpoint is not Unicode",
        )
    })?;
    let token = value
        .strip_prefix(PREFIX)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid context pipe family"))?;
    if token.len() != 64
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid context pipe token",
        ));
    }
    Ok(value)
}

fn instance(endpoint: &Path, first: bool) -> io::Result<NamedPipeServer> {
    let address = validate_endpoint(endpoint)?;
    crate::windows_fs::with_private_security(|security| unsafe {
        ServerOptions::new()
            .access_inbound(true)
            .access_outbound(true)
            .pipe_mode(PipeMode::Byte)
            .first_pipe_instance(first)
            .max_instances(PIPE_INSTANCES)
            .reject_remote_clients(true)
            .in_buffer_size(PIPE_BUFFER_BYTES)
            .out_buffer_size(PIPE_BUFFER_BYTES)
            .create_with_security_attributes_raw(
                address,
                (security as *const windows_sys::Win32::Security::SECURITY_ATTRIBUTES)
                    .cast_mut()
                    .cast(),
            )
    })
}

fn pipe_peer(pipe: &impl AsRawHandle, server: bool) -> io::Result<Arc<PinnedProcess>> {
    let mut pid = 0;
    let result = unsafe {
        if server {
            GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut pid)
        } else {
            GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid)
        }
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Arc::new(PinnedProcess::open_peer(pid)?))
}

struct Pipe<S: AsRawHandle> {
    stream: S,
    peer: Arc<PinnedProcess>,
}
impl<S: AsRawHandle> Pipe<S> {
    fn new(stream: S, peer: Arc<PinnedProcess>) -> Self {
        Self { stream, peer }
    }
}
impl<S: AsRawHandle> Drop for Pipe<S> {
    fn drop(&mut self) {
        unsafe {
            CancelIoEx(self.stream.as_raw_handle() as HANDLE, std::ptr::null());
        }
    }
}
impl<S: AsyncRead + AsRawHandle + Unpin> AsyncRead for Pipe<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buffer)
    }
}
impl<S: AsyncWrite + AsRawHandle + Unpin> AsyncWrite for Pipe<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, &bytes[..bytes.len().min(16 * 1024)])
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream)
            .poll_write(cx, &[])
            .map(|result| result.map(|_| ()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}
