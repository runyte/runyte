// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::plugin::activity::{self, State};
use std::time::{Duration, Instant};

fn acquire(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
    serial: u64,
    duration_seconds: u64,
) -> String {
    request(
        host,
        0,
        serial,
        api::Request::ActivityAcquire {
            title: "Playback".into(),
            duration_seconds,
        },
    );
    let api::HostMessage::Response {
        outcome:
            api::Response::Success {
                result: api::ResultValue::Activity(info),
            },
        ..
    } = next(output)
    else {
        panic!()
    };
    info.lease
}
fn error(output: &mut mpsc::Receiver<HostMessage>) -> api::ErrorCode {
    let api::HostMessage::Response {
        outcome: api::Response::Failure { error },
        ..
    } = next(output)
    else {
        panic!()
    };
    error.code
}
fn info(output: &mut mpsc::Receiver<HostMessage>) -> activity::Info {
    let api::HostMessage::Response {
        outcome:
            api::Response::Success {
                result: api::ResultValue::Activity(info),
            },
        ..
    } = next(output)
    else {
        panic!()
    };
    info
}
fn event(output: &mut mpsc::Receiver<HostMessage>) -> activity::Cancelled {
    let api::HostMessage::Event {
        event: "activity.cancel_requested",
        data: api::EventData::ActivityCancelled(event),
        ..
    } = next(output)
    else {
        panic!()
    };
    event
}
fn expire(host: &mut WorkspaceHost, lease: &str) {
    let state = &mut host.app.plugins.instances.get_mut(&0).unwrap().application;
    let now = Instant::now() - Duration::from_millis(1);
    state.activities.get_mut(lease).unwrap().deadline = now;
    state.deadlines.insert(lease.into(), now);
}
fn deadline(host: &mut WorkspaceHost, lease: &str) {
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Deadline {
            token: lease.into(),
        }),
    });
}

#[test]
fn activity_capability_and_bounds_refusals_leave_no_ownership_or_timer() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &[]);
    next(&mut output);
    request(
        &mut host,
        0,
        1,
        api::Request::ActivityAcquire {
            title: "Playback".into(),
            duration_seconds: 600,
        },
    );
    assert_eq!(error(&mut output), api::ErrorCode::CapabilityDenied);
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .capabilities
        .insert("activity".into());
    for (n, (title, duration_seconds)) in [
        ("".into(), 600),
        ("bad\nlabel".into(), 600),
        ("é".repeat(81), 600),
        ("Title".into(), 0),
        ("Title".into(), 601),
    ]
    .into_iter()
    .enumerate()
    {
        request(
            &mut host,
            0,
            n as u64 + 2,
            api::Request::ActivityAcquire {
                title,
                duration_seconds,
            },
        );
        assert_eq!(error(&mut output), api::ErrorCode::InvalidArgument);
    }
    let state = &host.app.plugins.instances[&0].application;
    assert!(state.activities.is_empty());
    assert!(state.deadlines.is_empty());
    assert_eq!(state.retained_payload, 0);
}

#[test]
fn activity_quota_counts_cancelling_and_release_is_scoped_idempotent_and_quiet() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["activity"]);
    next(&mut output);
    let first = acquire(&mut host, &mut output, 1, 600);
    let second = acquire(&mut host, &mut output, 2, 10);
    request(
        &mut host,
        0,
        3,
        api::Request::ActivityCancel {
            lease: first.clone(),
        },
    );
    assert_eq!(info(&mut output).state, State::Cancelling);
    assert_eq!(event(&mut output).reason, activity::Reason::Cancelled);
    request(
        &mut host,
        0,
        4,
        api::Request::ActivityAcquire {
            title: "Third".into(),
            duration_seconds: 600,
        },
    );
    assert_eq!(error(&mut output), api::ErrorCode::LimitExceeded);
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        2 * activity::LEASE_CHARGE
    );
    for (n, lease) in [(5, first.clone()), (6, second)] {
        request(&mut host, 0, n, api::Request::ActivityRelease { lease });
        assert!(matches!(
            next(&mut output),
            api::HostMessage::Response {
                outcome: api::Response::Success {
                    result: api::ResultValue::Empty(_)
                },
                ..
            }
        ));
    }
    assert!(
        host.app.plugins.instances[&0]
            .application
            .deadlines
            .is_empty()
    );
    request(
        &mut host,
        0,
        7,
        api::Request::ActivityRelease {
            lease: first.clone(),
        },
    );
    next(&mut output);
    assert!(
        output.try_recv().is_err(),
        "idempotent release must not arm another timer"
    );
    deadline(&mut host, &first);
    assert!(host.app.plugins.instances.contains_key(&0));
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}

#[test]
fn activity_foreign_and_future_handles_cannot_read_renew_cancel_or_release() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["activity"]);
    next(&mut output);
    let own = acquire(&mut host, &mut output, 1, 600);
    let mut other = setup(&mut host, 1, &["activity"]);
    next(&mut other);
    for (n, request_value) in [
        api::Request::ActivityGet { lease: own.clone() },
        api::Request::ActivityRenew {
            lease: own.clone(),
            duration_seconds: 10,
        },
        api::Request::ActivityCancel { lease: own.clone() },
        api::Request::ActivityRelease { lease: own.clone() },
    ]
    .into_iter()
    .enumerate()
    {
        request(&mut host, 1, n as u64 + 1, request_value);
        assert_eq!(error(&mut other), api::ErrorCode::NotFound);
    }
    let generation = host.app.plugins.instances[&0]
        .application
        .generation
        .clone();
    for (n, suffix) in ["0", "999", "01", "+1"].into_iter().enumerate() {
        request(
            &mut host,
            0,
            n as u64 + 2,
            api::Request::ActivityRelease {
                lease: format!("a:{generation}:{suffix}"),
            },
        );
        assert_eq!(error(&mut output), api::ErrorCode::NotFound);
    }
    assert_eq!(
        host.app.plugins.instances[&0].application.activities[&own]
            .info
            .state,
        State::Active
    );
}

#[test]
fn activity_renewal_fences_old_deadline_but_cannot_revive_elapsed_lease() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["activity"]);
    next(&mut output);
    let lease = acquire(&mut host, &mut output, 1, 1);
    request(
        &mut host,
        0,
        2,
        api::Request::ActivityRenew {
            lease: lease.clone(),
            duration_seconds: 600,
        },
    );
    assert_eq!(info(&mut output).duration_seconds, 600);
    let renewed = host.app.plugins.instances[&0].application.activities[&lease].deadline;
    deadline(&mut host, &lease);
    assert_eq!(
        host.app.plugins.instances[&0].application.activities[&lease].deadline,
        renewed
    );
    assert!(output.try_recv().is_err());
    expire(&mut host, &lease);
    request(
        &mut host,
        0,
        3,
        api::Request::ActivityRenew {
            lease: lease.clone(),
            duration_seconds: 600,
        },
    );
    assert_eq!(error(&mut output), api::ErrorCode::Cancelled);
    assert_eq!(
        event(&mut output),
        activity::Cancelled {
            lease: lease.clone(),
            reason: activity::Reason::Expired
        }
    );
    let grace = host.app.plugins.instances[&0].application.activities[&lease].deadline;
    request(
        &mut host,
        0,
        4,
        api::Request::ActivityCancel {
            lease: lease.clone(),
        },
    );
    assert_eq!(info(&mut output).state, State::Cancelling);
    assert_eq!(
        host.app.plugins.instances[&0].application.activities[&lease].deadline,
        grace
    );
    assert!(
        output.try_recv().is_err(),
        "repeated cancellation must not reset grace or notify again"
    );
    deadline(&mut host, &lease);
    assert!(host.app.plugins.instances.contains_key(&0));
}

#[test]
fn activity_expiry_and_late_cleanup_stop_only_the_uncooperative_owner() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["activity"]);
    next(&mut output);
    let mut quiet = setup(&mut host, 1, &["activity"]);
    next(&mut quiet);
    let lease = acquire(&mut host, &mut output, 1, 1);
    expire(&mut host, &lease);
    deadline(&mut host, &lease);
    assert_eq!(event(&mut output).reason, activity::Reason::Expired);
    expire(&mut host, &lease);
    request(
        &mut host,
        0,
        2,
        api::Request::ActivityRelease {
            lease: lease.clone(),
        },
    );
    assert!(!host.app.plugins.instances.contains_key(&0));
    assert!(host.app.plugins.instances.contains_key(&1));
    deadline(&mut host, &lease);
    assert!(host.app.plugins.instances.contains_key(&1));
}

#[test]
fn activity_timer_or_reliable_event_failure_cannot_leave_immortal_protection() {
    for capacity in [0usize, 2] {
        let (_root, mut host) = host();
        let mut output = setup(&mut host, 0, &["activity"]);
        next(&mut output);
        if capacity == 0 {
            drop(output);
            request(
                &mut host,
                0,
                1,
                api::Request::ActivityAcquire {
                    title: "Playback".into(),
                    duration_seconds: 600,
                },
            );
        } else {
            let lease = acquire(&mut host, &mut output, 1, 600);
            let (sender, _blocked) = mpsc::channel(capacity);
            host.app.plugins.instances.get_mut(&0).unwrap().sender = plugin::Sender::new(sender);
            // Timer and reply fill the queue; reliable cancellation delivery fails.
            request(&mut host, 0, 2, api::Request::ActivityCancel { lease });
        }
        assert!(!host.app.plugins.instances.contains_key(&0));
        assert_eq!(host.app.plugins.orphaned_payload, 0);
    }
}

#[test]
fn activity_wire_defaults_are_six_hundred_seconds() {
    for (method, params) in [
        ("activity.acquire", serde_json::json!({"title":"Playback"})),
        ("activity.renew", serde_json::json!({"lease":"a:g:1"})),
    ] {
        let request: api::Request =
            serde_json::from_value(serde_json::json!({"method":method,"params":params})).unwrap();
        match request {
            api::Request::ActivityAcquire {
                duration_seconds, ..
            }
            | api::Request::ActivityRenew {
                duration_seconds, ..
            } => assert_eq!(duration_seconds, 600),
            _ => panic!(),
        }
    }
}

#[tokio::test]
async fn actual_worker_expiry_release_and_missed_acknowledgement_drive_owned_cleanup() {
    for mode in ["release", "ignore"] {
        let (root, mut host) = host();
        let program = root.join("activity-worker");
        std::os::unix::fs::symlink(
            concat!(env!("CARGO_MANIFEST_DIR"), "/src/fixtures/stand-in"),
            &program,
        )
        .unwrap();
        std::fs::write(root.join("activity-worker.behavior"), r#"
exec python3 -u -c '
import json,sys
read=lambda: json.loads(sys.stdin.readline())
def send(value): print(json.dumps(value),flush=True)
read()
send({"type":"register","version":"runyte-experimental-2","name":"Activity","commands":[{"name":"open","description":"Open activity","context":"workspace"}],"required_capabilities":["activity","processes"],"optional_capabilities":[]})
read()
serial=1
if sys.argv[1]=="ignore":
 send({"type":"request","id":"p:1","method":"process.start","params":{"label":"Playback helper","executable":"/bin/cat"}})
 read()
 serial=2
send({"type":"request","id":"p:"+str(serial),"method":"activity.acquire","params":{"title":"Playback","duration_seconds":1}})
lease=read()["result"]["lease"]
for line in sys.stdin:
 message=json.loads(line)
 if message.get("event")=="activity.cancel_requested" and sys.argv[1]=="release":
  send({"type":"request","id":"p:"+str(serial+1),"method":"activity.release","params":{"lease":lease}})
' "$1"
"#).unwrap();
        let mut cfg = config("activity-worker");
        cfg.api = api::Api::Epoch2;
        cfg.executable = program;
        cfg.args = vec![mode.into()];
        cfg.capabilities = vec!["activity".into(), "processes".into()];
        host.app.config.plugins.push(cfg);
        let mut events = host.start_plugins().unwrap();
        let mut active = false;
        let mut cancelling = false;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap();
            host.handle_plugin_event(event);
            if let Some(instance) = host.app.plugins.instances.get(&0) {
                active |= instance
                    .application
                    .activities
                    .values()
                    .any(|lease| lease.info.state == State::Active);
                cancelling |= instance
                    .application
                    .activities
                    .values()
                    .any(|lease| lease.info.state == State::Cancelling);
                if mode == "release" && active && instance.application.activities.is_empty() {
                    assert!(instance.application.deadlines.is_empty());
                    assert_eq!(instance.application.retained_payload, 0);
                    break;
                }
            } else {
                break;
            }
        }
        assert!(
            active && cancelling,
            "actual worker must receive expiry: mode={mode}, active={active}, cancelling={cancelling}, status={}",
            host.app.status
        );
        if mode == "release" {
            assert!(host.app.plugins.instances.contains_key(&0));
            assert!(
                tokio::time::timeout(Duration::from_millis(30), events.recv())
                    .await
                    .is_err(),
                "released activity leaves no periodic wake"
            );
            host.stop_plugin(0, "test complete");
        } else {
            assert!(!host.app.plugins.instances.contains_key(&0));
            while !host.plugin_processes.is_empty() {
                model_complete(&mut host, &mut events).await;
            }
            assert_eq!(
                host.app.plugins.orphaned_payload, 0,
                "missed cleanup acknowledgement reaps the owned helper"
            );
        }
    }
}

#[test]
fn activity_get_discovers_expiry_and_release_can_acknowledge_before_expiry_delivery() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["activity"]);
    next(&mut output);
    let first = acquire(&mut host, &mut output, 1, 600);
    expire(&mut host, &first);
    request(
        &mut host,
        0,
        2,
        api::Request::ActivityGet {
            lease: first.clone(),
        },
    );
    assert_eq!(info(&mut output).state, State::Cancelling);
    assert_eq!(event(&mut output).reason, activity::Reason::Expired);
    assert_eq!(host.protected_state().activity_leases, 1);
    request(
        &mut host,
        0,
        3,
        api::Request::ActivityRelease {
            lease: first.clone(),
        },
    );
    next(&mut output);
    assert_eq!(host.protected_state().activity_leases, 0);
    let second = acquire(&mut host, &mut output, 4, 600);
    expire(&mut host, &second);
    request(
        &mut host,
        0,
        5,
        api::Request::ActivityRelease {
            lease: second.clone(),
        },
    );
    next(&mut output);
    assert!(
        output.try_recv().is_err(),
        "completed cleanup does not need an expiry callback"
    );
    deadline(&mut host, &second);
    assert!(host.app.plugins.instances.contains_key(&0));
    assert!(
        host.app.plugins.instances[&0]
            .application
            .deadlines
            .is_empty()
    );
    assert_eq!(host.protected_state().activity_leases, 0);
}
