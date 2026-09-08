// SPDX-License-Identifier: MPL-2.0

use super::*;

async fn existing(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
) -> (String, String, usize) {
    model_request(
        host,
        0,
        1,
        api::Request::ViewCreate {
            model: model(&[("one", "retained source")]),
        },
    )
    .await;
    let (view, revision) = view_result(output);
    let buffer = host.app.plugins.instances[&0].application.views[&view].buffer;
    (view, revision, buffer)
}

#[tokio::test]
async fn stopping_model_owner_retains_captured_source_charge_until_real_completion() {
    let (_root, mut host) = host();
    let mut output = view_setup(&mut host);
    let (view, revision, buffer) = existing(&mut host, &mut output).await;
    let source_charge = host.app.plugins.instances[&0].application.views[&view].charge;
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        2,
        api::Request::ViewPublish {
            view,
            expected_revision: revision,
            model: model(&[("one", "uncommitted replacement")]),
        },
    );
    let pending = &host.app.plugins.instances[&0].application.model_requests["p:2"];
    assert_eq!(pending.source_charge, source_charge);
    let charge = pending.charge + source_charge;
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        charge
    );
    host.stop_plugin(0, "stopped by user");
    assert_eq!(host.app.plugins.orphaned_payload, charge);
    assert_eq!(host.app.buffers[buffer].to_string(), "retained source\n");
    assert!(
        host.app.buffers[buffer]
            .display_name()
            .contains("unavailable")
    );
    model_complete(&mut host, &mut events).await;
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert!(host.plugin_local_orphans.is_empty());
    assert_eq!(host.app.buffers[buffer].to_string(), "retained source\n");
}

#[tokio::test]
async fn closing_preparing_view_transfers_source_once_then_stop_preserves_reservation() {
    let (_root, mut host) = host();
    let mut output = view_setup(&mut host);
    let (view, revision, _) = existing(&mut host, &mut output).await;
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        2,
        api::Request::ViewPublish {
            view: view.clone(),
            expected_revision: revision,
            model: model(&[("one", "uncommitted replacement")]),
        },
    );
    let state = &host.app.plugins.instances[&0].application;
    let old = &state.model_requests["p:2"];
    let transferred = old.charge + old.source_charge;
    assert_eq!(state.retained_payload, transferred);
    request(
        &mut host,
        0,
        3,
        api::Request::ViewClose { view: view.clone() },
    );
    let state = &host.app.plugins.instances[&0].application;
    assert!(!state.views.contains_key(&view));
    assert_eq!(state.model_requests["p:2"].source_charge, 0);
    assert_eq!(state.model_requests["p:2"].charge, transferred);
    assert_eq!(state.retained_payload, transferred);
    // Repeated collection and subsequent owner retirement cannot transfer twice.
    host.sync_plugin_observers();
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        transferred
    );
    host.stop_plugin(0, "stopped by user");
    assert_eq!(host.app.plugins.orphaned_payload, transferred);
    model_complete(&mut host, &mut events).await;
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert!(host.plugin_local_orphans.is_empty());
}

#[tokio::test]
async fn closed_source_completion_releases_transferred_charge_and_returns_cancelled() {
    let (_root, mut host) = host();
    let mut output = view_setup(&mut host);
    let (view, revision, _) = existing(&mut host, &mut output).await;
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        2,
        api::Request::ViewPublish {
            view: view.clone(),
            expected_revision: revision,
            model: model(&[("one", "uncommitted replacement")]),
        },
    );
    request(&mut host, 0, 3, api::Request::ViewClose { view });
    next(&mut output); // Close response precedes lifecycle notification.
    assert!(matches!(
        next(&mut output),
        api::HostMessage::Event {
            event: "view.closed",
            ..
        }
    ));
    model_complete(&mut host, &mut events).await;
    assert!(matches!(next(&mut output), api::HostMessage::Response {
        id, outcome: api::Response::Failure { error: api::Error { code: api::ErrorCode::Cancelled, .. } }
    } if id == "p:2"));
    let state = &host.app.plugins.instances[&0].application;
    assert!(state.model_requests.is_empty());
    assert_eq!(state.retained_payload, 0);
}
