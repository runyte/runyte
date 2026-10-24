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
    key(&mut host, KeyCode::Char('j'));
    assert_eq!(surface(&host).top, 1);
    // Frame 10 was prepared for the first line, which is no longer in view.
    host.app.note_context_presented(10);
    assert!(surface(&host).seen.iter().all(|seen| !seen));
    host.app.note_context_frame(80, 22, 11); // dropped/backpressured frame
    host.app.note_context_presented(10); // frontend still displays line one
    key(&mut host, KeyCode::Char('G'));
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
