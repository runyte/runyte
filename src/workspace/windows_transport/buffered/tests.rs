// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    protocol::{ClientKind, ClientRole, FeatureGroup, WaitStatus},
    test_support::TestRuntimeRoot,
    workspace::{
        windows_endpoint::{EndpointLocation, RegistrySet},
        windows_pipe::Listener,
    },
};
use futures_util::task::AtomicWaker;
use std::{
    future::poll_fn,
    io,
    pin::Pin,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context as TaskContext, Poll},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadBuf},
    time::timeout,
};

const BLOCK: usize = 1;
const FAIL: usize = 2;
const PARTIAL: usize = 3;

#[derive(Default)]
struct Probe {
    mode: AtomicUsize,
    remaining: AtomicUsize,
    written: AtomicUsize,
    write_polled: AtomicBool,
    read_panic: AtomicBool,
    reader: AtomicWaker,
    writer: AtomicWaker,
    read_bytes: Mutex<usize>,
    read_changed: Condvar,
}

struct ProbeStream {
    stream: DuplexStream,
    probe: Arc<Probe>,
    _proof: Arc<()>,
}

impl AsyncRead for ProbeStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.probe.reader.register(cx.waker());
        assert!(
            !self.probe.read_panic.load(Ordering::Acquire),
            "injected buffered reader panic"
        );
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.stream).poll_read(cx, buffer);
        let count = buffer.filled().len() - before;
        if count != 0 {
            *self.probe.read_bytes.lock().unwrap() += count;
            self.probe.read_changed.notify_all();
        }
        result
    }
}

impl AsyncWrite for ProbeStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.probe.writer.register(cx.waker());
        self.probe.write_polled.store(true, Ordering::Release);
        let mode = self.probe.mode.load(Ordering::Acquire);
        if mode == FAIL {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "injected request failure",
            )));
        }
        if mode == BLOCK {
            return Poll::Pending;
        }
        let bytes = if mode == PARTIAL {
            let remaining = self.probe.remaining.load(Ordering::Acquire);
            if remaining == 0 {
                return Poll::Pending;
            }
            &bytes[..bytes.len().min(remaining)]
        } else {
            bytes
        };
        let result = Pin::new(&mut self.stream).poll_write(cx, bytes);
        if let Poll::Ready(Ok(count)) = result {
            self.probe.written.fetch_add(count, Ordering::AcqRel);
            if mode == PARTIAL {
                self.probe.remaining.fetch_sub(count, Ordering::AcqRel);
            }
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        panic!("outgoing invalidation must not invoke native shutdown")
    }
}

fn hello() -> ClientRequest {
    ClientRequest::Hello {
        protocol: PROTOCOL_VERSION,
        features: vec![
            FeatureGroup::Snapshots,
            FeatureGroup::Input,
            FeatureGroup::Buffers,
            FeatureGroup::Wait,
        ],
        project_root_bytes: crate::protocol::encode_path(std::path::Path::new(
            r"C:\buffered-fixture",
        )),
        client_kind: ClientKind::Tui,
        client_version: crate::protocol::CLIENT_VERSION.into(),
        role: ClientRole::Interactive,
        geometry: FrameGeometry::default().into(),
        directory_handoff: true,
    }
}

fn welcome() -> HostResponse {
    HostResponse::Welcome {
        protocol: PROTOCOL_VERSION,
        pid: std::process::id(),
        features: vec![FeatureGroup::Snapshots, FeatureGroup::Input],
        host_version: crate::protocol::CLIENT_VERSION.into(),
    }
}

fn completion() -> HostResponse {
    HostResponse::WaitState {
        token: crate::workspace::WaitToken::new(7).into(),
        status: WaitStatus::Completed,
        interactive_attached: true,
    }
}

fn frame() -> HostResponse {
    let app = crate::app::App::new(crate::config::Config::default(), None).unwrap();
    let mut host = crate::workspace::WorkspaceHost::new(app);
    HostResponse::Frame {
        frame: Box::new(host.prepare_frame(FrameGeometry::default()).into()),
    }
}

fn controlled() -> (ProbeStream, DuplexStream, Arc<Probe>, Weak<()>) {
    let (stream, host) = tokio::io::duplex(64 * 1024);
    let probe = Arc::new(Probe::default());
    let proof = Arc::new(());
    let weak = Arc::downgrade(&proof);
    (
        ProbeStream {
            stream,
            probe: probe.clone(),
            _proof: proof,
        },
        host,
        probe,
        weak,
    )
}

async fn pair() -> (BufferedLocalClient, DuplexStream, Arc<Probe>, Weak<()>) {
    let (stream, mut host, probe, proof) = controlled();
    let client = BufferedLocalClient::start(move || async { Ok(stream) }, hello())
        .await
        .unwrap();
    assert!(matches!(
        MessageReader::new(&mut host)
            .read::<ClientRequest>()
            .await
            .unwrap(),
        Some(ClientRequest::Hello { .. })
    ));
    probe.written.store(0, Ordering::Release);
    probe.write_polled.store(false, Ordering::Release);
    (client, host, probe, proof)
}

async fn until(mut predicate: impl FnMut() -> bool) {
    timeout(Duration::from_secs(5), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(1)).await;
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

fn bytes(response: &HostResponse) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(response).unwrap();
    bytes.push(b'\n');
    bytes
}

#[tokio::test]
async fn startup_cancellation_joins_during_connect_and_hello_and_startup_errors_release_stream() {
    for during_hello in [false, true] {
        let (stream, _host, probe, proof) = controlled();
        probe.mode.store(BLOCK, Ordering::Release);
        let (entered, entered_rx) = oneshot::channel();
        let mut startup = Box::pin(BufferedLocalClient::start(
            move || async move {
                let _ = entered.send(());
                if !during_hello {
                    std::future::pending::<()>().await;
                }
                Ok(stream)
            },
            hello(),
        ));
        pending(&mut startup).await;
        timeout(Duration::from_secs(5), entered_rx)
            .await
            .unwrap()
            .unwrap();
        if during_hello {
            until(|| probe.write_polled.load(Ordering::Acquire)).await;
        }
        drop(startup);
        assert!(
            proof.upgrade().is_none(),
            "abandoned startup detached its worker/stream"
        );
    }
    let (stream, _host, probe, proof) = controlled();
    probe.mode.store(FAIL, Ordering::Release);
    let error = BufferedLocalClient::start(move || async { Ok(stream) }, hello())
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("injected request failure"));
    assert!(proof.upgrade().is_none());
    let error = BufferedLocalClient::start(
        || async {
            panic!("injected startup panic");
            #[allow(unreachable_code)]
            Ok::<DuplexStream, anyhow::Error>(tokio::io::duplex(1).0)
        },
        hello(),
    )
    .await
    .err()
    .unwrap();
    assert!(error.to_string().contains("panicked"));
}

fn send_harness() -> (
    BufferedLocalClient,
    mpsc::Receiver<Outgoing>,
    watch::Receiver<bool>,
    ResponseSender,
) {
    let (requests, receiver) = mpsc::channel(1);
    let (cancel, cancelled) = watch::channel(false);
    let (responses, response_rx) = response_channel();
    let (stop, _) = watch::channel(false);
    (
        BufferedLocalClient {
            outgoing: Some(OutgoingCapability { requests, cancel }),
            responses: response_rx,
            worker: Worker { stop, thread: None },
            peer: None,
        },
        receiver,
        cancelled,
        responses,
    )
}

#[tokio::test]
async fn escaping_oversize_is_refused_before_outgoing_queue_ownership() {
    let (mut client, mut requests, cancelled, _responses) = send_harness();
    let request = ClientRequest::Notify {
        message: "\0".repeat(crate::workspace::transport_shared::MAX_MESSAGE_BYTES / 6),
    };
    let error = client.send(&request).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("workspace transport message exceeds")
    );
    assert!(
        requests.try_recv().is_err(),
        "oversized request reached the worker queue"
    );
    assert!(client.outgoing.is_none());
    assert!(*cancelled.borrow());
}

#[tokio::test]
async fn send_cancellation_before_queue_after_queue_and_after_unobserved_ack_permanently_poison() {
    for boundary in 0..3 {
        let (mut client, mut requests, cancelled, _responses) = send_harness();
        if boundary == 0 {
            let (acknowledgement, _ack) = oneshot::channel();
            assert!(
                client
                    .outgoing
                    .as_ref()
                    .unwrap()
                    .requests
                    .try_send(Outgoing {
                        message: encode_message(&ClientRequest::Health).unwrap(),
                        acknowledgement
                    })
                    .is_ok()
            );
        }
        let mut sending = Box::pin(client.send(&ClientRequest::ListBuffers));
        pending(&mut sending).await;
        let outgoing = if boundary != 0 {
            Some(requests.try_recv().unwrap())
        } else {
            None
        };
        if boundary == 2 {
            outgoing.unwrap().acknowledgement.send(Ok(())).unwrap();
        }
        drop(sending);
        assert!(client.outgoing.is_none());
        assert!(*cancelled.borrow());
        assert!(
            client
                .send(&ClientRequest::Health)
                .await
                .unwrap_err()
                .to_string()
                .contains("writer is closed")
        );
    }
    let (mut client, mut requests, cancelled, _responses) = send_harness();
    let mut sending = Box::pin(client.send(&ClientRequest::Health));
    pending(&mut sending).await;
    requests
        .try_recv()
        .unwrap()
        .acknowledgement
        .send(Ok(()))
        .unwrap();
    sending.await.unwrap();
    assert!(client.outgoing.is_some());
    assert!(!*cancelled.borrow());
}

#[tokio::test]
async fn cancelled_and_failed_partial_sends_preserve_completion_before_and_during_failure() {
    for fail in [false, true] {
        for completion_first in [false, true] {
            let (mut client, mut host, probe, proof) = pair().await;
            probe.mode.store(PARTIAL, Ordering::Release);
            probe.remaining.store(1, Ordering::Release);
            if completion_first {
                write_message(&mut host, &completion()).await.unwrap();
            }
            let request = ClientRequest::Notify {
                message: "partial request".into(),
            };
            let mut sending = Box::pin(client.send(&request));
            pending(&mut sending).await;
            until(|| probe.written.load(Ordering::Acquire) == 1).await;
            if !completion_first {
                write_message(&mut host, &completion()).await.unwrap();
            }
            if fail {
                probe.mode.store(FAIL, Ordering::Release);
                probe.writer.wake();
                assert!(
                    timeout(Duration::from_secs(5), sending)
                        .await
                        .unwrap()
                        .unwrap_err()
                        .to_string()
                        .contains("injected request failure")
                );
            } else {
                drop(sending);
            }
            assert!(client.outgoing.is_none());
            assert!(matches!(
                timeout(Duration::from_secs(5), client.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                Some(HostResponse::WaitState {
                    status: WaitStatus::Completed,
                    ..
                })
            ));
            assert!(client.send(&ClientRequest::Health).await.is_err());
            let mut prefix = [0];
            host.read_exact(&mut prefix).await.unwrap();
            assert_eq!(prefix, [b'{']);
            assert!(proof.upgrade().is_some(), "send failure closed the reader");
            drop(client);
            assert!(proof.upgrade().is_none());
        }
    }
}

#[tokio::test]
async fn worker_observes_abandoned_ack_even_without_explicit_cancel() {
    let (stream, _host, probe, proof) = controlled();
    probe.mode.store(BLOCK, Ordering::Release);
    let (requests, receiver) = mpsc::channel(1);
    let (_cancel, cancelled) = watch::channel(false);
    let (acknowledgement, acknowledged) = oneshot::channel();
    assert!(
        requests
            .send(Outgoing {
                message: encode_message(&ClientRequest::Health).unwrap(),
                acknowledgement
            })
            .await
            .is_ok()
    );
    let mut writing = Box::pin(write_requests(stream, receiver, cancelled));
    pending(&mut writing).await;
    assert!(probe.write_polled.load(Ordering::Acquire));
    drop(acknowledged);
    timeout(Duration::from_secs(5), writing).await.unwrap();
    assert!(proof.upgrade().is_none());
}

#[tokio::test]
async fn response_reader_runs_while_frontend_thread_blocks_and_receive_cancellation_keeps_partial_frame()
 {
    let (mut client, mut host, probe, proof) = pair().await;
    let encoded = bytes(&completion());
    let middle = encoded.len() / 2;
    host.write_all(&encoded[..middle]).await.unwrap();
    // This deliberately blocks the sole frontend Tokio thread. Only the
    // dedicated worker can satisfy the condition variable with read progress.
    {
        let observed = probe.read_bytes.lock().unwrap();
        let (observed, result) = probe
            .read_changed
            .wait_timeout_while(observed, Duration::from_secs(5), |count| *count < middle)
            .unwrap();
        assert!(!result.timed_out());
        assert_eq!(*observed, middle);
    }
    let mut receiving = Box::pin(client.recv());
    pending(&mut receiving).await;
    drop(receiving);
    host.write_all(&encoded[middle..]).await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(5), client.recv())
            .await
            .unwrap()
            .unwrap(),
        Some(HostResponse::WaitState {
            status: WaitStatus::Completed,
            ..
        })
    ));
    drop(client);
    assert!(proof.upgrade().is_none());
}

#[tokio::test]
async fn queued_responses_precede_terminal_read_or_worker_panic_error_exactly_once() {
    for panic_reader in [false, true] {
        let (mut client, mut host, probe, proof) = pair().await;
        let encoded = bytes(&completion());
        host.write_all(&encoded).await.unwrap();
        until(|| *probe.read_bytes.lock().unwrap() >= encoded.len()).await;
        if panic_reader {
            probe.read_panic.store(true, Ordering::Release);
            probe.reader.wake();
        } else {
            host.write_all(b"invalid json\n").await.unwrap();
        }
        assert!(matches!(
            client.recv().await.unwrap(),
            Some(HostResponse::WaitState { .. })
        ));
        let error = timeout(Duration::from_secs(5), client.recv())
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains(if panic_reader {
            "panicked"
        } else {
            "malformed"
        }));
        assert!(client.recv().await.unwrap().is_none());
        assert!(proof.upgrade().is_none());
    }
}

#[tokio::test]
async fn drop_joins_idle_stalled_write_and_full_response_queue_workers() {
    for state in 0..3 {
        let (mut client, mut host, probe, proof) = pair().await;
        if state == 1 {
            probe.mode.store(BLOCK, Ordering::Release);
            let mut sending = Box::pin(client.send(&ClientRequest::Health));
            pending(&mut sending).await;
            until(|| probe.write_polled.load(Ordering::Acquire)).await;
            drop(sending);
        } else if state == 2 {
            let response = bytes(&HostResponse::Error {
                message: "x".repeat(8192),
            });
            let payload =
                response.repeat(crate::workspace::transport_shared::RESPONSE_CAPACITY + 2);
            timeout(Duration::from_secs(5), host.write_all(&payload))
                .await
                .unwrap()
                .unwrap();
            let full = response.len() * (crate::workspace::transport_shared::RESPONSE_CAPACITY + 1);
            until(|| *probe.read_bytes.lock().unwrap() >= full).await;
        }
        drop(client);
        assert!(proof.upgrade().is_none(), "Drop detached an owned worker");
    }
}

#[tokio::test]
async fn real_native_buffered_client_authenticates_and_fully_writes_hello_before_startup() {
    let root = TestRuntimeRoot::new("buffered-native-client").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let endpoint = EndpointLocation::new(
        &project,
        root.join("endpoint"),
        RegistrySet::open(&[root.join("registry")]).unwrap(),
    )
    .unwrap();
    let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
    let metadata = listener.metadata().clone();
    let (server, client) = tokio::join!(
        listener.accept(Instant::now() + Duration::from_secs(5)),
        BufferedLocalClient::connect_with_handoff(&metadata, FrameGeometry::default(), true)
    );
    let mut server = server.unwrap();
    let mut client = client.unwrap();
    assert_eq!(client.peer().unwrap().identity().pid, std::process::id());
    assert!(client.peer().unwrap().is_alive().unwrap());
    let request = MessageReader::new(&mut server)
        .read::<ClientRequest>()
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(request, ClientRequest::Hello { directory_handoff: true, client_kind: ClientKind::Tui, role: ClientRole::Interactive, project_root_bytes, .. } if project_root_bytes == metadata.project_root_bytes)
    );
    write_message(&mut server, &welcome()).await.unwrap();
    assert!(matches!(
        client.recv_handshake().await.unwrap(),
        Some(HostResponse::Welcome { .. })
    ));
    client.send(&ClientRequest::Health).await.unwrap();
    assert!(matches!(
        MessageReader::new(&mut server)
            .read::<ClientRequest>()
            .await
            .unwrap(),
        Some(ClientRequest::Health)
    ));
    write_message(&mut server, &completion()).await.unwrap();
    assert!(matches!(
        client.recv().await.unwrap(),
        Some(HostResponse::WaitState {
            status: WaitStatus::Completed,
            ..
        })
    ));
    drop(client);
}

#[tokio::test]
async fn handshake_is_semantic_only_and_welcome_precedes_visual_or_final_slots() {
    for visual in [false, true] {
        for welcome_sent in [false, true] {
            let (mut client, mut host, probe, _) = pair().await;
            let encoded = bytes(&if visual {
                frame()
            } else {
                HostResponse::ShuttingDown
            });
            host.write_all(&encoded).await.unwrap();
            until(|| *probe.read_bytes.lock().unwrap() >= encoded.len()).await;
            let mut handshake = Box::pin(client.recv_handshake());
            pending(&mut handshake).await;
            drop(handshake);
            if welcome_sent {
                write_message(&mut host, &welcome()).await.unwrap();
            } else {
                drop(host);
            }
            let response = timeout(Duration::from_secs(5), client.recv_handshake())
                .await
                .unwrap()
                .unwrap();
            if welcome_sent {
                assert!(matches!(response, Some(HostResponse::Welcome { .. })));
            } else {
                assert!(response.is_none());
            }
            let queued = client.recv().await.unwrap();
            assert!(if visual {
                matches!(queued, Some(HostResponse::Frame { .. }))
            } else {
                matches!(queued, Some(HostResponse::ShuttingDown))
            });
        }
    }
}

#[tokio::test]
async fn reader_eof_or_failure_cancels_active_send_without_whole_client_drop() {
    for malformed in [false, true] {
        let (mut client, mut host, probe, proof) = pair().await;
        probe.mode.store(BLOCK, Ordering::Release);
        let mut sending = Box::pin(client.send(&ClientRequest::Health));
        pending(&mut sending).await;
        until(|| probe.write_polled.load(Ordering::Acquire)).await;
        if malformed {
            host.write_all(b"invalid json\n").await.unwrap();
        } else {
            drop(host);
        }
        assert!(
            timeout(Duration::from_secs(5), sending)
                .await
                .unwrap()
                .is_err()
        );
        let received = timeout(Duration::from_secs(5), client.recv())
            .await
            .unwrap();
        if malformed {
            assert!(received.unwrap_err().to_string().contains("malformed"));
        } else {
            assert!(received.unwrap().is_none());
        }
        assert!(client.outgoing.is_none());
        assert!(
            client.worker.thread.is_none(),
            "terminated reader was not joined"
        );
        assert!(
            proof.upgrade().is_none(),
            "read termination retained the stalled writer"
        );
        assert!(client.recv().await.unwrap().is_none());
    }
}
