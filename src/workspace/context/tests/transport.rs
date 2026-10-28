// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::mpsc,
};

#[tokio::test]
async fn revocation_discards_a_reply_waiting_for_publication() {
    let root = TestRuntimeRoot::new("ctx-reply").unwrap();
    let socket = root.path().join("socket");
    let (send, mut events) = mpsc::channel(4);
    let _server = Server::bind(socket.clone(), send).unwrap();
    let mut client = UnixStream::connect(socket).await.unwrap();
    client.write_all(b"{}\n").await.unwrap();
    let Event::Frame {
        reply, connection, ..
    } = events.recv().await.unwrap()
    else {
        panic!("expected request")
    };
    let lease = Arc::new(Lease::default());
    lease.cancel();
    reply
        .send(Reply {
            value: json!({"private":"must not leave"}),
            lease: Some(lease.clone()),
            close: false,
        })
        .ok()
        .unwrap();
    let mut bytes = [0u8; 64];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), client.read(&mut bytes))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(matches!(events.recv().await,Some(Event::Closed(id)) if id==connection));
    tokio::time::timeout(Duration::from_millis(100), lease.cancelled())
        .await
        .unwrap();
}

#[tokio::test]
async fn oversized_input_closes_without_dispatch_and_other_reader_still_runs() {
    let root = TestRuntimeRoot::new("ctx-bounds").unwrap();
    let socket = root.path().join("socket");
    let (send, mut events) = mpsc::channel(4);
    let _server = Server::bind(socket.clone(), send).unwrap();
    let mut client = UnixStream::connect(&socket).await.unwrap();
    let writer = tokio::spawn(async move {
        let _ = client.write_all(&vec![b'x'; FRAME_BYTES + 1]).await;
    });
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap(),
        Some(Event::Closed(_))
    ));
    writer.await.unwrap();
    let mut healthy = UnixStream::connect(socket).await.unwrap();
    healthy.write_all(b"{}\n").await.unwrap();
    let Event::Frame { reply, .. } = events.recv().await.unwrap() else {
        panic!("expected request")
    };
    reply
        .send(Reply {
            value: json!({"ok":true}),
            lease: None,
            close: true,
        })
        .ok()
        .unwrap();
    let mut line = String::new();
    BufReader::new(healthy).read_line(&mut line).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap(),
        json!({"ok":true})
    );
}

#[tokio::test]
async fn lease_cancellation_wakes_an_idle_reader_without_another_request() {
    let root = TestRuntimeRoot::new("ctx-idle").unwrap();
    let socket = root.path().join("socket");
    let (send, mut events) = mpsc::channel(4);
    let _server = Server::bind(socket.clone(), send).unwrap();
    let mut client = BufReader::new(UnixStream::connect(socket).await.unwrap());
    client.get_mut().write_all(b"{}\n").await.unwrap();
    let Event::Frame { reply, .. } = events.recv().await.unwrap() else {
        panic!("expected request")
    };
    let lease = Arc::new(Lease::default());
    reply
        .send(Reply {
            value: json!({"ready":true}),
            lease: Some(lease.clone()),
            close: false,
        })
        .ok()
        .unwrap();
    let mut line = String::new();
    client.read_line(&mut line).await.unwrap();
    assert!(line.contains("ready"));
    lease.cancel();
    line.clear();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), client.read_line(&mut line))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

async fn notification_line(client: &mut BufReader<UnixStream>) -> Value {
    let mut line = String::new();
    let bytes = tokio::time::timeout(Duration::from_secs(2), client.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert!(bytes > 0, "connection closed before notification/reply");
    serde_json::from_str(&line).unwrap()
}

#[tokio::test]
async fn unsolicited_notifications_preserve_a_request_fragmented_across_multiple_wakes() {
    let root = TestRuntimeRoot::new("ctx-notify-fragments").unwrap();
    let socket = root.path().join("socket");
    let (send, mut events) = mpsc::channel(4);
    let _server = Server::bind(socket.clone(), send).unwrap();
    let mut client = BufReader::new(UnixStream::connect(socket).await.unwrap());
    client.get_mut().write_all(b"{}\n").await.unwrap();
    let Event::Frame {
        reply, connection, ..
    } = events.recv().await.unwrap()
    else {
        panic!("initial request")
    };
    let lease = Arc::new(Lease::default());
    lease.enable_notifications(true);
    reply
        .send(Reply {
            value: json!({"ready":true}),
            lease: Some(lease.clone()),
            close: false,
        })
        .ok()
        .unwrap();
    assert_eq!(notification_line(&mut client).await, json!({"ready":true}));
    let fragments: [&[u8]; 3] = [
        b"{\"id\":\"frag",
        b"mented\",\"params\":{\"text\":\"hel",
        b"lo\"}}\n",
    ];
    for fragment in &fragments[..2] {
        client.get_mut().write_all(fragment).await.unwrap();
        lease.clear_notification();
        assert_eq!(lease.queue_notification(), "queued");
        assert_eq!(
            notification_line(&mut client).await,
            json!({"type":"channel_notification"})
        );
        assert!(
            events.try_recv().is_err(),
            "partial request must not dispatch"
        );
    }
    client.get_mut().write_all(fragments[2]).await.unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    let Event::Frame {
        bytes,
        connection: actual,
        reply,
        ..
    } = event
    else {
        panic!("completed request")
    };
    assert_eq!(actual, connection);
    assert_eq!(bytes, fragments.concat());
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"id":"fragmented","params":{"text":"hello"}})
    );
    reply
        .send(Reply {
            value: json!({"preserved":true}),
            lease: Some(lease),
            close: true,
        })
        .ok()
        .unwrap();
    assert_eq!(
        notification_line(&mut client).await,
        json!({"preserved":true})
    );
    assert!(
        matches!(tokio::time::timeout(Duration::from_secs(2), events.recv()).await.unwrap(), Some(Event::Closed(id)) if id==connection)
    );
}

#[tokio::test]
async fn revocation_before_notification_publication_closes_idle_and_partial_readers() {
    for partial in [false, true] {
        let root = TestRuntimeRoot::new("ctx-notify-revoke").unwrap();
        let socket = root.path().join("socket");
        let (send, mut events) = mpsc::channel(4);
        let _server = Server::bind(socket.clone(), send).unwrap();
        let mut client = BufReader::new(UnixStream::connect(socket).await.unwrap());
        client.get_mut().write_all(b"{}\n").await.unwrap();
        let Event::Frame {
            reply, connection, ..
        } = events.recv().await.unwrap()
        else {
            panic!("initial request")
        };
        let lease = Arc::new(Lease::default());
        lease.enable_notifications(true);
        reply
            .send(Reply {
                value: json!({"ready":true}),
                lease: Some(lease.clone()),
                close: false,
            })
            .ok()
            .unwrap();
        assert_eq!(notification_line(&mut client).await, json!({"ready":true}));
        if partial {
            client.get_mut().write_all(b"{\"partial\":").await.unwrap();
        }
        // No await between queue and revoke: publication has not begun. The
        // revoked lease must win over both the pending wake and frame reader.
        assert_eq!(lease.queue_notification(), "queued");
        lease.cancel();
        let mut bytes = Vec::new();
        let closed = tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut bytes))
            .await
            .unwrap();
        assert!(
            matches!(&closed, Ok(0))
                || closed.as_ref().is_err_and(|error| matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                )),
            "unexpected close result: {closed:?}"
        );
        assert!(bytes.is_empty(), "revoked notification leaked: {bytes:?}");
        assert!(
            matches!(tokio::time::timeout(Duration::from_secs(2), events.recv()).await.unwrap(), Some(Event::Closed(id)) if id==connection)
        );
        assert!(
            events.try_recv().is_err(),
            "partial request dispatched after revoke"
        );
    }
}
