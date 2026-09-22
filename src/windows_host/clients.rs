// SPDX-License-Identifier: MPL-2.0

//! Per-peer semantic ordering and wait ownership. Native peer proof outlives
//! every accepted request; no metadata PID or control request is frontend input.

use crate::host_requests::{handle_workspace_request, is_workspace_request};
use futures_util::stream::FuturesUnordered;
use runyte::{
    protocol::{ClientRequest, FeatureGroup, HostResponse, WaitToken},
    workspace::{
        WorkspaceHost, windows_endpoint::EndpointMetadata, windows_pipe::MAX_CONNECTIONS,
        windows_process_identity::PinnedProcess, windows_transport::ResponseSender,
    },
};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
};

pub(super) type RenameFuture = Pin<Box<dyn Future<Output = io::Result<EndpointMetadata>> + Send>>;
type RenameCompletion = Pin<Box<dyn Future<Output = (u64, io::Result<EndpointMetadata>)> + Send>>;
const MAX_RENAMES: usize = MAX_CONNECTIONS * 2;

pub(super) enum Incoming {
    Request(ClientRequest),
    ProtocolError(String),
}

struct Peer {
    _proof: Arc<PinnedProcess>,
    responses: ResponseSender,
    waits: HashSet<WaitToken>,
    renaming: bool,
    deferred: Option<Incoming>,
}

#[derive(Default)]
pub(super) struct Clients {
    peers: HashMap<u64, Peer>,
    pub(super) renames: FuturesUnordered<RenameCompletion>,
}

impl Clients {
    pub(super) fn connected(
        &mut self,
        id: u64,
        proof: Arc<PinnedProcess>,
        responses: ResponseSender,
        interactive: bool,
    ) {
        if interactive || self.peers.len() >= MAX_CONNECTIONS {
            let _ = responses.try_send(HostResponse::Refused {
                message: if interactive {
                    "native interactive attachment is not available yet"
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
                features: vec![
                    FeatureGroup::Control,
                    FeatureGroup::Buffers,
                    FeatureGroup::Wait,
                ],
                host_version: env!("CARGO_PKG_VERSION").to_owned(),
            })
            .is_ok()
        {
            self.peers.insert(
                id,
                Peer {
                    _proof: proof,
                    responses,
                    waits: HashSet::new(),
                    renaming: false,
                    deferred: None,
                },
            );
        }
    }

    pub(super) fn disconnected(&mut self, host: &mut WorkspaceHost, id: u64) {
        if let Some(peer) = self.peers.remove(&id) {
            for token in peer.waits {
                let _ =
                    host.cancel_wait(token.into(), "wait client disconnected before completion");
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

    /// At most one later request/error waits behind a rename on this stream.
    /// Overflow closes the peer without replying out of order. Other peers and
    /// services continue; a dropped peer never receives its late rename result.
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
        if peer.renaming {
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
        match request {
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
            request if is_workspace_request(&request) => {
                let reply = handle_workspace_request(host, request, false, false)
                    .expect("semantic request classified");
                if let HostResponse::WaitCreated { token, .. } = &reply.response {
                    self.peers
                        .get_mut(&id)
                        .expect("admitted peer")
                        .waits
                        .insert(*token);
                }
                // There is no native frontend in this package. Mutations still
                // cross the same semantic handler and wait reconciliation.
                let _ = reply.publish_frame;
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
        for peer in self.peers.values_mut() {
            peer.waits.retain(|token| {
                matches!(
                    host.wait_status((*token).into()),
                    Some(runyte::workspace::WaitStatus::Pending { .. })
                )
            });
        }
    }

    pub(super) fn clear(&mut self) {
        self.renames.clear();
        self.peers.clear();
    }
}

#[cfg(test)]
mod tests;
