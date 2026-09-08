// SPDX-License-Identifier: MPL-2.0
use super::*;
use serde_json::{Value, json};

fn api_request(method: &str, params: Value) -> api::Request {
    serde_json::from_value(json!({"method": method, "params": params})).unwrap()
}
fn response_json(output: &mut mpsc::Receiver<HostMessage>) -> Value {
    let message = serde_json::to_value(next(output)).unwrap();
    assert_eq!(message["type"], "response", "{message}");
    message
}
fn small_model() -> Value {
    json!({"title":"Rows","purpose":"list","rows":[
        {"id":"first","text":"First","role":"ordinary"},
        {"id":"second","text":"Second","role":"muted"},
        {"id":"third","text":"Third","role":"ordinary"}]})
}
async fn create(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
    model: Value,
) -> Value {
    model_request(
        host,
        0,
        1,
        api_request("view.create", json!({"model":model})),
    )
    .await;
    let reply = response_json(output);
    assert!(reply.get("error").is_none(), "{reply}");
    reply["result"].clone()
}

#[tokio::test]
async fn patches_are_atomic_and_reordered_generated_rows_remain_read_only() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["views"]);
    next(&mut output);
    let initial = create(&mut host, &mut output, small_model()).await;
    let view = initial["view"].clone();
    model_request(
        &mut host,
        0,
        2,
        api_request(
            "view.patch",
            json!({"view":view,"expected_revision":initial["revision"],
        "operations":[{"kind":"update","row":{"id":"first","text":"Changed","role":"warning"}},
                      {"kind":"remove","ids":["missing"]}]}),
        ),
    )
    .await;
    assert!(response_json(&mut output).get("error").is_some());
    request(
        &mut host,
        0,
        3,
        api_request("view.get", json!({"view":view})),
    );
    assert_eq!(response_json(&mut output)["result"]["model"], small_model());
    model_request(
        &mut host,
        0,
        4,
        api_request(
            "view.patch",
            json!({"view":view,"expected_revision":initial["revision"],
        "operations":[{"kind":"update","row":{"id":"first","text":"Changed","role":"warning"}},
                      {"kind":"reorder","ids":["third","first","second"]}]}),
        ),
    )
    .await;
    let changed = response_json(&mut output);
    assert_eq!(changed["result"]["model"]["rows"][1]["text"], "Changed");
    let live = &host.app.plugins.instances[&0].application.views[view.as_str().unwrap()];
    assert!(host.app.buffers[live.buffer].is_read_only());
    assert!(!host.app.buffers[live.buffer].dirty);
    assert_ne!(changed["result"]["revision"], initial["revision"]);
}

#[tokio::test]
async fn large_staged_model_publishes_once_and_snapshot_survives_later_publication() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["views"]);
    next(&mut output);
    let initial = create(&mut host, &mut output, small_model()).await;
    let view = initial["view"].clone();
    let large = json!({"title":"Large Unicode","purpose":"document","rows":[{"id":"large","text":"é猫".repeat(300_000),"role":"ordinary"}]});
    let data = serde_json::to_string(&large).unwrap();
    request(
        &mut host,
        0,
        2,
        api_request(
            "view.stage.open",
            json!({"view":view,"expected_revision":initial["revision"],"kind":"model","bytes":data.len()}),
        ),
    );
    let stage = response_json(&mut output)["result"]["stage"].clone();
    let mut serial = 3;
    let mut offset = 0;
    while offset < data.len() {
        let mut end = (offset + 128 * 1024).min(data.len());
        while !data.is_char_boundary(end) {
            end -= 1;
        }
        request(
            &mut host,
            0,
            serial,
            api_request(
                "view.stage.write",
                json!({"stage":stage,"offset":offset,"text":&data[offset..end]}),
            ),
        );
        assert_eq!(response_json(&mut output)["result"]["offset"], end);
        offset = end;
        serial += 1;
    }
    request(
        &mut host,
        0,
        serial,
        api_request("view.get", json!({"view":view})),
    );
    serial += 1;
    assert_eq!(response_json(&mut output)["result"]["model"], small_model());
    model_request(
        &mut host,
        0,
        serial,
        api_request("view.stage.commit", json!({"stage":stage})),
    )
    .await;
    serial += 1;
    let published = response_json(&mut output);
    assert!(
        published["result"].get("model").is_none(),
        "Large replies must use bounded metadata"
    );
    assert_eq!(published["result"]["rows"], 1);
    let revision = published["result"]["revision"].clone();
    request(
        &mut host,
        0,
        serial,
        api_request(
            "view.snapshot.open",
            json!({"view":view,"expected_revision":revision}),
        ),
    );
    serial += 1;
    let snapshot = response_json(&mut output)["result"].clone();
    request(
        &mut host,
        0,
        serial,
        api_request(
            "view.snapshot.read",
            json!({"snapshot":snapshot["snapshot"],"offset":0,"limit":131072}),
        ),
    );
    serial += 1;
    let first = response_json(&mut output)["result"].clone();
    assert!(!first["text"].as_str().unwrap().is_empty());
    assert_eq!(first["eof"], false);
    // Expire pacing explicitly; this test exercises snapshot identity, not time.
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .views
        .get_mut(view.as_str().unwrap())
        .unwrap()
        .published = None;
    model_request(
        &mut host,
        0,
        serial,
        api_request(
            "view.publish",
            json!({"view":view,"expected_revision":revision,"model":small_model()}),
        ),
    )
    .await;
    serial += 1;
    assert!(response_json(&mut output).get("error").is_none());
    request(
        &mut host,
        0,
        serial,
        api_request(
            "view.snapshot.read",
            json!({"snapshot":snapshot["snapshot"],"offset":0,"limit":131072}),
        ),
    );
    serial += 1;
    assert_eq!(response_json(&mut output)["result"], first);
    request(
        &mut host,
        0,
        serial,
        api_request(
            "view.snapshot.close",
            json!({"snapshot":snapshot["snapshot"]}),
        ),
    );
    assert_eq!(response_json(&mut output)["result"], json!({}));
}

#[tokio::test]
async fn stages_enforce_contiguous_writes_quota_and_owned_expiry() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["views"]);
    next(&mut output);
    let initial = create(&mut host, &mut output, small_model()).await;
    let baseline = host.app.plugins.instances[&0].application.retained_payload;
    let mut stages = Vec::new();
    for serial in 2..=4 {
        request(
            &mut host,
            0,
            serial,
            api_request(
                "view.stage.open",
                json!({
            "view":initial["view"],"expected_revision":initial["revision"],"kind":"model","bytes":2}),
            ),
        );
        let response = response_json(&mut output);
        if serial == 4 {
            assert_eq!(response["error"]["code"], "limit_exceeded");
        } else {
            stages.push(response["result"]["stage"].clone());
        }
    }
    request(
        &mut host,
        0,
        5,
        api_request(
            "view.stage.write",
            json!({"stage":stages[0],"offset":1,"text":"{"}),
        ),
    );
    assert_eq!(response_json(&mut output)["error"]["code"], "stale");
    request(
        &mut host,
        0,
        6,
        api_request(
            "view.stage.write",
            json!({"stage":stages[0],"offset":0,"text":"{"}),
        ),
    );
    assert_eq!(response_json(&mut output)["result"]["offset"], 1);
    model_request(
        &mut host,
        0,
        7,
        api_request("view.stage.commit", json!({"stage":stages[0]})),
    )
    .await;
    assert_eq!(
        response_json(&mut output)["error"]["code"],
        "invalid_argument"
    );
    assert_eq!(
        host.app.plugins.instances[&0].application.view_stages.len(),
        2
    );
    // A close for another resource kind must not cancel its lifecycle deadline.
    let token = initial["view"].as_str().unwrap().to_owned();
    let deadline = std::time::Instant::now();
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .deadlines
        .insert(token.clone(), deadline);
    for (serial, method, key) in [
        (8, "view.stage.close", "stage"),
        (9, "view.snapshot.close", "snapshot"),
    ] {
        request(
            &mut host,
            0,
            serial,
            api_request(method, json!({key:token})),
        );
        assert_eq!(response_json(&mut output)["result"], json!({}));
        assert_eq!(
            host.app.plugins.instances[&0]
                .application
                .deadlines
                .get(&token),
            Some(&deadline)
        );
    }
    for stage in stages {
        assert!(host.model_deadline(0, stage.as_str().unwrap()));
    }
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        baseline
    );
}

#[tokio::test]
async fn closing_committed_stage_cancels_pending_publication() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["views"]);
    next(&mut output);
    let initial = create(&mut host, &mut output, small_model()).await;
    let baseline = host.app.plugins.instances[&0].application.retained_payload;
    let data = serde_json::to_string(&small_model()).unwrap();
    request(
        &mut host,
        0,
        2,
        api_request(
            "view.stage.open",
            json!({
        "view":initial["view"],"expected_revision":initial["revision"],"kind":"model","bytes":data.len()}),
        ),
    );
    let stage = response_json(&mut output)["result"]["stage"].clone();
    request(
        &mut host,
        0,
        3,
        api_request(
            "view.stage.write",
            json!({"stage":stage,"offset":0,"text":data}),
        ),
    );
    response_json(&mut output);
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        4,
        api_request("view.stage.commit", json!({"stage":stage})),
    );
    assert_eq!(
        host.app.plugins.instances[&0]
            .application
            .model_requests
            .len(),
        1
    );
    request(
        &mut host,
        0,
        5,
        api_request("view.stage.close", json!({"stage":stage})),
    );
    assert_eq!(response_json(&mut output)["result"], json!({}));
    model_complete(&mut host, &mut events).await;
    assert_eq!(response_json(&mut output)["error"]["code"], "cancelled");
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        baseline
    );
    request(
        &mut host,
        0,
        6,
        api_request("view.get", json!({"view":initial["view"]})),
    );
    assert_eq!(response_json(&mut output)["result"], initial);
}

#[tokio::test]
async fn stopped_model_worker_keeps_reservation_until_completion_delivery() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["views"]);
    next(&mut output);
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        1,
        api_request("view.create", json!({"model":small_model()})),
    );
    let reserved = host.app.plugins.instances[&0].application.model_requests["p:1"].charge;
    assert!(reserved > 0);
    host.stop_plugin(0, "test stop during model preparation");
    assert_eq!(host.app.plugins.orphaned_payload, reserved);
    assert_eq!(host.plugin_local_orphans.len(), 1);
    model_complete(&mut host, &mut events).await;
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert!(host.plugin_local_orphans.is_empty());
}
