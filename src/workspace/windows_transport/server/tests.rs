// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::FrameGeometry,
    protocol::{ClientKind, ClientRequest, ClientRole, FeatureGroup, HostResponse},
    test_support::TestRuntimeRoot,
    workspace::{
        transport_shared::{MessageReader, ResponseSender, write_message},
        windows_endpoint::{EndpointLocation, RegistrySet},
        windows_pipe::{self, Connection},
        windows_process_identity::{PinnedProcess, ProcessIdentity},
    },
};
use std::{
    future::poll_fn,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
};
use tokio::{
    io::{AsyncWriteExt, ReadHalf, WriteHalf},
    net::windows::named_pipe::NamedPipeClient,
    sync::watch,
    time::timeout_at,
};

type ClientStream = Connection<NamedPipeClient>;
type ClientReader = MessageReader<ReadHalf<ClientStream>>;

struct Peer {
    reader: ClientReader,
    writer: WriteHalf<ClientStream>,
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn fixture() -> (TestRuntimeRoot, EndpointLocation) {
    let root = TestRuntimeRoot::new("native-server-owner").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let registries = RegistrySet::open_fixture(&[root.join("registry")]).unwrap();
    let endpoint = EndpointLocation::new(&project, root.join("endpoint"), registries).unwrap();
    (root, endpoint)
}

fn bind(
    endpoint: &EndpointLocation,
    audit: Option<DropAudit>,
) -> (LocalServer, watch::Receiver<usize>) {
    bind_with_budget(endpoint, audit, Duration::from_millis(75))
}

fn bind_with_budget(
    endpoint: &EndpointLocation,
    audit: Option<DropAudit>,
    drain_budget: Duration,
) -> (LocalServer, watch::Receiver<usize>) {
    let (observer, progress) = watch::channel(0);
    let owner = Owner {
        connections: FuturesUnordered::new(),
        _before_listener_drop: audit,
        listener: Listener::bind(endpoint.prepare(None).unwrap()).unwrap(),
        drain_budget,
        observer: Some(observer),
    };
    (LocalServer::from_owner(owner), progress)
}

fn hello(metadata: &EndpointMetadata) -> ClientRequest {
    ClientRequest::Hello {
        protocol: crate::protocol::VERSION,
        features: vec![
            FeatureGroup::Control,
            FeatureGroup::Buffers,
            FeatureGroup::Wait,
        ],
        project_root_bytes: metadata.project_root_bytes.clone(),
        client_kind: ClientKind::Control,
        client_version: crate::protocol::CLIENT_VERSION.to_owned(),
        role: ClientRole::Control,
        geometry: FrameGeometry::default().into(),
        directory_handoff: true,
    }
}

fn welcome() -> HostResponse {
    HostResponse::Welcome {
        protocol: crate::protocol::VERSION,
        pid: std::process::id(),
        features: vec![
            FeatureGroup::Control,
            FeatureGroup::Buffers,
            FeatureGroup::Wait,
        ],
        host_version: crate::protocol::CLIENT_VERSION.to_owned(),
    }
}

async fn connect(metadata: &EndpointMetadata) -> Peer {
    let stream = windows_pipe::connect(metadata, deadline()).await.unwrap();
    let (reader, writer) = tokio::io::split(stream);
    Peer {
        reader: MessageReader::new(reader),
        writer,
    }
}

async fn next(server: &mut LocalServer) -> ServerEvent {
    timeout_at(deadline(), server.recv())
        .await
        .unwrap()
        .expect("owner closed unexpectedly")
}

async fn admitted(server: &mut LocalServer) -> (Peer, u64, ResponseSender, Weak<PinnedProcess>) {
    let mut peer = connect(server.metadata()).await;
    write_message(&mut peer.writer, &hello(server.metadata()))
        .await
        .unwrap();
    let ServerEvent::Connected {
        id,
        peer_process,
        responses,
        interactive,
        directory_handoff,
        ..
    } = next(server).await
    else {
        panic!("expected Connected");
    };
    assert_eq!(peer_process.identity(), ProcessIdentity::current().unwrap());
    assert!(!interactive);
    assert!(directory_handoff);
    let proof = Arc::downgrade(&peer_process);
    drop(peer_process);
    responses.send(welcome()).await.unwrap();
    assert!(matches!(
        timeout_at(deadline(), peer.reader.read::<HostResponse>())
            .await
            .unwrap()
            .unwrap(),
        Some(HostResponse::Welcome { .. })
    ));
    (peer, id, responses, proof)
}

async fn wait_count(progress: &mut watch::Receiver<usize>, expected: usize) {
    timeout_at(deadline(), async {
        loop {
            if *progress.borrow_and_update() == expected {
                break;
            }
            progress.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}

async fn pending<F: Future>(future: &mut Pin<Box<F>>) {
    poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

type RetainedProof = Arc<Mutex<Option<Weak<PinnedProcess>>>>;

fn managed(
    endpoint: &EndpointLocation,
    names: NameStore,
    audit: Option<DropAudit>,
    operation: impl FnMut(
        &mut crate::workspace::windows_endpoint::Publication,
        &NameStore,
        &str,
    ) -> io::Result<()>
    + Send
    + 'static,
) -> (LocalServer, watch::Receiver<usize>) {
    let listener = Listener::bind_with_name_operation(
        endpoint.prepare_named(&names, None).unwrap(),
        names,
        operation,
    )
    .unwrap();
    let (observer, progress) = watch::channel(0);
    let owner = Owner {
        connections: FuturesUnordered::new(),
        _before_listener_drop: audit,
        listener,
        drain_budget: Duration::from_millis(75),
        observer: Some(observer),
    };
    (LocalServer::from_owner(owner), progress)
}

fn audit(endpoint: &EndpointLocation) -> (DropAudit, RetainedProof, Arc<AtomicBool>) {
    let retained: RetainedProof = Arc::new(Mutex::new(None));
    let checked = Arc::new(AtomicBool::new(false));
    let root = endpoint.clone();
    let weak = retained.clone();
    let done = checked.clone();
    (
        DropAudit(Some(Box::new(move || {
            if let Some(proof) = weak.lock().unwrap().as_ref() {
                assert!(
                    proof.upgrade().is_none(),
                    "publication cleanup preceded connection proof/stream drop"
                );
            }
            assert!(
                root.read_ready().unwrap().is_some(),
                "publication retired before its owner boundary"
            );
            done.store(true, Ordering::Release);
        }))),
        retained,
        checked,
    )
}

#[test]
fn native_hello_requests_and_disconnect_preserve_retained_peer_identity() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut server = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
        assert_eq!(endpoint.read_ready().unwrap().as_ref(), Some(server.metadata()));
        let (mut peer, id, responses, proof) = admitted(&mut server).await;
        assert!(proof.upgrade().is_some());
        write_message(&mut peer.writer, &ClientRequest::Health).await.unwrap();
        assert!(matches!(next(&mut server).await, ServerEvent::Request { id: found, request: ClientRequest::Health } if found == id));
        responses.send(HostResponse::Error { message: "ordered response".into() }).await.unwrap();
        assert!(matches!(timeout_at(deadline(), peer.reader.read::<HostResponse>()).await.unwrap().unwrap(), Some(HostResponse::Error { message }) if message == "ordered response"));
        drop((peer, responses));
        assert!(matches!(next(&mut server).await, ServerEvent::Disconnected { id: found } if found == id));
        assert!(proof.upgrade().is_none());
        timeout_at(deadline(), server.shutdown()).await.unwrap().unwrap();
        server.shutdown().await.unwrap();
        assert!(endpoint.read_ready().unwrap().is_none());
    });
}

#[test]
fn invalid_native_hello_is_refused_before_host_admission() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let (mut server, mut progress) = bind(&endpoint, None);
        let mut peer = connect(server.metadata()).await;
        let mut request = hello(server.metadata());
        if let ClientRequest::Hello { protocol, .. } = &mut request {
            *protocol += 1;
        }
        write_message(&mut peer.writer, &request).await.unwrap();
        assert!(matches!(
            timeout_at(deadline(), peer.reader.read::<HostResponse>())
                .await
                .unwrap()
                .unwrap(),
            Some(HostResponse::Refused { .. })
        ));
        wait_count(&mut progress, 0).await;
        assert!(server.events.try_recv().is_err());
        server.shutdown().await.unwrap();
    });
}

#[test]
fn shutdown_bounds_partial_handshake_and_blocked_read_before_publication_cleanup() {
    runtime().block_on(async {
        for partial_handshake in [true, false] {
            let (_root, endpoint) = fixture();
            let (audit, retained, checked) = audit(&endpoint);
            let (mut server, mut progress) = bind(&endpoint, Some(audit));
            let (peer, responses) = if partial_handshake {
                let mut peer = connect(server.metadata()).await;
                peer.writer.write_all(b"{\"Hello\":").await.unwrap();
                (peer, None)
            } else {
                let (peer, _, responses, proof) = admitted(&mut server).await;
                *retained.lock().unwrap() = Some(proof);
                // Keep the response queue live through shutdown: the connection
                // cannot finish just because the host sender was dropped.
                (peer, Some(responses))
            };
            wait_count(&mut progress, 1).await;
            timeout_at(deadline(), server.shutdown())
                .await
                .unwrap()
                .unwrap();
            assert!(checked.load(Ordering::Acquire));
            assert!(endpoint.read_ready().unwrap().is_none());
            drop((peer, responses));
        }
    });
}

#[test]
fn cancelled_shutdown_retains_owner_and_drop_aborts_before_retiring_publication() {
    runtime().block_on(async {
        for drop_server in [false, true] {
            let (_root, endpoint) = fixture();
            let (audit, retained, checked) = audit(&endpoint);
            let (mut server, _) = bind(&endpoint, Some(audit));
            let (peer, _, responses, proof) = admitted(&mut server).await;
            *retained.lock().unwrap() = Some(proof.clone());
            let mut stopping = Box::pin(server.shutdown());
            pending(&mut stopping).await;
            drop(stopping);
            assert!(server.task.is_some());
            if drop_server {
                drop(server);
                timeout_at(deadline(), async {
                    while !checked.load(Ordering::Acquire) {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await
                .unwrap();
            } else {
                timeout_at(deadline(), server.shutdown())
                    .await
                    .unwrap()
                    .unwrap();
            }
            assert!(checked.load(Ordering::Acquire));
            assert!(proof.upgrade().is_none());
            assert!(endpoint.read_ready().unwrap().is_none());
            drop((peer, responses));
        }
    });
}

#[test]
fn saturation_keeps_existing_peers_progressing_and_admits_waiter_after_release() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let (mut server, mut progress) = bind(&endpoint, None);
        let mut peers = Vec::new();
        for _ in 0..windows_pipe::MAX_CONNECTIONS { peers.push(admitted(&mut server).await); }
        wait_count(&mut progress, windows_pipe::MAX_CONNECTIONS).await;
        let mut waiting = connect(server.metadata()).await;
        write_message(&mut waiting.writer, &hello(server.metadata())).await.unwrap();
        write_message(&mut peers[0].0.writer, &ClientRequest::Health).await.unwrap();
        assert!(matches!(next(&mut server).await, ServerEvent::Request { id, request: ClientRequest::Health } if id == peers[0].1));
        assert_eq!(*progress.borrow(), windows_pipe::MAX_CONNECTIONS);
        let released = peers.pop().unwrap();
        let released_id = released.1;
        drop(released);
        assert!(matches!(next(&mut server).await, ServerEvent::Disconnected { id } if id == released_id));
        let ServerEvent::Connected { responses, .. } = next(&mut server).await else { panic!("waiting client was not admitted"); };
        responses.send(welcome()).await.unwrap();
        assert!(matches!(timeout_at(deadline(), waiting.reader.read::<HostResponse>()).await.unwrap().unwrap(), Some(HostResponse::Welcome { .. })));
        drop((peers, waiting, responses));
        timeout_at(deadline(), server.shutdown()).await.unwrap().unwrap();
    });
}

#[test]
fn shutdown_drains_host_event_backpressure_to_allow_queued_final_response() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let (mut server, _) = bind_with_budget(&endpoint, None, FINAL_RESPONSE_DRAIN);
        let (mut peer, _, responses, _) = admitted(&mut server).await;
        for _ in 0..EVENT_CAPACITY + 4 {
            write_message(&mut peer.writer, &ClientRequest::Health)
                .await
                .unwrap();
        }
        timeout_at(deadline(), async {
            while server.events.len() != EVENT_CAPACITY {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        responses.try_send(HostResponse::ShuttingDown).unwrap();
        let receiving = async move {
            let received = peer.reader.read::<HostResponse>().await;
            drop(peer);
            received
        };
        let (stopped, received) = timeout_at(deadline(), async {
            tokio::join!(server.shutdown(), receiving)
        })
        .await
        .unwrap();
        stopped.unwrap();
        assert!(matches!(
            received.unwrap(),
            Some(HostResponse::ShuttingDown)
        ));
        assert!(endpoint.read_ready().unwrap().is_none());
    });
}

#[test]
fn shutdown_cancels_native_write_backpressure_and_releases_peer_proof() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let (audit, retained, checked) = audit(&endpoint);
        let (mut server, _) = bind(&endpoint, Some(audit));
        let (peer, _, responses, proof) = admitted(&mut server).await;
        *retained.lock().unwrap() = Some(proof.clone());
        responses
            .send(HostResponse::Error {
                message: "x".repeat(256 * 1024),
            })
            .await
            .unwrap();
        for _ in 1..crate::workspace::transport_shared::RESPONSE_CAPACITY {
            responses
                .try_send(HostResponse::Error {
                    message: "queued".into(),
                })
                .unwrap();
        }
        // Completion means the connection took the large first response and
        // began writing; its unread native peer cannot accept that full payload.
        timeout_at(
            deadline(),
            responses.send(HostResponse::Error {
                message: "last".into(),
            }),
        )
        .await
        .unwrap()
        .unwrap();
        timeout_at(deadline(), server.shutdown())
            .await
            .unwrap()
            .unwrap();
        assert!(proof.upgrade().is_none());
        assert!(checked.load(Ordering::Acquire));
        assert!(endpoint.read_ready().unwrap().is_none());
        drop((peer, responses));
    });
}

#[test]
fn owner_panic_reaches_shutdown_after_connection_drop_before_publication_cleanup() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
        let client = windows_pipe::connect(listener.metadata(), deadline())
            .await
            .unwrap();
        let stream = listener.accept(deadline()).await.unwrap();
        let peer = stream.peer().clone();
        let proof = Arc::downgrade(&peer);
        let project = listener.metadata().project_root_bytes.clone();
        let (events, _event_receiver) = mpsc::channel(EVENT_CAPACITY);
        let connections: FuturesUnordered<ConnectionFuture> = FuturesUnordered::new();
        connections.push(Box::pin(async move {
            let _ = serve_connection_with_peer(1, stream, events, project, peer).await;
        }));
        connections.push(Box::pin(async {
            panic!("injected native connection future panic");
        }));

        let observed = Arc::new(AtomicBool::new(false));
        let proper_order = Arc::new(AtomicBool::new(false));
        let seen = observed.clone();
        let ordered = proper_order.clone();
        let audit_proof = proof.clone();
        let audit_endpoint = endpoint.clone();
        // This audit runs during unwind. Record observations only: asserting
        // here could cause a double panic that hides the ownership regression.
        let audit = DropAudit(Some(Box::new(move || {
            let stream_proof_dropped = audit_proof.upgrade().is_none();
            let publication_present = matches!(audit_endpoint.read_ready(), Ok(Some(_)));
            ordered.store(
                stream_proof_dropped && publication_present,
                Ordering::Release,
            );
            seen.store(true, Ordering::Release);
        })));
        let mut server = LocalServer::from_owner(Owner {
            connections,
            _before_listener_drop: Some(audit),
            listener,
            drain_budget: FINAL_RESPONSE_DRAIN,
            observer: None,
        });
        let error = timeout_at(deadline(), server.shutdown())
            .await
            .unwrap()
            .unwrap_err();
        let panic = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<tokio::task::JoinError>())
            .unwrap();
        assert!(panic.is_panic());
        assert!(observed.load(Ordering::Acquire));
        assert!(proper_order.load(Ordering::Acquire));
        assert!(proof.upgrade().is_none());
        assert!(endpoint.read_ready().unwrap().is_none());
        drop(client);
    });
}

#[test]
fn named_server_canceled_rename_updates_snapshot_while_other_peer_io_progresses() {
    runtime().block_on(async {
        let (root, endpoint) = fixture();
        let names = NameStore::open(&root.join("state")).unwrap();
        let (entered, entry) = oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let mut entered = Some(entered);
        let (mut server, _) = managed(&endpoint, names.clone(), None, move |publication, names, name| {
            if let Some(entered) = entered.take() { let _ = entered.send(()); }
            released.recv_timeout(Duration::from_secs(5)).map_err(io::Error::other)?;
            publication.rename(names, name)
        });
        let (mut peer, id, responses, _) = admitted(&mut server).await;
        let ticket = server.try_rename("new-name").unwrap();
        timeout_at(deadline(), entry).await.unwrap().unwrap();
        let mut waiting = Box::pin(ticket.wait());
        pending(&mut waiting).await;
        drop(waiting);
        write_message(&mut peer.writer, &ClientRequest::Health).await.unwrap();
        assert!(matches!(next(&mut server).await, ServerEvent::Request { id: found, request: ClientRequest::Health } if found == id));
        responses.send(HostResponse::HostRenamed { name: "independent-response".into() }).await.unwrap();
        assert!(matches!(timeout_at(deadline(), peer.reader.read::<HostResponse>()).await.unwrap().unwrap(), Some(HostResponse::HostRenamed { name }) if name == "independent-response"));
        release.send(()).unwrap();
        timeout_at(deadline(), async {
            while server.metadata_snapshot().name.as_deref() != Some("new-name") {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        assert_eq!(endpoint.read_ready().unwrap().unwrap().name.as_deref(), Some("new-name"));
        assert_eq!(names.load(&server.metadata().id).unwrap().as_deref(), Some("new-name"));
        assert!(server.metadata().name.is_none(), "borrowed metadata remains the documented bind snapshot");
        drop(responses);
        drop(peer);
        server.shutdown().await.unwrap();
        assert!(endpoint.read_ready().unwrap().is_none());
    });
}

#[test]
fn managed_shutdown_drops_connections_before_waiting_for_active_publication_work() {
    runtime().block_on(async {
        let (root, endpoint) = fixture();
        let names = NameStore::open(&root.join("state")).unwrap();
        let (drop_audit, retained, checked) = audit(&endpoint);
        let (entered, entry) = oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let mut entered = Some(entered);
        let (mut server, mut progress) = managed(
            &endpoint,
            names,
            Some(drop_audit),
            move |publication, names, name| {
                if let Some(entered) = entered.take() {
                    let _ = entered.send(());
                }
                released
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(io::Error::other)?;
                publication.rename(names, name)
            },
        );
        let (peer, _, responses, proof) = admitted(&mut server).await;
        *retained.lock().unwrap() = Some(proof.clone());
        let ticket = server.try_rename("finishing").unwrap();
        timeout_at(deadline(), entry).await.unwrap().unwrap();
        let mut stopping = Box::pin(server.shutdown());
        pending(&mut stopping).await;
        wait_count(&mut progress, 0).await;
        assert!(proof.upgrade().is_none());
        assert!(checked.load(Ordering::Acquire));
        assert!(endpoint.read_ready().unwrap().is_some());
        drop(stopping);
        release.send(()).unwrap();
        ticket.wait().await.unwrap();
        server.shutdown().await.unwrap();
        assert!(endpoint.read_ready().unwrap().is_none());
        drop(responses);
        drop(peer);
    });
}

#[test]
fn publication_worker_panic_reaches_host_after_connection_owner_cleanup() {
    runtime().block_on(async {
        let (root, endpoint) = fixture();
        let names = NameStore::open(&root.join("state")).unwrap();
        let (drop_audit, retained, checked) = audit(&endpoint);
        let (mut server, _) = managed(
            &endpoint,
            names,
            Some(drop_audit),
            |publication, names, name| {
                publication.rename(names, name)?;
                panic!("injected named server publication worker failure");
            },
        );
        let (peer, _, responses, proof) = admitted(&mut server).await;
        *retained.lock().unwrap() = Some(proof.clone());
        assert!(
            server
                .try_rename("committed")
                .unwrap()
                .wait()
                .await
                .is_err()
        );
        assert!(server.shutdown().await.is_err());
        assert!(checked.load(Ordering::Acquire));
        assert!(proof.upgrade().is_none());
        assert!(endpoint.read_ready().unwrap().is_none());
        assert_eq!(
            server.metadata_snapshot().name.as_deref(),
            Some("committed")
        );
        drop(responses);
        drop(peer);
    });
}

#[test]
fn stopping_admission_preserves_existing_io_until_explicit_shutdown() {
    runtime().block_on(async {
        let (_root, endpoint) = fixture();
        let (mut server, _) = bind(&endpoint, None);
        let (mut active, id, responses, _) = admitted(&mut server).await;
        assert_eq!(server.try_rename("unsupported").unwrap_err().kind(), io::ErrorKind::Unsupported);
        server.stop_admission();
        let mut waiting = connect(server.metadata()).await;
        write_message(&mut waiting.writer, &hello(server.metadata())).await.unwrap();
        write_message(&mut active.writer, &ClientRequest::Health).await.unwrap();
        assert!(matches!(next(&mut server).await, ServerEvent::Request { id: found, request: ClientRequest::Health } if found == id));
        assert!(tokio::time::timeout(Duration::from_millis(50), server.recv()).await.is_err(), "new peer was admitted after stop_admission");
        assert!(endpoint.read_ready().unwrap().is_some());
        drop(responses);
        drop(active);
        drop(waiting);
        server.shutdown().await.unwrap();
        assert!(endpoint.read_ready().unwrap().is_none());
    });
}

#[test]
fn dropping_managed_server_retires_only_after_live_connection_owners() {
    runtime().block_on(async {
        let (root, endpoint) = fixture();
        let names = NameStore::open(&root.join("state")).unwrap();
        let (drop_audit, retained, checked) = audit(&endpoint);
        let (mut server, _) = managed(
            &endpoint,
            names,
            Some(drop_audit),
            crate::workspace::windows_endpoint::Publication::rename,
        );
        let (peer, _, responses, proof) = admitted(&mut server).await;
        *retained.lock().unwrap() = Some(proof.clone());
        server
            .try_rename("before-drop")
            .unwrap()
            .wait()
            .await
            .unwrap();
        drop(server);
        timeout_at(deadline(), async {
            while endpoint.read_ready().unwrap().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(checked.load(Ordering::Acquire));
        assert!(proof.upgrade().is_none());
        drop(responses);
        drop(peer);
    });
}
