// SPDX-License-Identifier: MPL-2.0

//! Private, authenticated local byte streams for the shared bundled protocol.
//! No lifecycle, detached-host startup, or frontend availability is enabled.

use super::{
    windows_endpoint::{EndpointMetadata, PipeAddress, PreparedEndpoint, Publication},
    windows_process_identity::PinnedProcess,
};
use std::{
    io,
    os::windows::io::AsRawHandle,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, PipeMode, ServerOptions,
    },
    sync::{OwnedSemaphorePermit, Semaphore},
    time::{Instant, sleep_until, timeout_at},
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, HANDLE},
    Storage::FileSystem::SECURITY_IDENTIFICATION,
    System::{
        IO::CancelIoEx,
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
    },
};

pub const MAX_CONNECTIONS: usize = 16;
const MAX_INSTANCES: usize = MAX_CONNECTIONS + 1;
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;
const WRITE_CHUNK_BYTES: usize = 16 * 1024;
const CONNECT_RETRY: Duration = Duration::from_millis(10);

/// Owns only the pending listener instance and its endpoint publication.
/// Returned connections belong to the caller and can outlive this value. The
/// future host supervisor must close them before retiring a live publication.
#[derive(Debug)]
pub struct Listener {
    // Drop pending first. Publication cleanup does not claim to close streams
    // already handed to the caller or to terminate any peer process.
    pending: Option<NamedPipeServer>,
    retiring: Option<io::Error>,
    publication: Publication,
    slots: Arc<Semaphore>,
}

impl Listener {
    /// Must run inside a Tokio I/O runtime. Creates the first private instance
    /// while PreparedEndpoint still owns every publication guard.
    pub fn bind(prepared: PreparedEndpoint) -> io::Result<Self> {
        let pending = instance(&prepared.metadata().address, true)?;
        let publication = prepared.publish()?;
        Ok(Self {
            pending: Some(pending),
            retiring: None,
            publication,
            slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        })
    }

    pub fn metadata(&self) -> &EndpointMetadata {
        self.publication.metadata()
    }

    /// Whether this listener still owns its pending instance. This reflects
    /// permanent replacement-failure poisoning, not a transport liveness probe.
    pub fn is_usable(&self) -> bool {
        self.pending.is_some()
    }

    /// Cancellation retains the pending instance and any recorded refusal.
    /// Saturation returns WouldBlock before connecting or authenticating it;
    /// one waiting client may remain connected until an admission slot is free.
    /// Refused instances are always replaced, never reused across peer streams.
    pub async fn accept(&mut self, deadline: Instant) -> io::Result<Connection<NamedPipeServer>> {
        self.accept_with(deadline, |address| instance(address, false), Instant::now)
            .await
    }

    async fn accept_with(
        &mut self,
        deadline: Instant,
        mut create: impl FnMut(&PipeAddress) -> io::Result<NamedPipeServer>,
        now: impl Fn() -> Instant,
    ) -> io::Result<Connection<NamedPipeServer>> {
        self.pending()?;
        check_deadline_at(deadline, now())?;
        // Reserve replacement capacity before connecting: at most fifteen
        // admitted streams, this pending instance and its fresh replacement.
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "native host connection capacity is exhausted",
            )
        })?;
        if self.retiring.is_some() {
            return Err(self.finish_refusal(deadline, &mut create, &now).await);
        }
        let connected = timeout_at(deadline, self.pending()?.connect()).await;
        match connected {
            Err(_) => return Err(timed_out()),
            Ok(Err(error)) => {
                self.retiring = Some(error);
                return Err(self.finish_refusal(deadline, &mut create, &now).await);
            }
            Ok(Ok(())) => {}
        }
        if let Err(error) = check_deadline_at(deadline, now()) {
            self.retiring = Some(error);
            return Err(self.finish_refusal(deadline, &mut create, &now).await);
        }
        let peer = match peer(self.pending()?, true) {
            Ok(peer) => peer,
            Err(error) => {
                self.retiring = Some(error);
                return Err(self.finish_refusal(deadline, &mut create, &now).await);
            }
        };
        let replacement = match self.replacement(deadline, &mut create, &now).await {
            Ok(pipe) => pipe,
            Err(error) => {
                // The deadline can expire while IOCP releases a previously
                // retired instance. Preserve ownership and refusal across the
                // next accept, rather than admitting this expired attempt.
                if self.pending.is_some() {
                    self.retiring = Some(timed_out());
                }
                return Err(error);
            }
        };
        let stream = self
            .pending
            .replace(replacement)
            .expect("pending instance is owned");
        // Even a synchronous native create can overrun the supplied deadline.
        // A replacement already exists, so dropping this old stream is safe.
        check_deadline_at(deadline, now())?;
        Ok(Connection {
            stream,
            peer,
            _permit: Some(permit),
        })
    }

    fn pending(&self) -> io::Result<&NamedPipeServer> {
        self.pending.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "native pipe listener is unusable after replacement failure",
            )
        })
    }

    async fn replacement(
        &mut self,
        deadline: Instant,
        create: &mut impl FnMut(&PipeAddress) -> io::Result<NamedPipeServer>,
        now: &impl Fn() -> Instant,
    ) -> io::Result<NamedPipeServer> {
        loop {
            check_deadline_at(deadline, now())?;
            match create(&self.metadata().address) {
                Ok(pipe) => return Ok(pipe),
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                    // A dropped Mio instance can retain its native handle until
                    // the runtime dispatches cancellation completions. Wait only
                    // for this documented capacity condition, within the caller's
                    // budget; no additional native instance can exceed max17.
                    sleep_until(deadline.min(Instant::now() + CONNECT_RETRY)).await;
                }
                Err(error) => {
                    self.pending.take();
                    self.retiring.take();
                    return Err(error);
                }
            }
        }
    }

    async fn finish_refusal(
        &mut self,
        deadline: Instant,
        create: &mut impl FnMut(&PipeAddress) -> io::Result<NamedPipeServer>,
        now: &impl Fn() -> Instant,
    ) -> io::Error {
        match self.replacement(deadline, create, now).await {
            Ok(replacement) => {
                // Never reconnect a Mio pipe: it may retain prefetched bytes or
                // a read completion/error belonging to the refused predecessor.
                self.pending.replace(replacement);
                self.retiring
                    .take()
                    .expect("refusal recorded before awaiting")
            }
            Err(error) => error,
        }
    }
}

/// Connects only to validated local metadata, then verifies the actual server
/// PID, creation FILETIME, and current account using one retained process handle.
/// Only missing/busy instances are retried. Dropping the future cancels retry
/// sleeps; failed admission drops any opened stream before returning.
pub async fn connect(
    metadata: &EndpointMetadata,
    deadline: Instant,
) -> io::Result<Connection<NamedPipeClient>> {
    metadata.validate()?;
    loop {
        check_deadline(deadline)?;
        let stream = match ClientOptions::new()
            .pipe_mode(PipeMode::Byte)
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(metadata.address.as_str())
        {
            Ok(stream) => stream,
            Err(error) if matches!(error.raw_os_error(), Some(code) if code == ERROR_FILE_NOT_FOUND as i32 || code == ERROR_PIPE_BUSY as i32) =>
            {
                sleep_until(deadline.min(Instant::now() + CONNECT_RETRY)).await;
                continue;
            }
            Err(error) => return Err(error),
        };
        let peer = peer(&stream, false)?;
        if peer.identity() != metadata.process {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "native pipe server does not match its published process identity",
            ));
        }
        check_deadline(deadline)?;
        return Ok(Connection {
            stream,
            peer,
            _permit: None,
        });
    }
}

/// The stream, actual native peer proof and server admission permit share one
/// lifetime. No raw-parts API permits dropping the proof/permit before the I/O.
/// Writes accept at most 16 KiB per call. Flush waits for preceding nonempty
/// writes to complete in the native pipe, not for the peer to consume them.
/// Drop requests cancellation of outstanding I/O; the Tokio runtime must still
/// dispatch completion packets before the underlying native handle is released.
#[derive(Debug)]
pub struct Connection<S: AsRawHandle> {
    stream: S,
    peer: Arc<PinnedProcess>,
    _permit: Option<OwnedSemaphorePermit>,
}

impl<S: AsRawHandle> Connection<S> {
    pub fn peer(&self) -> &Arc<PinnedProcess> {
        &self.peer
    }
}

impl<S: AsRawHandle> Drop for Connection<S> {
    fn drop(&mut self) {
        // Mio cancels reads/connects on drop, but deliberately leaves writes
        // pending to flush them. An unread peer must not retain our write buffer
        // and native handle indefinitely. Keep Mio's completion-owned buffers
        // alive: request cancellation, then let the underlying stream drop.
        // PIPE_WAIT (the unchanged server/client default) completes a successful
        // pending write only after all bytes fit; unlike PIPE_NOWAIT, a successful
        // partial write cannot schedule another remainder after cancellation.
        unsafe {
            CancelIoEx(self.stream.as_raw_handle(), std::ptr::null());
        }
    }
}

impl<S: AsyncRead + AsRawHandle + Unpin> AsyncRead for Connection<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buffer)
    }
}

impl<S: AsyncWrite + AsRawHandle + Unpin> AsyncWrite for Connection<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let bytes = &bytes[..bytes.len().min(WRITE_CHUNK_BYTES)];
        Pin::new(&mut self.get_mut().stream).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Tokio named-pipe flush/shutdown are no-ops. Mio checks its previous
        // write/error before accepting even an empty write. This waits for that
        // completion without queuing more payload; the zero-byte completion may
        // itself remain pending. Do not return Pending after accepting bytes.
        Pin::new(&mut self.get_mut().stream)
            .poll_write(cx, &[])
            .map(|result| result.map(|_| ()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}

fn instance(address: &PipeAddress, first: bool) -> io::Result<NamedPipeServer> {
    crate::windows_fs::with_private_security(|security| unsafe {
        ServerOptions::new()
            .access_inbound(true)
            .access_outbound(true)
            .pipe_mode(PipeMode::Byte)
            .first_pipe_instance(first)
            .max_instances(MAX_INSTANCES)
            .reject_remote_clients(true)
            .in_buffer_size(PIPE_BUFFER_BYTES)
            .out_buffer_size(PIPE_BUFFER_BYTES)
            .create_with_security_attributes_raw(
                address.as_str(),
                (security as *const windows_sys::Win32::Security::SECURITY_ATTRIBUTES)
                    .cast_mut()
                    .cast(),
            )
    })
}

fn peer(pipe: &impl AsRawHandle, server: bool) -> io::Result<Arc<PinnedProcess>> {
    let mut pid = 0;
    let handle: HANDLE = pipe.as_raw_handle();
    let result = unsafe {
        if server {
            GetNamedPipeClientProcessId(handle, &mut pid)
        } else {
            GetNamedPipeServerProcessId(handle, &mut pid)
        }
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Arc::new(PinnedProcess::open_peer(pid)?))
}

fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "native pipe operation timed out")
}
fn check_deadline(deadline: Instant) -> io::Result<()> {
    check_deadline_at(deadline, Instant::now())
}
fn check_deadline_at(deadline: Instant, now: Instant) -> io::Result<()> {
    if now >= deadline {
        Err(timed_out())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
