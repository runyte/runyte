// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::plugin::process;
use serde_json::{Value, json};

fn operation(method: &str, params: Value) -> api::Request {
    serde_json::from_value(json!({"method":method,"params":params})).unwrap()
}
fn reply(output: &mut mpsc::Receiver<HostMessage>) -> Value {
    let value = serde_json::to_value(next(output)).unwrap();
    assert_eq!(value["type"], "response", "{value}");
    value
}
fn setup_processes(host: &mut WorkspaceHost, owner: usize) -> mpsc::Receiver<HostMessage> {
    let mut initial = setup(host, owner, &["processes"]);
    next(&mut initial);
    let (sender, output) = mpsc::channel(64);
    host.app.plugins.instances.get_mut(&owner).unwrap().sender = plugin::Sender::new(sender);
    output
}
fn fixture(root: &std::path::Path, name: &str, behavior: &str) -> String {
    let executable = root.join(name);
    std::os::unix::fs::symlink(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/fixtures/stand-in"),
        &executable,
    )
    .unwrap();
    std::fs::write(root.join(format!("{name}.behavior")), behavior).unwrap();
    executable.to_str().unwrap().into()
}
async fn start(
    host: &mut WorkspaceHost,
    events: &mut mpsc::Receiver<Event>,
    output: &mut mpsc::Receiver<HostMessage>,
    owner: usize,
    serial: u64,
    executable: &str,
    capture_stderr: bool,
) -> Value {
    request(
        host,
        owner,
        serial,
        operation(
            "process.start",
            json!({"label":"Helper","executable":executable,"args":[],"capture_stderr":capture_stderr}),
        ),
    );
    while output.is_empty() {
        model_complete(host, events).await;
    }
    let value = reply(output);
    assert!(value.get("error").is_none(), "{value}");
    value["result"].clone()
}
async fn until_exit(
    host: &mut WorkspaceHost,
    events: &mut mpsc::Receiver<Event>,
    owner: usize,
    handle: &str,
) {
    while host
        .process_info(owner, handle)
        .is_some_and(|info| info.state != process::State::Exited)
    {
        model_complete(host, events).await;
    }
}

#[tokio::test]
async fn helper_binary_write_ack_matches_actual_bytes_and_exited_output_remains_readable() {
    let (root, mut host) = host();
    let executable = fixture(root.path(), "echo-helper", "printf '\\377\\000' >&2\ncat\n");
    let mut output = setup_processes(&mut host, 0);
    let mut events = model_service(&mut host);
    let initial = start(&mut host, &mut events, &mut output, 0, 1, &executable, true).await;
    let handle = initial["process"].as_str().unwrap();
    let bytes = b"\0binary\xff\n";
    request(
        &mut host,
        0,
        2,
        operation(
            "process.write",
            json!({"process":handle,"data":process::encode(bytes),"eof":true}),
        ),
    );
    assert_eq!(host.pending_process_requests(Some(0)), 1);
    while output.is_empty() {
        model_complete(&mut host, &mut events).await;
    }
    let written = reply(&mut output);
    assert_eq!(written["result"]["written"], bytes.len());
    assert_eq!(written["result"]["stdin_closed"], true);
    until_exit(&mut host, &mut events, 0, handle).await;
    request(
        &mut host,
        0,
        3,
        operation(
            "process.read",
            json!({"process":handle,"stream":"stdout","offset":0,"limit":65536}),
        ),
    );
    let read = reply(&mut output);
    assert_eq!(
        process::decode(read["result"]["data"].as_str().unwrap()).unwrap(),
        bytes
    );
    assert_eq!(read["result"]["eof"], true);
    request(
        &mut host,
        0,
        4,
        operation(
            "process.read",
            json!({"process":handle,"stream":"stderr","offset":0,"limit":65536}),
        ),
    );
    assert_eq!(
        process::decode(reply(&mut output)["result"]["data"].as_str().unwrap()).unwrap(),
        [255, 0]
    );
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        process::PROCESS_CHARGE
    );
    request(
        &mut host,
        0,
        5,
        operation("process.close", json!({"process":handle})),
    );
    assert!(reply(&mut output).get("error").is_none());
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
    assert!(host.plugin_processes.is_empty());
}

#[tokio::test]
async fn spawn_failure_releases_charge_and_never_issues_a_handle() {
    let (_root, mut host) = host();
    let mut output = setup_processes(&mut host, 0);
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        1,
        operation(
            "process.start",
            json!({"label":"Missing","executable":"/nonexistent/runyte-helper"}),
        ),
    );
    assert_eq!(host.protected_state().plugin_jobs, 1);
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        process::PROCESS_CHARGE
    );
    while output.is_empty() {
        model_complete(&mut host, &mut events).await;
    }
    assert!(reply(&mut output).get("error").is_some());
    assert_eq!(host.protected_state().plugin_jobs, 0);
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
    assert!(
        host.app.plugins.instances[&0]
            .application
            .processes
            .is_empty()
    );
    assert!(host.plugin_processes.is_empty());
}

#[tokio::test]
async fn close_waiters_are_bounded_and_all_acknowledged_before_closed_observation() {
    let (root, mut host) = host();
    let executable = fixture(root.path(), "idle-helper", "cat >/dev/null\n");
    let mut output = setup_processes(&mut host, 0);
    let mut events = model_service(&mut host);
    let initial = start(
        &mut host,
        &mut events,
        &mut output,
        0,
        1,
        &executable,
        false,
    )
    .await;
    let handle = initial["process"].as_str().unwrap();
    assert_eq!(
        host.protected_state().plugin_jobs,
        0,
        "idle helpers are not activity leases"
    );
    request(
        &mut host,
        0,
        2,
        operation(
            "event.subscribe",
            json!({"sources":[{"kind":"process","process":handle}]}),
        ),
    );
    reply(&mut output);
    for serial in 3..19 {
        request(
            &mut host,
            0,
            serial,
            operation("process.close", json!({"process":handle})),
        );
        assert!(output.is_empty(), "close is acknowledged only after reap");
    }
    assert_eq!(host.pending_process_requests(Some(0)), 16);
    assert_eq!(host.protected_state().plugin_jobs, 16);
    request(
        &mut host,
        0,
        19,
        operation("process.close", json!({"process":handle})),
    );
    assert_eq!(reply(&mut output)["error"]["code"], "busy");
    request(
        &mut host,
        0,
        20,
        operation("process.get", json!({"process":handle})),
    );
    assert_eq!(reply(&mut output)["result"]["state"], "closing");
    until_exit(&mut host, &mut events, 0, handle).await;
    for serial in 3..19 {
        let closed = reply(&mut output);
        assert_eq!(closed["id"], format!("p:{serial}"));
        assert!(closed.get("error").is_none());
    }
    let closed = serde_json::to_value(next(&mut output)).unwrap();
    assert_eq!(closed["event"], "event.closed");
    assert_eq!(closed["data"]["sources"][0]["state"]["kind"], "closed");
    assert_eq!(host.protected_state().plugin_jobs, 0);
    assert!(host.plugin_processes.is_empty());
}

#[tokio::test]
async fn process_owner_checks_disabled_stderr_and_future_offsets_do_not_mutate_helper() {
    let (root, mut host) = host();
    let executable = fixture(root.path(), "owned-helper", "cat >/dev/null\n");
    let mut output = setup_processes(&mut host, 0);
    let mut other = setup_processes(&mut host, 1);
    let mut events = model_service(&mut host);
    let initial = start(
        &mut host,
        &mut events,
        &mut output,
        0,
        1,
        &executable,
        false,
    )
    .await;
    let handle = initial["process"].as_str().unwrap();
    request(
        &mut host,
        1,
        1,
        operation("process.get", json!({"process":handle})),
    );
    assert_eq!(reply(&mut other)["error"]["code"], "not_found");
    request(
        &mut host,
        1,
        2,
        operation("process.close", json!({"process":handle})),
    );
    assert!(reply(&mut other).get("error").is_none());
    assert_eq!(
        host.process_info(0, handle).unwrap().state,
        process::State::Running
    );
    request(
        &mut host,
        0,
        2,
        operation(
            "process.read",
            json!({"process":handle,"stream":"stderr","offset":0,"limit":1}),
        ),
    );
    assert_eq!(reply(&mut output)["error"]["code"], "unsupported");
    request(
        &mut host,
        0,
        3,
        operation(
            "process.read",
            json!({"process":handle,"stream":"stdout","offset":1,"limit":1}),
        ),
    );
    assert_eq!(reply(&mut output)["error"]["code"], "invalid_argument");
    request(
        &mut host,
        0,
        4,
        operation(
            "process.write",
            json!({"process":handle,"data":"%%%","eof":false}),
        ),
    );
    assert_eq!(reply(&mut output)["error"]["code"], "invalid_argument");
    request(
        &mut host,
        0,
        5,
        operation("process.close", json!({"process":handle})),
    );
    until_exit(&mut host, &mut events, 0, handle).await;
    reply(&mut output);
}

#[tokio::test]
async fn stopped_generation_retains_configured_identity_quota_until_every_helper_is_reaped() {
    let (root, mut host) = host();
    let executable = fixture(root.path(), "orphan-helper", "cat >/dev/null\n");
    let mut output = setup_processes(&mut host, 0);
    let mut events = model_service(&mut host);
    for serial in 1..=4 {
        start(
            &mut host,
            &mut events,
            &mut output,
            0,
            serial,
            &executable,
            false,
        )
        .await;
    }
    host.stop_plugin(0, "stopped by user");
    assert_eq!(
        host.app.plugins.orphaned_payload,
        process::PROCESS_CHARGE * 4
    );
    let mut output = setup_processes(&mut host, 0);
    request(
        &mut host,
        0,
        1,
        operation(
            "process.start",
            json!({"label":"New generation","executable":executable}),
        ),
    );
    assert_eq!(reply(&mut output)["error"]["code"], "limit_exceeded");
    while !host.plugin_processes.is_empty() {
        model_complete(&mut host, &mut events).await;
    }
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert!(
        output.is_empty(),
        "old generation sends no replies to replacement"
    );
    let initial = start(
        &mut host,
        &mut events,
        &mut output,
        0,
        2,
        &executable,
        false,
    )
    .await;
    let handle = initial["process"].as_str().unwrap();
    request(
        &mut host,
        0,
        3,
        operation("process.close", json!({"process":handle})),
    );
    until_exit(&mut host, &mut events, 0, handle).await;
    reply(&mut output);
}

#[tokio::test]
async fn failed_start_acknowledgement_stops_owner_but_retains_reaping_charge() {
    let (root, mut host) = host();
    let executable = fixture(root.path(), "ack-helper", "cat >/dev/null\n");
    let output = setup_processes(&mut host, 0);
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        1,
        operation(
            "process.start",
            json!({"label":"Helper","executable":executable}),
        ),
    );
    drop(output);
    model_complete(&mut host, &mut events).await;
    assert!(!host.app.plugins.instances.contains_key(&0));
    assert_eq!(host.app.plugins.orphaned_payload, process::PROCESS_CHARGE);
    while !host.plugin_processes.is_empty() {
        model_complete(&mut host, &mut events).await;
    }
    assert_eq!(host.app.plugins.orphaned_payload, 0);
}

#[tokio::test]
async fn startup_timeout_replies_once_but_retains_unissued_helper_charge_until_final_reap() {
    let (root, mut host) = host();
    let executable = fixture(root.path(), "late-helper", "cat >/dev/null\n");
    let mut output = setup_processes(&mut host, 0);
    let mut events = model_service(&mut host);
    request(
        &mut host,
        0,
        1,
        operation(
            "process.start",
            json!({"label":"Late helper","executable":executable}),
        ),
    );
    let handle = host.plugin_processes.keys().next().unwrap().clone();
    let generation = host.app.plugins.instances[&0]
        .application
        .generation
        .clone();
    let permit = std::sync::Arc::new(tokio::sync::Semaphore::new(1))
        .try_acquire_owned()
        .unwrap();
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Process(process::runtime::Event {
            generation,
            process: handle,
            kind: process::runtime::Kind::StartFailed {
                error: api::Error::new(api::ErrorCode::Unavailable, "Helper startup timed out"),
            },
            _permit: permit,
            _lifetime: None,
        })),
    });
    assert_eq!(reply(&mut output)["error"]["code"], "unavailable");
    assert_eq!(host.pending_process_requests(Some(0)), 0);
    assert!(
        host.app.plugins.instances[&0]
            .application
            .processes
            .is_empty()
    );
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        process::PROCESS_CHARGE
    );
    while !host.plugin_processes.is_empty() {
        model_complete(&mut host, &mut events).await;
    }
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
    assert!(
        host.app.plugins.instances[&0]
            .application
            .processes
            .is_empty()
    );
    assert!(
        output.is_empty(),
        "late Started cannot create a second acknowledgement"
    );
}

#[tokio::test]
async fn reserved_terminal_settles_pending_write_and_close_while_output_events_hold_all_slots() {
    let (root, mut host) = host();
    let executable = fixture(
        root.path(),
        "saturated-helper",
        "head -c 131072 /dev/zero\ncat >/dev/null\n",
    );
    let mut output = setup_processes(&mut host, 0);
    let mut events = model_service(&mut host);
    let initial = start(
        &mut host,
        &mut events,
        &mut output,
        0,
        1,
        &executable,
        false,
    )
    .await;
    let handle = initial["process"].as_str().unwrap();
    let mut held = Vec::new();
    for _ in 0..3 {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(&event.result, Ok(ClientMessage::Process(event)) if matches!(event.kind, process::runtime::Kind::Output { .. }))
        );
        held.push(event);
    }
    request(
        &mut host,
        0,
        2,
        operation(
            "process.write",
            json!({"process":handle,"data":process::encode(&vec![1; 65536]),"eof":false}),
        ),
    );
    request(
        &mut host,
        0,
        3,
        operation("process.close", json!({"process":handle})),
    );
    let terminal = tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(&terminal.result, Ok(ClientMessage::Process(event)) if matches!(event.kind, process::runtime::Kind::Exited { .. }))
    );
    for event in held {
        host.handle_plugin_event(event);
    }
    host.handle_plugin_event(terminal);
    let written = reply(&mut output);
    assert_eq!(written["id"], "p:2");
    if written.get("error").is_some() {
        assert_eq!(written["error"]["code"], "outcome_unknown");
    } else {
        assert_eq!(written["result"]["written"], 65536);
        assert_eq!(written["result"]["stdin_closed"], true);
    }
    let closed = reply(&mut output);
    assert_eq!(closed["id"], "p:3");
    assert!(closed.get("error").is_none());
    assert!(host.plugin_processes.is_empty());
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}
