// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::{App, HostPorts},
    clipboard::SystemClipboard,
    terminal::{TerminalOutput, proposal::DeliveryState},
    test_support::TestRuntimeRoot,
    workspace::context::discovery,
};
use std::{future::Future, path::Path};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

struct Clipboard;
impl SystemClipboard for Clipboard {
    fn read(&mut self) -> anyhow::Result<String> {
        anyhow::bail!("inert fixture")
    }
    fn write(&mut self, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

fn host(
    root: &Path,
    name: &str,
    mode: HostMode,
) -> (WorkspaceHost, Identity, mpsc::Receiver<Event>) {
    let project = root.join(name);
    std::fs::create_dir(&project).unwrap();
    let app =
        App::new_in_isolated_project(&project, HostPorts::isolated(Box::new(Clipboard))).unwrap();
    let mut host = WorkspaceHost::new(app);
    let events = host.start_context(mode);
    host.context_open_storage(root.join("ctx")).unwrap();
    let identity = host
        .context
        .storage
        .as_ref()
        .unwrap()
        .identity("agent")
        .unwrap();
    host.context.grants.insert(
        identity.fingerprint(),
        (
            identity.clone(),
            [
                Scope::TerminalRead,
                Scope::EditorContextRead,
                Scope::TerminalPropose,
            ]
            .into(),
        ),
    );
    host.context_enable().unwrap();
    (host, identity, events)
}

async fn pump<T>(
    host: &mut WorkspaceHost,
    events: &mut mpsc::Receiver<Event>,
    future: impl Future<Output = T>,
) -> T {
    tokio::pin!(future);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                output = &mut future => return output,
                event = events.recv() => host.handle_context_event(event.unwrap()),
            }
        }
    })
    .await
    .expect("context transport deadline")
}

async fn exchange(
    host: &mut WorkspaceHost,
    events: &mut mpsc::Receiver<Event>,
    socket: &mut BufReader<UnixStream>,
    value: Value,
) -> Value {
    pump(host, events, async {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        socket.get_mut().write_all(&bytes).await.unwrap();
        let mut reply = String::new();
        assert!(socket.read_line(&mut reply).await.unwrap() > 0);
        serde_json::from_str(&reply).unwrap()
    })
    .await
}

async fn authenticate(
    host: &mut WorkspaceHost,
    events: &mut mpsc::Receiver<Event>,
    identity: &Identity,
) -> BufReader<UnixStream> {
    let endpoint = &host.context.registration.as_ref().unwrap().endpoint;
    let mut socket = BufReader::new(UnixStream::connect(endpoint).await.unwrap());
    let hello = exchange(
        host,
        events,
        &mut socket,
        json!({"type":"authenticate","credential":identity.credential()}),
    )
    .await;
    assert_eq!(hello["type"], "hello");
    let registered = exchange(host, events, &mut socket, json!({"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.4.0","name":"transport-fixture","commands":[],"required_capabilities":["terminal_read","editor_context_read","terminal_propose"],"optional_capabilities":[],"required_features":[wire::FEATURE],"optional_features":[]})).await;
    assert_eq!(registered["type"], "registered");
    socket
}

fn request(id: &str, method: &str, params: Value) -> Value {
    json!({"type":"request","id":id,"method":method,"params":params})
}

#[tokio::test]
async fn post_authentication_denial_is_delivered_before_connection_closes() {
    let root = TestRuntimeRoot::new("ctx-denial").unwrap();
    let (mut host, identity, mut events) = host(root.path(), "project", HostMode::Standalone);
    let endpoint = host.context.registration.as_ref().unwrap().endpoint.clone();
    let mut socket = BufReader::new(UnixStream::connect(endpoint).await.unwrap());
    let hello = exchange(
        &mut host,
        &mut events,
        &mut socket,
        json!({"type":"authenticate","credential":identity.credential()}),
    )
    .await;
    assert_eq!(hello["type"], "hello");
    let denied=exchange(&mut host,&mut events,&mut socket,json!({"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.4.0","name":"denied","commands":[],"required_capabilities":["editor_context_read","buffer_edit"],"optional_capabilities":[],"required_features":[wire::FEATURE],"optional_features":[]})).await;
    assert_eq!(denied["type"], "registration_error");
    assert_eq!(denied["code"], "capability_denied");
    assert!(host.context.readers.is_empty());
    let mut line = String::new();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), socket.read_line(&mut line))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}
fn read_terminal(id: &str, terminal: Value) -> Value {
    request(
        id,
        "terminal.read",
        json!({"terminal":terminal,"region":"screen","max_rows":10,"max_bytes":1024,"max_cells":4096}),
    )
}

#[tokio::test]
async fn two_live_hosts_are_discovered_and_reader_handles_do_not_cross_workspaces() {
    let root = TestRuntimeRoot::new("ctx-two").unwrap();
    let (mut first, identity, mut first_events) = host(root.path(), "first", HostMode::Standalone);
    let (mut second, _, mut second_events) = host(root.path(), "second", HostMode::Persistent);
    for (host, text) in [(&mut first, "Claude output"), (&mut second, "Codex output")] {
        let terminal = host.app.terminals.insert_test_session(40, 5);
        host.app.terminals.apply(TerminalOutput::Bytes {
            id: terminal,
            bytes: text.as_bytes().to_vec(),
        });
    }
    let environment = storage::environment_fingerprint();
    let discovery = discovery::discover(Some(root.path().join("ctx")), &environment, false);
    tokio::pin!(discovery);
    let discovered = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                result = &mut discovery => break result.unwrap(),
                event = first_events.recv() => first.handle_context_event(event.unwrap()),
                event = second_events.recv() => second.handle_context_event(event.unwrap()),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(discovered.workspaces.len(), 2);
    assert!(first.context.readers.is_empty() && second.context.readers.is_empty());
    assert_ne!(
        discovered.workspaces[0].registration.host_incarnation,
        discovered.workspaces[1].registration.host_incarnation
    );

    let mut first_client = authenticate(&mut first, &mut first_events, &identity).await;
    let mut second_client = authenticate(&mut second, &mut second_events, &identity).await;
    let first_list = exchange(
        &mut first,
        &mut first_events,
        &mut first_client,
        request("list", "terminal.list", json!({"offset":0,"limit":10})),
    )
    .await;
    let second_list = exchange(
        &mut second,
        &mut second_events,
        &mut second_client,
        request("list", "terminal.list", json!({"offset":0,"limit":10})),
    )
    .await;
    let first_handle = first_list["result"]["terminals"][0]["terminal"].clone();
    let second_handle = second_list["result"]["terminals"][0]["terminal"].clone();
    let first_text = exchange(
        &mut first,
        &mut first_events,
        &mut first_client,
        read_terminal("read", first_handle.clone()),
    )
    .await;
    let second_text = exchange(
        &mut second,
        &mut second_events,
        &mut second_client,
        read_terminal("read", second_handle),
    )
    .await;
    assert!(first_text["result"].to_string().contains("Claude output"));
    assert!(second_text["result"].to_string().contains("Codex output"));
    let crossed = exchange(
        &mut second,
        &mut second_events,
        &mut second_client,
        read_terminal("crossed", first_handle),
    )
    .await;
    assert!(crossed.get("error").is_some());
    // Detached hosts remain readable, but cannot create approval surfaces.
    let proposal = exchange(
        &mut first,
        &mut first_events,
        &mut first_client,
        request(
            "proposal",
            "terminal.input.propose",
            json!({"terminal":first_list["result"]["terminals"][0]["terminal"],"text":"literal"}),
        ),
    )
    .await;
    assert_eq!(proposal["error"]["code"], "no_frontend");
}

#[tokio::test]
async fn regrant_changes_incarnation_and_old_events_cannot_disconnect_new_reader() {
    let root = TestRuntimeRoot::new("ctx-regrant").unwrap();
    let (mut host, identity, mut events) = host(root.path(), "project", HostMode::Standalone);
    host.app.terminals.insert_test_session(20, 3);
    let mut client = authenticate(&mut host, &mut events, &identity).await;
    let old_id = *host.context.readers.keys().next().unwrap();
    let old_registration = host.context.registration.clone().unwrap();
    let list = exchange(
        &mut host,
        &mut events,
        &mut client,
        request("list", "terminal.list", json!({"offset":0,"limit":10})),
    )
    .await;
    let old_handle = list["result"]["terminals"][0]["terminal"].clone();
    host.context_revoke("agent");
    let mut closed = String::new();
    assert_eq!(
        pump(&mut host, &mut events, client.read_line(&mut closed))
            .await
            .unwrap(),
        0
    );
    assert!(!old_registration.endpoint.exists());
    host.context.grants.insert(
        identity.fingerprint(),
        (
            identity.clone(),
            [
                Scope::TerminalRead,
                Scope::EditorContextRead,
                Scope::TerminalPropose,
            ]
            .into(),
        ),
    );
    host.context_enable().unwrap();
    assert_ne!(
        host.context.registration.as_ref().unwrap().host_incarnation,
        old_registration.host_incarnation
    );
    let mut new_client = authenticate(&mut host, &mut events, &identity).await;
    let new_id = *host.context.readers.keys().next().unwrap();
    assert_ne!(old_id, new_id);
    host.handle_context_event(Event::Closed(old_id));
    let (reply, receiver) = tokio::sync::oneshot::channel();
    drop(receiver);
    host.handle_context_event(Event::Frame {
        connection: old_id,
        bytes: b"ignored".to_vec(),
        reply,
    });
    assert!(host.context.readers.contains_key(&new_id));
    let stale = exchange(
        &mut host,
        &mut events,
        &mut new_client,
        read_terminal("stale", old_handle),
    )
    .await;
    assert!(stale.get("error").is_some());
    let current = exchange(
        &mut host,
        &mut events,
        &mut new_client,
        request("new", "terminal.list", json!({"offset":0,"limit":10})),
    )
    .await;
    assert_eq!(current["result"]["terminals"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn expired_queued_and_writing_proposals_stop_delivery_polling() {
    let root = TestRuntimeRoot::new("ctx-expire").unwrap();
    let (mut host, identity, mut events) = host(root.path(), "project", HostMode::Standalone);
    host.note_plugin_frontend(true);
    let _client = authenticate(&mut host, &mut events, &identity).await;
    let owner = *host.context.readers.keys().next().unwrap();
    let terminal = host.app.terminals.insert_test_session(20, 3);
    for writing in [false, true] {
        let id = host
            .context_propose(owner, terminal, "literal", None)
            .unwrap()["proposal"]
            .as_str()
            .unwrap()
            .to_owned();
        let delivery = Delivery::queued_for_test();
        if writing {
            assert!(delivery.claim_for_test());
        }
        let proposal = host.context.proposals.get_mut(&id).unwrap();
        proposal.delivery = Some(delivery.clone());
        proposal.status = if writing { "writing" } else { "queued" };
        proposal.deadline = Instant::now() - Duration::from_secs(1);
        host.sync_context();
        assert_eq!(
            host.context.proposals[&id].status,
            if writing {
                "outcome_unknown"
            } else {
                "expired"
            }
        );
        assert!(host.context.proposals[&id].delivery.is_none());
        assert_eq!(
            delivery.state(),
            if writing {
                DeliveryState::Writing
            } else {
                DeliveryState::Cancelled
            }
        );
        assert!(host.context_delay().unwrap() > Duration::from_secs(1));
    }
}
