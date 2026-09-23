// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    test_support::TestRuntimeRoot,
    workspace::{
        context::storage::{HostMode, Registration, Storage, random_token},
        windows_process_identity::ProcessIdentity,
    },
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
    time::Instant,
};

fn expected(registration: &Registration) -> ProcessIdentity {
    ProcessIdentity {
        pid: registration.pid,
        creation_time: registration.creation_time,
    }
}

fn fixture(label: &str) -> (TestRuntimeRoot, Arc<Storage>, Registration) {
    let root = TestRuntimeRoot::new(label).unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let storage = Arc::new(Storage::new(root.path().join("store")).unwrap());
    let host_incarnation = random_token().unwrap();
    let process = ProcessIdentity::current().unwrap();
    let registration = Registration {
        root: project.canonicalize().unwrap(),
        workspace_id: crate::workspace::workspace_id(&project.canonicalize().unwrap()),
        endpoint: storage.socket_path(&host_incarnation).unwrap(),
        host_incarnation,
        mode: HostMode::Standalone,
        pid: process.pid,
        creation_time: process.creation_time,
        environment: random_token().unwrap(),
    };
    (root, storage, registration)
}

#[tokio::test]
async fn real_pipe_dispatches_and_joined_shutdown_retires_exact_publication() {
    let (_root, storage, registration) = fixture("ctx-native-basic");
    let (send, mut events) = mpsc::channel(4);
    let mut server = Server::bind(storage.clone(), registration.clone(), send).unwrap();
    let mut client = BufReader::new(
        connect(
            &registration.endpoint,
            ProcessIdentity {
                pid: registration.pid,
                creation_time: registration.creation_time,
            },
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap(),
    );
    client.get_mut().write_all(b"{}\n").await.unwrap();
    let Event::Frame { reply, .. } = events.recv().await.unwrap() else {
        panic!("expected frame")
    };
    assert!(
        reply
            .send(Reply {
                value: json!({"ok":true}),
                lease: None,
                close: true
            })
            .is_ok()
    );
    let mut line = String::new();
    client.read_line(&mut line).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap(),
        json!({"ok":true})
    );
    drop(client);
    server.shutdown().await.unwrap();
    assert!(
        storage
            .discover(&registration.environment, true)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn first_instance_collision_is_refused() {
    let (_root, storage, registration) = fixture("ctx-native-collision");
    let (send, _events) = mpsc::channel(1);
    let mut server = Server::bind(storage.clone(), registration.clone(), send).unwrap();
    let (other, _) = mpsc::channel(1);
    assert!(Server::bind(storage, registration, other).is_err());
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn idle_accept_timeout_keeps_pending_instance_connectable() {
    let (_root, storage, registration) = fixture("ctx-native-idle");
    let (send, mut events) = mpsc::channel(2);
    let mut server = Server::bind(storage, registration.clone(), send).unwrap();
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let mut client = connect(
        &registration.endpoint,
        ProcessIdentity {
            pid: registration.pid,
            creation_time: registration.creation_time,
        },
        Instant::now() + Duration::from_secs(2),
    )
    .await
    .unwrap();
    client.write_all(b"{}\n").await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(events.recv().await, Some(Event::Frame { .. })) {
                break;
            }
        }
    })
    .await
    .unwrap();
    drop(client);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn full_event_queue_cannot_hold_joined_shutdown() {
    let (_root, storage, registration) = fixture("ctx-native-full-queue");
    let (send, _events) = mpsc::channel(1);
    let mut server = Server::bind(storage, registration.clone(), send).unwrap();
    let expected = ProcessIdentity {
        pid: registration.pid,
        creation_time: registration.creation_time,
    };
    let mut first = connect(
        &registration.endpoint,
        expected,
        Instant::now() + Duration::from_secs(2),
    )
    .await
    .unwrap();
    first.write_all(b"{}\n").await.unwrap();
    let mut second = connect(
        &registration.endpoint,
        expected,
        Instant::now() + Duration::from_secs(2),
    )
    .await
    .unwrap();
    second.write_all(b"{}\n").await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), server.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn ninth_frame_waits_for_one_of_eight_connection_leases() {
    let (_root, storage, registration) = fixture("ctx-native-saturation");
    let (send, mut events) = mpsc::channel(16);
    let mut server = Server::bind(storage, registration.clone(), send).unwrap();
    let expected = ProcessIdentity {
        pid: registration.pid,
        creation_time: registration.creation_time,
    };
    let mut clients = Vec::new();
    for _ in 0..CONNECTIONS + 1 {
        let mut client = connect(
            &registration.endpoint,
            expected,
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap();
        client.write_all(b"{}\n").await.unwrap();
        clients.push(client);
    }
    let mut replies = Vec::new();
    for _ in 0..CONNECTIONS {
        let Event::Frame { reply, .. } = events.recv().await.unwrap() else {
            panic!("expected frame")
        };
        replies.push(reply);
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err()
    );
    assert!(
        replies
            .remove(0)
            .send(Reply {
                value: json!({}),
                lease: None,
                close: true
            })
            .is_ok()
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(events.recv().await, Some(Event::Frame { .. })) {
                break;
            }
        }
    })
    .await
    .unwrap();
    drop(clients);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn retirement_preserves_a_replacement_registration() {
    let (_root, storage, registration) = fixture("ctx-native-replacement");
    let (send, _events) = mpsc::channel(1);
    let mut server = Server::bind(storage.clone(), registration.clone(), send).unwrap();
    let published = storage
        .root()
        .join(format!("host-{}.json", registration.host_incarnation));
    std::fs::remove_file(&published).unwrap();
    let mut replacement = registration.clone();
    replacement.environment = random_token().unwrap();
    let replacement_publication = storage.register_owned(&replacement).unwrap();
    server.shutdown().await.unwrap();
    assert_eq!(
        storage.discover(&replacement.environment, false).unwrap(),
        vec![replacement]
    );
    replacement_publication.retire().unwrap();
    assert!(!published.exists());
}

#[tokio::test]
async fn oversized_and_unterminated_peers_do_not_block_a_healthy_connection() {
    let (_root, storage, registration) = fixture("ctx-native-bounds");
    let (send, mut events) = mpsc::channel(8);
    let mut server = Server::bind(storage, registration.clone(), send).unwrap();
    let mut oversized = connect(
        &registration.endpoint,
        expected(&registration),
        Instant::now() + Duration::from_secs(2),
    )
    .await
    .unwrap();
    let writer = tokio::spawn(async move {
        let _ = oversized.write_all(&vec![b'x'; FRAME_BYTES + 1]).await;
    });
    let mut partial = connect(
        &registration.endpoint,
        expected(&registration),
        Instant::now() + Duration::from_secs(2),
    )
    .await
    .unwrap();
    partial.write_all(b"{").await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                if matches!(events.recv().await, Some(Event::Frame { .. })) {
                    break;
                }
            }
        })
        .await
        .is_err()
    );
    let mut healthy = BufReader::new(
        connect(
            &registration.endpoint,
            expected(&registration),
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap(),
    );
    healthy.get_mut().write_all(b"{}\n").await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(Event::Frame { reply, .. }) = events.recv().await {
                break reply;
            }
        }
    })
    .await
    .unwrap();
    assert!(
        reply
            .send(Reply {
                value: json!({"healthy":true}),
                lease: None,
                close: true,
            })
            .is_ok()
    );
    let mut line = String::new();
    healthy.read_line(&mut line).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap(),
        json!({"healthy":true})
    );
    drop(partial);
    writer.await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn lease_revocation_closes_idle_read_and_suppresses_pending_reply() {
    let (_root, storage, registration) = fixture("ctx-native-revocation");
    let (send, mut events) = mpsc::channel(4);
    let mut server = Server::bind(storage, registration.clone(), send).unwrap();
    let mut idle = BufReader::new(
        connect(
            &registration.endpoint,
            expected(&registration),
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap(),
    );
    idle.get_mut().write_all(b"{}\n").await.unwrap();
    let Event::Frame {
        connection: idle_connection,
        reply,
        ..
    } = events.recv().await.unwrap()
    else {
        panic!("expected frame")
    };
    let idle_lease = Arc::new(Lease::default());
    assert!(
        reply
            .send(Reply {
                value: json!({"ready":true}),
                lease: Some(idle_lease.clone()),
                close: false
            })
            .is_ok()
    );
    let mut line = String::new();
    idle.read_line(&mut line).await.unwrap();
    idle_lease.cancel();
    line.clear();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), idle.read_line(&mut line))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap(),
        Some(Event::Closed(connection)) if connection == idle_connection
    ));

    let mut pending = BufReader::new(
        connect(
            &registration.endpoint,
            expected(&registration),
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap(),
    );
    pending.get_mut().write_all(b"{}\n").await.unwrap();
    let Event::Frame {
        connection: pending_connection,
        reply,
        ..
    } = events.recv().await.unwrap()
    else {
        panic!("expected frame")
    };
    let pending_lease = Arc::new(Lease::default());
    assert!(
        reply
            .send(Reply {
                value: json!({"ready":true}),
                lease: Some(pending_lease.clone()),
                close: false
            })
            .is_ok()
    );
    line.clear();
    pending.read_line(&mut line).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap(),
        json!({"ready":true})
    );

    pending.get_mut().write_all(b"{}\n").await.unwrap();
    let Event::Frame {
        connection, reply, ..
    } = events.recv().await.unwrap()
    else {
        panic!("expected second frame")
    };
    assert_eq!(connection, pending_connection);
    pending_lease.cancel();
    let _ = reply.send(Reply {
        value: json!({"private":"must not leave"}),
        lease: None,
        close: false,
    });
    line.clear();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), pending.read_line(&mut line))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(line.is_empty());
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn repeated_disconnects_with_a_full_queue_stay_bounded_and_shutdown() {
    let (_root, storage, registration) = fixture("ctx-native-disconnect-full");
    let (send, mut events) = mpsc::channel(1);
    let pressure = send.clone();
    let mut server = Server::bind(storage, registration.clone(), send).unwrap();
    let first = connect(
        &registration.endpoint,
        expected(&registration),
        Instant::now() + Duration::from_secs(2),
    )
    .await
    .unwrap();
    drop(first);
    tokio::time::timeout(Duration::from_secs(2), async {
        while pressure.capacity() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for _ in 0..CONNECTIONS {
        let client = connect(
            &registration.endpoint,
            expected(&registration),
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap();
        drop(client);
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let pending = connect(
        &registration.endpoint,
        expected(&registration),
        Instant::now() + Duration::from_secs(2),
    )
    .await
    .unwrap();
    let refused = match connect(
        &registration.endpoint,
        expected(&registration),
        Instant::now() + Duration::from_millis(100),
    )
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("connection exceeded bounded native capacity"),
    };
    assert_eq!(refused.kind(), std::io::ErrorKind::TimedOut);
    for _ in 0..=CONNECTIONS {
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap(),
            Some(Event::Closed(_))
        ));
    }
    let recovered = connect(
        &registration.endpoint,
        expected(&registration),
        Instant::now() + Duration::from_secs(2),
    )
    .await
    .unwrap();
    drop(pending);
    drop(recovered);
    tokio::time::timeout(Duration::from_secs(2), server.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cancelled_shutdown_future_can_be_awaited_again() {
    use std::{future::poll_fn, task::Poll};
    let (_root, storage, registration) = fixture("ctx-native-shutdown-cancel");
    let (send, _events) = mpsc::channel(1);
    let mut server = Server::bind(storage.clone(), registration.clone(), send).unwrap();
    let mut shutdown = Box::pin(server.shutdown());
    poll_fn(|cx| {
        let _ = shutdown.as_mut().poll(cx);
        Poll::Ready(())
    })
    .await;
    drop(shutdown);
    server.shutdown().await.unwrap();
    assert!(
        storage
            .discover(&registration.environment, true)
            .unwrap()
            .is_empty()
    );
}
