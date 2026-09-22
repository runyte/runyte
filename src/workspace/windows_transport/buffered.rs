// SPDX-License-Identifier: MPL-2.0

//! A native duplex connection whose runtime is independent of terminal drawing.

use std::{future::Future, sync::Arc, thread, time::Duration};

use anyhow::{Context, Result, anyhow, ensure};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, oneshot, watch},
    time::Instant,
};

use super::{ClientRequest, HostResponse, PROTOCOL_VERSION, client::client_hello};
use crate::{
    app::FrameGeometry,
    workspace::{
        transport_shared::{
            EncodedMessage, FrameCoalescer, MessageReader, ResponseReceiver, ResponseSender,
            encode_message, response_channel, write_encoded_message, write_message,
        },
        windows_endpoint::EndpointMetadata,
        windows_pipe,
        windows_process_identity::PinnedProcess,
    },
};

const CONNECT_BUDGET: Duration = Duration::from_secs(2);
const OUTGOING_CAPACITY: usize = 1;

pub struct BufferedLocalClient {
    outgoing: Option<OutgoingCapability>,
    responses: ResponseReceiver,
    worker: Worker,
    peer: Option<Arc<PinnedProcess>>,
}

struct OutgoingCapability {
    requests: mpsc::Sender<Outgoing>,
    cancel: watch::Sender<bool>,
}

struct Outgoing {
    message: EncodedMessage,
    acknowledgement: oneshot::Sender<Result<()>>,
}

/// Taking the capability before the first await makes every abandoned send
/// permanent, including cancellation after a successful but unobserved ack.
struct Sending(Option<OutgoingCapability>);

impl Drop for Sending {
    fn drop(&mut self) {
        if let Some(capability) = &self.0 {
            capability.cancel.send_replace(true);
        }
    }
}

struct Worker {
    stop: watch::Sender<bool>,
    thread: Option<thread::JoinHandle<Result<()>>>,
}

impl Worker {
    fn finish(&mut self) -> Result<()> {
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| anyhow!("native buffered transport worker panicked"))??;
        }
        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        let _ = self.finish();
    }
}

impl BufferedLocalClient {
    /// The dedicated thread creates and registers the pipe on its own runtime.
    /// Success means authentication and the complete Hello write both finished.
    pub async fn connect_with_handoff(
        metadata: &EndpointMetadata,
        geometry: FrameGeometry,
        directory_handoff: bool,
    ) -> Result<Self> {
        metadata.validate()?;
        ensure!(
            metadata.protocol == PROTOCOL_VERSION,
            "workspace host protocol {} is incompatible with client protocol {}",
            metadata.protocol,
            PROTOCOL_VERSION
        );
        let hello = client_hello(metadata, geometry, true, directory_handoff);
        let metadata = metadata.clone();
        Self::start_with_peer(
            move || async move {
                let stream =
                    windows_pipe::connect(&metadata, Instant::now() + CONNECT_BUDGET).await?;
                let peer = Arc::clone(stream.peer());
                Ok((stream, Some(peer)))
            },
            hello,
        )
        .await
    }

    #[cfg(test)]
    async fn start<F, C, S>(connect: F, hello: ClientRequest) -> Result<Self>
    where
        F: FnOnce() -> C + Send + 'static,
        C: Future<Output = Result<S>>,
        S: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        Self::start_with_peer(move || async move { Ok((connect().await?, None)) }, hello).await
    }

    async fn start_with_peer<F, C, S>(connect: F, hello: ClientRequest) -> Result<Self>
    where
        F: FnOnce() -> C + Send + 'static,
        C: Future<Output = Result<(S, Option<Arc<PinnedProcess>>)>>,
        S: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        let (responses, response_rx) = response_channel();
        let (requests, request_rx) = mpsc::channel(OUTGOING_CAPACITY);
        let (cancel, cancel_rx) = watch::channel(false);
        let (stop, mut stop_rx) = watch::channel(false);
        let (startup, started) = oneshot::channel();
        let thread = thread::Builder::new()
            .name("runyte-native-transport".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("cannot start native buffered transport runtime")?;
                runtime.block_on(async move {
                    let (mut stream, peer) = tokio::select! {
                        biased;
                        _ = cancelled(&mut stop_rx) => return Ok(()),
                        connected = connect() => connected?,
                    };
                    tokio::select! {
                        biased;
                        _ = cancelled(&mut stop_rx) => return Ok(()),
                        result = write_message(&mut stream, &hello) => result?,
                    }
                    if startup.send(peer).is_err() {
                        return Ok(());
                    }
                    run_connected(stream, responses, request_rx, cancel_rx, stop_rx).await
                })
            })
            .context("cannot start native buffered transport thread")?;
        // This guard exists before awaiting startup. Its Drop interrupts any
        // connect/Hello wait and joins, including an abandoned startup future.
        let mut worker = Worker {
            stop,
            thread: Some(thread),
        };
        let peer = match started.await {
            Ok(peer) => peer,
            Err(_) => {
                worker.finish()?;
                return Err(anyhow!("native buffered transport stopped before startup"));
            }
        };
        Ok(Self {
            outgoing: Some(OutgoingCapability { requests, cancel }),
            responses: response_rx,
            worker,
            peer,
        })
    }

    /// Actual authenticated pipe peer retained at connect, never metadata PID.
    pub fn peer(&self) -> Option<&Arc<PinnedProcess>> {
        self.peer.as_ref()
    }

    /// Failure or cancellation disables all later sends. Incoming responses
    /// remain available for bounded wait-completion recovery; Windows pipes do
    /// not provide the Unix half-close contract.
    pub async fn send(&mut self, request: &ClientRequest) -> Result<()> {
        let capability = self
            .outgoing
            .take()
            .context("workspace transport writer is closed")?;
        let mut sending = Sending(Some(capability));
        // Serialize once through the bounded encoder before the queue takes
        // ownership. Escaping expansion counts against the actual wire limit.
        let message = encode_message(request)?;
        let (acknowledgement, acknowledged) = oneshot::channel();
        sending
            .0
            .as_ref()
            .expect("send capability is owned")
            .requests
            .send(Outgoing {
                message,
                acknowledgement,
            })
            .await
            .context("native buffered transport writer stopped")?;
        acknowledged
            .await
            .context("native buffered transport write acknowledgement lost")??;
        // No await may separate successful acknowledgement from restoration.
        self.outgoing = sending.0.take();
        Ok(())
    }

    pub async fn recv_handshake(&mut self) -> Result<Option<HostResponse>> {
        if let Some(response) = self.responses.recv_handshake().await {
            return Ok(Some(response));
        }
        // Handshake admission is semantic-only, even if a malformed peer left
        // visual/final slots queued before EOF. Ordinary recv can still consume
        // those slots; they must never stand in for the missing Welcome here.
        self.worker.finish()?;
        Ok(None)
    }

    /// Cancellation retains complete queued responses and any partially read
    /// frame in the dedicated reader. The terminal worker error is returned once,
    /// after all semantic, final and coalesced visual responses are consumed.
    pub async fn recv(&mut self) -> Result<Option<HostResponse>> {
        if let Some(response) = self.responses.recv().await {
            self.responses.mark_delivered();
            return Ok(Some(response));
        }
        self.worker.finish()?;
        Ok(None)
    }
}

impl Drop for BufferedLocalClient {
    fn drop(&mut self) {
        // Wake the independent runtime before dropping bounded queue owners.
        // Worker Drop joins; it never detaches a timed-out native thread.
        self.worker.stop.send_replace(true);
    }
}

async fn cancelled(signal: &mut watch::Receiver<bool>) {
    loop {
        if *signal.borrow() {
            return;
        }
        if signal.changed().await.is_err() {
            return;
        }
    }
}

async fn run_connected<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    responses: ResponseSender,
    requests: mpsc::Receiver<Outgoing>,
    cancel: watch::Receiver<bool>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let (reader, writer) = tokio::io::split(stream);
    let read = read_responses(reader, responses);
    let write = write_requests(writer, requests, cancel);
    tokio::pin!(read, write);
    tokio::select! {
        biased;
        _ = cancelled(&mut stop) => Ok(()),
        result = &mut read => result,
        () = &mut write => {
            // A failed/cancelled send poisons only outgoing requests. Keep the
            // read half and already queued responses for caller recovery.
            tokio::select! {
                biased;
                _ = cancelled(&mut stop) => Ok(()),
                result = &mut read => result,
            }
        }
    }
}

async fn read_responses<R: AsyncRead + Unpin>(reader: R, responses: ResponseSender) -> Result<()> {
    let mut reader = MessageReader::new(reader);
    let mut frames = FrameCoalescer::default();
    while let Some(response) = reader.read::<HostResponse>().await? {
        responses
            .send(frames.coalesce(response))
            .await
            .context("native response consumer stopped")?;
    }
    Ok(())
}

async fn write_requests<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut requests: mpsc::Receiver<Outgoing>,
    mut cancel: watch::Receiver<bool>,
) {
    loop {
        let outgoing = tokio::select! {
            biased;
            _ = cancelled(&mut cancel) => return,
            request = requests.recv() => {
                let Some(request) = request else { return; };
                request
            }
        };
        let mut acknowledgement = outgoing.acknowledgement;
        let result = tokio::select! {
            biased;
            _ = cancelled(&mut cancel) => return,
            _ = acknowledgement.closed() => return,
            result = write_encoded_message(&mut writer, &outgoing.message) => result,
        };
        let successful = result.is_ok();
        if acknowledgement.send(result).is_err() || !successful {
            return;
        }
    }
}

#[cfg(test)]
mod tests;
