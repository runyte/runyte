// SPDX-License-Identifier: MPL-2.0

//! Per-peer semantic ordering and wait ownership. Native peer proof outlives
//! every accepted request; no metadata PID or control request is frontend input.

use crate::host_requests::{handle_workspace_request, is_workspace_request};
use anyhow::Context;
use futures_util::stream::FuturesUnordered;
use runyte::{
    app::FrameGeometry,
    key_hints::KeyHintState,
    protocol::{ClientRequest, FeatureGroup, HostResponse, WaitToken, decode_path},
    workspace::{
        HostCommand, HostInputOutcome, WorkspaceHost, windows_endpoint::EndpointMetadata,
        windows_pipe::MAX_CONNECTIONS, windows_process_identity::PinnedProcess,
        windows_transport::ResponseSender,
    },
};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    time::Instant,
};

pub(super) type RenameFuture = Pin<Box<dyn Future<Output = io::Result<EndpointMetadata>> + Send>>;
type RenameCompletion = Pin<Box<dyn Future<Output = (u64, io::Result<EndpointMetadata>)> + Send>>;
const MAX_RENAMES: usize = MAX_CONNECTIONS * 2;

pub(super) enum Incoming {
    Request(ClientRequest),
    ProtocolError(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NativeSwitchAction {
    Commit { owner: u64, receipt: u64 },
    ParentCommitObserved { owner: u64, receipt: u64 },
    Abort { owner: u64, receipt: u64 },
}

pub(super) struct ParentAttachIntent {
    pub(super) child: u64,
    pub(super) terminal: runyte::terminal::TerminalId,
    pub(super) generation: u64,
    pub(super) child_proof: Arc<PinnedProcess>,
    pub(super) frontend_proof: Arc<PinnedProcess>,
    pub(super) capability: String,
    pub(super) selector: std::path::PathBuf,
    pub(super) directory: std::path::PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SwitchReservationKind {
    Ordinary,
    ParentPreparing,
    ParentCommitAccepted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SwitchReservation {
    owner: u64,
    receipt: u64,
    kind: SwitchReservationKind,
}

pub(super) struct ConnectedPeer {
    pub(super) id: u64,
    pub(super) proof: Arc<PinnedProcess>,
    pub(super) responses: ResponseSender,
    pub(super) interactive: bool,
    pub(super) geometry: FrameGeometry,
}

struct Peer {
    proof: Arc<PinnedProcess>,
    responses: ResponseSender,
    waits: HashSet<WaitToken>,
    parent_waits: HashSet<WaitToken>,
    subscribed_waits: HashSet<WaitToken>,
    attachment_generation: Option<u64>,
    geometry: Option<FrameGeometry>,
    pending_ready_frames: Option<(runyte::protocol::FrameId, runyte::protocol::FrameId)>,
    renaming: bool,
    parent_attaching: bool,
    deferred: Option<Incoming>,
}

pub(super) struct Clients {
    peers: HashMap<u64, Peer>,
    active: Option<u64>,
    active_ready: Option<u64>,
    hints: KeyHintState,
    publish_requested: bool,
    last_detached: Instant,
    pub(super) renames: FuturesUnordered<RenameCompletion>,
    pending_switch: Option<SwitchReservation>,
    switch_action: Option<NativeSwitchAction>,
    parent_attach: Option<ParentAttachIntent>,
    released_parent_request: Option<(u64, Incoming)>,
}

impl Default for Clients {
    fn default() -> Self {
        Self {
            peers: HashMap::new(),
            active: None,
            active_ready: None,
            hints: KeyHintState::default(),
            publish_requested: false,
            last_detached: Instant::now(),
            renames: FuturesUnordered::new(),
            pending_switch: None,
            switch_action: None,
            parent_attach: None,
            released_parent_request: None,
        }
    }
}

impl Clients {
    pub(super) fn connected(&mut self, host: &mut WorkspaceHost, connection: ConnectedPeer) {
        let ConnectedPeer {
            id,
            proof,
            responses,
            interactive,
            geometry,
        } = connection;
        let attachment_generation = (!interactive).then_some(self.active).flatten();
        if (interactive && self.active.is_some()) || self.peers.len() >= MAX_CONNECTIONS {
            let _ = responses.try_send(HostResponse::Refused {
                message: if interactive {
                    "another interactive TUI is already attached"
                } else {
                    "native control connection limit reached"
                }
                .to_owned(),
            });
            return;
        }
        if responses
            .try_send(HostResponse::Welcome {
                protocol: runyte::protocol::VERSION,
                pid: std::process::id(),
                features: if interactive {
                    vec![
                        FeatureGroup::Snapshots,
                        FeatureGroup::Input,
                        FeatureGroup::Buffers,
                        FeatureGroup::Wait,
                    ]
                } else {
                    vec![
                        FeatureGroup::Control,
                        FeatureGroup::Buffers,
                        FeatureGroup::Wait,
                    ]
                },
                host_version: env!("CARGO_PKG_VERSION").to_owned(),
            })
            .is_ok()
        {
            self.peers.insert(
                id,
                Peer {
                    proof,
                    responses,
                    waits: HashSet::new(),
                    parent_waits: HashSet::new(),
                    subscribed_waits: HashSet::new(),
                    attachment_generation,
                    geometry: interactive.then_some(geometry),
                    pending_ready_frames: None,
                    renaming: false,
                    parent_attaching: false,
                    deferred: None,
                },
            );
            if interactive {
                self.active = Some(id);
                self.active_ready = None;
                // A private wire client cannot grant shell handoff until the
                // public native frontend owns and validates that operation.
                host.app_mut().set_quit_directory_handoff(false);
                host.app_mut().note_frontend_attached();
                host.note_plugin_frontend(true);
                self.publish_frame(host);
            }
        }
    }

    pub(super) fn attached(&self) -> bool {
        self.active.is_some()
    }

    pub(super) fn active_id(&self) -> Option<u64> {
        self.active
    }

    pub(super) fn switch_pending(&self) -> bool {
        self.pending_switch.is_some()
    }

    pub(super) fn owns_switch_reservation(&self, owner: u64, receipt: u64) -> bool {
        self.active == Some(owner)
            && self
                .pending_switch
                .is_some_and(|pending| pending.owner == owner && pending.receipt == receipt)
    }

    pub(super) fn begin_switch(
        &mut self,
        host: &mut WorkspaceHost,
        owner: u64,
        receipt: u64,
        response: HostResponse,
    ) -> bool {
        if !self.owns_switch_reservation(owner, receipt) {
            return false;
        }
        self.send(host, owner, response)
    }

    pub(super) fn reserve_switch(
        &mut self,
        host: &mut WorkspaceHost,
        owner: u64,
        receipt: u64,
    ) -> bool {
        if self.active != Some(owner) || self.pending_switch.is_some() {
            return false;
        }
        self.hints.clear();
        host.cancel_pointer_drag();
        self.pending_switch = Some(SwitchReservation {
            owner,
            receipt,
            kind: SwitchReservationKind::Ordinary,
        });
        true
    }

    pub(super) fn reserve_parent_switch(
        &mut self,
        host: &mut WorkspaceHost,
        owner: u64,
        receipt: u64,
    ) -> bool {
        if self.active != Some(owner) || self.pending_switch.is_some() {
            return false;
        }
        self.hints.clear();
        host.cancel_pointer_drag();
        self.pending_switch = Some(SwitchReservation {
            owner,
            receipt,
            kind: SwitchReservationKind::ParentPreparing,
        });
        true
    }

    pub(super) fn cancel_switch_reservation(&mut self, owner: u64, receipt: u64) {
        if self
            .pending_switch
            .is_some_and(|pending| pending.owner == owner && pending.receipt == receipt)
        {
            self.pending_switch = None;
        }
    }

    pub(super) fn take_parent_attach(&mut self) -> Option<ParentAttachIntent> {
        self.parent_attach.take()
    }

    pub(super) fn take_released_parent_request(&mut self) -> Option<(u64, Incoming)> {
        self.released_parent_request.take()
    }

    pub(super) fn take_switch_action(&mut self) -> Option<NativeSwitchAction> {
        self.switch_action.take()
    }

    pub(super) fn abort_switch(
        &mut self,
        host: &mut WorkspaceHost,
        owner: u64,
        receipt: u64,
    ) -> bool {
        if !self.owns_switch_reservation(owner, receipt) {
            return false;
        }
        self.pending_switch = None;
        self.publish_requested = true;
        self.send(host, owner, HostResponse::NativeSwitchAborted { receipt })
    }

    pub(super) fn commit_switch(
        &mut self,
        host: &mut WorkspaceHost,
        owner: u64,
        receipt: u64,
    ) -> bool {
        if !self.owns_switch_reservation(owner, receipt) {
            return false;
        }
        self.pending_switch = None;
        let waits = self
            .peers
            .get(&owner)
            .map(|peer| peer.waits.iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for token in waits {
            let _ = host.cancel_wait(token.into(), "TUI switched to another workspace");
        }
        if let Some(peer) = self.peers.get_mut(&owner) {
            peer.waits.clear();
            peer.subscribed_waits.clear();
        }
        let sent = self.send(host, owner, HostResponse::NativeSwitchCommitted { receipt });
        if sent {
            self.disconnected(host, owner);
        }
        sent
    }

    pub(super) fn accept_parent_commit(
        &mut self,
        host: &mut WorkspaceHost,
        owner: u64,
        receipt: u64,
    ) -> bool {
        let Some(pending) = self.pending_switch.as_mut() else {
            return false;
        };
        if pending.owner != owner
            || pending.receipt != receipt
            || pending.kind != SwitchReservationKind::ParentPreparing
        {
            return false;
        }
        pending.kind = SwitchReservationKind::ParentCommitAccepted;
        self.send(
            host,
            owner,
            HostResponse::NativeParentSwitchCommitAccepted { receipt },
        )
    }

    pub(super) fn parent_commit_confirmed(&self, owner: u64, receipt: u64) -> bool {
        self.active == Some(owner)
            && self.pending_switch.is_some_and(|pending| {
                pending.owner == owner
                    && pending.receipt == receipt
                    && pending.kind == SwitchReservationKind::ParentCommitAccepted
            })
    }

    pub(super) fn finish_confirmed_parent_switch(
        &mut self,
        host: &mut WorkspaceHost,
        owner: u64,
        receipt: u64,
        committed: bool,
    ) {
        self.finish_confirmed_parent_switch_with_receipt(host, owner, receipt, committed, true);
    }

    #[cfg(test)]
    pub(super) fn drop_confirmed_parent_switch_receipt(
        &mut self,
        host: &mut WorkspaceHost,
        owner: u64,
        receipt: u64,
    ) {
        self.finish_confirmed_parent_switch_with_receipt(host, owner, receipt, true, false);
    }

    fn finish_confirmed_parent_switch_with_receipt(
        &mut self,
        host: &mut WorkspaceHost,
        owner: u64,
        receipt: u64,
        committed: bool,
        send_receipt: bool,
    ) {
        if self
            .pending_switch
            .is_some_and(|pending| pending.owner == owner && pending.receipt == receipt)
        {
            self.pending_switch = None;
        }
        let waits = self
            .peers
            .get(&owner)
            .map(|peer| peer.waits.iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for token in waits {
            let _ = host.cancel_wait(token.into(), "TUI switched to another workspace");
        }
        if let Some(peer) = self.peers.get_mut(&owner) {
            peer.waits.clear();
            peer.subscribed_waits.clear();
        }
        let response = if committed {
            HostResponse::NativeSwitchCommitted { receipt }
        } else {
            HostResponse::NativeSwitchAborted { receipt }
        };
        if send_receipt && self.peers.contains_key(&owner) {
            let _ = self.send(host, owner, response);
        }
        if self.active == Some(owner) {
            self.disconnected(host, owner);
        }
        self.publish_requested = true;
    }

    pub(super) fn reply_parent_attach(
        &mut self,
        host: &mut WorkspaceHost,
        child: u64,
        result: Result<(), String>,
    ) -> bool {
        let response = match result {
            Ok(()) => HostResponse::ParentAttached,
            Err(message) => HostResponse::Error { message },
        };
        let deferred = self.peers.get_mut(&child).and_then(|peer| {
            peer.parent_attaching = false;
            peer.deferred.take()
        });
        let sent = self.send(host, child, response);
        if sent && let Some(deferred) = deferred {
            debug_assert!(self.released_parent_request.is_none());
            self.released_parent_request = Some((child, deferred));
        }
        sent
    }

    #[cfg(test)]
    pub(super) fn drop_switch_without_ack(
        &mut self,
        host: &mut WorkspaceHost,
        owner: u64,
        receipt: u64,
    ) {
        if self
            .pending_switch
            .is_some_and(|pending| pending.owner == owner && pending.receipt == receipt)
        {
            self.pending_switch = None;
            self.disconnected(host, owner);
        }
    }

    pub(super) fn send_active(&mut self, host: &mut WorkspaceHost, response: HostResponse) {
        if let Some(id) = self.active {
            self.send(host, id, response);
        }
    }

    pub(super) fn last_detached(&self) -> Instant {
        self.last_detached
    }

    pub(super) fn hint_delay(&self, now: Instant) -> Option<std::time::Duration> {
        self.active.and_then(|_| self.hints.time_until_expiry(now))
    }

    pub(super) fn expire_hints(&mut self, now: Instant) {
        self.hints.expire_at(now);
        self.publish_requested = true;
    }

    pub(super) fn take_publish_requested(&mut self) -> bool {
        std::mem::take(&mut self.publish_requested)
    }

    pub(super) fn refuse_switch(&mut self, host: &mut WorkspaceHost) {
        self.refuse_switch_with(host, "native session switching route is not available yet");
    }

    pub(super) fn refuse_switch_with(&mut self, host: &mut WorkspaceHost, message: &str) {
        host.report_host_error(message.to_owned());
        if let Some(id) = self.active {
            self.send(host, id, HostResponse::NativeSwitchUnchanged);
        }
        self.publish_requested = true;
    }

    fn finish_active_waits(&mut self, host: &mut WorkspaceHost, id: u64) {
        let tokens = self
            .peers
            .get(&id)
            .map(|peer| peer.subscribed_waits.iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for token in tokens {
            let status = match host.complete_wait_request(token.into()) {
                Ok(()) => host
                    .wait_status(token.into())
                    .expect("completed wait exists"),
                Err(error) => {
                    let _ = host.cancel_wait(
                        token.into(),
                        format!("attached TUI ended before successful wait completion: {error}"),
                    );
                    host.wait_status(token.into())
                        .expect("cancelled wait exists")
                }
            };
            self.send(
                host,
                id,
                HostResponse::WaitState {
                    token,
                    status: status.into(),
                    interactive_attached: false,
                },
            );
        }
        if let Some(peer) = self.peers.get_mut(&id) {
            peer.waits.clear();
            peer.subscribed_waits.clear();
        }
    }

    pub(super) fn finish_exit(
        &mut self,
        host: &mut WorkspaceHost,
        request: runyte::app::PersistentExitRequest,
    ) -> bool {
        let Some(id) = self.active else { return false };
        self.hints.clear();
        match request {
            runyte::app::PersistentExitRequest::Detach => {
                self.finish_active_waits(host, id);
                self.send(
                    host,
                    id,
                    HostResponse::Detached {
                        directory_bytes: None,
                    },
                );
                self.disconnected(host, id);
                false
            }
            runyte::app::PersistentExitRequest::Quit { force } => {
                self.finish_active_waits(host, id);
                let mut protected = host.protected_state();
                if force {
                    protected.unsaved_buffers = 0;
                }
                if !protected.is_empty() {
                    host.report_host_error(format!("cannot quit persistent session: {}; finish or close that state, or use :detach", protected.refusal()));
                    self.publish_requested = true;
                    return false;
                }
                self.send(host, id, HostResponse::ShuttingDown);
                true
            }
        }
    }

    pub(super) fn publish_frame(&mut self, host: &mut WorkspaceHost) {
        if self.pending_switch.is_some() {
            return;
        }
        let Some(id) = self.active else { return };
        let Some(peer) = self.peers.get(&id) else {
            return;
        };
        let geometry = peer.geometry.expect("interactive geometry retained");
        host.mark_visible_terminals_viewed();
        let frame: runyte::protocol::HostFrame = host
            .prepare_frame_with_hints(geometry, Some(&self.hints))
            .into();
        let frame_id = frame.id;
        // Complete frames can replace an unseen visual response without a
        // delta base. The transport keeps exactly one coalesced visual slot.
        let Some(peer) = self.peers.get_mut(&id) else {
            return;
        };
        match peer.responses.try_send(HostResponse::Frame {
            frame: Box::new(frame),
        }) {
            Ok(()) => {
                if self.active_ready != Some(id) {
                    peer.pending_ready_frames = Some(match peer.pending_ready_frames {
                        Some((first, _)) => (first, frame_id),
                        None => (frame_id, frame_id),
                    });
                }
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.disconnected(host, id);
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
        }
    }

    pub(super) fn disconnected(&mut self, host: &mut WorkspaceHost, id: u64) {
        if self
            .pending_switch
            .is_some_and(|pending| pending.owner == id)
        {
            self.pending_switch = None;
            self.switch_action = None;
        }
        if self
            .parent_attach
            .as_ref()
            .is_some_and(|attach| attach.child == id || attach.generation == id)
        {
            self.parent_attach = None;
        }
        if self.active == Some(id) {
            self.cancel_parent_waits_for_generation(host, id);
            self.active = None;
            self.active_ready = None;
            self.hints.clear();
            self.publish_requested = false;
            self.last_detached = Instant::now();
            host.cancel_pointer_drag();
            host.app_mut().set_quit_directory_handoff(false);
            host.note_plugin_frontend(false);
        }
        if let Some(peer) = self.peers.remove(&id) {
            for token in peer.waits {
                let _ =
                    host.cancel_wait(token.into(), "wait client disconnected before completion");
            }
        }
    }

    fn cancel_parent_waits_for_generation(&mut self, host: &mut WorkspaceHost, generation: u64) {
        for peer in self.peers.values_mut().filter(|peer| {
            peer.attachment_generation == Some(generation) && !peer.parent_waits.is_empty()
        }) {
            for token in peer.parent_waits.drain() {
                let _ = host.cancel_wait(
                    token.into(),
                    "owning native TUI attachment ended before parent editing completed",
                );
            }
        }
    }

    fn send(&mut self, host: &mut WorkspaceHost, id: u64, response: HostResponse) -> bool {
        if self
            .peers
            .get(&id)
            .is_some_and(|peer| peer.responses.try_send(response).is_ok())
        {
            true
        } else {
            self.disconnected(host, id);
            false
        }
    }

    /// At most one later request/error waits behind a rename or ParentAttach on
    /// this stream. Overflow closes the peer without replying out of order.
    /// Other peers and services continue; a dropped peer never receives a late
    /// rename or parent handoff result.
    pub(super) fn incoming(
        &mut self,
        host: &mut WorkspaceHost,
        id: u64,
        incoming: Incoming,
        rename: impl FnOnce(&str) -> io::Result<RenameFuture>,
    ) -> bool {
        let Some(peer) = self.peers.get_mut(&id) else {
            return false;
        };
        if peer.renaming || peer.parent_attaching {
            if peer.deferred.is_some() {
                self.disconnected(host, id);
            } else {
                peer.deferred = Some(incoming);
            }
            return false;
        }
        self.dispatch(host, id, incoming, rename)
    }

    fn dispatch(
        &mut self,
        host: &mut WorkspaceHost,
        id: u64,
        incoming: Incoming,
        rename: impl FnOnce(&str) -> io::Result<RenameFuture>,
    ) -> bool {
        let request = match incoming {
            Incoming::ProtocolError(message) => {
                self.send(host, id, HostResponse::Error { message });
                return false;
            }
            Incoming::Request(request) => request,
        };
        let interactive = self.active == Some(id);
        if interactive && self.pending_switch.is_some() {
            match &request {
                ClientRequest::NativeSwitchCommit { receipt } => {
                    if self.owns_switch_reservation(id, *receipt)
                        && self.pending_switch.is_some_and(|pending| {
                            pending.kind != SwitchReservationKind::ParentCommitAccepted
                        })
                    {
                        self.switch_action = Some(NativeSwitchAction::Commit {
                            owner: id,
                            receipt: *receipt,
                        });
                    } else {
                        self.send(
                            host,
                            id,
                            HostResponse::Error {
                                message: "native switch receipt is stale".to_owned(),
                            },
                        );
                    }
                    return false;
                }
                ClientRequest::NativeParentSwitchCommitObserved { receipt } => {
                    if self.parent_commit_confirmed(id, *receipt) {
                        self.switch_action = Some(NativeSwitchAction::ParentCommitObserved {
                            owner: id,
                            receipt: *receipt,
                        });
                    } else {
                        self.send(
                            host,
                            id,
                            HostResponse::Error {
                                message: "native parent switch receipt is stale".to_owned(),
                            },
                        );
                    }
                    return false;
                }
                ClientRequest::NativeSwitchAbort { receipt } => {
                    if self.owns_switch_reservation(id, *receipt) {
                        self.switch_action = Some(NativeSwitchAction::Abort {
                            owner: id,
                            receipt: *receipt,
                        });
                    } else {
                        self.send(
                            host,
                            id,
                            HostResponse::Error {
                                message: "native switch receipt is stale".to_owned(),
                            },
                        );
                    }
                    return false;
                }
                ClientRequest::Input { .. }
                | ClientRequest::FrameDrawn { .. }
                | ClientRequest::Pointer { .. }
                | ClientRequest::Invoke { .. }
                | ClientRequest::Notify { .. }
                | ClientRequest::Resynchronize => return false,
                _ => {}
            }
        }
        match request {
            ClientRequest::FrameDrawn { frame } if interactive => {
                let peer = self.peers.get_mut(&id).expect("active peer retained");
                if peer
                    .pending_ready_frames
                    .is_some_and(|(first, last)| frame >= first && frame <= last)
                {
                    peer.pending_ready_frames = None;
                    self.active_ready = Some(id);
                }
                false
            }
            ClientRequest::Input {
                event,
                repeated,
                presented_frame,
            } if interactive => {
                if !repeated && let Some(frame) = presented_frame {
                    host.context_frame_presented(frame.into());
                }
                if repeated {
                    // Auto-repeat is useful editor input, but it is not a new
                    // physical approval gesture. Replayed input cannot accept
                    // a plugin confirmation or capture native handoff authority.
                    super::super::dispatch_host_repeated_key_or_text(
                        host,
                        &mut self.hints,
                        event.into(),
                    );
                } else {
                    super::super::dispatch_host_key_or_text(
                        host,
                        &mut self.hints,
                        event.into(),
                        false,
                    );
                }
                self.publish_requested = true;
                false
            }
            ClientRequest::Pointer {
                event,
                frame,
                repetitions,
            } if interactive => {
                self.hints.clear();
                match host.execute(HostCommand::Pointer {
                    event: event.into(),
                    frame: frame.into(),
                    repetitions,
                }) {
                    Ok(HostInputOutcome::Applied) => self.publish_requested = true,
                    Ok(
                        HostInputOutcome::AppliedWithoutVisualChange
                        | HostInputOutcome::IgnoredStaleFrame,
                    ) => {}
                    Err(error) => host.report_host_error(error.to_string()),
                }
                false
            }
            ClientRequest::Resize { geometry } if interactive => {
                if let Some(peer) = self.peers.get_mut(&id) {
                    peer.geometry = Some(geometry.into());
                }
                self.publish_requested = true;
                false
            }
            ClientRequest::Resynchronize if interactive => {
                self.publish_requested = true;
                false
            }
            ClientRequest::Detach if interactive => {
                self.send(
                    host,
                    id,
                    HostResponse::Detached {
                        directory_bytes: None,
                    },
                );
                self.disconnected(host, id);
                false
            }
            ClientRequest::Notify { message } if interactive => {
                host.report_host_error(message);
                self.publish_requested = true;
                false
            }
            ClientRequest::AttachWait { token } if interactive => {
                let response = match host.wait_status(token.into()) {
                    Some(status) => {
                        if matches!(status, runyte::workspace::WaitStatus::Pending { .. }) {
                            self.peers
                                .get_mut(&id)
                                .expect("active peer")
                                .subscribed_waits
                                .insert(token);
                        }
                        HostResponse::WaitState {
                            token,
                            status: status.into(),
                            interactive_attached: true,
                        }
                    }
                    None => HostResponse::Error {
                        message: format!("unknown wait token {token}"),
                    },
                };
                self.send(host, id, response);
                false
            }
            ClientRequest::Shutdown => {
                let protected = host.protected_state();
                if !protected.is_empty() {
                    self.send(
                        host,
                        id,
                        HostResponse::Refused {
                            message: protected.refusal(),
                        },
                    );
                    return false;
                }
                self.send(host, id, HostResponse::ShuttingDown);
                true
            }
            ClientRequest::ForceShutdown => {
                self.send(host, id, HostResponse::ShuttingDown);
                true
            }
            ClientRequest::RenameHost { name } => {
                let admitted = if self.renames.len() < MAX_RENAMES {
                    rename(&name)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "native rename completion limit reached",
                    ))
                };
                match admitted {
                    Ok(ticket) => {
                        self.peers.get_mut(&id).expect("admitted peer").renaming = true;
                        self.renames
                            .push(Box::pin(async move { (id, ticket.await) }));
                    }
                    Err(error) => {
                        self.send(
                            host,
                            id,
                            HostResponse::Error {
                                message: error.to_string(),
                            },
                        );
                    }
                }
                false
            }
            ClientRequest::ParentWait {
                terminal,
                capability,
                paths,
            } if !interactive => {
                let terminal = runyte::terminal::TerminalId::from_raw(terminal);
                let authority = self.peers.get(&id).and_then(|peer| {
                    peer.attachment_generation
                        .filter(|generation| {
                            self.active == Some(*generation)
                                && self.active_ready == Some(*generation)
                                && self.pending_switch.is_none()
                        })
                        .and_then(|generation| {
                            self.peers.get(&generation).map(|active| {
                                (
                                    generation,
                                    Arc::clone(&peer.proof),
                                    Arc::clone(&active.proof),
                                )
                            })
                        })
                });
                let result = match authority {
                    Some((_generation, proof, active_proof)) => active_proof
                        .is_alive()
                        .map_err(anyhow::Error::from)
                        .and_then(|valid| {
                            anyhow::ensure!(
                                valid,
                                "parent editing context is stale or detached; return to the owning persistent session"
                            );
                            host.app()
                                .terminals
                                .validates_parent(
                                    terminal,
                                    &capability,
                                    Some(proof.as_ref()),
                                )
                                .map_err(anyhow::Error::from)
                        })
                        .and_then(|valid| {
                            anyhow::ensure!(
                                valid,
                                "parent editing context is stale, detached, or not owned by this terminal"
                            );
                            paths
                                .into_iter()
                                .map(decode_path)
                                .collect::<io::Result<Vec<_>>>()
                                .map_err(anyhow::Error::from)
                        })
                        .and_then(|paths| host.create_parent_wait_request(terminal, paths)),
                    None => Err(anyhow::anyhow!(
                        "parent editing context is stale or detached; return to the owning persistent session"
                    )),
                };
                let response = result.map_or_else(
                    |error| HostResponse::Error {
                        message: error.to_string(),
                    },
                    |(token, buffers)| {
                        let token: WaitToken = token.into();
                        let peer = self.peers.get_mut(&id).expect("admitted parent peer");
                        peer.waits.insert(token);
                        peer.parent_waits.insert(token);
                        peer.subscribed_waits.insert(token);
                        HostResponse::WaitCreated {
                            token,
                            buffers: buffers.into_iter().map(Into::into).collect(),
                            interactive_attached: true,
                        }
                    },
                );
                self.publish_requested |= matches!(response, HostResponse::WaitCreated { .. });
                self.send(host, id, response);
                false
            }
            ClientRequest::ParentAttach {
                terminal,
                capability,
                selector,
                directory,
            } if !interactive => {
                let terminal = runyte::terminal::TerminalId::from_raw(terminal);
                let authority = self.parent_authority(host, id, terminal, &capability);
                let result = authority.and_then(|(generation, child_proof, frontend_proof)| {
                    anyhow::ensure!(
                        self.parent_attach.is_none(),
                        "another parent attach is already in progress"
                    );
                    Ok(ParentAttachIntent {
                        child: id,
                        terminal,
                        generation,
                        child_proof,
                        frontend_proof,
                        capability,
                        selector: decode_path(selector)?,
                        directory: decode_path(directory)?,
                    })
                });
                match result {
                    Ok(intent) => {
                        self.peers
                            .get_mut(&id)
                            .expect("admitted parent peer")
                            .parent_attaching = true;
                        self.parent_attach = Some(intent);
                    }
                    Err(error) => {
                        self.send(
                            host,
                            id,
                            HostResponse::Error {
                                message: error.to_string(),
                            },
                        );
                    }
                }
                false
            }
            request if is_workspace_request(&request) => {
                let mutation = match &request {
                    ClientRequest::CompleteWaitBuffer { token, .. }
                    | ClientRequest::CancelWait { token } => Some(*token),
                    _ => None,
                };
                if mutation.is_some_and(|token| {
                    !self
                        .peers
                        .get(&id)
                        .is_some_and(|peer| peer.waits.contains(&token))
                }) {
                    self.send(
                        host,
                        id,
                        HostResponse::Error {
                            message: "wait token is not owned by this native connection".to_owned(),
                        },
                    );
                    return false;
                }
                let reply = handle_workspace_request(host, request, self.attached(), interactive)
                    .expect("semantic request classified");
                if let HostResponse::WaitCreated { token, .. } = &reply.response {
                    let peer = self.peers.get_mut(&id).expect("admitted peer");
                    peer.waits.insert(*token);
                    if interactive {
                        peer.subscribed_waits.insert(*token);
                    } else if let Some(active) =
                        self.active.and_then(|active| self.peers.get_mut(&active))
                    {
                        active.subscribed_waits.insert(*token);
                    }
                }
                self.publish_requested |= reply.publish_frame;
                self.send(host, id, reply.response);
                false
            }
            _ => {
                self.send(host, id, HostResponse::Error {
                    message: "request requires native attachment or parent-terminal authorization, which is not available yet".to_owned(),
                });
                false
            }
        }
    }

    pub(super) fn renamed(
        &mut self,
        host: &mut WorkspaceHost,
        id: u64,
        result: io::Result<EndpointMetadata>,
        rename: impl FnOnce(&str) -> io::Result<RenameFuture>,
    ) -> bool {
        let Some(peer) = self.peers.get_mut(&id) else {
            return false;
        };
        peer.renaming = false;
        let deferred = peer.deferred.take();
        let response = match result {
            Ok(metadata) => match metadata.name {
                Some(name) => HostResponse::HostRenamed { name },
                None => HostResponse::Error {
                    message: "native rename completed without a published name".to_owned(),
                },
            },
            Err(error) => HostResponse::Error {
                message: error.to_string(),
            },
        };
        if self.send(host, id, response)
            && let Some(deferred) = deferred
        {
            return self.dispatch(host, id, deferred, rename);
        }
        false
    }

    pub(super) fn reconcile(&mut self, host: &mut WorkspaceHost) {
        host.reconcile_wait_requests();
        let mut completions = Vec::new();
        for (&id, peer) in &mut self.peers {
            peer.waits
                .retain(|token| host.wait_status((*token).into()).is_some());
            peer.parent_waits.retain(|token| {
                matches!(
                    host.wait_status((*token).into()),
                    Some(runyte::workspace::WaitStatus::Pending { .. })
                )
            });
            peer.subscribed_waits
                .retain(|token| match host.wait_status((*token).into()) {
                    Some(runyte::workspace::WaitStatus::Pending { .. }) => true,
                    Some(status) => {
                        completions.push((id, *token, status));
                        false
                    }
                    None => false,
                });
        }
        for (id, token, status) in completions {
            self.send(
                host,
                id,
                HostResponse::WaitState {
                    token,
                    status: status.into(),
                    interactive_attached: self.attached(),
                },
            );
        }
    }

    pub(super) fn clear(&mut self) {
        self.renames.clear();
        self.peers.clear();
        self.active = None;
        self.active_ready = None;
        self.hints.clear();
        self.pending_switch = None;
        self.switch_action = None;
        self.parent_attach = None;
        self.released_parent_request = None;
    }

    pub(super) fn parent_attach_valid(
        &self,
        host: &WorkspaceHost,
        attach: &ParentAttachIntent,
    ) -> bool {
        self.active == Some(attach.generation)
            && self.active_ready == Some(attach.generation)
            && self.peers.get(&attach.child).is_some_and(|peer| {
                Arc::ptr_eq(&peer.proof, &attach.child_proof)
                    && peer.attachment_generation == Some(attach.generation)
            })
            && self
                .peers
                .get(&attach.generation)
                .is_some_and(|peer| Arc::ptr_eq(&peer.proof, &attach.frontend_proof))
            && attach.child_proof.is_alive().unwrap_or(false)
            && attach.frontend_proof.is_alive().unwrap_or(false)
            && host.app().active_terminal() == Some(attach.terminal)
            && host.parent_request_ready()
            && host
                .app()
                .terminals
                .validates_parent(
                    attach.terminal,
                    &attach.capability,
                    Some(attach.child_proof.as_ref()),
                )
                .unwrap_or(false)
    }

    /// After the original frontend confirms the nonfinal commit reply, its
    /// control connection may close as it adopts the destination. Retained
    /// process proof plus the exact child and terminal remain authoritative
    /// until the host makes its atomic destination commit decision.
    pub(super) fn parent_attach_confirmed_valid(
        &self,
        host: &WorkspaceHost,
        attach: &ParentAttachIntent,
    ) -> bool {
        self.peers.get(&attach.child).is_some_and(|peer| {
            Arc::ptr_eq(&peer.proof, &attach.child_proof)
                && peer.attachment_generation == Some(attach.generation)
        }) && attach.child_proof.is_alive().unwrap_or(false)
            && attach.frontend_proof.is_alive().unwrap_or(false)
            && host.app().active_terminal() == Some(attach.terminal)
            && host.parent_request_ready()
            && host
                .app()
                .terminals
                .validates_parent(
                    attach.terminal,
                    &attach.capability,
                    Some(attach.child_proof.as_ref()),
                )
                .unwrap_or(false)
    }

    fn parent_authority(
        &self,
        host: &WorkspaceHost,
        id: u64,
        terminal: runyte::terminal::TerminalId,
        capability: &str,
    ) -> anyhow::Result<(u64, Arc<PinnedProcess>, Arc<PinnedProcess>)> {
        let peer = self
            .peers
            .get(&id)
            .context("parent control connection ended")?;
        let generation = peer.attachment_generation.context(
            "parent editing context is stale or detached; return to the owning persistent session",
        )?;
        anyhow::ensure!(
            self.active == Some(generation)
                && self.active_ready == Some(generation)
                && self.pending_switch.is_none(),
            "parent editing context is stale or detached; return to the owning persistent session"
        );
        let active = self.peers.get(&generation).context(
            "parent editing context is stale or detached; return to the owning persistent session",
        )?;
        anyhow::ensure!(
            active.proof.is_alive()? && peer.proof.is_alive()?,
            "parent editing context is stale or detached; return to the owning persistent session"
        );
        anyhow::ensure!(
            host.app().active_terminal() == Some(terminal) && host.parent_request_ready(),
            "originating terminal is no longer visible or ready"
        );
        anyhow::ensure!(
            host.app().terminals.validates_parent(
                terminal,
                capability,
                Some(peer.proof.as_ref()),
            )?,
            "parent editing context is stale, detached, or not owned by this terminal"
        );
        Ok((
            generation,
            Arc::clone(&peer.proof),
            Arc::clone(&active.proof),
        ))
    }
}

#[cfg(test)]
mod tests;
