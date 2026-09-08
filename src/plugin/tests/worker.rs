// SPDX-License-Identifier: MPL-2.0

use super::*;

fn message(id: usize) -> HostMessage {
    HostMessage::Complete {
        invocation: id.to_string(),
        status: "applied",
        revision: None,
        message: String::new(),
    }
}

fn drain(sender: &Sender, receiver: &mut mpsc::Receiver<HostMessage>) -> HostMessage {
    let message = receiver.try_recv().unwrap();
    sender
        .bytes
        .fetch_sub(encoded_len(&message).unwrap(), Ordering::Relaxed);
    message
}

#[test]
fn states_reserve_control_slots_preserve_fifo_and_do_not_charge_refusals() {
    let (tx, mut receiver) = mpsc::channel(32);
    let sender = Sender::new(tx);
    for id in 0..24 {
        assert!(sender.try_send_state(message(id)).unwrap());
    }
    let charged = sender.bytes.load(Ordering::Relaxed);
    assert!(!sender.try_send_state(message(24)).unwrap());
    assert_eq!(sender.bytes.load(Ordering::Relaxed), charged);
    assert!(
        !sender
            .output
            .ready(receiver.capacity(), charged, sender.limit)
    );
    for id in 24..32 {
        sender.try_send(message(id)).unwrap();
    }
    let charged = sender.bytes.load(Ordering::Relaxed);
    assert!(sender.try_send(message(32)).is_err());
    assert!(!sender.try_send_state(message(32)).unwrap());
    assert_eq!(sender.bytes.load(Ordering::Relaxed), charged);
    for id in 0..32 {
        let HostMessage::Complete { invocation, .. } = drain(&sender, &mut receiver) else {
            panic!("unexpected message");
        };
        assert_eq!(invocation, id.to_string());
    }
    assert_eq!(sender.bytes.load(Ordering::Relaxed), 0);
    assert!(sender.output.ready(receiver.capacity(), 0, sender.limit));
    assert!(sender.try_send_state(message(32)).unwrap());
}

#[test]
fn states_reserve_encoded_bytes_and_wait_for_enough_space() {
    let (tx, mut receiver) = mpsc::channel(32);
    let sender = Sender::new(tx);
    let large = || HostMessage::Complete {
        invocation: "large".into(),
        status: "applied",
        revision: None,
        message: "x".repeat(950_000),
    };
    for _ in 0..3 {
        assert!(sender.try_send_state(large()).unwrap());
    }
    assert!(!sender.try_send_state(large()).unwrap());
    assert!(!sender.output.ready(
        receiver.capacity(),
        sender.bytes.load(Ordering::Relaxed),
        sender.limit
    ));
    // The same frame still fits the reliable reserve.
    sender.try_send(large()).unwrap();
    drain(&sender, &mut receiver);
    assert!(!sender.output.ready(
        receiver.capacity(),
        sender.bytes.load(Ordering::Relaxed),
        sender.limit
    ));
    drain(&sender, &mut receiver);
    assert!(sender.output.ready(
        receiver.capacity(),
        sender.bytes.load(Ordering::Relaxed),
        sender.limit
    ));
    assert!(sender.try_send_state(large()).unwrap());
}

#[test]
fn state_closed_and_oversize_are_errors_without_retained_charge() {
    let (tx, receiver) = mpsc::channel(32);
    let sender = Sender::new(tx);
    let huge = HostMessage::Complete {
        invocation: "huge".into(),
        status: "failed",
        revision: None,
        message: "x".repeat(MAX_BYTES),
    };
    assert!(sender.try_send_state(huge).is_err());
    assert_eq!(sender.bytes.load(Ordering::Relaxed), 0);
    drop(receiver);
    assert!(sender.try_send_state(message(0)).is_err());
    assert_eq!(sender.bytes.load(Ordering::Relaxed), 0);
    assert_eq!(sender.output.waiting.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn readiness_is_coalesced_and_rearming_while_host_handles_notice_wakes_again() {
    let wake = Arc::new(OutputWake::default());
    let limit = application::MAX_QUEUE_BYTES;
    assert!(!wake.ready(32, 0, limit));
    wake.arm(100);
    wake.arm(200);
    wake.changed.notified().await;
    assert!(!wake.ready(8, 0, limit));
    assert!(wake.ready(9, 0, limit));
    let notice = wake.notification();
    assert!(!wake.ready(32, 0, limit));
    // Host processing the notice hits capacity again, then the worker drains
    // everything before the host drops the first notice.
    wake.arm(300);
    wake.changed.notified().await;
    assert!(!wake.ready(32, 0, limit));
    drop(notice);
    tokio::time::timeout(Duration::from_secs(1), wake.changed.notified())
        .await
        .unwrap();
    assert!(wake.ready(32, 0, limit));
    drop(wake.notification());
    assert!(!wake.ready(32, 0, limit));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), wake.changed.notified())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn arming_after_the_last_dequeue_still_wakes_an_idle_worker() {
    let wake = Arc::new(OutputWake::default());
    // Capacity returned before the producer could arm its failed admission.
    wake.arm(100);
    tokio::time::timeout(Duration::from_secs(1), wake.changed.notified())
        .await
        .unwrap();
    assert!(wake.ready(32, 0, application::MAX_QUEUE_BYTES));
}

#[test]
fn inbound_and_deadline_events_share_owner_permits_and_release_on_consumption() {
    let slots = Arc::new(tokio::sync::Semaphore::new(PRODUCER_EVENTS));
    let bytes = Arc::new(tokio::sync::Semaphore::new(100));
    let mut messages = Vec::new();
    for id in 0..PRODUCER_EVENTS {
        messages.push(
            queued(
                ClientMessage::Deadline {
                    token: id.to_string(),
                },
                0,
                &slots,
                &bytes,
            )
            .unwrap(),
        );
    }
    assert!(
        queued(
            ClientMessage::Unsupported {
                id: "request".into()
            },
            1,
            &slots,
            &bytes
        )
        .is_err()
    );
    drop(messages.pop());
    assert!(
        queued(
            ClientMessage::Unsupported {
                id: "request".into()
            },
            101,
            &slots,
            &bytes
        )
        .is_err()
    );
    assert_eq!(slots.available_permits(), 1);
    let inbound = queued(
        ClientMessage::Unsupported {
            id: "request".into(),
        },
        100,
        &slots,
        &bytes,
    )
    .unwrap();
    assert_eq!(bytes.available_permits(), 0);
    drop(inbound);
    drop(messages);
    assert_eq!(slots.available_permits(), PRODUCER_EVENTS);
    assert_eq!(bytes.available_permits(), 100);
}

#[test]
fn all_noisy_owner_quotas_leave_the_quiet_owner_and_local_io_their_full_admission() {
    let (events, mut receiver) = mpsc::channel(EVENT_CAPACITY);
    for plugin in 0..MAX_PLUGINS {
        let slots = Arc::new(tokio::sync::Semaphore::new(PRODUCER_EVENTS));
        let bytes = Arc::new(tokio::sync::Semaphore::new(application::MAX_QUEUE_BYTES));
        for n in 0..PRODUCER_EVENTS {
            // A mixture of wire traffic and deadlines uses exactly one quota.
            let message = if n % 2 == 0 {
                ClientMessage::Unsupported {
                    id: "request".into(),
                }
            } else {
                ClientMessage::Deadline {
                    token: n.to_string(),
                }
            };
            events
                .try_send(Event {
                    plugin,
                    result: Ok(queued(message, 1, &slots, &bytes).unwrap()),
                })
                .unwrap();
        }
        assert!(
            queued(
                ClientMessage::Unsupported {
                    id: "request".into()
                },
                1,
                &slots,
                &bytes
            )
            .is_err()
        );
        let output = Arc::new(OutputWake::default());
        events
            .try_send(Event {
                plugin,
                result: Ok(ClientMessage::OutputReady {
                    _notification: output.notification(),
                }),
            })
            .unwrap();
        events
            .try_send(Event {
                plugin,
                result: Err("bounded worker failure".into()),
            })
            .unwrap();
    }
    // These stand in for globally admitted local IO completions, whose permits
    // remain attached until the host consumes the event.
    for _ in 0..16 {
        events
            .try_send(Event {
                plugin: 0,
                result: Ok(ClientMessage::Unsupported {
                    id: "request".into(),
                }),
            })
            .unwrap();
    }
    assert_eq!(events.capacity(), 0);
    let mut quiet = 0;
    while let Ok(event) = receiver.try_recv() {
        quiet += usize::from(event.plugin == MAX_PLUGINS - 1);
    }
    assert_eq!(quiet, PRODUCER_EVENTS + 2);
    assert_eq!(events.capacity(), EVENT_CAPACITY);
}

#[tokio::test]
#[cfg(unix)]
async fn actual_worker_delivers_final_capacity_notice_without_input_and_then_stays_idle() {
    let root = crate::test_support::TestRuntimeRoot::new("plugin-output-ready").unwrap();
    let program = root.join("worker");
    std::os::unix::fs::symlink(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        &program,
    )
    .unwrap();
    std::fs::write(
        root.join("worker.behavior"),
        "while read -r line; do :; done\n",
    )
    .unwrap();
    let config = PluginConfig {
        id: "output-ready".into(),
        api: application::Api::Epoch2,
        capabilities: vec![],
        enabled: true,
        executable: program,
        args: vec![],
        bindings: Default::default(),
    };
    let (events, mut receiver) = mpsc::channel(EVENT_CAPACITY);
    // Current-thread runtime guarantees the queue is filled before IO starts.
    let (_worker, sender) = spawn(config, root.path().to_path_buf(), 0, events);
    for id in 0..24 {
        assert!(sender.try_send_state(message(id)).unwrap());
    }
    assert!(!sender.try_send_state(message(24)).unwrap());
    let event = tokio::time::timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event.result,
        Ok(ClientMessage::OutputReady { .. })
    ));
    drop(event);
    assert!(sender.try_send_state(message(24)).unwrap());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), receiver.recv())
            .await
            .is_err()
    );
}

#[test]
fn epoch_one_cannot_construct_internal_admission_or_output_notifications() {
    for kind in ["queued", "output_ready", "deadline"] {
        let frame = format!("{{\"type\":\"{kind}\"}}");
        assert!(serde_json::from_str::<ClientMessage>(&frame).is_err());
    }
    assert!(matches!(
        serde_json::from_str::<ClientMessage>(
            r#"{"type":"register","version":"runyte-experimental-1","commands":[]}"#
        )
        .unwrap(),
        ClientMessage::Register { .. }
    ));
}

#[tokio::test]
#[cfg(unix)]
async fn simultaneous_host_deadlines_wait_for_owner_admission_without_stopping_worker() {
    let root = crate::test_support::TestRuntimeRoot::new("plugin-deadline-admission").unwrap();
    let program = root.join("worker");
    std::os::unix::fs::symlink(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        &program,
    )
    .unwrap();
    std::fs::write(
        root.join("worker.behavior"),
        "while read -r line; do :; done\n",
    )
    .unwrap();
    let config = PluginConfig {
        id: "deadline-admission".into(),
        api: application::Api::Epoch2,
        capabilities: vec![],
        enabled: true,
        executable: program,
        args: vec![],
        bindings: Default::default(),
    };
    let (events, mut receiver) = mpsc::channel(EVENT_CAPACITY);
    let (_worker, sender) = spawn(config, root.path().to_path_buf(), 0, events);
    for id in 0..20 {
        sender
            .try_send(HostMessage::Deadline {
                token: id.to_string(),
                after_ms: Some(0),
            })
            .unwrap();
    }
    let mut batch = Vec::new();
    let mut tokens = std::collections::BTreeSet::new();
    for _ in 0..PRODUCER_EVENTS {
        let event = tokio::time::timeout(Duration::from_secs(3), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let Ok(ClientMessage::Queued { message, .. }) = &event.result else {
            panic!("deadline failed: {event:?}");
        };
        let ClientMessage::Deadline { token } = message.as_ref() else {
            panic!()
        };
        assert!(tokens.insert(token.clone()));
        batch.push(event);
    }
    // All this owner's permits remain in the batch. No failure or unbounded
    // extra timer event may escape while the host is still processing it.
    assert!(
        tokio::time::timeout(Duration::from_millis(30), receiver.recv())
            .await
            .is_err()
    );
    // Outbound control still runs while timer delivery waits for owner capacity.
    sender
        .try_send(HostMessage::Registered { commands: vec![] })
        .unwrap();
    drop(batch);
    for _ in PRODUCER_EVENTS..20 {
        let event = tokio::time::timeout(Duration::from_secs(3), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let Ok(ClientMessage::Queued { message, .. }) = event.result else {
            panic!("deadline failed");
        };
        let ClientMessage::Deadline { token } = *message else {
            panic!()
        };
        assert!(tokens.insert(token));
    }
    assert_eq!(tokens.len(), 20);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), receiver.recv())
            .await
            .is_err()
    );
}
