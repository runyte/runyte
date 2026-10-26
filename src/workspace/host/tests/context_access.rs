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
fn grant(host: &mut WorkspaceHost, identity: &Identity, scopes: BTreeSet<Scope>) {
    host.context
        .grants
        .insert(identity.fingerprint(), (identity.clone(), scopes));
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
    grant(&mut host, &identity, [Scope::TerminalRead].into());
    let response = authenticate(&mut host, &identity, 2, &["editor_context_read"]);
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
    ] {
        let response = request(&mut host, 3, "bad", method, json!({}));
        assert_eq!(response["type"], "registration_error");
        if method != "terminal.input.approve" {
            authenticate(&mut host, &identity, 3, &["terminal_read"]);
        }
    }
}

#[tokio::test]
async fn disconnect_and_revocation_invalidate_reply_leases_and_all_owner_resources() {
    let (_root, mut host, identity, _events) = fixture();
    grant(
        &mut host,
        &identity,
        [Scope::TerminalRead, Scope::TerminalPropose].into(),
    );
    let lease = authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_propose"],
    )
    .lease
    .unwrap();
    let terminal = host.app.terminals.insert_test_session(20, 3);
    let proposal = host
        .context_propose(1, terminal, "literal", None, None)
        .unwrap();
    assert_eq!(host.context.proposals.len(), 1);
    host.context_revoke("agent");
    assert!(!lease.active());
    assert!(host.context.proposals.is_empty());
    assert!(host.context.readers.is_empty());
    assert!(host.context.registration.is_none());
    assert!(!proposal["proposal"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn default_reject_expiry_attachment_and_input_races_are_inert() {
    let (_root, mut host, identity, _events) = fixture();
    grant(
        &mut host,
        &identity,
        [Scope::TerminalRead, Scope::TerminalPropose].into(),
    );
    authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_propose"],
    );
    let terminal = host.app.terminals.insert_test_session(20, 3);
    for expected in ["rejected", "expired", "cancelled", "stale"] {
        host.note_plugin_frontend(true);
        let result = host
            .context_propose(1, terminal, "echo approved", None, None)
            .unwrap();
        let id = result["proposal"].as_str().unwrap().to_owned();
        host.sync_context();
        match expected {
            "rejected" => key(&mut host, KeyCode::Enter),
            "expired" => host.context.proposals.get_mut(&id).unwrap().deadline = Instant::now(),
            "cancelled" => host.note_plugin_frontend(false),
            _ => {
                review_all(&mut host);
                host.app.terminals.get_mut(terminal).unwrap().send_text("");
                key(&mut host, KeyCode::Tab);
                key(&mut host, KeyCode::Enter);
            }
        }
        host.sync_context();
        assert_eq!(host.context.proposals[&id].status, expected);
        assert!(host.context.proposals[&id].delivery.is_none());
        host.context.proposals.clear();
    }
    assert_eq!(
        host.context_propose(1, terminal, "bad\n", None, None)
            .unwrap_err()
            .code,
        Code::InvalidArgument
    );
    host.note_plugin_frontend(false);
    assert_eq!(
        host.context_propose(1, terminal, "ok", None, None)
            .unwrap_err()
            .code,
        Code::NoFrontend
    );
}

#[tokio::test]
async fn proposal_limits_dedup_and_cleanup_are_bounded() {
    let (_root, mut host, identity, _events) = fixture();
    grant(
        &mut host,
        &identity,
        [Scope::TerminalRead, Scope::TerminalPropose].into(),
    );
    authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_propose"],
    );
    host.app.terminals.insert_test_session(20, 3);
    let list = request(
        &mut host,
        1,
        "list",
        "terminal.list",
        json!({"offset":0,"limit":10}),
    );
    let handle = list["result"]["terminals"][0]["terminal"].clone();
    let first = request(
        &mut host,
        1,
        "proposal",
        "terminal.input.propose",
        json!({"terminal":handle,"text":"literal"}),
    );
    assert_eq!(first["result"]["state"], "pending");
    assert_eq!(
        request(
            &mut host,
            1,
            "proposal",
            "terminal.input.propose",
            json!({"terminal":handle,"text":"literal"})
        )["error"]["code"],
        "conflict"
    );
    for id in ["p2", "p3", "p4"] {
        assert_eq!(
            request(
                &mut host,
                1,
                id,
                "terminal.input.propose",
                json!({"terminal":handle,"text":"literal"})
            )["result"]["state"],
            "pending"
        );
    }
    assert_eq!(
        request(
            &mut host,
            1,
            "p5",
            "terminal.input.propose",
            json!({"terminal":handle,"text":"literal"})
        )["error"]["code"],
        "limit_exceeded"
    );
    for proposal in host.context.proposals.values_mut() {
        proposal.deadline = Instant::now() - Duration::from_secs(121);
        proposal.status = "expired";
    }
    host.sync_context();
    assert!(host.context.proposals.is_empty());
    assert!(host.context_delay().is_none());
}

#[tokio::test]
async fn approval_requires_every_page_to_be_painted_and_consumes_enter() {
    let (_root, mut host, identity, _events) = fixture();
    grant(
        &mut host,
        &identity,
        [Scope::TerminalRead, Scope::TerminalPropose].into(),
    );
    authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_propose"],
    );
    let terminal = host.app.terminals.insert_test_session(20, 3);
    let id = host
        .context_propose(
            1,
            terminal,
            &"a".repeat(1024),
            Some("untrusted reason".into()),
            None,
        )
        .unwrap()["proposal"]
        .as_str()
        .unwrap()
        .to_owned();
    host.sync_context();
    let generation = host.app.terminals.get(terminal).unwrap().input_generation();
    key(&mut host, KeyCode::Tab);
    key(&mut host, KeyCode::Enter);
    assert!(host.app.context_overlay_active());
    assert!(host.app.context_ui.decision.is_none());
    // Skipping pages without a frame must not make them reviewed.
    for _ in 0..20 {
        key(&mut host, KeyCode::Char('j'));
    }
    host.app.note_context_frame(80, 22, 1);
    host.app.note_context_presented(1);
    key(&mut host, KeyCode::Enter);
    assert!(host.app.context_ui.decision.is_none());
    for _ in 0..20 {
        key(&mut host, KeyCode::Char('k'));
    }
    review_all(&mut host);
    key(&mut host, KeyCode::Enter);
    assert!(!host.app.context_overlay_active());
    assert!(matches!(
        host.app.context_ui.decision,
        Some(Decision::Proposal { accepted: true, .. })
    ));
    assert_eq!(
        host.app.terminals.get(terminal).unwrap().input_generation(),
        generation
    );
    host.sync_context();
    assert_eq!(host.context.proposals[&id].status, "cancelled"); // inert fixture has no PTY
    assert!(crate::app::context_access::visible("a \u{202e}\u{200b}").contains("\\u{202e}"));
}

#[tokio::test]
async fn native_grant_overlay_controls_listener_and_remembered_scope() {
    let (_root, mut host, identity, _events) = fixture();
    host.app.context_ui.requested_identity = Some("agent".into());
    host.sync_context();
    review_all(&mut host);
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
            .scopes(&host.app.project_root, &identity)
            .unwrap(),
        [Scope::TerminalRead, Scope::EditorContextRead].into()
    );
    host.app.context_ui.requested_identity = Some("agent".into());
    host.sync_context();
    assert!(matches!(
        host.app.context_ui.surface.as_ref().unwrap().kind,
        Kind::Grant { remember: true, .. }
    ));
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
            .scopes(&host.app.project_root, &identity)
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
    assert!(matches!(
        &host.app.context_ui.surface.as_ref().unwrap().kind,
        Kind::Grant { identity, remember: false, .. } if identity == "codex"
    ));
    key(&mut host, KeyCode::Escape);

    // Pairing after startup is visible the next time the overlay opens.
    store.identity("agent").unwrap();
    host.app.context_ui.requested_identity = Some("agent".into());
    host.sync_context();
    assert_eq!(host.app.context_ui.identities, ["agent", "codex"]);
    key(&mut host, KeyCode::Char('n'));
    host.sync_context();
    assert!(matches!(
        &host.app.context_ui.surface.as_ref().unwrap().kind,
        Kind::Grant { identity, .. } if identity == "codex"
    ));
    assert!(host.context.storage.is_none());
    assert!(host.context.grants.is_empty());
    assert!(!host.context_enabled());
    assert!(host.context_delay().is_none());
    assert!(store.scopes(&project, &codex).unwrap().is_empty());

    // Applying the first grant still opens a writable store and enables access.
    review_all(&mut host);
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

#[tokio::test]
async fn real_pty_overlay_enter_only_inserts_and_separate_native_enter_submits() {
    use crate::terminal::{TerminalOutput, TerminalRequest, proposal::DeliveryState};
    let (_root, mut host, identity, _events) = fixture();
    grant(
        &mut host,
        &identity,
        [Scope::TerminalRead, Scope::TerminalPropose].into(),
    );
    authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_propose"],
    );
    let terminal = host.app.terminals.open(TerminalRequest {
        program: "/bin/sh".into(),
        arguments: vec!["-c".into(), "stty -echo; printf READY; IFS= read -r line; printf 'SUBMITTED:%s' \"$line\"; IFS= read -r hold".into()],
        directory: host.app.project_root.clone(), label: "approval fixture".into(),
    }, 40, 10).unwrap();
    let mut output = host.take_terminal_events().unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !bytes.windows(5).any(|part| part == b"READY") {
            let event = output.recv().await.unwrap();
            if let TerminalOutput::Bytes { bytes: part, .. } = &event {
                bytes.extend(part);
            }
            host.app.terminals.apply(event);
        }
    })
    .await
    .unwrap();
    let id = host
        .context_propose(1, terminal, "echo literal", None, None)
        .unwrap()["proposal"]
        .as_str()
        .unwrap()
        .to_owned();
    host.sync_context();
    assert!(host.context.proposals[&id].delivery.is_none());
    review_all(&mut host);
    key(&mut host, KeyCode::Tab);
    key(&mut host, KeyCode::Enter);
    host.sync_context();
    let delivery = host.context.proposals[&id]
        .delivery
        .as_ref()
        .unwrap()
        .clone();
    tokio::time::timeout(Duration::from_secs(5), async {
        while delivery.state() != DeliveryState::Delivered {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    // The child is blocked in read, despite the Enter consumed by the overlay.
    assert!(
        tokio::time::timeout(Duration::from_millis(50), output.recv())
            .await
            .is_err()
    );
    assert!(
        host.app
            .terminals
            .get_mut(terminal)
            .unwrap()
            .send_key(KeyStroke::plain(KeyCode::Enter))
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !String::from_utf8_lossy(&bytes).contains("SUBMITTED:echo literal") {
            let event = output.recv().await.unwrap();
            if let TerminalOutput::Bytes { bytes: part, .. } = &event {
                bytes.extend(part);
            }
            host.app.terminals.apply(event);
        }
    })
    .await
    .unwrap();
    host.sync_context();
    assert_eq!(host.context.proposals[&id].status, "delivered");
}

#[tokio::test]
async fn prepared_but_unpainted_lines_cannot_be_counted_or_approved() {
    let (_root, mut host, identity, _events) = fixture();
    grant(
        &mut host,
        &identity,
        [Scope::TerminalRead, Scope::TerminalPropose].into(),
    );
    authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_propose"],
    );
    let terminal = host.app.terminals.insert_test_session(20, 3);
    host.context_propose(1, terminal, &"x".repeat(1024), None, None)
        .unwrap();
    host.sync_context();
    fn surface(host: &WorkspaceHost) -> &crate::app::context_access::Surface {
        host.app.context_ui.surface.as_ref().unwrap()
    }
    host.app.note_context_frame(80, 22, 10);
    key(&mut host, KeyCode::PageDown);
    assert!(surface(&host).top > 0);
    // Frame 10 was prepared for the first line, which is no longer in view.
    host.app.note_context_presented(10);
    assert!(surface(&host).seen.iter().all(|seen| !seen));
    host.app.note_context_frame(80, 22, 11); // dropped/backpressured frame
    host.app.note_context_presented(10); // frontend still displays line one
    key(&mut host, KeyCode::End);
    key(&mut host, KeyCode::Tab);
    key(&mut host, KeyCode::Enter);
    assert!(surface(&host).seen.iter().all(|seen| !seen));
    assert_eq!(surface(&host).presented_top, None);
    assert!(host.app.context_ui.decision.is_none());
}

#[tokio::test]
async fn direct_insertion_grant_skips_review_but_never_sends_enter() {
    use crate::terminal::{TerminalOutput, TerminalRequest, proposal::DeliveryState};
    let (_root, mut host, identity, _events) = fixture();
    grant(
        &mut host,
        &identity,
        [
            Scope::TerminalRead,
            Scope::TerminalPropose,
            Scope::TerminalInsert,
        ]
        .into(),
    );
    // The reader negotiates nothing new: direct insertion is the grant's.
    let registered = authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_propose"],
    );
    assert_eq!(
        registered.value["capabilities"],
        json!(["terminal_read", "terminal_propose"])
    );
    let terminal = host.app.terminals.open(TerminalRequest {
        program: "/bin/sh".into(),
        // A line editor asks for bracketed paste; this one stands in for it.
        arguments: vec!["-c".into(), "stty -echo; printf '\\033[?2004h'; printf READY; IFS= read -r line; printf 'SUBMITTED:%s' \"$line\"; IFS= read -r hold".into()],
        directory: host.app.project_root.clone(), label: "direct fixture".into(),
    }, 40, 10).unwrap();
    let mut output = host.take_terminal_events().unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !bytes.windows(5).any(|part| part == b"READY") {
            let event = output.recv().await.unwrap();
            if let TerminalOutput::Bytes { bytes: part, .. } = &event {
                bytes.extend(part);
            }
            host.app.terminals.apply(event);
        }
    })
    .await
    .unwrap();
    let list = request(
        &mut host,
        1,
        "list",
        "terminal.list",
        json!({"offset":0,"limit":10}),
    );
    let handle = list["result"]["terminals"][0]["terminal"].clone();
    let proposed = request(
        &mut host,
        1,
        "propose",
        "terminal.input.propose",
        json!({"terminal":handle,"text":"echo direct"}),
    );
    assert_eq!(proposed["result"]["state"], "queued");
    let id = proposed["result"]["proposal"].as_str().unwrap().to_owned();
    host.sync_context();
    assert!(host.app.context_ui.surface.is_none(), "no review overlay");
    let notice = host.app.notifications().entries().last().unwrap();
    assert_eq!(notice.title, "Terminal text inserted");
    assert!(
        notice
            .body
            .contains("inserted 11 characters into terminal #1 without review"),
        "{}",
        notice.body
    );
    let delivery = host.context.proposals[&id]
        .delivery
        .as_ref()
        .unwrap()
        .clone();
    tokio::time::timeout(Duration::from_secs(5), async {
        while delivery.state() != DeliveryState::Delivered {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    // Still blocked in read: the text arrived without Enter.
    assert!(
        tokio::time::timeout(Duration::from_millis(50), output.recv())
            .await
            .is_err()
    );
    assert!(
        host.app
            .terminals
            .get_mut(terminal)
            .unwrap()
            .send_key(KeyStroke::plain(KeyCode::Enter))
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        // The text arrives framed as a paste, which is what makes it inert
        // until the person submits it.
        while !String::from_utf8_lossy(&bytes).contains("SUBMITTED:\x1b[200~echo direct\x1b[201~") {
            let event = output.recv().await.unwrap();
            if let TerminalOutput::Bytes { bytes: part, .. } = &event {
                bytes.extend(part);
            }
            host.app.terminals.apply(event);
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn direct_insertion_falls_back_to_review_and_ends_with_its_grant() {
    let (_root, mut host, identity, _events) = fixture();
    grant(
        &mut host,
        &identity,
        [
            Scope::TerminalRead,
            Scope::TerminalPropose,
            Scope::TerminalInsert,
        ]
        .into(),
    );
    authenticate(
        &mut host,
        &identity,
        1,
        &["terminal_read", "terminal_propose"],
    );
    // A program that has not asked for bracketed paste would read the text
    // as keystrokes, so the person reviews it even under this grant.
    let terminal = host.app.terminals.insert_test_session(20, 3);
    assert!(
        !host
            .app
            .terminals
            .get(terminal)
            .unwrap()
            .proposal_input_signature()
            .modes
            .bracketed_paste
    );
    let list = request(
        &mut host,
        1,
        "list",
        "terminal.list",
        json!({"offset":0,"limit":10}),
    );
    let handle = list["result"]["terminals"][0]["terminal"].clone();
    let notices = host.app.notifications().entries().len();
    let proposed = request(
        &mut host,
        1,
        "propose",
        "terminal.input.propose",
        json!({"terminal":handle,"text":"y"}),
    );
    assert_eq!(proposed["result"]["state"], "pending");
    assert_eq!(host.app.notifications().entries().len(), notices);
    host.sync_context();
    assert!(host.app.context_ui.surface.is_some(), "the review opens");
    key(&mut host, KeyCode::Escape);
    host.sync_context();

    // Applying the grant again without `5` ends the connection that held it,
    // so the authority cannot outlive the person's choice.
    host.app.context_ui.decision = Some(crate::app::context_access::Decision::Grant {
        identity: "agent".into(),
        scopes: [Scope::TerminalRead, Scope::TerminalPropose].into(),
        remember: false,
    });
    host.sync_context();
    let stale = request(
        &mut host,
        1,
        "again",
        "terminal.list",
        json!({"offset":0,"limit":10}),
    );
    assert!(stale.get("result").is_none(), "{stale}");
    let registered = authenticate(
        &mut host,
        &identity,
        2,
        &["terminal_read", "terminal_propose"],
    );
    assert_eq!(registered.value["type"], "registered");
    assert!(
        !host.context.readers[&2]
            .granted
            .contains(&Scope::TerminalInsert)
    );
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
        app.config.mcp.default_grants = [(
            "agent".to_owned(),
            vec!["terminal_read".to_owned(), "editor_context_read".to_owned()],
        )]
        .into();
        let mut host = WorkspaceHost::new(app);
        let events = host.start_context_with_root(HostMode::Standalone, Some(storage.clone()));
        (host, events)
    };
    let scopes_of = |host: &WorkspaceHost| {
        host.context
            .grants
            .values()
            .find(|(identity, _)| identity.name() == "agent")
            .map(|(_, scopes)| scopes.clone())
    };

    // No storage yet: the default creates it and the identity, and listens.
    let (mut host, _events) = start();
    assert_eq!(
        scopes_of(&host),
        Some([Scope::TerminalRead, Scope::EditorContextRead].into())
    );
    assert!(host.context.defaulted.contains("agent"));
    assert!(host.context_enabled());

    // Revoking keeps an empty record, so the next start stays off.
    host.app.context_ui.decision =
        Some(crate::app::context_access::Decision::Revoke("agent".into()));
    host.sync_context();
    drop(host);
    let (mut host, _events) = start();
    assert_eq!(scopes_of(&host), None);
    assert!(host.context.denied.contains("agent"));

    // A remembered grant of the person's own wins over the default.
    host.app.context_ui.decision = Some(crate::app::context_access::Decision::Grant {
        identity: "agent".into(),
        scopes: [Scope::TerminalRead].into(),
        remember: true,
    });
    host.sync_context();
    drop(host);
    let (mut host, _events) = start();
    assert_eq!(scopes_of(&host), Some([Scope::TerminalRead].into()));
    assert!(host.context.remembered.contains("agent"));
    assert!(!host.context.defaulted.contains("agent"));

    // A grant applied for this session only leaves no record, so the
    // default is what the next start sees.
    host.app.context_ui.decision = Some(crate::app::context_access::Decision::Grant {
        identity: "agent".into(),
        scopes: [Scope::TerminalRead].into(),
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
        store
            .grant(&project, &agent, [Scope::TerminalRead].into())
            .unwrap();
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
    grant(&mut host, &identity, [Scope::AgentChannel].into());
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
    grant(&mut host, &identity, [Scope::AgentChannel].into());
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

/// Reads terminal output until it holds `needle`, moving the host on as the
/// event loop would so held and pasting nudges advance.
#[cfg(unix)]
async fn pump_until(
    host: &mut WorkspaceHost,
    output: &mut crate::terminal::TerminalEvents,
    bytes: &mut Vec<u8>,
    needle: &str,
) {
    use crate::terminal::TerminalOutput;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !String::from_utf8_lossy(bytes).contains(needle) {
            host.sync_context();
            if let Ok(Some(event)) =
                tokio::time::timeout(Duration::from_millis(20), output.recv()).await
            {
                if let TerminalOutput::Bytes { bytes: part, .. } = &event {
                    bytes.extend(part);
                }
                host.app.terminals.apply(event);
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no {needle:?} in {:?}", String::from_utf8_lossy(bytes)));
}

#[cfg(unix)]
#[tokio::test]
async fn a_requested_nudge_types_one_fixed_line_and_enter_once_the_person_stops_typing() {
    use crate::terminal::TerminalRequest;
    use crate::workspace::parent::{ParentContext, ParentLaunch};
    let (_root, mut host, identity, _events) = fixture();
    let plain = host
        .context
        .storage
        .as_ref()
        .unwrap()
        .identity("plain")
        .unwrap();
    grant(
        &mut host,
        &identity,
        [Scope::AgentChannel, Scope::ChannelNudge].into(),
    );
    grant(&mut host, &plain, [Scope::AgentChannel].into());
    let register = |host: &mut WorkspaceHost, identity: &Identity, id: u64, nudge: bool| {
        frame(
            host,
            id,
            json!({"type":"authenticate","credential":identity.credential()}),
        );
        let features = if nudge {
            json!([wire::CHANNEL_FEATURE, wire::NUDGE_FEATURE])
        } else {
            json!([wire::CHANNEL_FEATURE])
        };
        let registered = frame(
            host,
            id,
            json!({"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.5.0","name":"fixture","commands":[],"required_capabilities":["agent_channel"],"optional_capabilities":[],"required_features":[wire::FEATURE],"optional_features":features}),
        );
        assert_eq!(registered.value["type"], "registered");
    };
    // The recipient negotiates nothing for nudges: accepting them is its grant.
    register(&mut host, &identity, 1, false);
    register(&mut host, &identity, 2, true);
    register(&mut host, &plain, 3, false);

    let launch = ParentLaunch::new(&host.app.project_root.join("endpoint.json")).unwrap();
    host.app.terminals.set_parent_launch(launch.clone());
    let terminal = host.app.terminals.open(TerminalRequest {
        program: "/bin/sh".into(),
        // A line editor that reads pastes and reports every submitted line.
        arguments: vec!["-c".into(), "stty -echo; printf '\\033[?2004h'; printf READY; while IFS= read -r line; do printf 'SUBMITTED:%s;' \"$line\"; done".into()],
        directory: host.app.project_root.clone(), label: "nudge fixture".into(),
    }, 80, 10).unwrap();
    let mut output = host.take_terminal_events().unwrap();
    let mut bytes = Vec::new();
    pump_until(&mut host, &mut output, &mut bytes, "READY").await;
    let parent: ParentContext = serde_json::from_str(&launch.context(terminal)).unwrap();
    let process = host.app.terminals.get(terminal).unwrap().process_id();
    let location = json!({"directory":"/work","repository":null,"worktree":null,"branch":"main"});
    for (connection, provider) in [(1, "claude"), (2, "codex"), (3, "gemini")] {
        host.context.peers.insert(connection, process);
        let joined = request(
            &mut host,
            connection,
            "join",
            "channel.join",
            json!({"provider":provider,"task":"work","location":location,
                "terminal":{"terminal":terminal.get(),"capability":parent.capability}}),
        );
        assert_eq!(joined["result"]["participant"]["terminal"]["number"], 1);
    }
    let mut serial = 0;
    let mut post = |host: &mut WorkspaceHost, connection: u64, params: Value| {
        serial += 1;
        let mut params = params;
        params["location"] = location.clone();
        request(
            host,
            connection,
            &format!("post-{serial}"),
            "channel.post",
            params,
        )
    };
    let states = |reply: &Value| {
        reply["result"]["nudges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["state"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let submitted = format!(
        "SUBMITTED:\x1b[200~{}\x1b[201~;",
        super::super::context_channel::NUDGE_TEXT
    );

    // Nothing nudges unless the sender asks.
    let quiet = post(&mut host, 2, json!({"text":"fyi"}));
    assert!(quiet["result"].get("nudges").is_none());
    assert!(host.context.nudges.is_empty());

    // Nobody is typing: the line is pasted and Enter follows.
    let sent = post(
        &mut host,
        2,
        json!({"text":"review ready","nudge":["claude-1"]}),
    );
    assert_eq!(states(&sent), ["sent"]);
    assert_eq!(sent["result"]["nudges"][0]["nick"], "claude-1");
    pump_until(&mut host, &mut output, &mut bytes, &submitted).await;
    let notice = host.app.notifications().entries().last().unwrap();
    assert_eq!(notice.title, "Channel nudge sent");
    assert!(
        notice
            .body
            .contains("codex-1 nudged claude-1 in terminal #1"),
        "{}",
        notice.body
    );

    // Until the nudged agent reads, its terminal is not nudged again, for
    // anyone in it.
    let again = post(&mut host, 2, json!({"text":"ping","nudge":["claude-1"]}));
    assert_eq!(states(&again), ["already_pending"]);
    let read = request(
        &mut host,
        1,
        "read",
        "channel.read",
        json!({"since":0,"limit":200}),
    );
    assert!(read["result"]["messages"].is_array());
    assert!(host.context.nudges.is_empty(), "a read clears it");

    // An identity whose grant does not accept nudges, and a sender that did
    // not negotiate them, are refused without the reason reaching the sender.
    let ungranted = post(&mut host, 2, json!({"text":"you too","nudge":["gemini-1"]}));
    assert_eq!(states(&ungranted), ["unavailable"]);
    let unnegotiated = post(&mut host, 1, json!({"text":"hi","nudge":["codex-1"]}));
    assert_eq!(unnegotiated["error"]["code"], "unsupported");
    // A refused name stores no message.
    let head = host.context.channel.head();
    let refused = post(
        &mut host,
        2,
        json!({"text":"x","to":"claude-1","nudge":["gemini-1"]}),
    );
    assert_eq!(refused["error"]["code"], "invalid_argument");
    assert_eq!(host.context.channel.head(), head);

    // The person is typing: the nudge holds while the line has a draft and
    // for the quiet interval after their last key.
    {
        let session = host.app.terminals.get_mut(terminal).unwrap();
        assert!(session.send_key(KeyStroke::plain(KeyCode::Char('x'))));
        assert!(session.has_draft());
    }
    let held = post(
        &mut host,
        2,
        json!({"text":"when you can","to":"claude-1","nudge":["claude-1"]}),
    );
    assert_eq!(states(&held), ["held"]);
    host.app
        .terminals
        .get_mut(terminal)
        .unwrap()
        .backdate_typing(super::super::context_channel::NUDGE_QUIET * 2);
    host.sync_context();
    assert!(
        matches!(host.context_nudge_delay(), Some(delay) if delay <= Duration::from_secs(1)),
        "a held nudge keeps the host looking"
    );
    let session = host.app.terminals.get_mut(terminal).unwrap();
    assert!(session.send_key(KeyStroke::plain(KeyCode::Enter)));
    assert!(!session.has_draft());
    pump_until(&mut host, &mut output, &mut bytes, "SUBMITTED:x;").await;
    for _ in 0..20 {
        host.sync_context();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        String::from_utf8_lossy(&bytes).matches(&submitted).count(),
        1,
        "still within the quiet interval"
    );
    host.app
        .terminals
        .get_mut(terminal)
        .unwrap()
        .backdate_typing(super::super::context_channel::NUDGE_QUIET * 2);
    pump_until(
        &mut host,
        &mut output,
        &mut bytes,
        &format!("{submitted}SUBMITTED:x;{submitted}"),
    )
    .await;

    // A connection that has gone takes its nudge with it.
    request(
        &mut host,
        1,
        "read2",
        "channel.read",
        json!({"since":0,"limit":200}),
    );
    host.context_disconnect(1);
    let gone = post(
        &mut host,
        2,
        json!({"text":"still there?","nudge":["claude-1"]}),
    );
    assert_eq!(states(&gone), ["unavailable"]);
}
