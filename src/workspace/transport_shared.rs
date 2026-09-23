// SPDX-License-Identifier: MPL-2.0

//! Shared bounded framing, response delivery, and protocol admission.
//!
//! Endpoint discovery, OS peer authentication, connection ownership, and
//! shutdown belong to each platform adapter. This module performs no OS I/O
//! except through the supplied AsyncRead/AsyncWrite stream. Unix keeps its
//! existing API; a native adapter must supply a retained authenticated peer.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::mpsc,
};

use crate::{
    app::FrameGeometry,
    protocol::{
        ClientKind, ClientRequest, ClientRole, FeatureGroup, HostResponse, MAX_FEATURE_GROUPS,
    },
};

const PROTOCOL_VERSION: u32 = crate::protocol::VERSION;
pub(super) const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
/// Deep enough that an ordinary burst of frames — one per keystroke, plus
/// overlay redraws — never backs up against a client that is still draining.
/// Reaching this depth means the client has genuinely stopped reading.
pub(super) const RESPONSE_CAPACITY: usize = 64;
/// How long a peer may accept no bytes at all before its connection is
/// abandoned. A peer that has stopped reading must not retain its connection
/// task or keep the host's interactive attachment occupied indefinitely, but
/// the budget measures a stall rather than a whole message: see
/// `write_message_with_timeout`.
pub(super) const CONNECTION_WRITE_STALL: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum ServerEvent<P = Option<u32>> {
    Connected {
        id: u64,
        /// Platform-authenticated peer value. The connection retains its own
        /// clone until teardown; Windows can carry an Arc<PinnedProcess> here.
        peer_process: P,
        geometry: FrameGeometry,
        interactive: bool,
        /// Whether this client can hand a `:quit-here` directory to its shell.
        directory_handoff: bool,
        responses: ResponseSender,
    },
    Request {
        id: u64,
        request: ClientRequest,
    },
    /// A decoded request that violates validation or role rules. This crosses
    /// the same FIFO event boundary as valid requests so its error response
    /// cannot overtake an earlier semantic response on an unnumbered stream.
    ProtocolError {
        id: u64,
        message: String,
    },
    /// The connection ended because its framing failed: a malformed or
    /// truncated message, or a write that could not be completed.
    ///
    /// Separate from [`ServerEvent::ProtocolError`], which is a decoded
    /// request the host answers. Nothing can be answered here — the stream is
    /// no longer readable — so this carries the reason to whatever records it
    /// and is always followed by [`ServerEvent::Disconnected`].
    TransportFailure {
        id: u64,
        message: String,
    },
    Disconnected {
        id: u64,
    },
}

/// Semantic responses retain FIFO order; complete frames and terminal damage
/// share one replaceable slot. A slow client therefore holds at most one
/// pending visual update while lifecycle/command replies remain explicit.
#[derive(Clone, Debug)]
pub struct ResponseSender {
    messages: mpsc::Sender<HostResponse>,
    final_messages: mpsc::Sender<HostResponse>,
    frame: tokio::sync::watch::Sender<Option<VisualResponse>>,
    next_visual: Arc<AtomicU64>,
    delivered_visual: Arc<AtomicU64>,
}

pub struct ResponseReceiver {
    messages: mpsc::Receiver<HostResponse>,
    final_messages: mpsc::Receiver<HostResponse>,
    pending_final: Option<HostResponse>,
    frame: tokio::sync::watch::Receiver<Option<VisualResponse>>,
    delivered_visual: Arc<AtomicU64>,
    in_flight_visual: Option<u64>,
    messages_closed: bool,
    final_messages_closed: bool,
    frame_closed: bool,
}

#[derive(Clone, Debug)]
struct VisualResponse {
    sequence: u64,
    response: HostResponse,
}

pub fn response_channel() -> (ResponseSender, ResponseReceiver) {
    let (messages, message_rx) = mpsc::channel(RESPONSE_CAPACITY);
    let (final_messages, final_message_rx) = mpsc::channel(1);
    let (frame, frame_rx) = tokio::sync::watch::channel(None);
    let next_visual = Arc::new(AtomicU64::new(0));
    let delivered_visual = Arc::new(AtomicU64::new(0));
    (
        ResponseSender {
            messages,
            final_messages,
            frame,
            next_visual,
            delivered_visual: delivered_visual.clone(),
        },
        ResponseReceiver {
            messages: message_rx,
            final_messages: final_message_rx,
            pending_final: None,
            frame: frame_rx,
            delivered_visual,
            in_flight_visual: None,
            messages_closed: false,
            final_messages_closed: false,
            frame_closed: false,
        },
    )
}

fn is_final_response(response: &HostResponse) -> bool {
    matches!(
        response,
        HostResponse::Detached { .. }
            | HostResponse::ShuttingDown
            | HostResponse::SwitchWorkspace { .. }
            | HostResponse::ParentSwitchWorkspace { .. }
            | HostResponse::NativeSwitchCommitted { .. }
    )
}

impl ResponseSender {
    /// Whether the replaceable visual slot contains a frame the connection
    /// task has not finished writing. New damage cannot use that unseen frame
    /// as a base; its replacement must be self-contained.
    pub fn visual_pending(&self) -> bool {
        self.frame
            .borrow()
            .as_ref()
            .is_some_and(|visual| visual.sequence != self.delivered_visual.load(Ordering::Acquire))
    }

    pub fn try_send(
        &self,
        response: HostResponse,
    ) -> Result<(), mpsc::error::TrySendError<HostResponse>> {
        if is_final_response(&response) {
            return self.final_messages.try_send(response);
        }
        if matches!(
            response,
            HostResponse::Frame { .. } | HostResponse::TerminalDamage { .. }
        ) {
            let visual = VisualResponse {
                sequence: self
                    .next_visual
                    .fetch_add(1, Ordering::Relaxed)
                    .wrapping_add(1),
                response,
            };
            return self.frame.send(Some(visual)).map_err(|error| {
                mpsc::error::TrySendError::Closed(
                    error.0.expect("visual response is present").response,
                )
            });
        }
        self.messages.try_send(response)
    }

    pub async fn send(
        &self,
        response: HostResponse,
    ) -> Result<(), mpsc::error::SendError<HostResponse>> {
        if is_final_response(&response) {
            return self.final_messages.send(response).await;
        }
        if matches!(
            response,
            HostResponse::Frame { .. } | HostResponse::TerminalDamage { .. }
        ) {
            let visual = VisualResponse {
                sequence: self
                    .next_visual
                    .fetch_add(1, Ordering::Relaxed)
                    .wrapping_add(1),
                response,
            };
            return self.frame.send(Some(visual)).map_err(|error| {
                mpsc::error::SendError(error.0.expect("visual response is present").response)
            });
        }
        self.messages.send(response).await
    }

    /// Thread-blocking counterpart used by the attached TUI's independent
    /// socket reader. Visual updates remain replaceable; only irreplaceable
    /// semantic and lifecycle replies apply bounded backpressure.
    #[cfg(unix)]
    pub(super) fn blocking_send(
        &self,
        response: HostResponse,
    ) -> Result<(), mpsc::error::SendError<HostResponse>> {
        if is_final_response(&response) {
            return self.final_messages.blocking_send(response);
        }
        if matches!(
            response,
            HostResponse::Frame { .. } | HostResponse::TerminalDamage { .. }
        ) {
            let visual = VisualResponse {
                sequence: self
                    .next_visual
                    .fetch_add(1, Ordering::Relaxed)
                    .wrapping_add(1),
                response,
            };
            return self.frame.send(Some(visual)).map_err(|error| {
                mpsc::error::SendError(error.0.expect("visual response is present").response)
            });
        }
        self.messages.blocking_send(response)
    }
}

impl ResponseReceiver {
    /// A final reply can become ready after `select!` has polled the semantic
    /// lane empty. Recheck that lane before returning the final reply, and
    /// retain the one final reply if a semantic response arrived meanwhile.
    fn semantic_before_final(&mut self, final_response: HostResponse) -> HostResponse {
        if !self.messages_closed {
            match self.messages.try_recv() {
                Ok(response) => {
                    self.pending_final = Some(final_response);
                    return response;
                }
                Err(mpsc::error::TryRecvError::Disconnected) => self.messages_closed = true,
                Err(mpsc::error::TryRecvError::Empty) => {}
            }
        }
        final_response
    }

    /// Receives the semantic response that establishes a connection before
    /// visual updates are allowed onto the wire.
    ///
    /// A biased `select!` only prioritizes branches that are ready when they
    /// are polled. If the semantic queue is empty at that instant and a frame
    /// arrives before the watch branch is polled, the frame can otherwise win
    /// even when the host queued `Welcome` first. Waiting on the semantic
    /// queue alone makes the handshake ordering an explicit transport rule.
    pub(super) async fn recv_handshake(&mut self) -> Option<HostResponse> {
        if self.messages_closed {
            return None;
        }
        let response = self.messages.recv().await;
        if response.is_none() {
            self.messages_closed = true;
        }
        response
    }

    pub(super) async fn recv(&mut self) -> Option<HostResponse> {
        self.in_flight_visual = None;
        loop {
            if let Some(response) = self.pending_final.take() {
                return Some(self.semantic_before_final(response));
            }
            tokio::select! {
                biased;
                response = self.messages.recv(), if !self.messages_closed => {
                    match response {
                        Some(response) => return Some(response),
                        None => self.messages_closed = true,
                    }
                }
                response = self.final_messages.recv(), if !self.final_messages_closed => {
                    match response {
                        Some(response) => return Some(self.semantic_before_final(response)),
                        None => self.final_messages_closed = true,
                    }
                }
                changed = self.frame.changed(), if !self.frame_closed => {
                    match changed {
                        Ok(()) => {
                            if let Some(visual) = self.frame.borrow_and_update().clone() {
                                self.in_flight_visual = Some(visual.sequence);
                                return Some(visual.response);
                            }
                        }
                        Err(_) => self.frame_closed = true,
                    }
                }
                else => return None,
            }
        }
    }

    pub(super) fn mark_delivered(&mut self) {
        if let Some(sequence) = self.in_flight_visual.take() {
            self.delivered_visual.store(sequence, Ordering::Release);
        }
    }
}

/// Folds terminal damage before entering a replaceable response slot. The
/// native reader owns this value independently of the frontend renderer.
#[derive(Default)]
pub(super) struct FrameCoalescer {
    latest_frame: Option<crate::protocol::HostFrame>,
}

impl FrameCoalescer {
    pub(super) fn coalesce(&mut self, response: HostResponse) -> HostResponse {
        // Socket delivery lets the host use that frame as the base of later
        // terminal damage, but the synchronous frontend may not have rendered
        // it yet. Fold every received delta into the reader's latest complete
        // frame before entering the local replaceable slot. Replacing one
        // complete frame with another is always safe, however far the renderer
        // is behind.
        match response {
            HostResponse::Frame { frame } => {
                self.latest_frame = Some((*frame).clone());
                HostResponse::Frame { frame }
            }
            HostResponse::TerminalDamage { damage } => {
                if let Some(frame) = self.latest_frame.as_mut()
                    && damage.apply(frame)
                {
                    HostResponse::Frame {
                        frame: Box::new(frame.clone()),
                    }
                } else {
                    HostResponse::TerminalDamage { damage }
                }
            }
            response => response,
        }
    }
}

/// Keeps the existing Unix connection API and its inferred Option<u32> peer.
/// Native adapters call serve_connection_with_peer with a retained OS proof.
#[cfg(unix)]
pub(super) async fn serve_connection<S>(
    id: u64,
    stream: S,
    events: mpsc::Sender<ServerEvent>,
    expected_project_root: Vec<u8>,
    peer_process: Option<u32>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    serve_connection_with_peer(id, stream, events, expected_project_root, peer_process).await
}

pub(super) async fn serve_connection_with_peer<S, P>(
    id: u64,
    stream: S,
    events: mpsc::Sender<ServerEvent<P>>,
    expected_project_root: Vec<u8>,
    peer_process: P,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    P: Clone + std::fmt::Debug + Send + Sync + 'static,
{
    let (reader, writer) = tokio::io::split(stream);
    let mut reader = MessageReader::new(reader);
    let hello = tokio::time::timeout(Duration::from_secs(2), reader.read::<ClientRequest>())
        .await
        .context("workspace client handshake timed out")??;
    let Some(hello) = hello else {
        bail!("workspace client disconnected before its handshake")
    };
    if let Err(message) = hello.validate() {
        let mut writer = writer;
        write_refusal(
            &mut writer,
            &mut reader,
            format!("invalid client handshake: {message}"),
        )
        .await?;
        return Ok(());
    }
    let ClientRequest::Hello {
        protocol,
        features,
        project_root_bytes,
        client_kind,
        client_version,
        role,
        geometry,
        directory_handoff,
    } = hello
    else {
        bail!("workspace client did not begin with a handshake")
    };
    let (responses, mut response_rx) = response_channel();
    if protocol != PROTOCOL_VERSION {
        let mut writer = writer;
        write_refusal(
            &mut writer,
            &mut reader,
            format!(
                "client protocol {protocol} is incompatible with host protocol {PROTOCOL_VERSION}"
            ),
        )
        .await?;
        return Ok(());
    }
    if project_root_bytes != expected_project_root {
        let mut writer = writer;
        write_refusal(
            &mut writer,
            &mut reader,
            "client requested a different workspace".to_owned(),
        )
        .await?;
        return Ok(());
    }
    let interactive = role == ClientRole::Interactive;
    let identity_matches_role = matches!(
        (client_kind, role),
        (ClientKind::Tui, ClientRole::Interactive) | (ClientKind::Control, ClientRole::Control)
    );
    let expected_features: &[FeatureGroup] = if interactive {
        &[
            FeatureGroup::Snapshots,
            FeatureGroup::Input,
            FeatureGroup::Buffers,
            FeatureGroup::Wait,
        ]
    } else {
        &[
            FeatureGroup::Control,
            FeatureGroup::Buffers,
            FeatureGroup::Wait,
        ]
    };
    // The client must support everything its role needs, but may advertise
    // more in any order: a later bundled client can gain a feature group
    // without the host having to refuse it outright. Compatibility itself is
    // still gated by `PROTOCOL_VERSION` above.
    let features_cover_role = expected_features
        .iter()
        .all(|expected| features.contains(expected));
    if !identity_matches_role
        || client_version.len() > 128
        || features.len() > MAX_FEATURE_GROUPS
        || !features_cover_role
    {
        let mut writer = writer;
        write_refusal(
            &mut writer,
            &mut reader,
            "client handshake identity or feature set is invalid".to_owned(),
        )
        .await?;
        return Ok(());
    }
    events
        .send(ServerEvent::Connected {
            id,
            peer_process: peer_process.clone(),
            geometry: geometry.into(),
            interactive,
            directory_handoff,
            responses,
        })
        .await
        .context("workspace host stopped")?;
    let mut writer = writer;
    let result: Result<()> = async {
        let response = response_rx
            .recv_handshake()
            .await
            .context("workspace host stopped before answering the client handshake")?;
        write_message(&mut writer, &response).await?;
        #[cfg(windows)]
        if matches!(response, HostResponse::Refused { .. }) {
            retain_refused_peer(&mut reader).await;
            return Ok(());
        }
        loop {
            tokio::select! {
                // Semantic replies are bounded and must drain before another
                // ready request can advance teardown. In particular, a wait
                // client's periodic status poll can otherwise win this race
                // after the host has queued WaitState and ShuttingDown, then
                // observe the socket closing before either reply is written.
                // Visual responses already occupy one replaceable slot, so
                // this priority cannot starve reads indefinitely.
                biased;
                response = response_rx.recv() => {
                    let Some(response) = response else { break };
                    write_message(&mut writer, &response).await?;
                    response_rx.mark_delivered();
                }
                request = reader.read::<ClientRequest>() => {
                    match request? {
                        Some(ClientRequest::Hello { .. }) => {
                            if events.send(ServerEvent::ProtocolError {
                                id,
                                message: "handshake is only valid once".to_owned(),
                            }).await.is_err() {
                                break;
                            }
                        }
                        Some(request) => {
                            let protocol_error = request
                                .validate()
                                .err()
                                .or_else(|| (!request_allowed_for_role(&request, role)).then(|| {
                                    "request is not valid for this connection role".to_owned()
                                }));
                            let event = protocol_error.map_or(
                                ServerEvent::Request { id, request },
                                |message| ServerEvent::ProtocolError { id, message },
                            );
                            if events.send(event).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        }
        Ok(())
    }
    .await;
    // The reason the loop ended is discarded by the caller that spawned this
    // task, so it is reported here or nowhere. A host would otherwise see a
    // truncated frame as an ordinary disconnection. A peer that simply went
    // away is the opposite case: the write that observes it fails, but the
    // connection ended the way connections are meant to, so only
    // `Disconnected` follows.
    if let Some(error) = result.as_ref().err().filter(|error| !peer_hung_up(error)) {
        let _ = events
            .send(ServerEvent::TransportFailure {
                id,
                message: error.to_string(),
            })
            .await;
    }
    let _ = events.send(ServerEvent::Disconnected { id }).await;
    // Retain the OS peer proof even if the host discards its Connected event.
    drop(peer_process);
    result
}

async fn write_refusal<W: AsyncWrite + Unpin, R: AsyncRead + Unpin>(
    writer: &mut W,
    reader: &mut MessageReader<R>,
    message: String,
) -> Result<()> {
    write_message(writer, &HostResponse::Refused { message }).await?;
    #[cfg(windows)]
    retain_refused_peer(reader).await;
    #[cfg(not(windows))]
    let _ = reader;
    Ok(())
}

/// Keep a native pipe alive while its client reads a terminal refusal. The
/// first byte, EOF, or read error ends the wait; no later request is decoded or
/// dispatched. One absolute deadline bounds a client that leaves the pipe open.
#[cfg(windows)]
async fn retain_refused_peer<R: AsyncRead + Unpin>(reader: &mut MessageReader<R>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let _ = tokio::time::timeout_at(deadline, reader.reader.fill_buf()).await;
}

/// Whether a connection ended because the peer closed it rather than because
/// the transport itself broke.
///
/// A client that has what it came for exits, and the host's next write to it
/// fails with a broken pipe or a reset. That is the ordinary end of a
/// connection reported from the only side still holding it, so it must not be
/// recorded as a transport failure; a truncated frame or a stalled write
/// still is, because those name a peer that is present and wrong.
fn peer_hung_up(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|error| {
            matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
            )
        })
    })
}

pub(super) fn request_allowed_for_role(request: &ClientRequest, role: ClientRole) -> bool {
    match request {
        ClientRequest::Hello { .. } => false,
        #[cfg(windows)]
        ClientRequest::FrameDrawn { .. } => role == ClientRole::Interactive,
        ClientRequest::Input { .. }
        | ClientRequest::Invoke { .. }
        | ClientRequest::VisitDestination { .. }
        | ClientRequest::Notify { .. }
        | ClientRequest::AttachWait { .. }
        | ClientRequest::Pointer { .. }
        | ClientRequest::Resize { .. }
        | ClientRequest::Resynchronize
        | ClientRequest::NativeSwitchCommit { .. }
        | ClientRequest::NativeParentSwitchCommitObserved { .. }
        | ClientRequest::NativeSwitchAbort { .. }
        | ClientRequest::Detach => role == ClientRole::Interactive,
        ClientRequest::RenameHost { .. }
        | ClientRequest::ParentAttach { .. }
        | ClientRequest::ParentWait { .. }
        | ClientRequest::ParentHandoffResult { .. } => role == ClientRole::Control,
        ClientRequest::Health
        | ClientRequest::SessionPreview
        | ClientRequest::DestinationInventory
        | ClientRequest::ListBuffers
        | ClientRequest::ReadBuffer { .. }
        | ClientRequest::OpenBuffers { .. }
        | ClientRequest::ApplyTransaction { .. }
        | ClientRequest::SaveBuffer { .. }
        | ClientRequest::CloseBuffer { .. }
        | ClientRequest::CreateWait { .. }
        | ClientRequest::WaitStatus { .. }
        | ClientRequest::CompleteWaitBuffer { .. }
        | ClientRequest::CancelWait { .. }
        | ClientRequest::Shutdown
        | ClientRequest::ForceShutdown => true,
    }
}

/// A framed reader that owns the bytes of a partially received message.
///
/// Both transport loops read inside `tokio::select!`, which drops the losing
/// branch's future. Accumulating into a future-local buffer would discard
/// bytes already taken from the `BufReader` and resume mid-message, so the
/// partial message lives in the reader instead. `read` is therefore
/// cancellation-safe: its only await point is `fill_buf`, which is itself
/// cancellation-safe, and every byte consumed is already recorded in
/// `pending`.
pub(super) struct MessageReader<R> {
    reader: BufReader<R>,
    pending: Vec<u8>,
}

impl<R: AsyncRead + Unpin> MessageReader<R> {
    #[cfg(all(test, windows))]
    pub(super) fn pending_bytes_for_test(&self) -> usize {
        self.pending.len()
    }

    pub(super) fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            pending: Vec::new(),
        }
    }

    pub(super) async fn read<T: DeserializeOwned>(&mut self) -> Result<Option<T>> {
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                if self.pending.is_empty() {
                    return Ok(None);
                }
                let pending = self.pending.len();
                self.pending = Vec::new();
                bail!("workspace transport ended {pending} bytes inside a message");
            }
            let take = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |position| position + 1);
            if self.pending.len().saturating_add(take) > MAX_MESSAGE_BYTES {
                self.pending = Vec::new();
                bail!("workspace transport message exceeds {MAX_MESSAGE_BYTES} bytes");
            }
            self.pending.extend_from_slice(&available[..take]);
            self.reader.consume(take);
            if self.pending.last() == Some(&b'\n') {
                self.pending.pop();
                let message = serde_json::from_slice(&self.pending)
                    .context("malformed workspace transport message");
                self.pending = Vec::new();
                return message.map(Some);
            }
        }
    }
}

pub(super) async fn write_message<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    message: &T,
) -> Result<()> {
    write_message_with_timeout(writer, message, CONNECTION_WRITE_STALL).await
}

#[cfg(unix)]
pub(super) async fn write_client_message<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut Option<W>,
    message: &T,
) -> Result<()> {
    let mut live = writer
        .take()
        .context("workspace transport writer is closed")?;
    match write_message(&mut live, message).await {
        Ok(()) => {
            *writer = Some(live);
            Ok(())
        }
        Err(error) => {
            let _ = live.shutdown().await;
            Err(error)
        }
    }
}

/// Frames one message, giving up only on a peer that accepts nothing.
///
/// Abandoning a write is safe before its first byte and never after: half a
/// frame on the stream leaves the peer to read a message that ends inside
/// itself, which is a transport error for what may only be a slow reader. A
/// local socket send buffer is small enough on some platforms that a whole
/// editor frame cannot fit in it, so a deadline spanning the message would
/// truncate every frame a client took a moment too long to drain. The budget
/// therefore covers a single write and restarts whenever the peer accepts any
/// byte at all, which still ends a connection that has genuinely stopped
/// reading.
pub(super) async fn write_message_with_timeout<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    message: &T,
    stall: Duration,
) -> Result<()> {
    let message = encode_message(message)?;
    write_encoded_with_timeout(writer, &message, stall).await
}

/// A bounded JSON message with its required newline. Only the encoder can
/// construct this value, so an outgoing queue cannot admit an unframed payload.
pub(super) struct EncodedMessage(Vec<u8>);

pub(super) fn encode_message<T: Serialize>(message: &T) -> Result<EncodedMessage> {
    struct BoundedJson(Vec<u8>);
    impl std::io::Write for BoundedJson {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > (MAX_MESSAGE_BYTES - 1).saturating_sub(self.0.len()) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("workspace transport message exceeds {MAX_MESSAGE_BYTES} bytes"),
                ));
            }
            let needed = self.0.len() + bytes.len();
            if needed > self.0.capacity() {
                let capacity = needed
                    .saturating_add(1)
                    .max(self.0.capacity().saturating_mul(2))
                    .clamp(1024, MAX_MESSAGE_BYTES);
                // Bound the requested payload capacity, including newline room;
                // allocator bookkeeping/rounding remains allocator-owned.
                self.0.reserve_exact(capacity - self.0.len());
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut bytes = BoundedJson(Vec::new());
    serde_json::to_writer(&mut bytes, message)?;
    bytes.0.push(b'\n');
    Ok(EncodedMessage(bytes.0))
}

#[cfg(windows)]
pub(super) async fn write_encoded_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &EncodedMessage,
) -> Result<()> {
    write_encoded_with_timeout(writer, message, CONNECTION_WRITE_STALL).await
}

async fn write_encoded_with_timeout<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &EncodedMessage,
    stall: Duration,
) -> Result<()> {
    let bytes = &message.0;
    let mut written = 0;
    while written < bytes.len() {
        let count = tokio::time::timeout(stall, writer.write(&bytes[written..]))
            .await
            .context("workspace transport write timed out")??;
        ensure!(
            count > 0,
            "workspace transport peer stopped accepting bytes"
        );
        written += count;
    }
    tokio::time::timeout(stall, writer.flush())
        .await
        .context("workspace transport write timed out")??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn control_hello(project: Vec<u8>, protocol: u32) -> ClientRequest {
        ClientRequest::Hello {
            protocol,
            features: vec![
                FeatureGroup::Control,
                FeatureGroup::Buffers,
                FeatureGroup::Wait,
            ],
            project_root_bytes: project,
            client_kind: ClientKind::Control,
            client_version: crate::protocol::CLIENT_VERSION.to_owned(),
            role: ClientRole::Control,
            geometry: FrameGeometry::default().into(),
            directory_handoff: false,
        }
    }

    #[cfg(windows)]
    async fn refused_host_connection() -> (
        tokio::task::JoinHandle<Result<()>>,
        MessageReader<tokio::io::DuplexStream>,
        mpsc::Receiver<ServerEvent<Arc<()>>>,
        std::sync::Weak<()>,
    ) {
        let (server, mut client) = tokio::io::duplex(4096);
        let (events, mut received) = mpsc::channel(4);
        let proof = Arc::new(());
        let retained = Arc::downgrade(&proof);
        let project = crate::protocol::encode_path(std::path::Path::new("/refused-peer"));
        let task = tokio::spawn(serve_connection_with_peer(
            1,
            server,
            events,
            project.clone(),
            proof,
        ));
        write_message(&mut client, &control_hello(project, PROTOCOL_VERSION))
            .await
            .unwrap();
        let ServerEvent::Connected { responses, .. } =
            tokio::time::timeout(Duration::from_secs(2), received.recv())
                .await
                .unwrap()
                .unwrap()
        else {
            panic!("expected admitted connection")
        };
        responses
            .try_send(HostResponse::Refused {
                message: "busy".to_owned(),
            })
            .unwrap();
        drop(responses);
        let mut client = MessageReader::new(client);
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), client.read::<HostResponse>())
                .await
                .unwrap()
                .unwrap(),
            Some(HostResponse::Refused { message }) if message == "busy"
        ));
        (task, client, received, retained)
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn host_refusal_is_delivered_before_peer_close_and_releases_on_eof() {
        let (task, mut client, mut events, retained) = refused_host_connection().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), client.read::<HostResponse>())
                .await
                .is_err(),
            "server closed the refusal pipe before its reader released it"
        );
        assert!(!task.is_finished());
        drop(client);
        tokio::time::timeout(Duration::from_millis(1500), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(retained.upgrade().is_none());
        assert!(matches!(
            events.recv().await,
            Some(ServerEvent::Disconnected { id: 1 })
        ));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn host_refusal_ignores_followup_request_and_releases_the_peer() {
        let (task, mut client, mut events, retained) = refused_host_connection().await;
        write_message(client.reader.get_mut(), &ClientRequest::Health)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_millis(1500), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(retained.upgrade().is_none());
        assert!(matches!(
            events.recv().await,
            Some(ServerEvent::Disconnected { id: 1 })
        ));
        assert!(
            events.recv().await.is_none(),
            "request escaped a terminal refusal"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn host_refusal_has_one_deadline_for_a_stalled_peer() {
        let (task, mut client, mut events, retained) = refused_host_connection().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), client.read::<HostResponse>())
                .await
                .is_err(),
            "stalled peer was released before the refusal deadline"
        );
        assert!(!task.is_finished());
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(retained.upgrade().is_none());
        assert!(matches!(
            events.recv().await,
            Some(ServerEvent::Disconnected { id: 1 })
        ));
        drop(client);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn early_handshake_refusal_is_delivered_before_peer_close() {
        let (server, mut client) = tokio::io::duplex(4096);
        let (events, mut received) = mpsc::channel(1);
        let project = crate::protocol::encode_path(std::path::Path::new("/early-refusal"));
        let task = tokio::spawn(serve_connection_with_peer(
            1,
            server,
            events,
            project.clone(),
            Arc::new(()),
        ));
        write_message(&mut client, &control_hello(project, PROTOCOL_VERSION + 1))
            .await
            .unwrap();
        let mut client = MessageReader::new(client);
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), client.read::<HostResponse>())
                .await
                .unwrap()
                .unwrap(),
            Some(HostResponse::Refused { message }) if message.contains("incompatible")
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), client.read::<HostResponse>())
                .await
                .is_err(),
            "early refusal closed its pipe before the reader released it"
        );
        assert!(!task.is_finished());
        drop(client);
        tokio::time::timeout(Duration::from_millis(1500), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(received.recv().await.is_none());
    }

    #[tokio::test]
    async fn semantic_reply_queued_during_final_selection_precedes_shutdown() {
        let (responses, mut receiver) = response_channel();
        let command_result = HostResponse::CommandResult {
            outcome: crate::protocol::CommandOutcome::Completed,
        };

        // Model a select pass that polled the semantic lane while it was
        // empty, then selected the final lane after both sends completed.
        assert!(matches!(
            receiver.messages.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        responses.try_send(command_result.clone()).unwrap();
        responses.try_send(HostResponse::ShuttingDown).unwrap();
        let selected_final = receiver.final_messages.try_recv().unwrap();
        assert_eq!(
            receiver.semantic_before_final(selected_final),
            command_result
        );
        drop(responses);

        assert_eq!(receiver.recv().await, Some(HostResponse::ShuttingDown));
        assert_eq!(receiver.recv().await, None);
    }

    #[test]
    fn bounded_encoder_preserves_wire_bytes_and_exact_limit_including_json_escaping() {
        let value = serde_json::json!({ "quote": "\"\\\n", "unicode": "雪", "number": 42 });
        let mut expected = serde_json::to_vec(&value).unwrap();
        expected.push(b'\n');
        assert_eq!(encode_message(&value).unwrap().0, expected);

        // String quotes count toward JSON, and the final newline occupies the
        // last permitted wire byte. The next JSON byte must be rejected.
        let maximum = "x".repeat(MAX_MESSAGE_BYTES - 3);
        let encoded = encode_message(&maximum).unwrap();
        assert_eq!(encoded.0.len(), MAX_MESSAGE_BYTES);
        assert_eq!(encoded.0.last(), Some(&b'\n'));
        drop(encoded);
        let oversized = "x".repeat(MAX_MESSAGE_BYTES - 2);
        let error = encode_message(&oversized).err().unwrap();
        assert!(
            error
                .to_string()
                .contains("workspace transport message exceeds")
        );
        let escaped = "\0".repeat(MAX_MESSAGE_BYTES / 6);
        assert!(escaped.len() < MAX_MESSAGE_BYTES);
        assert!(
            encode_message(&escaped)
                .err()
                .unwrap()
                .to_string()
                .contains("workspace transport message exceeds")
        );
    }

    #[tokio::test]
    async fn connection_retains_peer_proof_after_event_consumption_until_exit_or_cancellation() {
        for cancel in [false, true] {
            let (server, mut client) = tokio::io::duplex(4096);
            let (events, mut received) = mpsc::channel(4);
            // A lifetime sentinel stands in for Arc<PinnedProcess>; this test
            // covers proof ownership, not the platform's peer authentication.
            let proof = Arc::new(());
            let retained = Arc::downgrade(&proof);
            let project = crate::protocol::encode_path(std::path::Path::new("/peer-proof"));
            let task = tokio::spawn(serve_connection_with_peer(
                1,
                server,
                events,
                project.clone(),
                proof,
            ));
            write_message(
                &mut client,
                &ClientRequest::Hello {
                    protocol: PROTOCOL_VERSION,
                    features: vec![
                        FeatureGroup::Control,
                        FeatureGroup::Buffers,
                        FeatureGroup::Wait,
                    ],
                    project_root_bytes: project,
                    client_kind: ClientKind::Control,
                    client_version: crate::protocol::CLIENT_VERSION.to_owned(),
                    role: ClientRole::Control,
                    geometry: FrameGeometry::default().into(),
                    directory_handoff: false,
                },
            )
            .await
            .unwrap();
            let ServerEvent::Connected {
                peer_process,
                responses,
                ..
            } = tokio::time::timeout(Duration::from_secs(2), received.recv())
                .await
                .unwrap()
                .unwrap()
            else {
                panic!("expected admitted connection")
            };
            drop(peer_process);
            assert_eq!(
                retained.strong_count(),
                1,
                "connection must keep its own proof"
            );
            responses
                .send(HostResponse::Welcome {
                    protocol: PROTOCOL_VERSION,
                    pid: std::process::id(),
                    features: vec![
                        FeatureGroup::Control,
                        FeatureGroup::Buffers,
                        FeatureGroup::Wait,
                    ],
                    host_version: crate::protocol::CLIENT_VERSION.to_owned(),
                })
                .await
                .unwrap();
            let mut client = MessageReader::new(client);
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(2), client.read::<HostResponse>())
                    .await
                    .unwrap()
                    .unwrap(),
                Some(HostResponse::Welcome { .. })
            ));
            assert_eq!(retained.strong_count(), 1);
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                drop(client);
                tokio::time::timeout(Duration::from_secs(2), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert!(matches!(
                    received.recv().await,
                    Some(ServerEvent::Disconnected { id: 1 })
                ));
            }
            assert!(
                retained.upgrade().is_none(),
                "ended connection retained its proof"
            );
        }
    }
}
