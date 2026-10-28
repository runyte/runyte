// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::mcp::{
    client::WakeEvent,
    wake::{Delivery, WakeMode},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    sync::{mpsc, oneshot},
};

fn event(active: bool) -> WakeEvent {
    WakeEvent {
        workspace: "workspace-fixture".into(),
        active: Arc::new(AtomicBool::new(active)),
    }
}

async fn frame(source: &mut (impl AsyncBufRead + Unpin)) -> Value {
    let raw = tokio::time::timeout(Duration::from_secs(2), read_frame(source, MAX_FRAME_BYTES))
        .await
        .unwrap()
        .unwrap();
    parse(&raw).unwrap()
}

#[tokio::test]
async fn idle_server_publishes_wake_without_losing_fragmented_input() {
    let mut server = server();
    server.ready = true;
    server.bridge.wake_mode = WakeMode::Claude;
    let (send, receive) = mpsc::channel(2);
    server.bridge.wake_receiver = Some(receive);
    let (mut input, source) = tokio::io::duplex(1024);
    let (sink, output) = tokio::io::duplex(2048);
    let task = tokio::spawn(async move { server.run(BufReader::new(source), sink).await });
    let mut output = BufReader::new(output);
    input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":7,")
        .await
        .unwrap();
    send.send(event(true)).await.unwrap();
    let wake = frame(&mut output).await;
    assert_eq!(wake["method"], "notifications/claude/channel");
    assert_eq!(wake["params"]["meta"]["workspace"], "workspace-fixture");
    input.write_all(b"\"method\":\"ping\"}\n").await.unwrap();
    let reply = frame(&mut output).await;
    assert_eq!(reply, json!({"jsonrpc":"2.0","id":7,"result":{}}));
    input.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn notification_preserves_a_pending_operation_and_its_result() {
    let (send, mut receive) = mpsc::channel(2);
    let (started, ready) = oneshot::channel();
    let (finish, finished) = oneshot::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let invoked = calls.clone();
    let operation = async move {
        invoked.fetch_add(1, Ordering::SeqCst);
        started.send(()).unwrap();
        finished.await.unwrap()
    };
    let (mut sink, output) = tokio::io::duplex(2048);
    let task = tokio::spawn(async move {
        with_wakes(
            operation,
            &mut receive,
            &mut sink,
            &Delivery::new(WakeMode::Claude),
        )
        .await
    });
    ready.await.unwrap();
    send.send(event(true)).await.unwrap();
    let mut output = BufReader::new(output);
    assert_eq!(
        frame(&mut output).await["method"],
        "notifications/claude/channel"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!task.is_finished());
    finish.send(42).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        42
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn revoked_wakes_and_disabled_delivery_do_not_publish_notifications() {
    for (mode, active) in [(WakeMode::Claude, false), (WakeMode::Disabled, true)] {
        let mut server = server();
        server.bridge.wake_mode = mode;
        let (send, receive) = mpsc::channel(2);
        server.bridge.wake_receiver = Some(receive);
        let (mut input, source) = tokio::io::duplex(1024);
        let (sink, output) = tokio::io::duplex(2048);
        let task = tokio::spawn(async move { server.run(BufReader::new(source), sink).await });
        send.send(event(active)).await.unwrap();
        input
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"ping\"}\n")
            .await
            .unwrap();
        input.shutdown().await.unwrap();
        let mut output = BufReader::new(output);
        assert_eq!(
            frame(&mut output).await,
            json!({"jsonrpc":"2.0","id":8,"result":{}})
        );
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            read_frame(&mut output, MAX_FRAME_BYTES)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn closed_notification_sink_ends_a_pending_operation_with_an_error() {
    let (send, mut receive) = mpsc::channel(1);
    send.send(event(true)).await.unwrap();
    let (mut sink, output) = tokio::io::duplex(1024);
    drop(output);
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        with_wakes(
            std::future::pending::<()>(),
            &mut receive,
            &mut sink,
            &Delivery::new(WakeMode::Claude),
        ),
    )
    .await
    .unwrap();
    assert!(result.is_err());
}

#[tokio::test]
async fn closed_wake_stream_keeps_driving_the_pending_operation() {
    let (send, mut receive) = mpsc::channel(1);
    drop(send);
    let mut sink = tokio::io::sink();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        with_wakes(
            async {
                tokio::task::yield_now().await;
                17
            },
            &mut receive,
            &mut sink,
            &Delivery::new(WakeMode::Claude),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result, 17);
}
