// SPDX-License-Identifier: MPL-2.0
use super::*;
use serde_json::{Value, json};

fn request_value(method: &str, params: Value) -> api::Request {
    serde_json::from_value(json!({"method":method,"params":params})).unwrap()
}
fn reply(output: &mut mpsc::Receiver<HostMessage>) -> Value {
    let value = serde_json::to_value(next(output)).unwrap();
    assert_eq!(value["type"], "response", "{value}");
    value
}
async fn existing(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
) -> (String, String, usize) {
    model_request(
        host,
        0,
        1,
        api::Request::ViewCreate {
            model: model(&[("one", "original"), ("two", "second")]),
        },
    )
    .await;
    let (view, revision) = view_result(output);
    let buffer = host.app.plugins.instances[&0].application.views[&view].buffer;
    (view, revision, buffer)
}
fn query(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
    n: u64,
    view: &str,
    expected: Option<&str>,
) -> Value {
    request(
        host,
        0,
        n,
        request_value(
            "view.query.set",
            json!({"view":view,"expected_revision":"m:1","expected_query_revision":expected,"text":"filter"}),
        ),
    );
    let value = reply(output);
    assert!(value.get("error").is_none(), "{value}");
    value["result"].clone()
}
fn larger_output(host: &mut WorkspaceHost) -> mpsc::Receiver<HostMessage> {
    let (sender, receiver) = mpsc::channel(32);
    host.app.plugins.instances.get_mut(&0).unwrap().sender = plugin::Sender::new(sender);
    receiver
}
fn geometry() -> crate::app::FrameGeometry {
    crate::ui::frame_geometry(ratatui::layout::Rect::new(0, 0, 40, 12))
}

#[tokio::test]
async fn enabling_query_after_worker_admission_rejects_legacy_completion_without_text_mutation() {
    let (_root, mut host) = host();
    let mut output = view_setup(&mut host);
    let (view, revision, buffer) = existing(&mut host, &mut output).await;
    let text_revision = host.app.buffers[buffer].revision();
    let original_charge = host.app.plugins.instances[&0].application.retained_payload;
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        2,
        api::Request::ViewPublish {
            view: view.clone(),
            expected_revision: revision,
            expected_query_revision: None,
            model: model(&[("one", "obsolete")]),
        },
    );
    query(&mut host, &mut output, 3, &view, None);
    model_complete(&mut host, &mut events).await;
    assert_eq!(reply(&mut output)["error"]["code"], "stale");
    assert_eq!(host.app.buffers[buffer].to_string(), "original\nsecond\n");
    assert_eq!(host.app.buffers[buffer].revision(), text_revision);
    let state = &host.app.plugins.instances[&0].application;
    assert!(state.views[&view].query.as_ref().unwrap().pending);
    assert_eq!(
        state.retained_payload,
        original_charge + plugin::view::QUERY_CHARGE
    );
}

#[tokio::test]
async fn stage_open_binds_query_and_rejected_commit_keeps_closeable_charge() {
    let (_root, mut host) = host();
    let mut output = view_setup(&mut host);
    let (view, revision, _) = existing(&mut host, &mut output).await;
    let original = host.app.plugins.instances[&0].application.retained_payload;
    query(&mut host, &mut output, 2, &view, None);
    let data = serde_json::to_string(&model(&[("one", "obsolete stage")])).unwrap();
    request(
        &mut host,
        0,
        3,
        request_value(
            "view.stage.open",
            json!({"view":view,"expected_revision":revision,"expected_query_revision":"qv:1","kind":"model","bytes":data.len()}),
        ),
    );
    let stage = reply(&mut output)["result"]["stage"].clone();
    request(
        &mut host,
        0,
        4,
        request_value(
            "view.stage.write",
            json!({"stage":stage,"offset":0,"text":data}),
        ),
    );
    reply(&mut output);
    query(&mut host, &mut output, 5, &view, Some("qv:1"));
    request(
        &mut host,
        0,
        6,
        request_value("view.stage.commit", json!({"stage":stage})),
    );
    assert_eq!(reply(&mut output)["error"]["code"], "stale");
    assert_eq!(
        host.app.plugins.instances[&0].application.view_stages.len(),
        1
    );
    request(
        &mut host,
        0,
        7,
        request_value("view.stage.close", json!({"stage":stage})),
    );
    reply(&mut output);
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        original + plugin::view::QUERY_CHARGE
    );
}

#[tokio::test]
async fn matching_publication_preserves_query_charge_and_owner_orphan_until_completion() {
    let (_root, mut host) = host();
    let mut output = view_setup(&mut host);
    let (view, revision, _) = existing(&mut host, &mut output).await;
    query(&mut host, &mut output, 2, &view, None);
    model_request(
        &mut host,
        0,
        3,
        api::Request::ViewPublish {
            view: view.clone(),
            expected_revision: revision,
            expected_query_revision: Some("qv:1".into()),
            model: model(&[("one", "matched")]),
        },
    )
    .await;
    let result = reply(&mut output)["result"].clone();
    assert_eq!(result["query"]["pending"], false);
    let source_charge = host.app.plugins.instances[&0].application.views[&view].charge;
    let prepared = plugin::view::PreparedModel::build(
        model(&[("one", "matched")]),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(source_charge, prepared.charge + plugin::view::QUERY_CHARGE);
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .views
        .get_mut(&view)
        .unwrap()
        .published = None;
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        4,
        api::Request::ViewPublish {
            view: view.clone(),
            expected_revision: "m:2".into(),
            expected_query_revision: Some("qv:1".into()),
            model: model(&[("one", "uncommitted")]),
        },
    );
    let retained = host.app.plugins.instances[&0].application.retained_payload;
    host.stop_plugin(0, "stopped by user");
    assert_eq!(host.app.plugins.orphaned_payload, retained);
    model_complete(&mut host, &mut events).await;
    assert_eq!(host.app.plugins.orphaned_payload, 0);
}

#[tokio::test]
async fn viewport_first_frame_delivers_final_rows_without_another_input_and_unsubscribe_releases_watch()
 {
    let (_root, mut host) = host();
    let mut output = view_setup(&mut host);
    let (view, _, buffer) = existing(&mut host, &mut output).await;
    let mut output = larger_output(&mut host);
    let pane_index = host.app.active_pane;
    let pane = host
        .app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .pane_handle(pane_index)
        .unwrap();
    request(
        &mut host,
        0,
        2,
        request_value(
            "event.subscribe",
            json!({"sources":[{"kind":"viewport","view":view,"pane":pane}]}),
        ),
    );
    let baseline = reply(&mut output)["result"].clone();
    assert!(baseline["sources"][0]["state"]["model_revision"].is_null());
    host.app.present_plugin_view(buffer);
    host.prepare_frame(geometry());
    let event = serde_json::to_value(next(&mut output)).unwrap();
    assert_eq!(event["event"], "event.changed", "{event}");
    assert_eq!(event["data"]["sources"][0]["state"]["top"], "one");
    assert_eq!(event["data"]["sources"][0]["state"]["bottom"], "two");
    host.prepare_frame(geometry());
    assert!(output.try_recv().is_err());
    request(
        &mut host,
        0,
        3,
        request_value(
            "event.unsubscribe",
            json!({"subscription":baseline["subscription"]}),
        ),
    );
    reply(&mut output);
    assert!(host.app.plugin_viewport(0, &view, pane_index).is_none());
}

#[tokio::test]
async fn viewport_baseline_does_not_relabel_old_prepared_rows_after_new_model_installation() {
    let (_root, mut host) = host();
    let mut output = view_setup(&mut host);
    let (view, revision, buffer) = existing(&mut host, &mut output).await;
    host.app.present_plugin_view(buffer);
    host.prepare_frame(geometry());
    model_request(
        &mut host,
        0,
        2,
        api::Request::ViewPublish {
            view: view.clone(),
            expected_revision: revision,
            expected_query_revision: None,
            model: model(&[("new", "new model")]),
        },
    )
    .await;
    reply(&mut output);
    let mut output = larger_output(&mut host);
    let pane_index = host.app.active_pane;
    let pane = host
        .app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .pane_handle(pane_index)
        .unwrap();
    request(
        &mut host,
        0,
        3,
        request_value(
            "event.subscribe",
            json!({"sources":[{"kind":"viewport","view":view,"pane":pane}]}),
        ),
    );
    let baseline = reply(&mut output)["result"].clone();
    assert!(baseline["sources"][0]["state"]["model_revision"].is_null());
    host.prepare_frame(geometry());
    let event = serde_json::to_value(next(&mut output)).unwrap();
    let state = &event["data"]["sources"][0]["state"];
    assert_eq!(state["model_revision"], "m:2");
    assert_eq!(state["top"], "new");
    assert_eq!(state["bottom"], "new");
}

#[tokio::test]
async fn accepted_action_is_reliably_correlated_before_unsubscribe_ack_and_baseline_keeps_counter()
{
    let (_root, mut host) = host();
    let mut output = view_setup(&mut host);
    let (view, _, buffer) = existing(&mut host, &mut output).await;
    let mut output = larger_output(&mut host);
    host.app.present_plugin_view(buffer);
    host.prepare_frame(geometry());
    request(
        &mut host,
        0,
        2,
        request_value(
            "event.subscribe",
            json!({"sources":[{"kind":"view_actions","view":view}]}),
        ),
    );
    let baseline = reply(&mut output)["result"].clone();
    assert_eq!(baseline["sources"][0]["state"]["accepted"], "a:0");
    assert!(matches!(
        invoke(&mut host, "plugin.tasks.toggle"),
        CommandOutcome::AsynchronousRequest(_)
    ));
    request(
        &mut host,
        0,
        3,
        request_value(
            "event.unsubscribe",
            json!({"subscription":baseline["subscription"]}),
        ),
    );
    let invocation = serde_json::to_value(next(&mut output)).unwrap();
    assert_eq!(invocation["method"], "command.invoke");
    assert_eq!(invocation["params"]["rows"], json!(["one"]));
    let action = serde_json::to_value(next(&mut output)).unwrap();
    assert_eq!(action["event"], "event.action");
    assert_eq!(action["data"]["action"]["request"], invocation["id"]);
    assert_eq!(action["data"]["action"]["command"], "toggle");
    assert_eq!(action["data"]["action"]["selected_count"], 1);
    assert!(action["data"]["action"].get("rows").is_none());
    assert!(reply(&mut output).get("error").is_none());
    request(
        &mut host,
        0,
        4,
        request_value(
            "event.subscribe",
            json!({"sources":[{"kind":"view_actions","view":view}]}),
        ),
    );
    assert_eq!(
        reply(&mut output)["result"]["sources"][0]["state"]["accepted"],
        "a:1"
    );
}

#[tokio::test]
async fn viewport_flush_failure_retires_owner_and_captures_frame_after_form_cleanup() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["views", "interaction"]);
    next(&mut output);
    let (view, _, buffer) = existing(&mut host, &mut output).await;
    let mut output = larger_output(&mut host);
    host.app.present_plugin_view(buffer);
    super::interaction::begin(
        &mut host,
        &mut output,
        2,
        vec![plugin::interaction::Field::text("name", "Name".into())],
    );
    let pane_index = host.app.active_pane;
    let pane = host
        .app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .pane_handle(pane_index)
        .unwrap();
    request(
        &mut host,
        0,
        3,
        request_value(
            "event.subscribe",
            json!({"sources":[{"kind":"viewport","view":view,"pane":pane}]}),
        ),
    );
    reply(&mut output);
    assert!(host.app.plugins.input.is_some());
    drop(output);
    let frame = host.prepare_frame(geometry());
    assert!(host.app.plugins.instances.is_empty());
    assert!(host.app.plugins.input.is_none());
    assert!(frame.overlays.is_empty());
    assert!(
        host.app.buffers[buffer]
            .display_name()
            .contains("unavailable")
    );
    let final_view = host.app.prepare_view(geometry());
    assert_eq!(host.prepared.as_ref().unwrap().view, final_view);
    assert!(host.app.plugin_viewport(0, &view, pane_index).is_none());
}
