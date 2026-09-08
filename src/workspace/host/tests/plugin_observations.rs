// SPDX-License-Identifier: MPL-2.0

use super::*;
use serde_json::{Value, json};

fn observations(host: &mut WorkspaceHost, capabilities: &[&str]) -> mpsc::Receiver<HostMessage> {
    let mut old = setup(host, 0, capabilities);
    next(&mut old);
    let (sender, receiver) = mpsc::channel(32);
    host.app.plugins.instances.get_mut(&0).unwrap().sender = plugin::Sender::new(sender);
    receiver
}

fn call(
    host: &mut WorkspaceHost,
    receiver: &mut mpsc::Receiver<HostMessage>,
    n: u64,
    method: &str,
    params: Value,
) -> Value {
    request(
        host,
        0,
        n,
        serde_json::from_value(json!({"method": method, "params": params})).unwrap(),
    );
    serde_json::to_value(next(receiver)).unwrap()
}

fn baseline(
    host: &mut WorkspaceHost,
    receiver: &mut mpsc::Receiver<HostMessage>,
    n: u64,
    sources: Value,
) -> Value {
    let reply = call(
        host,
        receiver,
        n,
        "event.subscribe",
        json!({"sources": sources}),
    );
    assert!(reply.get("error").is_none(), "{reply}");
    reply["result"].clone()
}

fn change(receiver: &mut mpsc::Receiver<HostMessage>, event: &str) -> Value {
    let message = serde_json::to_value(next(receiver)).unwrap();
    assert_eq!(message["event"], event, "{message}");
    message
}

#[test]
fn wildcard_baseline_discovers_open_buffers_and_mutation_response_precedes_change() {
    let (_root, mut host) = host();
    let mut receiver = observations(&mut host, &["workspace", "text"]);
    let initial = baseline(&mut host, &mut receiver, 1, json!([{"kind": "buffers"}]));
    assert_eq!(initial["sources"].as_array().unwrap().len(), 1);
    let buffer = initial["sources"][0]["source"]["buffer"].clone();
    let revision = initial["sources"][0]["state"]["revision"].clone();
    let reply = call(
        &mut host,
        &mut receiver,
        2,
        "buffer.edit",
        json!({
            "buffer": buffer, "expected_revision": revision,
            "changes": [{"from": 0, "to": 0, "text": "hello"}]
        }),
    );
    assert!(reply.get("error").is_none(), "{reply}");
    host.sync_plugin_observers();
    let updated = change(&mut receiver, "event.changed");
    assert_eq!(updated["data"]["subscription"], initial["subscription"]);
    assert_eq!(updated["data"]["sources"][0]["state"]["chars"], 5);
    host.app
        .execute(CommandInvocation::editor(EditorCommand::NewBuffer, Default::default()).unwrap())
        .unwrap();
    host.sync_plugin_observers();
    let opened = change(&mut receiver, "event.changed");
    assert_ne!(opened["data"]["sources"][0]["source"]["buffer"], buffer);
    assert_ne!(opened["sequence"], updated["sequence"]);
}

#[test]
fn unsubscribe_is_idempotent_and_new_subscription_has_a_fresh_baseline() {
    let (_root, mut host) = host();
    let mut receiver = observations(&mut host, &["workspace"]);
    let initial = baseline(&mut host, &mut receiver, 1, json!([{"kind": "buffers"}]));
    seed(&mut host, "first");
    host.sync_plugin_observers();
    change(&mut receiver, "event.changed");
    for id in 2..=3 {
        let reply = call(
            &mut host,
            &mut receiver,
            id,
            "event.unsubscribe",
            json!({"subscription": initial["subscription"]}),
        );
        assert_eq!(reply["result"], json!({}));
    }
    seed(&mut host, "second");
    host.sync_plugin_observers();
    assert!(receiver.try_recv().is_err());
    let renewed = baseline(&mut host, &mut receiver, 4, json!([{"kind": "buffers"}]));
    assert_ne!(renewed["subscription"], initial["subscription"]);
    assert_eq!(renewed["sources"][0]["state"]["chars"], 11);
}

#[test]
fn observation_admission_is_atomic_for_capabilities_foreign_handles_and_limits() {
    let (_root, mut host) = host();
    let mut receiver = observations(&mut host, &[]);
    let denied = call(
        &mut host,
        &mut receiver,
        1,
        "event.subscribe",
        json!({"sources": [{"kind": "buffers"}]}),
    );
    assert_eq!(denied["error"]["code"], "capability_denied");
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .capabilities
        .insert("workspace".into());
    let foreign = call(
        &mut host,
        &mut receiver,
        2,
        "event.subscribe",
        json!({"sources": [{"kind": "buffer", "buffer": "b:foreign"}]}),
    );
    assert_eq!(foreign["error"]["code"], "not_found");
    for id in 3..35 {
        baseline(
            &mut host,
            &mut receiver,
            id,
            json!([{"kind": "attachment"}]),
        );
    }
    let limited = call(
        &mut host,
        &mut receiver,
        35,
        "event.subscribe",
        json!({"sources": [{"kind": "attachment"}]}),
    );
    assert_eq!(limited["error"]["code"], "limit_exceeded");
}

#[test]
fn latest_state_survives_queue_pressure_without_consuming_reliable_reserve() {
    let (_root, mut host) = host();
    let mut receiver = observations(&mut host, &["workspace"]);
    baseline(&mut host, &mut receiver, 1, json!([{"kind": "buffers"}]));
    for _ in 0..24 {
        host.app.plugins.instances[&0]
            .sender
            .try_send(HostMessage::Deadline {
                token: "filler".into(),
                after_ms: None,
            })
            .unwrap();
    }
    for _ in 0..3 {
        seed(&mut host, "x");
        host.sync_plugin_observers();
    }
    assert!(host.app.plugins.instances.contains_key(&0));
    for _ in 0..24 {
        assert!(matches!(
            receiver.try_recv().unwrap(),
            HostMessage::Deadline { .. }
        ));
    }
    assert!(receiver.try_recv().is_err());
    host.sync_plugin_observers();
    let latest = change(&mut receiver, "event.changed");
    assert_eq!(latest["data"]["sources"][0]["state"]["chars"], 3);
    assert!(receiver.try_recv().is_err());
}

#[test]
fn saved_baselines_and_attachment_transitions_use_reserved_lifecycle_delivery() {
    let (_root, mut host) = host();
    let mut receiver = observations(&mut host, &["workspace"]);
    let initial = baseline(
        &mut host,
        &mut receiver,
        1,
        json!([{"kind": "buffers"}, {"kind": "attachment"}]),
    );
    assert_eq!(initial["sources"].as_array().unwrap().len(), 2);
    seed(&mut host, "saved");
    host.sync_plugin_observers();
    change(&mut receiver, "event.changed");
    for _ in 0..24 {
        host.app.plugins.instances[&0]
            .sender
            .try_send(HostMessage::Deadline {
                token: "filler".into(),
                after_ms: None,
            })
            .unwrap();
    }
    host.app.buffers[0].mark_saved();
    host.app.note_plugin_frontend(true);
    host.sync_plugin_observers();
    for _ in 0..24 {
        receiver.try_recv().unwrap();
    }
    let events = [
        change(&mut receiver, "event.changed"),
        change(&mut receiver, "event.changed"),
    ];
    let states = events
        .iter()
        .map(|event| &event["data"]["sources"][0]["state"])
        .collect::<Vec<_>>();
    assert!(states.iter().any(|state| state["kind"] == "buffer"
        && state["saved_revision"] == state["revision"]
        && state["dirty"] == false));
    assert!(
        states
            .iter()
            .any(|state| state["kind"] == "attachment" && state["attached"] == true)
    );
}

#[test]
fn explicit_close_is_reliable_and_resync_can_report_its_tombstone() {
    let (_root, mut host) = host();
    let mut receiver = observations(&mut host, &["workspace", "documents"]);
    let listed = call(
        &mut host,
        &mut receiver,
        1,
        "buffer.list",
        json!({"offset": 0, "limit": 32}),
    );
    let buffer = listed["result"]["buffers"][0]["buffer"].clone();
    let revision = listed["result"]["buffers"][0]["revision"].clone();
    let initial = baseline(
        &mut host,
        &mut receiver,
        2,
        json!([{"kind": "buffer", "buffer": buffer}]),
    );
    let reply = call(
        &mut host,
        &mut receiver,
        3,
        "buffer.close",
        json!({"buffer": buffer, "expected_revision": revision}),
    );
    assert!(reply.get("error").is_none(), "{reply}");
    host.sync_plugin_observers();
    let closed = change(&mut receiver, "event.closed");
    assert_eq!(
        closed["data"]["sources"][0]["state"],
        json!({"kind": "closed"})
    );
    let resync = call(
        &mut host,
        &mut receiver,
        4,
        "event.resync",
        json!({"subscription": initial["subscription"]}),
    );
    assert_eq!(
        resync["result"]["sources"][0]["state"],
        json!({"kind": "closed"})
    );
}

#[test]
fn wildcard_overflow_suspends_then_resync_recovers_without_retaining_failed_handles() {
    let (_root, mut host) = host();
    let mut receiver = observations(&mut host, &["workspace"]);
    let initial = baseline(&mut host, &mut receiver, 1, json!([{"kind": "buffers"}]));
    // Add all new buffers in one host checkpoint, forcing a bounded overflow witness.
    for _ in 0..256 {
        host.app.buffers.push(crate::buffer::Buffer::scratch());
    }
    host.sync_plugin_observers();
    change(&mut receiver, "event.resync_required");
    host.sync_plugin_observers();
    assert!(receiver.try_recv().is_err());
    let failed = call(
        &mut host,
        &mut receiver,
        2,
        "event.resync",
        json!({"subscription": initial["subscription"]}),
    );
    assert_eq!(failed["error"]["code"], "limit_exceeded");
    assert_eq!(host.app.plugins.instances[&0].application.buffers.len(), 1);
    // The test's synthetic buffers are never attached; remove them before reset.
    host.app.buffers.truncate(1);
    let reset = call(
        &mut host,
        &mut receiver,
        3,
        "event.resync",
        json!({"subscription": initial["subscription"]}),
    );
    assert_eq!(reset["result"]["sources"].as_array().unwrap().len(), 1);
    seed(&mut host, "after reset");
    host.sync_plugin_observers();
    change(&mut receiver, "event.changed");
}

#[test]
fn pane_observations_follow_selection_and_target_without_reading_text() {
    let (_root, mut host) = host();
    seed(&mut host, "abc");
    let mut receiver = observations(&mut host, &["workspace"]);
    let panes = call(&mut host, &mut receiver, 1, "pane.list", json!({}));
    let pane = panes["result"]["panes"][0]["pane"].clone();
    let initial = baseline(
        &mut host,
        &mut receiver,
        2,
        json!([{"kind": "pane", "pane": pane}]),
    );
    host.app
        .execute(CommandInvocation::editor(EditorCommand::MoveRight, Default::default()).unwrap())
        .unwrap();
    host.sync_plugin_observers();
    let selected = change(&mut receiver, "event.changed");
    assert_ne!(
        selected["data"]["sources"][0]["state"]["selection_revision"],
        initial["sources"][0]["state"]["selection_revision"]
    );
    host.app
        .execute(CommandInvocation::editor(EditorCommand::NewBuffer, Default::default()).unwrap())
        .unwrap();
    host.sync_plugin_observers();
    let target = change(&mut receiver, "event.changed");
    assert_ne!(
        target["data"]["sources"][0]["state"]["buffer"],
        initial["sources"][0]["state"]["buffer"]
    );
    assert!(target["data"]["sources"][0]["state"].get("text").is_none());
}

#[tokio::test]
async fn owned_view_and_job_sources_capture_models_progress_and_final_state() {
    let (_root, mut host) = host();
    let mut receiver = observations(&mut host, &["views", "jobs"]);
    model_request(
        &mut host,
        0,
        1,
        api::Request::ViewCreate {
            model: model(&[("row", "Before")]),
        },
    )
    .await;
    let created = serde_json::to_value(next(&mut receiver)).unwrap();
    let view = created["result"]["view"].clone();
    let created_job = call(
        &mut host,
        &mut receiver,
        2,
        "job.create",
        json!({"title": "Watch", "deadline_seconds": 60}),
    );
    let job = created_job["result"]["job"].clone();
    // Job creation also retains the preexisting reliable job event.
    change(&mut receiver, "job.changed");
    let initial = baseline(
        &mut host,
        &mut receiver,
        3,
        json!([{"kind": "view", "view": view}, {"kind": "job", "job": job}]),
    );
    assert_eq!(initial["sources"].as_array().unwrap().len(), 2);
    model_request(
        &mut host,
        0,
        4,
        api::Request::ViewPublish {
            expected_query_revision: None,
            view: view.as_str().unwrap().into(),
            expected_revision: created["result"]["revision"].as_str().unwrap().into(),
            model: model(&[("row", "After")]),
        },
    )
    .await;
    let reply = serde_json::to_value(next(&mut receiver)).unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    host.sync_plugin_observers();
    let updated = change(&mut receiver, "event.changed");
    assert_eq!(
        updated["data"]["sources"][0]["state"]["revision"],
        reply["result"]["revision"]
    );
    let reply = call(
        &mut host,
        &mut receiver,
        5,
        "job.finish",
        json!({"job": job, "state": "succeeded"}),
    );
    assert!(reply.get("error").is_none(), "{reply}");
    change(&mut receiver, "job.changed");
    host.sync_plugin_observers();
    let finished = change(&mut receiver, "event.changed");
    assert_eq!(
        finished["data"]["sources"][0]["state"]["state"],
        "succeeded"
    );
}
