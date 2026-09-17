// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use serde_json::json;

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
