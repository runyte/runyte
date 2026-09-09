// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::plugin::handoff;
use serde_json::json;

fn grant(host: &mut WorkspaceHost, output: &mut mpsc::Receiver<HostMessage>) -> String {
    host.app.note_plugin_frontend(true);
    invoke(host, "plugin.app-0.open");
    let api::HostMessage::Request { id, .. } = next(output) else {
        panic!()
    };
    id
}
fn external(invocation: &str, target: &str) -> api::Request {
    api::Request::ExternalOpen {
        invocation: invocation.into(),
        target: handoff::Target::Url { url: target.into() },
    }
}
fn terminal(invocation: &str, executable: &str, cwd: Option<&str>) -> api::Request {
    api::Request::TerminalOpen(handoff::TerminalOpen {
        invocation: invocation.into(),
        label: "Native tool".into(),
        executable: executable.into(),
        args: vec![],
        cwd: cwd.map(str::to_owned),
    })
}
async fn settled(host: &mut WorkspaceHost, events: &mut mpsc::Receiver<Event>) {
    while !host.plugin_handoffs.is_empty() {
        model_complete(host, events).await;
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

#[tokio::test]
async fn handoff_capability_foreground_and_worker_capacity_refusals_do_not_consume_grant() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &[]);
    next(&mut output);
    let invocation = grant(&mut host, &mut output);
    request(
        &mut host,
        0,
        1,
        external(&invocation, "https://example.test"),
    );
    assert_eq!(code(&mut output), api::ErrorCode::CapabilityDenied);
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .capabilities
        .insert("external".into());
    request(
        &mut host,
        0,
        2,
        external("h:unknown", "https://example.test"),
    );
    assert_eq!(code(&mut output), api::ErrorCode::ContextChanged);
    let _events = model_service(&mut host);
    host.plugin_local_slots = Some(std::sync::Arc::new(tokio::sync::Semaphore::new(0)));
    request(
        &mut host,
        0,
        3,
        external(&invocation, "https://example.test"),
    );
    assert_eq!(code(&mut output), api::ErrorCode::Busy);
    assert!(host.plugin_handoffs.is_empty());
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
    assert!(host.app.plugins.instances[&0].application.requests[&invocation].foreground_allowed);
}

#[tokio::test]
async fn invalid_external_target_preserves_authority_and_releases_reservation() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["external"]);
    next(&mut output);
    let invocation = grant(&mut host, &mut output);
    let mut events = model_service(&mut host);
    request(&mut host, 0, 1, external(&invocation, "file:///etc/passwd"));
    request(
        &mut host,
        0,
        2,
        external(&invocation, "https://example.test"),
    );
    assert_eq!(code(&mut output), api::ErrorCode::Busy);
    settled(&mut host, &mut events).await;
    assert_eq!(code(&mut output), api::ErrorCode::InvalidArgument);
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
    assert!(host.app.plugins.instances[&0].application.requests[&invocation].foreground_allowed);
    assert_eq!(
        host.plugin_local_slots
            .as_ref()
            .unwrap()
            .available_permits(),
        16
    );
    request(&mut host, 0, 3, external(&invocation, &"x".repeat(4097)));
    assert_eq!(code(&mut output), api::ErrorCode::InvalidArgument);
    assert!(host.plugin_handoffs.is_empty());
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}

#[tokio::test]
async fn prepared_external_target_cannot_launch_after_invocation_reply_or_owner_stop() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["external"]);
    next(&mut output);
    let invocation = grant(&mut host, &mut output);
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        1,
        external(&invocation, "https://example.test"),
    );
    let event = events.recv().await.unwrap();
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .requests
        .remove(&invocation);
    host.handle_plugin_event(event);
    settled(&mut host, &mut events).await;
    assert_eq!(code(&mut output), api::ErrorCode::ContextChanged);
    let invocation = grant(&mut host, &mut output);
    request(
        &mut host,
        0,
        2,
        external(&invocation, "https://example.test"),
    );
    let event = events.recv().await.unwrap();
    host.stop_plugin(0, "test stop");
    assert_eq!(host.app.plugins.orphaned_payload, 128 * 1024);
    host.handle_plugin_event(event);
    settled(&mut host, &mut events).await;
    assert_eq!(host.app.plugins.orphaned_payload, 0);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn terminal_handoff_installs_native_ownership_that_survives_plugin_stop() {
    let _guard = crate::terminal::pending_test_guard();
    let (root, mut host) = host();
    let executable = root.path().join("native-helper");
    std::os::unix::fs::symlink(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/fixtures/stand-in"),
        &executable,
    )
    .unwrap();
    std::fs::write(
        root.path().join("native-helper.behavior"),
        "printf 'native output'\n",
    )
    .unwrap();
    let mut output = setup(&mut host, 0, &["terminals"]);
    next(&mut output);
    let invocation = grant(&mut host, &mut output);
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        1,
        terminal(&invocation, executable.to_str().unwrap(), None),
    );
    assert!(host.protected_state().plugin_jobs > 0);
    settled(&mut host, &mut events).await;
    let value = serde_json::to_value(next(&mut output)).unwrap();
    assert!(value["result"]["terminal"].as_str().is_some(), "{value}");
    let id = host.app.active_terminal().unwrap();
    assert!(!host.app.plugins.instances[&0].application.requests[&invocation].foreground_allowed);
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
    // Replacing a terminal pane has no visible-buffer fingerprint change.
    // The explicit presentation marker must still wake an idle frontend.
    let invocation = grant(&mut host, &mut output);
    host.take_plugin_presentation_change();
    request(
        &mut host,
        0,
        2,
        terminal(&invocation, executable.to_str().unwrap(), None),
    );
    settled(&mut host, &mut events).await;
    let value = serde_json::to_value(next(&mut output)).unwrap();
    assert!(value["result"]["terminal"].as_str().is_some(), "{value}");
    let second = host.app.active_terminal().unwrap();
    assert_ne!(id, second);
    assert!(host.plugin_presentation_pending());
    host.stop_plugin(0, "test stop");
    assert!(host.app.terminals.get(id).is_some());
    assert!(host.app.terminals.get(second).is_some());
    host.app.terminals.close(id);
    host.app.terminals.close(second);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn terminal_cwd_escape_and_late_owner_stop_never_publish_a_terminal() {
    let _guard = crate::terminal::pending_test_guard();
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["terminals"]);
    next(&mut output);
    let invocation = grant(&mut host, &mut output);
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        1,
        terminal(&invocation, "/bin/true", Some("/")),
    );
    settled(&mut host, &mut events).await;
    assert_eq!(code(&mut output), api::ErrorCode::InvalidArgument);
    assert_eq!(host.app.terminals.len(), 0);
    request(&mut host, 0, 2, terminal(&invocation, "/bin/true", None));
    let event = events.recv().await.unwrap();
    host.stop_plugin(0, "test stop");
    assert_eq!(
        host.app.plugins.orphaned_payload,
        crate::terminal::PENDING_TERMINAL_CHARGE
    );
    host.handle_plugin_event(event);
    assert!(host.app.plugins.orphaned_payload > 0);
    settled(&mut host, &mut events).await;
    assert_eq!(host.app.terminals.len(), 0);
    assert_eq!(host.app.plugins.orphaned_payload, 0);
}

#[test]
fn terminal_wire_argument_bounds_and_unknown_fields_are_strict() {
    let frame = |params| {
        serde_json::to_vec(
            &json!({"type":"request","id":"p:1","method":"terminal.open","params":params}),
        )
        .unwrap()
    };
    assert!(
        api::decode(&frame(
            json!({"invocation":"h:1","label":"Tool","executable":"tool","args":vec!["";65]})
        ))
        .is_err()
    );
    assert!(
        api::decode(&frame(
            json!({"invocation":"h:1","label":"Tool","executable":"tool","capture_stderr":true})
        ))
        .is_err()
    );
    let start = handoff::TerminalOpen {
        invocation: "h:1".into(),
        label: "Tool".into(),
        executable: "tool".into(),
        args: vec!["x".repeat(4097)],
        cwd: None,
    };
    assert_eq!(
        start.validate().unwrap_err().code,
        api::ErrorCode::LimitExceeded
    );
}

#[tokio::test]
async fn accepted_external_spawn_consumes_foreground_grant_and_cannot_repeat() {
    let (root, mut host) = host();
    let executable = root.path().join("system-handler");
    std::os::unix::fs::symlink(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/fixtures/stand-in"),
        &executable,
    )
    .unwrap();
    std::fs::write(root.path().join("system-handler.behavior"), "exit 0\n").unwrap();
    host.plugin_external_launcher = Some(executable);
    let mut output = setup(&mut host, 0, &["external"]);
    next(&mut output);
    let invocation = grant(&mut host, &mut output);
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        1,
        external(&invocation, "https://example.test"),
    );
    settled(&mut host, &mut events).await;
    let value = serde_json::to_value(next(&mut output)).unwrap();
    assert_eq!(value["result"], json!({}), "{value}");
    assert!(!host.app.plugins.instances[&0].application.requests[&invocation].foreground_allowed);
    request(
        &mut host,
        0,
        2,
        external(&invocation, "https://example.test"),
    );
    assert_eq!(code(&mut output), api::ErrorCode::ContextChanged);
    assert!(host.plugin_handoffs.is_empty());
}

#[tokio::test]
async fn delayed_preparation_exhausts_absolute_budget_without_consuming_authority() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["external"]);
    next(&mut output);
    let invocation = grant(&mut host, &mut output);
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        1,
        external(&invocation, "https://example.test"),
    );
    let event = events.recv().await.unwrap();
    host.plugin_handoffs.values_mut().next().unwrap().deadline = std::time::Instant::now();
    host.handle_plugin_event(event);
    assert_eq!(code(&mut output), api::ErrorCode::Timeout);
    assert!(host.app.plugins.instances[&0].application.requests[&invocation].foreground_allowed);
    settled(&mut host, &mut events).await;
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}
