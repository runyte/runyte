// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::{App, HostPorts},
    clipboard::SystemClipboard,
    input::{InputEvent, KeyCode, KeyStroke},
    test_support::TestRuntimeRoot,
};

struct Clipboard;
impl SystemClipboard for Clipboard {
    fn read(&mut self) -> anyhow::Result<String> {
        anyhow::bail!("inert")
    }
    fn write(&mut self, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
fn fixture() -> (
    TestRuntimeRoot,
    WorkspaceHost,
    Identity,
    mpsc::Receiver<Event>,
) {
    let root = TestRuntimeRoot::new("context-admit").unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let app =
        App::new_in_isolated_project(&project, HostPorts::isolated(Box::new(Clipboard))).unwrap();
    let mut host = WorkspaceHost::new(app);
    let events = host.start_context(HostMode::Standalone);
    host.context_open_storage(root.path().join("ctx")).unwrap();
    let identity = host
        .context
        .storage
        .as_ref()
        .unwrap()
        .identity("agent")
        .unwrap();
    host.note_plugin_frontend(true);
    (root, host, identity, events)
}
fn grant(host: &mut WorkspaceHost, identity: &Identity, permissions: BTreeSet<Permission>) {
    host.context
        .grants
        .insert(identity.fingerprint(), (identity.clone(), permissions));
}
fn read() -> BTreeSet<Permission> {
    [Permission::Read].into()
}
fn write() -> BTreeSet<Permission> {
    [Permission::Read, Permission::Write].into()
}
fn frame(host: &mut WorkspaceHost, connection: u64, value: Value) -> Reply {
    host.context_frame(
        connection,
        serde_json::to_string(&value).unwrap().as_bytes(),
    )
}
fn authenticate(host: &mut WorkspaceHost, identity: &Identity, id: u64, scopes: &[&str]) -> Reply {
    assert_eq!(
        frame(
            host,
            id,
            json!({"type":"authenticate","credential":identity.credential()})
        )
        .value["type"],
        "hello"
    );
    frame(
        host,
        id,
        json!({"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.5.0","name":"fixture","commands":[],"required_capabilities":scopes,"optional_capabilities":[],"required_features":[wire::FEATURE],"optional_features":[]}),
    )
}
fn request(
    host: &mut WorkspaceHost,
    connection: u64,
    id: &str,
    method: &str,
    params: Value,
) -> Value {
    frame(
        host,
        connection,
        json!({"type":"request","id":id,"method":method,"params":params}),
    )
    .value
}
fn key(host: &mut WorkspaceHost, code: KeyCode) {
    host.app
        .handle_input(InputEvent::Key(KeyStroke::plain(code)))
        .unwrap();
}
fn review_all(host: &mut WorkspaceHost) {
    // Bounded, so a review that can never complete fails instead of hanging.
    let lines = host.app.context_ui.surface.as_ref().unwrap().lines.len();
    for _ in 0..=lines {
        host.app.note_context_frame(80, 22, 1);
        host.app.note_context_presented(1);
        if host
            .app
            .context_ui
            .surface
            .as_ref()
            .unwrap()
            .fully_reviewed()
        {
            return;
        }
        key(host, KeyCode::PageDown);
    }
    panic!("review did not complete within {lines} lines");
}

#[tokio::test]
async fn disabled_access_has_no_listener_or_timer_and_requires_exact_grants() {
    let (_root, mut host, identity, _events) = fixture();
    assert!(!host.context_enabled());
    assert!(host.context_delay().is_none());
    assert!(
        frame(
            &mut host,
            1,
            json!({"type":"authenticate","credential":identity.credential()})
        )
        .close
    );
    grant(&mut host, &identity, read());
    let response = authenticate(&mut host, &identity, 2, &["buffer_edit"]);
    assert!(response.close);
    assert!(host.context.readers.is_empty());
    assert_eq!(
        authenticate(&mut host, &identity, 3, &["terminal_read"]).value["type"],
        "registered"
    );
    let denied = request(
        &mut host,
        3,
        "read",
        "buffer.list",
        json!({"offset":0,"limit":10}),
    );
    assert_eq!(denied["error"]["code"], "capability_denied");
    for method in [
        "command.execute",
        "buffer.save",
        "terminal.send",
        "terminal.input.approve",
        "terminal.input.propose",
    ] {
        let response = request(&mut host, 3, "bad", method, json!({}));
        assert_eq!(response["type"], "registration_error");
        if !method.starts_with("terminal.input") {
            authenticate(&mut host, &identity, 3, &["terminal_read"]);
        }
    }
}

#[tokio::test]
async fn disconnect_and_revocation_invalidate_reply_leases_and_all_owner_resources() {
    let (_root, mut host, identity, _events) = fixture();
    grant(&mut host, &identity, write());
    let lease = authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_write"],
    )
    .lease
    .unwrap();
    let terminal = host.app.terminals.insert_test_session(20, 3);
    let revision = host.app.terminals.get(terminal).unwrap().read_revision();
    host.context_request_enter(1, terminal, &format!("r:{revision}"))
        .unwrap();
    assert_eq!(host.context.enters.len(), 1);
    assert!(
        host.app
            .terminals
            .get(terminal)
            .unwrap()
            .agent_enter_pending()
    );
    host.context_revoke("agent");
    assert!(!lease.active());
    assert!(host.context.enters.is_empty());
    assert!(
        !host
            .app
            .terminals
            .get(terminal)
            .unwrap()
            .agent_enter_pending()
    );
    assert!(host.context.readers.is_empty());
    assert!(host.context.registration.is_none());
}

#[tokio::test]
async fn write_is_negotiated_through_its_feature_and_needs_the_write_permission() {
    let (_root, mut host, identity, _events) = fixture();
    grant(&mut host, &identity, read());
    let hello = frame(
        &mut host,
        1,
        json!({"type":"authenticate","credential":identity.credential()}),
    );
    assert!(
        hello.value["features"]
            .as_array()
            .unwrap()
            .contains(&json!(wire::TERMINAL_WRITE_FEATURE))
    );
    assert!(
        hello.value["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("terminal_write"))
    );
    // Read alone: an older adapter's proposal request and a write request
    // are both accepted at registration and neither is granted.
    let registered = frame(
        &mut host,
        1,
        json!({"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.5.0","name":"fixture","commands":[],"required_capabilities":[],"optional_capabilities":["terminal_read","terminal_propose","terminal_write","buffer_edit"],"required_features":[wire::FEATURE],"optional_features":[wire::TERMINAL_WRITE_FEATURE,"runyte.context.channel.nudge.v1"]}),
    );
    assert_eq!(
        registered.value["type"], "registered",
        "{}",
        registered.value
    );
    assert_eq!(registered.value["capabilities"], json!(["terminal_read"]));
    assert_eq!(
        registered.value["features"],
        json!([wire::FEATURE, wire::TERMINAL_WRITE_FEATURE])
    );
    let terminal = host.app.terminals.insert_test_session(20, 3);
    let list = request(
        &mut host,
        1,
        "list",
        "terminal.list",
        json!({"offset":0,"limit":10}),
    );
    let handle = list["result"]["terminals"][0]["terminal"].clone();
    for (method, params) in [
        (
            "terminal.input.write",
            json!({"terminal":handle,"text":"ls"}),
        ),
        (
            "terminal.input.enter",
            json!({"terminal":handle,"expected_revision":"r:0"}),
        ),
    ] {
        let denied = request(&mut host, 1, method, method, params);
        assert_eq!(denied["error"]["code"], "capability_denied", "{denied}");
    }
    assert!(host.context.enters.is_empty());
    assert_eq!(
        host.app.terminals.get(terminal).unwrap().input_generation(),
        0,
        "nothing reached the terminal"
    );
}

#[tokio::test]
async fn writes_and_enters_need_an_attached_person_and_a_live_terminal() {
    let (_root, mut host, identity, _events) = fixture();
    grant(&mut host, &identity, write());
    authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_write"],
    );
    let terminal = host.app.terminals.insert_test_session(20, 3);
    let list = request(
        &mut host,
        1,
        "list",
        "terminal.list",
        json!({"offset":0,"limit":10}),
    );
    let handle = list["result"]["terminals"][0]["terminal"].clone();
    let revision = list["result"]["terminals"][0]["revision"].clone();
    host.note_plugin_frontend(false);
    for (id, method, params) in [
        (
            "w",
            "terminal.input.write",
            json!({"terminal":handle,"text":"ls"}),
        ),
        (
            "e",
            "terminal.input.enter",
            json!({"terminal":handle,"expected_revision":revision}),
        ),
    ] {
        let refused = request(&mut host, 1, id, method, params);
        assert_eq!(refused["error"]["code"], "no_frontend", "{refused}");
    }
    host.note_plugin_frontend(true);
    // Text is checked before anything reaches the terminal, and a reader
    // that sends a control is closed.
    let refused = request(
        &mut host,
        1,
        "bad",
        "terminal.input.write",
        json!({"terminal":handle,"text":"ls\n"}),
    );
    assert_eq!(refused["type"], "registration_error", "{refused}");
    assert!(host.context.readers.is_empty());
    authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_write"],
    );
    let list = request(
        &mut host,
        1,
        "relist",
        "terminal.list",
        json!({"offset":0,"limit":10}),
    );
    let handle = list["result"]["terminals"][0]["terminal"].clone();
    // The fixture has no child, so a line cannot be queued at all.
    let refused = request(
        &mut host,
        1,
        "nochild",
        "terminal.input.write",
        json!({"terminal":handle,"text":"ls"}),
    );
    assert_eq!(refused["error"]["code"], "unavailable", "{refused}");
    host.app.terminals.close(terminal);
    let refused = request(
        &mut host,
        1,
        "closed",
        "terminal.input.enter",
        json!({"terminal":handle,"expected_revision":revision}),
    );
    assert!(refused.get("error").is_some(), "{refused}");
    assert!(host.context.enters.is_empty());
}

#[tokio::test]
async fn an_enter_is_dropped_by_output_typing_detachment_and_lost_write_access() {
    use crate::terminal::TerminalOutput;
    let (_root, mut host, identity, _events) = fixture();
    grant(&mut host, &identity, write());
    authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_write"],
    );
    let terminal = host.app.terminals.insert_test_session(20, 3);
    let list = request(
        &mut host,
        1,
        "list",
        "terminal.list",
        json!({"offset":0,"limit":10}),
    );
    let handle = list["result"]["terminals"][0]["terminal"].clone();
    let current = |host: &WorkspaceHost| {
        format!(
            "r:{}",
            host.app.terminals.get(terminal).unwrap().read_revision()
        )
    };
    let enter = |host: &mut WorkspaceHost, id: &str| -> String {
        let revision = current(host);
        let reply = request(
            host,
            1,
            id,
            "terminal.input.enter",
            json!({"terminal":handle,"expected_revision":revision}),
        );
        assert_eq!(reply["result"]["state"], "waiting", "{reply}");
        assert_eq!(reply["result"]["delay_ms"], wire::ENTER_DELAY_MILLIS);
        reply["result"]["enter"].as_str().unwrap().to_owned()
    };
    let status = |host: &mut WorkspaceHost, id: &str, enter: &str| {
        request(
            host,
            1,
            id,
            "terminal.input.enter.status",
            json!({"enter":enter}),
        )["result"]["state"]
            .clone()
    };

    // A revision the terminal no longer shows is refused at once.
    let stale = request(
        &mut host,
        1,
        "old",
        "terminal.input.enter",
        json!({"terminal":handle,"expected_revision":"r:99999"}),
    );
    assert_eq!(stale["error"]["code"], "stale");

    // Output that arrives during the delay, such as a dialog, drops it.
    let first = enter(&mut host, "e1");
    assert!(
        host.app
            .terminals
            .get(terminal)
            .unwrap()
            .agent_enter_pending()
    );
    let revision = current(&host);
    let busy = request(
        &mut host,
        1,
        "e1b",
        "terminal.input.enter",
        json!({"terminal":handle,"expected_revision":revision}),
    );
    assert_eq!(busy["error"]["code"], "conflict", "one per terminal");
    host.sync_context();
    assert_eq!(status(&mut host, "s1", &first), "waiting");
    host.app.terminals.apply(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"Allow this command? [y/n]".to_vec(),
    });
    host.sync_context();
    assert_eq!(status(&mut host, "s1b", &first), "stale");
    assert!(
        !host
            .app
            .terminals
            .get(terminal)
            .unwrap()
            .agent_enter_pending()
    );

    // So does the person typing there.
    let second = enter(&mut host, "e2");
    host.app
        .terminals
        .get_mut(terminal)
        .unwrap()
        .send_key(KeyStroke::plain(KeyCode::Char('n')));
    host.sync_context();
    assert_eq!(status(&mut host, "s2", &second), "stale");

    // Detaching the person's window cancels it.
    let third = enter(&mut host, "e3");
    host.note_plugin_frontend(false);
    host.sync_context();
    assert_eq!(status(&mut host, "s3", &third), "cancelled");
    host.note_plugin_frontend(true);

    // Nothing changed, and the delay is over: Enter is pressed if it can
    // be. This fixture has no child, so the write itself fails.
    let fourth = enter(&mut host, "e4");
    host.sync_context();
    assert_eq!(status(&mut host, "s4", &fourth), "waiting");
    host.context_expire_enter_delays();
    host.sync_context();
    assert_eq!(status(&mut host, "s4b", &fourth), "waiting", "settling");
    host.context_expire_enter_delays();
    host.sync_context();
    assert_eq!(status(&mut host, "s4c", &fourth), "stale");

    // Output that arrives during the settling pass still drops it.
    let settling = enter(&mut host, "e4d");
    host.context_expire_enter_delays();
    host.sync_context();
    host.app.terminals.apply(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"Proceed? [y/n]".to_vec(),
    });
    host.context_expire_enter_delays();
    host.sync_context();
    assert_eq!(status(&mut host, "s4d", &settling), "stale");

    // Another reader cannot ask about it.
    authenticate(
        &mut host,
        &identity,
        2,
        &["terminal_read", "terminal_write"],
    );
    let foreign = request(
        &mut host,
        2,
        "foreign",
        "terminal.input.enter.status",
        json!({"enter":fourth}),
    );
    assert_eq!(foreign["error"]["code"], "not_found");

    // Narrowing the grant to Read disconnects the reader and its Enters.
    enter(&mut host, "e5");
    assert_eq!(host.context.enters.len(), 6, "five finished, one waiting");
    host.app.context_ui.decision = Some(Decision::Grant {
        identity: "agent".into(),
        permissions: read(),
        remember: false,
    });
    host.sync_context();
    assert!(host.context.readers.is_empty());
    assert!(host.context.enters.is_empty());
    assert!(
        !host
            .app
            .terminals
            .get(terminal)
            .unwrap()
            .agent_enter_pending()
    );
}

#[tokio::test]
async fn queued_text_ends_with_its_reader_and_with_the_frontend_it_was_typed_for() {
    use crate::terminal::proposal::{Delivery, DeliveryState};
    let (_root, mut host, identity, _events) = fixture();
    grant(&mut host, &identity, write());
    authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_write"],
    );
    let queued = |host: &mut WorkspaceHost, reader: u64| {
        let delivery = Delivery::queued_for_test();
        let attachment = host.app.plugins.attachment_generation;
        host.context
            .readers
            .get_mut(&reader)
            .unwrap()
            .writes
            .push((delivery.clone(), attachment));
        delivery
    };

    // A reader closed for a malformed request takes its text with it.
    let first = queued(&mut host, 1);
    let refused = request(
        &mut host,
        1,
        "bad",
        "terminal.input.write",
        json!({"terminal":"t:1","text":"ls\n"}),
    );
    assert_eq!(refused["type"], "registration_error");
    assert_eq!(first.state(), DeliveryState::Cancelled);

    // Text typed for a frontend that has since detached never arrives.
    authenticate(
        &mut host,
        &identity,
        2,
        &["terminal_read", "terminal_write"],
    );
    let second = queued(&mut host, 2);
    host.sync_context();
    assert_eq!(second.state(), DeliveryState::Queued, "still attached");
    host.note_plugin_frontend(false);
    host.sync_context();
    assert_eq!(second.state(), DeliveryState::Cancelled);
}

#[tokio::test]
async fn an_enter_is_dropped_by_another_agents_text_and_cancelled_by_exit() {
    use crate::terminal::{TerminalOutput, proposal::Text};
    let (_root, mut host, identity, _events) = fixture();
    grant(&mut host, &identity, write());
    authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_write"],
    );
    let terminal = host.app.terminals.insert_test_session(20, 3);
    let request_enter = |host: &mut WorkspaceHost| {
        let revision = host.app.terminals.get(terminal).unwrap().read_revision();
        host.context_request_enter(1, terminal, &format!("r:{revision}"))
            .unwrap()["enter"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let enter = request_enter(&mut host);
    // Another agent's line changes the input even before it is written.
    let _ = host
        .app
        .terminals
        .get_mut(terminal)
        .unwrap()
        .queue_agent_text(&Text::new("other").unwrap());
    host.sync_context();
    assert_eq!(host.context_enter_state(&enter), Some("stale"));

    let enter = request_enter(&mut host);
    host.app.terminals.apply(TerminalOutput::Exited {
        id: terminal,
        code: Some(0),
    });
    host.sync_context();
    assert_eq!(host.context_enter_state(&enter), Some("cancelled"));
    assert!(
        !host
            .app
            .terminals
            .get(terminal)
            .unwrap()
            .agent_enter_pending()
    );
}

#[tokio::test]
async fn enters_are_bounded_per_host() {
    let (_root, mut host, identity, _events) = fixture();
    grant(&mut host, &identity, write());
    let readers = (1..=(wire::MAX_HOST_ENTERS / wire::MAX_ENTERS) as u64 + 1).collect::<Vec<_>>();
    for reader in &readers {
        authenticate(
            &mut host,
            &identity,
            *reader,
            &["terminal_read", "terminal_write"],
        );
    }
    let mut accepted = 0;
    for reader in &readers {
        for _ in 0..wire::MAX_ENTERS {
            let terminal = host.app.terminals.insert_test_session(20, 3);
            let revision = host.app.terminals.get(terminal).unwrap().read_revision();
            match host.context_request_enter(*reader, terminal, &format!("r:{revision}")) {
                Ok(_) => accepted += 1,
                Err(error) => assert_eq!(error.code, Code::LimitExceeded),
            }
        }
    }
    assert_eq!(accepted, wire::MAX_HOST_ENTERS);
}

#[tokio::test]
async fn enters_are_bounded_per_reader() {
    let (_root, mut host, identity, _events) = fixture();
    grant(&mut host, &identity, write());
    authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_write"],
    );
    let terminals = (0..=wire::MAX_ENTERS)
        .map(|_| host.app.terminals.insert_test_session(20, 3))
        .collect::<Vec<_>>();
    for (index, terminal) in terminals.iter().enumerate() {
        let revision = host.app.terminals.get(*terminal).unwrap().read_revision();
        let result = host.context_request_enter(1, *terminal, &format!("r:{revision}"));
        if index < wire::MAX_ENTERS {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err().code, Code::LimitExceeded);
        }
    }
}

#[tokio::test]
async fn changing_a_grant_reconnects_its_readers_without_moving_the_endpoint() {
    let (_root, mut host, identity, _events) = fixture();
    host.app.context_ui.decision = Some(Decision::Grant {
        identity: "agent".into(),
        permissions: write(),
        remember: false,
    });
    host.sync_context();
    let incarnation = host
        .context
        .registration
        .as_ref()
        .unwrap()
        .host_incarnation
        .clone();
    let registered = authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_write"],
    );
    assert_eq!(registered.value["type"], "registered");
    let lease = registered.lease.unwrap();
    // Write for a job, then back to Read: agents keep the same workspace.
    host.app.context_ui.decision = Some(Decision::Grant {
        identity: "agent".into(),
        permissions: read(),
        remember: false,
    });
    host.sync_context();
    assert!(!lease.active(), "the old negotiation is gone");
    assert_eq!(
        host.context.registration.as_ref().unwrap().host_incarnation,
        incarnation
    );
    assert!(host.context_enabled());
    assert!(
        authenticate(
            &mut host,
            &identity,
            2,
            &["terminal_read", "terminal_write"]
        )
        .close,
        "write is no longer granted"
    );
    assert_eq!(
        authenticate(&mut host, &identity, 3, &["terminal_read"]).value["type"],
        "registered"
    );
    // Applying nothing revokes, and then nobody is listened for.
    host.app.context_ui.decision = Some(Decision::Grant {
        identity: "agent".into(),
        permissions: BTreeSet::new(),
        remember: false,
    });
    host.sync_context();
    assert!(!host.context_enabled());
    assert!(host.context.registration.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn a_written_line_waits_for_a_guarded_enter_which_submits_it_after_the_delay() {
    let (_root, mut host, identity, _events) = fixture();
    grant(&mut host, &identity, write());
    authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_write", "agent_channel"],
    );
    let mut prompts = Prompts::new(&mut host);
    let terminal = prompts.open(&mut host, 1, "codex", true).await;
    let list = request(
        &mut host,
        1,
        "list",
        "terminal.list",
        json!({"offset":0,"limit":10}),
    );
    let row = &list["result"]["terminals"][0];
    assert_eq!(row["number"], 1);
    assert!(row["title"].as_str().unwrap().starts_with("[terminal #1]"));
    let handle = row["terminal"].clone();
    let written = request(
        &mut host,
        1,
        "write",
        "terminal.input.write",
        json!({"terminal":handle,"text":"review the parser"}),
    );
    assert_eq!(written["result"]["state"], "queued", "{written}");
    prompts.idle(&mut host, Duration::from_millis(100)).await;
    assert!(
        !prompts.text(terminal).contains("SUBMITTED"),
        "text alone is never submitted"
    );
    let read = request(
        &mut host,
        1,
        "read",
        "terminal.read",
        json!({"terminal":handle,"region":"screen","max_rows":10,"max_bytes":4096,"max_cells":4096}),
    );
    let enter = request(
        &mut host,
        1,
        "enter",
        "terminal.input.enter",
        json!({"terminal":handle,"expected_revision":read["result"]["revision"]}),
    );
    let enter = enter["result"]["enter"].as_str().unwrap().to_owned();
    assert!(
        host.app
            .terminals
            .get(terminal)
            .unwrap()
            .agent_enter_pending()
    );
    prompts.idle(&mut host, Duration::from_millis(100)).await;
    assert!(
        !prompts.text(terminal).contains("SUBMITTED"),
        "still waiting"
    );
    host.context_expire_enter_delays();
    prompts
        .wait(
            &mut host,
            terminal,
            "SUBMITTED:\x1b[200~review the parser\x1b[201~;",
        )
        .await;
    assert_eq!(host.context_enter_state(&enter), Some("sent"));
    assert!(
        !host
            .app
            .terminals
            .get(terminal)
            .unwrap()
            .agent_enter_pending()
    );
    host.app.terminals.close(terminal);
}

#[tokio::test]
async fn native_grant_overlay_controls_listener_and_remembered_permissions() {
    let (_root, mut host, identity, _events) = fixture();
    host.app.context_ui.requested_identity = Some("agent".into());
    host.sync_context();
    review_all(&mut host);
    key(&mut host, KeyCode::Char('1'));
    key(&mut host, KeyCode::Char('r'));
    key(&mut host, KeyCode::Tab);
    key(&mut host, KeyCode::Enter);
    host.sync_context();
    assert!(host.context_enabled());
    let endpoint = host.context.registration.as_ref().unwrap().endpoint.clone();
    assert!(endpoint.exists());
    assert_eq!(
        host.context
            .storage
            .as_ref()
            .unwrap()
            .permissions(&host.app.project_root, &identity)
            .unwrap(),
        read()
    );
    host.app.context_ui.requested_identity = Some("agent".into());
    host.sync_context();
    let surface = host.app.context_ui.surface.as_ref().unwrap();
    assert!(surface.grant.remember);
    assert_eq!(surface.grant.permissions, read());
    assert_eq!(
        host.app.context_ui.surface.as_ref().unwrap().title,
        "MCP permissions"
    );
    assert!(
        host.app
            .context_ui
            .surface
            .as_ref()
            .unwrap()
            .lines
            .iter()
            .any(|line| line.value.contains("remembered"))
    );
    key(&mut host, KeyCode::Char('x'));
    host.sync_context();
    assert!(!endpoint.exists());
    assert!(!host.context_enabled());
    assert!(
        host.context
            .storage
            .as_ref()
            .unwrap()
            .permissions(&host.app.project_root, &identity)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn ungranted_workspace_lists_paired_identities_without_enabling_access() {
    let root = TestRuntimeRoot::new("context-identities").unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let app =
        App::new_in_isolated_project(&project, HostPorts::isolated(Box::new(Clipboard))).unwrap();
    let mut host = WorkspaceHost::new(app);
    let store_root = root.path().join("ctx");
    let store = Storage::new(store_root.clone()).unwrap();
    let codex = store.identity("codex").unwrap();
    let _events = host.start_context_with_root(HostMode::Standalone, Some(store_root));
    host.note_plugin_frontend(true);

    // No-argument opening selects a known identity even without local grants.
    host.app.context_ui.requested_identity = Some(String::new());
    host.sync_context();
    assert_eq!(host.app.context_ui.identities, ["codex"]);
    let surface = host.app.context_ui.surface.as_ref().unwrap();
    assert_eq!(surface.grant.identity, "codex");
    assert!(!surface.grant.remember);
    assert!(surface.grant.permissions.is_empty(), "nothing preselected");
    key(&mut host, KeyCode::Escape);

    // Pairing after startup is visible the next time the overlay opens.
    store.identity("agent").unwrap();
    host.app.context_ui.requested_identity = Some("agent".into());
    host.sync_context();
    assert_eq!(host.app.context_ui.identities, ["agent", "codex"]);
    key(&mut host, KeyCode::Char('n'));
    host.sync_context();
    assert_eq!(
        host.app.context_ui.surface.as_ref().unwrap().grant.identity,
        "codex"
    );
    assert!(host.context.storage.is_none());
    assert!(host.context.grants.is_empty());
    assert!(!host.context_enabled());
    assert!(host.context_delay().is_none());
    assert!(store.permissions(&project, &codex).unwrap().is_empty());

    // Applying the first grant still opens a writable store and enables access.
    review_all(&mut host);
    key(&mut host, KeyCode::Char('1'));
    key(&mut host, KeyCode::Tab);
    key(&mut host, KeyCode::Enter);
    host.sync_context();
    assert!(host.context_enabled());
    assert!(host.context.grants.contains_key(&codex.fingerprint()));
}

#[test]
fn opening_permissions_does_not_create_missing_private_storage() {
    let root = TestRuntimeRoot::new("context-identities-absent").unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let app =
        App::new_in_isolated_project(&project, HostPorts::isolated(Box::new(Clipboard))).unwrap();
    let mut host = WorkspaceHost::new(app);
    let store_root = root.path().join("ctx");
    let _events = host.start_context_with_root(HostMode::Standalone, Some(store_root.clone()));
    host.note_plugin_frontend(true);
    host.app.context_ui.requested_identity = Some(String::new());
    host.sync_context();
    assert_eq!(host.app.context_ui.identities, ["agent"]);
    assert!(!store_root.exists());
    assert!(!host.context_enabled());
}

#[cfg(unix)]
#[tokio::test]
async fn configured_defaults_apply_where_nothing_was_decided_and_a_revocation_sticks() {
    let root = TestRuntimeRoot::new("context-defaults").unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let storage = root.path().join("ctx");
    let start = || {
        let mut app =
            App::new_in_isolated_project(&project, HostPorts::isolated(Box::new(Clipboard)))
                .unwrap();
        app.config.mcp.default_grants = [("agent".to_owned(), vec!["read".to_owned()])].into();
        let mut host = WorkspaceHost::new(app);
        let events = host.start_context_with_root(HostMode::Standalone, Some(storage.clone()));
        (host, events)
    };
    let permissions_of = |host: &WorkspaceHost| {
        host.context
            .grants
            .values()
            .find(|(identity, _)| identity.name() == "agent")
            .map(|(_, permissions)| permissions.clone())
    };

    // No storage yet: the default creates it and the identity, and listens.
    let (mut host, _events) = start();
    assert_eq!(permissions_of(&host), Some(read()));
    assert!(host.context.defaulted.contains("agent"));
    assert!(host.context_enabled());

    // Revoking keeps an empty record, so the next start stays off.
    host.app.context_ui.decision =
        Some(crate::app::context_access::Decision::Revoke("agent".into()));
    host.sync_context();
    drop(host);
    let (mut host, _events) = start();
    assert_eq!(permissions_of(&host), None);
    assert!(host.context.denied.contains("agent"));

    // A remembered grant of the person's own wins over the default.
    host.app.context_ui.decision = Some(crate::app::context_access::Decision::Grant {
        identity: "agent".into(),
        permissions: write(),
        remember: true,
    });
    host.sync_context();
    drop(host);
    let (mut host, _events) = start();
    assert_eq!(permissions_of(&host), Some(write()));
    assert!(host.context.remembered.contains("agent"));
    assert!(!host.context.defaulted.contains("agent"));

    // A grant applied for this session only leaves no record, so the
    // default is what the next start sees.
    host.app.context_ui.decision = Some(crate::app::context_access::Decision::Grant {
        identity: "agent".into(),
        permissions: write(),
        remember: false,
    });
    host.sync_context();
    drop(host);
    let (host, _events) = start();
    assert!(host.context.defaulted.contains("agent"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_revocation_made_without_defaults_still_holds_once_one_is_configured() {
    let root = TestRuntimeRoot::new("context-defaults-later").unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let storage = root.path().join("ctx");
    let start = |defaults: bool| {
        let mut app =
            App::new_in_isolated_project(&project, HostPorts::isolated(Box::new(Clipboard)))
                .unwrap();
        if defaults {
            app.config.mcp.default_grants =
                [("agent".to_owned(), vec!["terminal_read".to_owned()])].into();
        }
        let mut host = WorkspaceHost::new(app);
        let events = host.start_context_with_root(HostMode::Standalone, Some(storage.clone()));
        host.note_plugin_frontend(true);
        (host, events)
    };
    // A workspace that never had a grant has not opened the store; revoking
    // there must still be kept.
    let (mut host, _events) = start(false);
    assert!(host.context.storage.is_none());
    host.app.context_ui.decision =
        Some(crate::app::context_access::Decision::Revoke("agent".into()));
    host.sync_context();
    drop(host);

    let (mut host, _events) = start(true);
    assert!(host.context.grants.is_empty());
    host.app.context_ui.requested_identity = Some("agent".into());
    host.sync_context();
    let status = |host: &WorkspaceHost| {
        host.app
            .context_ui
            .surface
            .as_ref()
            .unwrap()
            .lines
            .iter()
            .find(|line| line.label == "Status")
            .unwrap()
            .value
            .clone()
    };
    assert!(
        status(&host).starts_with("Revoked here"),
        "{}",
        status(&host)
    );
    drop(host);
    // Without a default there is nothing for the revocation to hold off.
    let (mut host, _events) = start(false);
    host.app.context_ui.requested_identity = Some("agent".into());
    host.sync_context();
    assert!(
        status(&host).starts_with("Not granted"),
        "{}",
        status(&host)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_default_that_cannot_be_applied_leaves_remembered_grants_in_place() {
    let root = TestRuntimeRoot::new("context-defaults-full").unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let storage = root.path().join("ctx");
    {
        let store = Storage::new(storage.clone()).unwrap();
        let agent = store.identity("agent").unwrap();
        store.grant(&project, &agent, read()).unwrap();
        // Fill the store's identity capacity, so a new default cannot be made.
        for index in 1..32 {
            store.identity(&format!("other{index}")).unwrap();
        }
    }
    let mut app =
        App::new_in_isolated_project(&project, HostPorts::isolated(Box::new(Clipboard))).unwrap();
    app.config.mcp.default_grants =
        [("newcomer".to_owned(), vec!["terminal_read".to_owned()])].into();
    let mut host = WorkspaceHost::new(app);
    let _events = host.start_context_with_root(HostMode::Standalone, Some(storage));
    assert!(host.context.remembered.contains("agent"));
    assert!(!host.context.defaulted.contains("newcomer"));
    assert!(host.context_enabled());
    assert!(
        host.app
            .notifications()
            .entries()
            .iter()
            .any(|entry| entry.body.contains("newcomer"))
    );
}

#[cfg(unix)]
#[tokio::test]
async fn agents_share_the_channel_and_a_verified_terminal_counts_what_is_unread() {
    use crate::terminal::TerminalRequest;
    use crate::workspace::parent::{ParentContext, ParentLaunch};
    let (_root, mut host, identity, _events) = fixture();
    grant(&mut host, &identity, read());
    authenticate(&mut host, &identity, 1, &["agent_channel"]);
    authenticate(&mut host, &identity, 2, &["agent_channel"]);
    let launch = ParentLaunch::new(&host.app.project_root.join("endpoint.json")).unwrap();
    host.app.terminals.set_parent_launch(launch.clone());
    let terminal = host
        .app
        .terminals
        .open(
            TerminalRequest {
                program: "/bin/cat".into(),
                arguments: vec![],
                directory: host.app.project_root.clone(),
                label: "agent".to_owned(),
            },
            40,
            10,
        )
        .unwrap();
    let parent: ParentContext = serde_json::from_str(&launch.context(terminal)).unwrap();
    // The first agent runs in that terminal; the second only claims to.
    let process = host.app.terminals.get(terminal).unwrap().process_id();
    host.context.peers.insert(1, process);
    host.context.peers.insert(2, Some(std::process::id()));
    let location = json!({"directory":"/work","repository":null,"worktree":null,"branch":"main"});
    let claude = request(
        &mut host,
        1,
        "join",
        "channel.join",
        json!({"provider":"claude","task":"refactor the parser","location":location,
            "client":{"name":"claude-code","version":"2.0"},
            "terminal":{"terminal":terminal.get(),"capability":parent.capability}}),
    );
    assert!(
        claude["result"]["resume"].is_string(),
        "only the joiner gets it"
    );
    let claude = &claude["result"]["participant"];
    assert!(claude.get("resume").is_none());
    assert_eq!(claude["nick"], "claude-1");
    assert_eq!(claude["identity"], "agent");
    assert_eq!(claude["client"]["name"], "claude-code");
    assert_eq!(claude["terminal"]["number"], 1);
    let codex = request(
        &mut host,
        2,
        "join",
        "channel.join",
        json!({"provider":"codex","task":"review","location":location,
            "terminal":{"terminal":terminal.get(),"capability":parent.capability}}),
    );
    assert!(
        codex["result"]["participant"]["terminal"].is_null(),
        "a process outside the terminal's session cannot claim it"
    );

    request(
        &mut host,
        2,
        "public",
        "channel.post",
        json!({"text":"hello all","location":location}),
    );
    request(
        &mut host,
        2,
        "direct",
        "channel.post",
        json!({"text":"just you","to":"claude-1","location":location}),
    );
    assert_eq!(
        host.app.terminals.get(terminal).unwrap().channel_unread(),
        2
    );
    let read = request(
        &mut host,
        1,
        "read",
        "channel.read",
        json!({"since":0,"limit":200}),
    );
    let texts = read["result"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["kind"] == "message")
        .map(|m| {
            (
                m["from_nick"].clone(),
                m["to_nick"].clone(),
                m["text"].clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        texts,
        [
            (json!("codex-1"), Value::Null, json!("hello all")),
            (json!("codex-1"), json!("claude-1"), json!("just you"))
        ]
    );
    assert_eq!(
        host.app.terminals.get(terminal).unwrap().channel_unread(),
        0
    );
    let who = request(&mut host, 1, "who", "channel.who", json!({}));
    assert_eq!(who["result"]["participants"].as_array().unwrap().len(), 2);

    // The person's page shows everything, direct messages included.
    host.app.context_ui.channel_page_requested = true;
    host.app.plugins.presentation_dirty = false;
    host.sync_context();
    assert!(
        host.app.plugins.presentation_dirty,
        "the page asks to be drawn"
    );
    let page = host.app.active_buffer().text().to_string();
    assert!(
        page.contains("claude-1  claude · refactor the parser"),
        "{page}"
    );
    assert!(
        page.contains("codex-1 → claude-1 (direct): just you"),
        "{page}"
    );
    // A later message refreshes the open page.
    request(
        &mut host,
        2,
        "later",
        "channel.post",
        json!({"text":"done","location":location}),
    );
    assert!(
        host.app
            .active_buffer()
            .text()
            .to_string()
            .contains("codex-1: done")
    );

    // A disconnected participant stays known but no longer marks its terminal,
    // and the open page says so.
    host.handle_context_event(Event::Closed(1));
    assert_eq!(
        host.app.terminals.get(terminal).unwrap().channel_unread(),
        0
    );
    assert!(
        host.app
            .active_buffer()
            .text()
            .to_string()
            .contains("· disconnected"),
        "{}",
        host.app.active_buffer().text()
    );
    let who = request(&mut host, 2, "who", "channel.who", json!({}));
    assert!(
        who["result"]["participants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["nick"] == "claude-1" && p["connected"] == false)
    );
    host.app.terminals.close(terminal);
}

#[tokio::test]
async fn durable_dispatch_requires_negotiation_scope_store_and_live_lease() {
    let (root, mut host, identity, _events) = fixture();
    grant(&mut host, &identity, read());
    let hello = frame(
        &mut host,
        1,
        json!({"type":"authenticate","credential":identity.credential()}),
    );
    assert!(
        !hello.value["features"]
            .as_array()
            .unwrap()
            .contains(&json!(wire::DURABLE_FEATURE))
    );
    host.context_disconnect(1);
    let store = root.path().join("durable");
    host.context.durable_root = Some(store.clone());
    authenticate(&mut host, &identity, 1, &["agent_channel"]);
    async fn dispatch(host: &mut WorkspaceHost, id: &str, store: &str) -> Value {
        let bytes = serde_json::to_vec(&json!({"type":"request","id":id,"method":"channel.durable","params":{"credential":"a".repeat(64),"store":store,"operation":{"action":"register","provider":"test","task":"fixture"}}})).unwrap();
        let (send, receive) = tokio::sync::oneshot::channel();
        let mut send = Some(send);
        assert!(host.context_durable_frame(1, &bytes, &mut send));
        tokio::time::timeout(Duration::from_secs(3), receive)
            .await
            .unwrap()
            .unwrap()
            .value
    }
    let store_key = crate::hash::sha256_hex(&crate::native_path::encode_path(&store));
    assert!(
        dispatch(&mut host, "not-negotiated", &store_key)
            .await
            .get("error")
            .is_some()
    );
    assert!(!store.exists());
    host.context.readers.get_mut(&1).unwrap().durable = true;
    assert!(
        dispatch(&mut host, "wrong-store", &"b".repeat(64))
            .await
            .get("error")
            .is_some()
    );
    host.context.readers.get_mut(&1).unwrap().scopes.clear();
    assert!(
        dispatch(&mut host, "no-scope", &store_key)
            .await
            .get("error")
            .is_some()
    );
    host.context
        .readers
        .get_mut(&1)
        .unwrap()
        .scopes
        .insert(Scope::AgentChannel);
    // A real context registration supplies the immutable workspace route.
    host.context.registration = Some(Registration {
        root: host.app.project_root.clone(),
        workspace_id: crate::workspace::identity::workspace_id(&host.app.project_root),
        host_incarnation: "fixture".into(),
        endpoint: root.path().join("socket"),
        mode: HostMode::Standalone,
        pid: std::process::id(),
        environment: "fixture".into(),
    });
    let registered = dispatch(&mut host, "register", &store_key).await;
    assert!(
        registered["result"]["participant"].is_string(),
        "{registered}"
    );
    assert!(
        dispatch(&mut host, "register", &store_key)
            .await
            .get("error")
            .is_some()
    );
    assert_eq!(
        registered["result"]["participant"],
        dispatch(&mut host, "retry", &store_key).await["result"]["participant"]
    );
    // A saturated worker refuses admission without blocking the host.
    host.context.durable_worker = Some(durable::Worker::disconnected_for_test());
    assert!(
        dispatch(&mut host, "full", &store_key)
            .await
            .get("error")
            .is_some()
    );
    host.context.readers.get_mut(&1).unwrap().lease.cancel();
    assert!(
        dispatch(&mut host, "cancelled", &store_key)
            .await
            .get("error")
            .is_some()
    );
}

/// Agents at line-editor prompts in their own verified terminals, with each
/// terminal's output kept apart.
#[cfg(unix)]
struct Prompts {
    output: crate::terminal::TerminalEvents,
    bytes: BTreeMap<TerminalId, Vec<u8>>,
    launch: crate::workspace::parent::ParentLaunch,
}

#[cfg(unix)]
impl Prompts {
    fn new(host: &mut WorkspaceHost) -> Self {
        let launch = crate::workspace::parent::ParentLaunch::new(
            &host.app.project_root.join("endpoint.json"),
        )
        .unwrap();
        host.app.terminals.set_parent_launch(launch.clone());
        Self {
            output: host.take_terminal_events().unwrap(),
            bytes: BTreeMap::new(),
            launch,
        }
    }

    /// Opens a terminal whose program reports every submitted line, reading
    /// pastes when `paste` says so, and joins `connection` from inside it.
    async fn open(
        &mut self,
        host: &mut WorkspaceHost,
        connection: u64,
        provider: &str,
        paste: bool,
    ) -> TerminalId {
        let mode = if paste { "printf '\\033[?2004h'; " } else { "" };
        let terminal = host
            .app
            .terminals
            .open(
                crate::terminal::TerminalRequest {
                    program: "/bin/sh".into(),
                    arguments: vec![
                        "-c".into(),
                        format!("stty -echo; {mode}printf READY; while IFS= read -r line; do printf 'SUBMITTED:%s;' \"$line\"; done"),
                    ],
                    directory: host.app.project_root.clone(),
                    label: provider.into(),
                },
                80,
                10,
            )
            .unwrap();
        self.wait(host, terminal, "READY").await;
        let parent: crate::workspace::parent::ParentContext =
            serde_json::from_str(&self.launch.context(terminal)).unwrap();
        let process = host.app.terminals.get(terminal).unwrap().process_id();
        host.context.peers.insert(connection, process);
        let joined = request(
            host,
            connection,
            "join",
            "channel.join",
            json!({"provider":provider,"task":"work","location":{"directory":"/work"},
                "terminal":{"terminal":terminal.get(),"capability":parent.capability}}),
        );
        assert!(
            joined["result"]["participant"]["terminal"].is_object(),
            "{joined}"
        );
        terminal
    }

    fn text(&self, terminal: TerminalId) -> String {
        String::from_utf8_lossy(self.bytes.get(&terminal).map_or(&[][..], Vec::as_slice))
            .into_owned()
    }

    /// Moves the host on as its event loop would until `terminal` printed
    /// `needle`.
    async fn wait(&mut self, host: &mut WorkspaceHost, terminal: TerminalId, needle: &str) {
        let found = tokio::time::timeout(Duration::from_secs(5), async {
            while !self.text(terminal).contains(needle) {
                self.step(host).await;
            }
        })
        .await;
        assert!(found.is_ok(), "no {needle:?} in {:?}", self.text(terminal));
    }

    async fn step(&mut self, host: &mut WorkspaceHost) {
        host.sync_context();
        if let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(20), self.output.recv()).await
        {
            if let crate::terminal::TerminalOutput::Bytes { id, bytes } = &event {
                self.bytes.entry(*id).or_default().extend(bytes);
            }
            host.app.terminals.apply(event);
        }
    }

    /// Runs the host for `period`, for checking that nothing happens.
    async fn idle(&mut self, host: &mut WorkspaceHost, period: Duration) {
        let until = tokio::time::Instant::now() + period;
        while tokio::time::Instant::now() < until {
            self.step(host).await;
        }
    }
}
