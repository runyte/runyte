// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::plugin::state;
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
fn document(data: &str) -> state::Document {
    state::Document {
        version: 1,
        data: serde_json::value::RawValue::from_string(data.into()).unwrap(),
    }
}
fn set(revision: &str, data: &str) -> api::Request {
    api::Request::StateSet {
        expected_revision: revision.into(),
        document: document(data),
    }
}
fn code(output: &mut mpsc::Receiver<HostMessage>) -> api::ErrorCode {
    let api::HostMessage::Response {
        outcome: api::Response::Failure { error },
        ..
    } = next(output)
    else {
        panic!()
    };
    error.code
}
fn value(output: &mut mpsc::Receiver<HostMessage>) -> state::Info {
    let api::HostMessage::Response {
        outcome:
            api::Response::Success {
                result: api::ResultValue::State(info),
            },
        ..
    } = next(output)
    else {
        panic!()
    };
    info
}
async fn complete(host: &mut WorkspaceHost, events: &mut mpsc::Receiver<Event>) {
    while !host.plugin_state_requests.is_empty() {
        model_complete(host, events).await;
    }
}
fn expire(host: &mut WorkspaceHost) {
    let pending = host.plugin_state_requests.values_mut().next().unwrap();
    pending.deadline = Instant::now() - Duration::from_millis(1);
    let token = pending.token.clone();
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .deadlines
        .insert(token.clone(), pending.deadline);
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Deadline { token }),
    });
}
struct Gate {
    open: Mutex<bool>,
    ready: Condvar,
}
impl Gate {
    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.ready.notify_all();
    }
}
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}
fn gated(
    host: &mut WorkspaceHost,
    phase: state::Checkpoint,
) -> (Release, tokio::sync::oneshot::Receiver<()>) {
    let gate = Arc::new(Gate {
        open: Mutex::new(false),
        ready: Condvar::new(),
    });
    let worker = gate.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = Mutex::new(Some(sender));
    let editor_thread = std::thread::current().id();
    host.plugin_state_hook = Some(Arc::new(move |point| {
        if point == phase {
            assert_ne!(
                std::thread::current().id(),
                editor_thread,
                "state IO stays off editor thread"
            );
            if let Some(sender) = sender.lock().unwrap().take() {
                let _ = sender.send(());
            }
            let mut open = worker.open.lock().unwrap();
            while !*open {
                open = worker.ready.wait(open).unwrap();
            }
        }
        Ok(())
    }));
    (Release(gate), receiver)
}
async fn entered(receiver: tokio::sync::oneshot::Receiver<()>) {
    tokio::time::timeout(Duration::from_secs(3), receiver)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn state_async_round_trip_uses_injected_runtime_root_and_releases_all_charges() {
    let (root, mut host) = host();
    host.app.state_root = root.join("private-state");
    let store = host.app.state_root.clone();
    let mut output = setup(&mut host, 0, &["state"]);
    next(&mut output);
    let mut events = model_service(&mut host);
    request(&mut host, 0, 1, api::Request::StateGet(api::Empty {}));
    complete(&mut host, &mut events).await;
    assert_eq!(value(&mut output).revision, "s:missing");
    request(
        &mut host,
        0,
        2,
        set("s:missing", "{\"last\":\"猫\",\"count\":2}"),
    );
    assert!(host.app.plugins.instances[&0].application.state_pending);
    complete(&mut host, &mut events).await;
    let saved = value(&mut output);
    assert!(store.join("plugins/app-0/state.json").is_file());
    assert!(!root.join(".runyte/plugins").exists());
    request(
        &mut host,
        0,
        3,
        api::Request::StateDelete {
            expected_revision: saved.revision,
        },
    );
    complete(&mut host, &mut events).await;
    assert_eq!(value(&mut output).revision, "s:missing");
    let instance = &host.app.plugins.instances[&0].application;
    assert!(!instance.state_pending);
    assert_eq!(instance.retained_payload, 0);
    assert!(instance.deadlines.is_empty());
    assert_eq!(
        host.plugin_local_slots
            .as_ref()
            .unwrap()
            .available_permits(),
        16
    );
}

#[tokio::test]
async fn state_capability_and_tree_peak_reservation_refuse_before_storage_io() {
    let (root, mut host) = host();
    host.app.state_root = root.join("no-io");
    let mut output = setup(&mut host, 0, &[]);
    next(&mut output);
    let _events = model_service(&mut host);
    request(&mut host, 0, 1, api::Request::StateGet(api::Empty {}));
    assert_eq!(code(&mut output), api::ErrorCode::CapabilityDenied);
    let mut item = "0".to_owned();
    for _ in 0..13 {
        item = format!("{{\"k\":{item}}}");
    }
    let data = format!("[{}]", vec![item; 1024].join(","));
    let full = format!("{{\"version\":1,\"data\":{data}}}");
    crate::plugin::json::validate(&full, state::LIMITS).unwrap();
    let instance = &mut host.app.plugins.instances.get_mut(&0).unwrap().application;
    instance.capabilities.insert("state".into());
    instance.retained_payload = 40 * 1024 * 1024;
    request(&mut host, 0, 2, set("s:missing", &data));
    assert_eq!(code(&mut output), api::ErrorCode::LimitExceeded);
    assert!(!host.app.state_root.exists());
    assert!(host.plugin_state_requests.is_empty());
    assert!(!host.app.plugins.instances[&0].application.state_pending);
}

#[tokio::test]
async fn state_pre_mutation_timeout_keeps_worker_slot_and_refuses_duplicate_until_completion() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["state"]);
    next(&mut output);
    let mut events = model_service(&mut host);
    let (release, ready) = gated(&mut host, state::Checkpoint::BeforeMutation);
    request(&mut host, 0, 1, set("s:missing", "1"));
    entered(ready).await;
    expire(&mut host);
    assert_eq!(code(&mut output), api::ErrorCode::Timeout);
    assert!(host.app.plugins.instances[&0].application.state_pending);
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        state::PREPARE_CHARGE
    );
    assert_eq!(
        host.plugin_local_slots
            .as_ref()
            .unwrap()
            .available_permits(),
        15
    );
    request(&mut host, 0, 2, api::Request::StateGet(api::Empty {}));
    assert_eq!(code(&mut output), api::ErrorCode::Busy);
    release.0.release();
    complete(&mut host, &mut events).await;
    while let Ok(message) = output.try_recv() {
        assert!(
            matches!(message, HostMessage::Deadline { .. }),
            "no late second response"
        );
    }
    assert!(
        !host
            .app
            .state_root
            .join("plugins/app-0/state.json")
            .exists()
    );
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}

#[tokio::test]
async fn state_post_mutation_timeout_is_unknown_and_late_success_is_not_replied_twice() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["state"]);
    next(&mut output);
    let mut events = model_service(&mut host);
    let (release, ready) = gated(&mut host, state::Checkpoint::AfterMutation);
    request(&mut host, 0, 1, set("s:missing", "[1,2]"));
    entered(ready).await;
    expire(&mut host);
    assert_eq!(code(&mut output), api::ErrorCode::OutcomeUnknown);
    release.0.release();
    complete(&mut host, &mut events).await;
    while let Ok(message) = output.try_recv() {
        assert!(matches!(message, HostMessage::Deadline { .. }));
    }
    host.plugin_state_hook = None;
    request(&mut host, 0, 2, api::Request::StateGet(api::Empty {}));
    complete(&mut host, &mut events).await;
    assert_eq!(
        value(&mut output).document.unwrap().get(),
        "{\"data\":[1,2],\"version\":1}"
    );
}

#[tokio::test]
async fn state_stopped_generation_keeps_identity_quota_and_quit_protection_until_actual_completion()
{
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["state"]);
    next(&mut output);
    let mut events = model_service(&mut host);
    let (release, ready) = gated(&mut host, state::Checkpoint::BeforeMutation);
    request(&mut host, 0, 1, set("s:missing", "1"));
    entered(ready).await;
    host.stop_plugin(0, "test stop");
    assert_eq!(host.app.plugins.orphaned_payload, state::PREPARE_CHARGE);
    assert_eq!(host.app.plugins.state_orphans, 1);
    assert!(host.protected_state().plugin_jobs > 0);
    let mut replacement = setup(&mut host, 0, &["state"]);
    next(&mut replacement);
    request(&mut host, 0, 1, api::Request::StateGet(api::Empty {}));
    assert_eq!(code(&mut replacement), api::ErrorCode::Busy);
    release.0.release();
    complete(&mut host, &mut events).await;
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert_eq!(host.app.plugins.state_orphans, 0);
    assert!(replacement.try_recv().is_err());
    host.plugin_state_hook = None;
    request(&mut host, 0, 2, api::Request::StateGet(api::Empty {}));
    complete(&mut host, &mut events).await;
    assert_eq!(value(&mut replacement).revision, "s:missing");
}

#[tokio::test]
async fn state_failed_response_delivery_stops_owner_without_leaking_completed_io_charge() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["state"]);
    next(&mut output);
    let mut events = model_service(&mut host);
    request(&mut host, 0, 1, api::Request::StateGet(api::Empty {}));
    drop(output);
    complete(&mut host, &mut events).await;
    assert!(!host.app.plugins.instances.contains_key(&0));
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert_eq!(host.app.plugins.state_orphans, 0);
}

#[test]
fn state_runtime_teardown_keeps_service_slot_until_blocking_worker_exits() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (_root, mut host) = host();
    let store = host.app.state_root.clone();
    let mut output = setup(&mut host, 0, &["state"]);
    next(&mut output);
    let events = model_service(&mut host);
    let (release, ready) = gated(&mut host, state::Checkpoint::BeforeMutation);
    runtime.block_on(async {
        request(&mut host, 0, 1, set("s:missing", "1"));
        entered(ready).await;
    });
    let slots = host.plugin_local_slots.as_ref().unwrap().clone();
    runtime.shutdown_background();
    assert_eq!(slots.available_permits(), 15);
    // Dropping the host cancels the pre-mutation worker, but cannot release its
    // service slot while it is still executing outside the destroyed runtime.
    drop(host);
    drop(events);
    assert_eq!(slots.available_permits(), 15);
    release.0.release();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let all = tokio::time::timeout(Duration::from_secs(3), slots.acquire_many_owned(16))
            .await
            .unwrap()
            .unwrap();
        drop(all);
    });
    assert!(!store.join("plugins/app-0/state.json").exists());
}
