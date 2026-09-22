// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    protocol::WaitStatus,
    test_support::TestRuntimeRoot,
    workspace::{
        windows_endpoint::{EndpointLocation, RegistrySet},
        windows_pipe::Listener,
        windows_process_identity::ProcessIdentity,
    },
};
use std::{
    future::{Future, poll_fn},
    io,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::NamedPipeServer,
    time::timeout,
};

fn fixture() -> (TestRuntimeRoot, EndpointLocation) {
    let root = TestRuntimeRoot::new("native-protocol-client").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let registries = RegistrySet::open(&[root.join("registry")]).unwrap();
    let endpoint = EndpointLocation::new(&project, root.join("endpoint"), registries).unwrap();
    (root, endpoint)
}

fn geometry() -> FrameGeometry {
    use crate::layout::Rect;
    FrameGeometry {
        screen: Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 32,
        },
        editor: Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 30,
        },
        status: Rect {
            x: 0,
            y: 30,
            width: 100,
            height: 1,
        },
        message: Rect {
            x: 0,
            y: 31,
            width: 100,
            height: 1,
        },
    }
}

async fn pair(
    endpoint: &EndpointLocation,
    interactive: bool,
    handoff: bool,
) -> (Listener, Connection<NamedPipeServer>, LocalClient) {
    let mut listener = Listener::bind(endpoint.prepare(None).unwrap()).unwrap();
    let metadata = listener.metadata().clone();
    let (server, client) = tokio::join!(
        listener.accept(Instant::now() + Duration::from_secs(5)),
        LocalClient::connect_with_handoff(&metadata, geometry(), interactive, handoff)
    );
    (listener, server.unwrap(), client.unwrap())
}

fn completion() -> HostResponse {
    HostResponse::WaitState {
        token: crate::workspace::WaitToken::new(7).into(),
        status: WaitStatus::Completed,
        interactive_attached: true,
    }
}

async fn pending<F: Future>(future: &mut Pin<Box<F>>) {
    poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn native_hello_preserves_identity_role_features_and_directory_handoff() {
    for interactive in [false, true] {
        let (_root, endpoint) = fixture();
        let (listener, mut server, mut client) = pair(&endpoint, interactive, true).await;
        assert_eq!(
            client.peer().identity(),
            ProcessIdentity::current().unwrap()
        );
        let mut reader = MessageReader::new(&mut server);
        let hello = timeout(Duration::from_secs(5), reader.read::<ClientRequest>())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(hello.validate().is_ok());
        let ClientRequest::Hello {
            protocol,
            features,
            client_kind,
            client_version,
            geometry: received_geometry,
            role,
            directory_handoff,
            project_root_bytes,
        } = hello
        else {
            panic!("native client did not send Hello first");
        };
        assert_eq!(protocol, PROTOCOL_VERSION);
        assert_eq!(client_version, CLIENT_VERSION);
        assert_eq!(received_geometry, geometry().into());
        assert_eq!(
            client_kind,
            if interactive {
                ClientKind::Tui
            } else {
                ClientKind::Control
            }
        );
        assert_eq!(
            features,
            if interactive {
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
            }
        );
        assert_eq!(
            role,
            if interactive {
                ClientRole::Interactive
            } else {
                ClientRole::Control
            }
        );
        assert!(directory_handoff);
        assert_eq!(project_root_bytes, listener.metadata().project_root_bytes);
        client.send(&ClientRequest::Health).await.unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(5), reader.read::<ClientRequest>())
                .await
                .unwrap()
                .unwrap(),
            Some(ClientRequest::Health)
        ));
    }
}

#[tokio::test]
async fn cancelled_native_send_poisoning_preserves_incoming_wait_completion() {
    let (_root, endpoint) = fixture();
    let (_listener, mut server, mut client) = pair(&endpoint, true, false).await;
    let hello: Option<ClientRequest> = timeout(
        Duration::from_secs(5),
        MessageReader::new(&mut server).read(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(hello, Some(ClientRequest::Hello { .. })));
    write_message(&mut server, &completion()).await.unwrap();
    let request = ClientRequest::Notify {
        message: "x".repeat(128 * 1024),
    };
    let mut sending = Box::pin(client.send(&request));
    pending(&mut sending).await;
    drop(sending);
    assert!(client.writer.is_none());
    assert!(
        client
            .send(&ClientRequest::Health)
            .await
            .unwrap_err()
            .to_string()
            .contains("writer is closed")
    );
    let response = timeout(Duration::from_secs(5), client.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        response,
        Some(HostResponse::WaitState {
            status: WaitStatus::Completed,
            ..
        })
    ));
    // Confirm cancellation happened after a request prefix was sent, rather
    // than before the native writer ever accepted any bytes.
    let mut prefix = [0; 1];
    timeout(Duration::from_secs(5), server.read_exact(&mut prefix))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(prefix, [b'{']);
}

#[tokio::test]
async fn native_receive_retains_partial_framing_when_cancelled() {
    let (_root, endpoint) = fixture();
    let (_listener, mut server, mut client) = pair(&endpoint, false, false).await;
    let mut bytes = serde_json::to_vec(&completion()).unwrap();
    bytes.push(b'\n');
    let middle = bytes.len() / 2;
    timeout(Duration::from_secs(5), async {
        server.write_all(&bytes[..middle]).await.unwrap();
        server.flush().await.unwrap();
        // A Pending poll alone could precede native read completion. Observe
        // retained framing bytes so cancellation definitely interrupts a frame.
        loop {
            let mut receiving = Box::pin(client.recv());
            pending(&mut receiving).await;
            drop(receiving);
            if client.reader.pending_bytes_for_test() == middle {
                break;
            }
            tokio::task::yield_now().await;
        }
        server.write_all(&bytes[middle..]).await.unwrap();
        server.flush().await.unwrap();
    })
    .await
    .unwrap();
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
}

struct FailingWriter;
impl AsyncWrite for FailingWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut TaskContext<'_>,
        _: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "fixture write failure",
        )))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        panic!("failed write must not be flushed");
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        panic!("native shutdown could wait indefinitely after a failed write");
    }
}

#[tokio::test]
async fn failed_send_returns_original_error_without_unbounded_shutdown() {
    let mut writer = Some(FailingWriter);
    let error = send_request(&mut writer, &ClientRequest::Health)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("fixture write failure"));
    assert!(writer.is_none());
    assert!(
        send_request(&mut writer, &ClientRequest::Health)
            .await
            .unwrap_err()
            .to_string()
            .contains("writer is closed")
    );
}

#[tokio::test]
async fn incompatible_metadata_is_refused_before_sending_a_handshake() {
    let (_root, endpoint) = fixture();
    let prepared = endpoint.prepare(None).unwrap();
    let mut metadata = prepared.metadata().clone();
    metadata.protocol = PROTOCOL_VERSION + 1;
    let error = LocalClient::connect(&metadata, FrameGeometry::default(), false)
        .await
        .err()
        .expect("incompatible metadata accepted");
    assert!(error.to_string().contains("incompatible"));
}
