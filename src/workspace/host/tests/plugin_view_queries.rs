// SPDX-License-Identifier: MPL-2.0
use super::*;
use serde_json::{Value, json};

fn operation(method: &str, params: Value) -> api::Request {
    serde_json::from_value(json!({"method":method,"params":params})).unwrap()
}
fn reply(output: &mut mpsc::Receiver<HostMessage>) -> Value {
    let value = serde_json::to_value(next(output)).unwrap();
    assert_eq!(value["type"], "response", "{value}");
    value
}
async fn initial(host: &mut WorkspaceHost, output: &mut mpsc::Receiver<HostMessage>) -> Value {
    model_request(
        host,
        0,
        1,
        api::Request::ViewCreate {
            model: model(&[("a", "A"), ("b", "B")]),
        },
    )
    .await;
    reply(output)["result"].clone()
}
fn query(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
    serial: u64,
    initial: &Value,
    expected: Option<&str>,
    text: &str,
) -> Value {
    let mut params =
        json!({"view":initial["view"],"expected_revision":initial["revision"],"text":text});
    if let Some(expected) = expected {
        params["expected_query_revision"] = json!(expected);
    }
    request(host, 0, serial, operation("view.query.set", params));
    reply(output)
}

#[tokio::test]
async fn identical_query_text_cannot_authorize_an_older_inflight_model() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["views"]);
    next(&mut output);
    let initial = initial(&mut host, &mut output).await;
    let handle = initial["view"].as_str().unwrap();
    let buffer = host.app.plugins.instances[&0].application.views[handle].buffer;
    let original_text_revision = host.app.buffers[buffer].revision();
    assert_eq!(
        query(&mut host, &mut output, 2, &initial, None, "A")["result"]["query"]["revision"],
        "qv:1"
    );
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        3,
        operation(
            "view.publish",
            json!({"view":handle,"expected_revision":initial["revision"],"expected_query_revision":"qv:1","model":{"title":"Old A","purpose":"list","rows":[]}}),
        ),
    );
    assert_eq!(
        query(&mut host, &mut output, 4, &initial, Some("qv:1"), "B")["result"]["query"]["revision"],
        "qv:2"
    );
    assert_eq!(
        query(&mut host, &mut output, 5, &initial, Some("qv:2"), "A")["result"]["query"]["revision"],
        "qv:3"
    );
    assert_eq!(host.app.buffers[buffer].revision(), original_text_revision);
    model_complete(&mut host, &mut events).await;
    assert_eq!(reply(&mut output)["error"]["code"], "stale");
    assert_eq!(host.app.buffers[buffer].to_string(), "A\nB\n");
    assert!(
        host.app.plugins.instances[&0].application.views[handle]
            .query
            .as_ref()
            .unwrap()
            .pending
    );
    model_request(&mut host, 0, 6, operation("view.publish", json!({"view":handle,"expected_revision":initial["revision"],"expected_query_revision":"qv:3","model":{"title":"New A","purpose":"list","rows":[{"id":"a","text":"A current","role":"ordinary"}]}}))).await;
    let result = reply(&mut output);
    assert_eq!(result["result"]["query"]["pending"], false);
    assert_eq!(result["result"]["query"]["revision"], "qv:3");
    assert_eq!(host.app.buffers[buffer].to_string(), "A current\n");
    assert_eq!(host.app.buffers[buffer].display_name(), "New A");
}

#[tokio::test]
async fn invalid_queries_and_failed_models_preserve_current_pending_intent() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["views"]);
    next(&mut output);
    let initial = initial(&mut host, &mut output).await;
    let handle = initial["view"].as_str().unwrap();
    let baseline = host.app.plugins.instances[&0].application.retained_payload;
    assert_eq!(
        query(&mut host, &mut output, 2, &initial, None, "bad\nquery")["error"]["code"],
        "invalid_argument"
    );
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        baseline
    );
    assert_eq!(
        query(&mut host, &mut output, 3, &initial, None, "")["result"]["query"]["revision"],
        "qv:1"
    );
    let retained = host.app.plugins.instances[&0].application.retained_payload;
    assert_eq!(
        query(&mut host, &mut output, 4, &initial, None, "missing token")["error"]["code"],
        "stale"
    );
    assert_eq!(
        query(
            &mut host,
            &mut output,
            5,
            &initial,
            Some("qv:1"),
            &"é".repeat(513)
        )["error"]["code"],
        "limit_exceeded"
    );
    model_request(&mut host, 0, 6, operation("view.publish", json!({"view":handle,"expected_revision":initial["revision"],"expected_query_revision":"qv:1","model":{"title":"Invalid","purpose":"list","rows":[{"id":"same","text":"one","role":"ordinary"},{"id":"same","text":"two","role":"ordinary"}]}}))).await;
    assert_eq!(reply(&mut output)["error"]["code"], "invalid_argument");
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        retained
    );
    let query = host.app.plugins.instances[&0].application.views[handle]
        .query
        .as_ref()
        .unwrap();
    assert!(query.pending);
    assert_eq!(query.text, "");
    assert_eq!(query.revision, 1);
}

#[tokio::test]
async fn cancelling_matching_staged_publication_keeps_query_pending() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["views"]);
    next(&mut output);
    let initial = initial(&mut host, &mut output).await;
    let handle = initial["view"].as_str().unwrap();
    query(&mut host, &mut output, 2, &initial, None, "A");
    let baseline = host.app.plugins.instances[&0].application.retained_payload;
    let text = serde_json::to_string(&model(&[("a", "A")])).unwrap();
    request(
        &mut host,
        0,
        3,
        operation(
            "view.stage.open",
            json!({"view":handle,"expected_revision":initial["revision"],"expected_query_revision":"qv:1","kind":"model","bytes":text.len()}),
        ),
    );
    let stage = reply(&mut output)["result"]["stage"].clone();
    request(
        &mut host,
        0,
        4,
        operation(
            "view.stage.write",
            json!({"stage":stage,"offset":0,"text":text}),
        ),
    );
    reply(&mut output);
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        5,
        operation("view.stage.commit", json!({"stage":stage})),
    );
    request(
        &mut host,
        0,
        6,
        operation("view.stage.close", json!({"stage":stage})),
    );
    reply(&mut output);
    model_complete(&mut host, &mut events).await;
    assert_eq!(reply(&mut output)["error"]["code"], "cancelled");
    assert!(
        host.app.plugins.instances[&0].application.views[handle]
            .query
            .as_ref()
            .unwrap()
            .pending
    );
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        baseline
    );
}
