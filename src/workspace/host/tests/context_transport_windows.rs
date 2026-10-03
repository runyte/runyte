// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::{App, HostPorts},
    clipboard::SystemClipboard,
    terminal::TerminalOutput,
    test_support::TestRuntimeRoot,
    text::Transaction,
    workspace::{
        context::{
            storage::{HostMode, Identity, StorageLocation, random_token},
            transport::{NativeConnection, connect},
        },
        windows_process_identity::ProcessIdentity,
    },
};
use std::{
    future::Future,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

const CLIENT_FIXTURE: &str = "workspace::host::context::transport_tests::compiled_context_client";
const CLIENT_ENDPOINT: &str = "RUNYTE_CONTEXT_CLIENT_ENDPOINT";
const CLIENT_PID: &str = "RUNYTE_CONTEXT_CLIENT_PID";
const CLIENT_CREATION: &str = "RUNYTE_CONTEXT_CLIENT_CREATION";
const CLIENT_CREDENTIAL: &str = "RUNYTE_CONTEXT_CLIENT_CREDENTIAL";

struct Clipboard;
impl SystemClipboard for Clipboard {
    fn read(&mut self) -> anyhow::Result<String> {
        anyhow::bail!("inert fixture")
    }
    fn write(&mut self, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

struct Fixture {
    environment: String,
    host: WorkspaceHost,
    identity: Identity,
    events: mpsc::Receiver<Event>,
    _root: TestRuntimeRoot,
}

impl Fixture {
    fn new(label: &str, scopes: BTreeSet<Scope>) -> Self {
        let root = TestRuntimeRoot::new(label).unwrap();
        let project = root.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let app = App::new_in_isolated_project(&project, HostPorts::isolated(Box::new(Clipboard)))
            .unwrap();
        let mut host = WorkspaceHost::new(app);
        let context_root = root.path().join("context");
        let environment = random_token().unwrap();
        let events = host
            .start_context_windows(
                HostMode::Standalone,
                Some(StorageLocation::explicit(context_root.clone()).unwrap()),
                environment.clone(),
            )
            .unwrap();
        host.context_open_storage(context_root.clone()).unwrap();
        let identity = host
            .context
            .storage
            .as_ref()
            .unwrap()
            .identity("agent")
            .unwrap();
        host.context
            .grants
            .insert(identity.fingerprint(), (identity.clone(), scopes));
        host.context_enable().unwrap();
        Self {
            environment,
            host,
            identity,
            events,
            _root: root,
        }
    }

    async fn connect(&self) -> BufReader<NativeConnection> {
        let registration = self.host.context.registration.as_ref().unwrap();
        BufReader::new(
            connect(
                &registration.endpoint,
                ProcessIdentity {
                    pid: registration.pid,
                    creation_time: registration.creation_time,
                },
                tokio::time::Instant::now() + Duration::from_secs(2),
            )
            .await
            .unwrap(),
        )
    }
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
    .expect("native context host deadline")
}

async fn exchange(
    host: &mut WorkspaceHost,
    events: &mut mpsc::Receiver<Event>,
    stream: &mut BufReader<NativeConnection>,
    value: Value,
) -> Value {
    pump(host, events, async {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        stream.get_mut().write_all(&bytes).await.unwrap();
        let mut line = String::new();
        assert!(stream.read_line(&mut line).await.unwrap() > 0);
        serde_json::from_str(&line).unwrap()
    })
    .await
}

async fn connect_host(host: &WorkspaceHost) -> BufReader<NativeConnection> {
    let registration = host.context.registration.as_ref().unwrap();
    BufReader::new(
        connect(
            &registration.endpoint,
            ProcessIdentity {
                pid: registration.pid,
                creation_time: registration.creation_time,
            },
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap(),
    )
}

async fn register(
    host: &mut WorkspaceHost,
    events: &mut mpsc::Receiver<Event>,
    stream: &mut BufReader<NativeConnection>,
    identity: &Identity,
    scopes: &[&str],
) -> Value {
    let hello = exchange(
        host,
        events,
        stream,
        json!({"type":"authenticate", "credential":identity.credential()}),
    )
    .await;
    assert_eq!(hello["type"], "hello");
    exchange(
        host,
        events,
        stream,
        json!({"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.4.0","name":"native-fixture","commands":[],"required_capabilities":scopes,"optional_capabilities":[],"required_features":[wire::FEATURE],"optional_features":[]}),
    )
    .await
}

fn request(id: &str, method: &str, params: Value) -> Value {
    json!({"type":"request", "id":id, "method":method, "params":params})
}

async fn wait_for_retirement(host: &mut WorkspaceHost, events: &mut mpsc::Receiver<Event>) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while host.context.retiring.is_some() {
            host.handle_context_event(events.recv().await.unwrap());
        }
    })
    .await
    .unwrap();
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "launched by the native Windows context host test"]
fn compiled_context_client() {
    let endpoint = std::env::var(CLIENT_ENDPOINT).unwrap();
    let pid = std::env::var(CLIENT_PID).unwrap().parse().unwrap();
    let creation_time = std::env::var(CLIENT_CREATION).unwrap().parse().unwrap();
    let credential = std::env::var(CLIENT_CREDENTIAL).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut stream = BufReader::new(
            connect(
                std::path::Path::new(&endpoint),
                ProcessIdentity { pid, creation_time },
                tokio::time::Instant::now() + Duration::from_secs(5),
            )
            .await
            .unwrap(),
        );
        for value in [
            json!({"type":"authenticate","credential":credential}),
            json!({"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.4.0","name":"compiled-child","commands":[],"required_capabilities":["editor_context_read"],"optional_capabilities":[],"required_features":[wire::FEATURE],"optional_features":[]}),
            request("child-list", "buffer.list", json!({"offset":0,"limit":10})),
        ] {
            let mut bytes = serde_json::to_vec(&value).unwrap();
            bytes.push(b'\n');
            stream.get_mut().write_all(&bytes).await.unwrap();
            let mut line = String::new();
            assert!(stream.read_line(&mut line).await.unwrap() > 0);
            let reply: Value = serde_json::from_str(&line).unwrap();
            assert!(reply.get("error").is_none(), "{reply}");
        }
    });
}

#[tokio::test]
async fn compiled_native_client_authenticates_reads_and_exits_before_shutdown() {
    use std::os::windows::process::CommandExt;

    let mut fixture = Fixture::new(
        "context-host-compiled-client",
        [Scope::EditorContextRead].into(),
    );
    let registration = fixture.host.context.registration.clone().unwrap();
    let output = fixture._root.path().join("compiled-client-output");
    let stdout = std::fs::File::create(&output).unwrap();
    let mut child = ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CLIENT_FIXTURE, "--ignored", "--nocapture"])
            .env(CLIENT_ENDPOINT, &registration.endpoint)
            .env(CLIENT_PID, registration.pid.to_string())
            .env(CLIENT_CREATION, registration.creation_time.to_string())
            .env(CLIENT_CREDENTIAL, fixture.identity.credential())
            .env("XDG_CONFIG_HOME", fixture._root.path().join("config"))
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(stdout.try_clone().unwrap())
            .stderr(stdout)
            .spawn()
            .unwrap(),
    );
    let status = pump(&mut fixture.host, &mut fixture.events, async {
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(
        status.success(),
        "compiled context client failed: {}",
        std::fs::read_to_string(&output).unwrap()
    );
    fixture.host.shutdown_context().await.unwrap();
}

#[tokio::test]
async fn shutdown_joins_a_native_connection_with_a_queued_host_frame() {
    let mut fixture = Fixture::new(
        "context-host-queued-shutdown",
        [Scope::EditorContextRead].into(),
    );
    let mut client = fixture.connect().await;
    let mut authentication = serde_json::to_vec(
        &json!({"type":"authenticate","credential":fixture.identity.credential()}),
    )
    .unwrap();
    authentication.push(b'\n');
    client.get_mut().write_all(&authentication).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.events.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture.host.shutdown_context().await.unwrap();
    let mut closed = String::new();
    assert_eq!(client.read_line(&mut closed).await.unwrap(), 0);
}

#[tokio::test]
async fn real_native_clients_enforce_credentials_and_requested_read_scopes() {
    let mut fixture = Fixture::new(
        "context-host-native-auth",
        [Scope::TerminalRead, Scope::EditorContextRead].into(),
    );
    let storage = fixture.host.context.storage.as_ref().unwrap().clone();
    assert_eq!(
        storage.discover(&fixture.environment, false).unwrap().len(),
        1
    );

    let mut denied = fixture.connect().await;
    let response = exchange(
        &mut fixture.host,
        &mut fixture.events,
        &mut denied,
        json!({"type":"authenticate", "credential":"0".repeat(64)}),
    )
    .await;
    assert_eq!(response["type"], "registration_error");
    let mut closed = String::new();
    assert_eq!(denied.read_line(&mut closed).await.unwrap(), 0);
    assert!(fixture.host.context.readers.is_empty());

    let mut malformed = fixture.connect().await;
    let hello = exchange(
        &mut fixture.host,
        &mut fixture.events,
        &mut malformed,
        json!({"type":"authenticate", "credential":fixture.identity.credential()}),
    )
    .await;
    assert_eq!(hello["type"], "hello");
    let response = exchange(
        &mut fixture.host,
        &mut fixture.events,
        &mut malformed,
        json!({"type":"register","version":"runyte-1"}),
    )
    .await;
    assert_eq!(response["type"], "registration_error");
    assert_eq!(response["code"], "invalid_argument");
    let mut closed = String::new();
    assert_eq!(malformed.read_line(&mut closed).await.unwrap(), 0);

    let mut denied_scope = fixture.connect().await;
    let hello = exchange(
        &mut fixture.host,
        &mut fixture.events,
        &mut denied_scope,
        json!({"type":"authenticate", "credential":fixture.identity.credential()}),
    )
    .await;
    assert_eq!(hello["type"], "hello");
    let denied_registration = exchange(
        &mut fixture.host,
        &mut fixture.events,
        &mut denied_scope,
        json!({"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.4.0","name":"denied-scope","commands":[],"required_capabilities":["buffer_edit"],"optional_capabilities":[],"required_features":[wire::FEATURE],"optional_features":[]}),
    )
    .await;
    assert_eq!(denied_registration["type"], "registration_error");
    assert_eq!(denied_registration["code"], "capability_denied");
    let mut closed = String::new();
    assert_eq!(denied_scope.read_line(&mut closed).await.unwrap(), 0);

    let mut limited = fixture.connect().await;
    assert_eq!(
        register(
            &mut fixture.host,
            &mut fixture.events,
            &mut limited,
            &fixture.identity,
            &["terminal_read"],
        )
        .await["type"],
        "registered"
    );
    let denied_read = exchange(
        &mut fixture.host,
        &mut fixture.events,
        &mut limited,
        request("denied", "buffer.list", json!({"offset":0,"limit":10})),
    )
    .await;
    assert_eq!(denied_read["error"]["code"], "capability_denied");

    let mut reader = fixture.connect().await;
    assert_eq!(
        register(
            &mut fixture.host,
            &mut fixture.events,
            &mut reader,
            &fixture.identity,
            &["editor_context_read"],
        )
        .await["type"],
        "registered"
    );
    let buffers = exchange(
        &mut fixture.host,
        &mut fixture.events,
        &mut reader,
        request("read", "buffer.list", json!({"offset":0,"limit":10})),
    )
    .await;
    assert_eq!(buffers["type"], "response");
    assert!(!buffers["result"]["buffers"].as_array().unwrap().is_empty());

    drop(denied);
    drop(limited);
    drop(reader);
    fixture.host.shutdown_context().await.unwrap();
    assert!(
        storage
            .discover(&fixture.environment, true)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn native_buffer_reads_and_edits_are_unicode_exact_revision_checked_and_atomic() {
    let mut fixture = Fixture::new(
        "context-host-native-buffer-edit",
        [Scope::EditorContextRead, Scope::BufferEdit].into(),
    );
    fixture
        .host
        .app
        .apply_to_buffer(0, &Transaction::insert(0, "a\u{00e7}\u{754c}\u{1f642}z"));
    fixture.host.app.buffers[0].commit_undo_group();
    let focus = fixture.host.app.active_pane;
    let mut client = fixture.connect().await;
    assert_eq!(
        register(
            &mut fixture.host,
            &mut fixture.events,
            &mut client,
            &fixture.identity,
            &["editor_context_read", "buffer_edit"],
        )
        .await["type"],
        "registered"
    );
    let listed = exchange(
        &mut fixture.host,
        &mut fixture.events,
        &mut client,
        request("list", "buffer.list", json!({"offset":0,"limit":10})),
    )
    .await;
    let buffer = listed["result"]["buffers"][0]["buffer"]
        .as_str()
        .unwrap()
        .to_owned();
    let revision = listed["result"]["buffers"][0]["revision"]
        .as_str()
        .unwrap()
        .to_owned();
    let read = exchange(
        &mut fixture.host,
        &mut fixture.events,
        &mut client,
        request(
            "read",
            "buffer.read",
            json!({"buffer":buffer,"expected_revision":revision,"from":1,"to":4}),
        ),
    )
    .await;
    assert_eq!(read["result"]["text"], "\u{00e7}\u{754c}\u{1f642}");
    let edit = exchange(
        &mut fixture.host,
        &mut fixture.events,
        &mut client,
        request(
            "edit",
            "buffer.edit",
            json!({"buffer":buffer,"expected_revision":revision,"changes":[{"from":1,"to":2,"text":"two\nlines"},{"from":4,"to":5,"text":"!"}]}),
        ),
    )
    .await;
    assert_eq!(edit["type"], "response");
    assert_eq!(
        fixture.host.app.buffers[0].to_string(),
        "atwo\nlines\u{754c}\u{1f642}!"
    );
    assert_eq!(fixture.host.app.active_pane, focus);
    let stale = exchange(
        &mut fixture.host,
        &mut fixture.events,
        &mut client,
        request(
            "stale",
            "buffer.read",
            json!({"buffer":buffer,"expected_revision":revision,"from":0,"to":1}),
        ),
    )
    .await;
    assert_eq!(stale["error"]["code"], "stale");
    assert!(fixture.host.app.buffers[0].undo());
    assert_eq!(
        fixture.host.app.buffers[0].to_string(),
        "a\u{00e7}\u{754c}\u{1f642}z"
    );
    fixture.host.shutdown_context().await.unwrap();
}

fn terminal_read(id: &str, terminal: Value) -> Value {
    request(
        id,
        "terminal.read",
        json!({"terminal":terminal,"region":"screen","max_rows":10,"max_bytes":1024,"max_cells":4096}),
    )
}

#[tokio::test]
async fn hosts_connections_and_reconnects_keep_native_handles_separate() {
    let root = TestRuntimeRoot::new("context-host-native-handle-isolation").unwrap();
    let context_root = root.path().join("context");
    let location = StorageLocation::explicit(context_root.clone()).unwrap();
    let environment = random_token().unwrap();

    let make_host = |name: &str| {
        let project = root.path().join(name);
        std::fs::create_dir(&project).unwrap();
        WorkspaceHost::new(
            App::new_in_isolated_project(&project, HostPorts::isolated(Box::new(Clipboard)))
                .unwrap(),
        )
    };
    let mut first = make_host("first");
    let mut first_events = first
        .start_context_windows(
            HostMode::Standalone,
            Some(location.clone()),
            environment.clone(),
        )
        .unwrap();
    first.context_open_storage(context_root.clone()).unwrap();
    let identity = first
        .context
        .storage
        .as_ref()
        .unwrap()
        .identity("agent")
        .unwrap();
    first.context.grants.insert(
        identity.fingerprint(),
        (
            identity.clone(),
            [Scope::TerminalRead, Scope::TerminalPropose].into(),
        ),
    );
    first.context_enable().unwrap();

    let mut second = make_host("second");
    let mut second_events = second
        .start_context_windows(HostMode::Persistent, Some(location), environment.clone())
        .unwrap();
    second.context_open_storage(context_root).unwrap();
    second.context.grants.insert(
        identity.fingerprint(),
        (
            identity.clone(),
            [Scope::TerminalRead, Scope::TerminalPropose].into(),
        ),
    );
    second.context_enable().unwrap();
    assert_eq!(
        first
            .context
            .storage
            .as_ref()
            .unwrap()
            .discover(&environment, false)
            .unwrap()
            .len(),
        2
    );
    for (host, text) in [(&mut first, "first-output"), (&mut second, "second-output")] {
        let terminal = host.app.terminals.insert_test_session(40, 5);
        host.app.terminals.apply(TerminalOutput::Bytes {
            id: terminal,
            bytes: text.as_bytes().to_vec(),
        });
    }

    let mut first_a = connect_host(&first).await;
    let mut first_b = connect_host(&first).await;
    let mut second_client = connect_host(&second).await;
    for client in [&mut first_a, &mut first_b] {
        assert_eq!(
            register(
                &mut first,
                &mut first_events,
                client,
                &identity,
                &["terminal_read", "terminal_propose"],
            )
            .await["type"],
            "registered"
        );
    }
    assert_eq!(
        register(
            &mut second,
            &mut second_events,
            &mut second_client,
            &identity,
            &["terminal_read", "terminal_propose"],
        )
        .await["type"],
        "registered"
    );
    let first_list = exchange(
        &mut first,
        &mut first_events,
        &mut first_a,
        request(
            "first-list",
            "terminal.list",
            json!({"offset":0,"limit":10}),
        ),
    )
    .await;
    let first_handle = first_list["result"]["terminals"][0]["terminal"].clone();
    let detached = exchange(
        &mut first,
        &mut first_events,
        &mut first_a,
        request(
            "detached-proposal",
            "terminal.input.propose",
            json!({"terminal":first_handle.clone(),"text":"literal"}),
        ),
    )
    .await;
    assert_eq!(detached["error"]["code"], "no_frontend");
    let crossed_connection = exchange(
        &mut first,
        &mut first_events,
        &mut first_b,
        terminal_read("cross-connection", first_handle.clone()),
    )
    .await;
    assert!(crossed_connection.get("error").is_some());
    let crossed_host = exchange(
        &mut second,
        &mut second_events,
        &mut second_client,
        terminal_read("cross-host", first_handle.clone()),
    )
    .await;
    assert!(crossed_host.get("error").is_some());

    drop(first_a);
    tokio::time::timeout(Duration::from_secs(3), async {
        while first.context.readers.len() == 2 {
            first.handle_context_event(first_events.recv().await.unwrap());
        }
    })
    .await
    .unwrap();
    let mut reconnected = connect_host(&first).await;
    assert_eq!(
        register(
            &mut first,
            &mut first_events,
            &mut reconnected,
            &identity,
            &["terminal_read", "terminal_propose"],
        )
        .await["type"],
        "registered"
    );
    let reconnected_list = exchange(
        &mut first,
        &mut first_events,
        &mut reconnected,
        request("new-list", "terminal.list", json!({"offset":0,"limit":10})),
    )
    .await;
    assert_ne!(
        reconnected_list["result"]["terminals"][0]["terminal"],
        first_handle
    );

    drop((first_b, second_client, reconnected));
    first.shutdown_context().await.unwrap();
    second.shutdown_context().await.unwrap();
}

#[tokio::test]
async fn revoke_joins_before_regrant_and_stale_events_cannot_replace_authority() {
    let mut fixture = Fixture::new(
        "context-host-native-regrant",
        [Scope::EditorContextRead].into(),
    );
    let storage = fixture.host.context.storage.as_ref().unwrap().clone();
    let first_registration = fixture.host.context.registration.clone().unwrap();
    let mut first = fixture.connect().await;
    assert_eq!(
        register(
            &mut fixture.host,
            &mut fixture.events,
            &mut first,
            &fixture.identity,
            &["editor_context_read"],
        )
        .await["type"],
        "registered"
    );
    let old_connection = *fixture.host.context.readers.keys().next().unwrap();
    fixture.host.context_revoke("agent");
    assert!(!fixture.host.context_enabled());
    assert!(fixture.host.context.readers.is_empty());
    assert!(fixture.host.context.retiring.is_some());
    fixture.host.context.grants.insert(
        fixture.identity.fingerprint(),
        (fixture.identity.clone(), [Scope::EditorContextRead].into()),
    );
    fixture.host.context_enable().unwrap();
    assert!(!fixture.host.context_enabled());
    let (reply, response) = tokio::sync::oneshot::channel();
    fixture.host.handle_context_event(Event::Frame {
        connection: old_connection,
        bytes: serde_json::to_vec(
            &json!({"type":"authenticate", "credential":fixture.identity.credential()}),
        )
        .unwrap(),
        reply,
    });
    let refused = response.await.unwrap();
    assert!(refused.close);
    assert_eq!(refused.value["code"], "unavailable");
    assert!(fixture.host.context.readers.is_empty());
    wait_for_retirement(&mut fixture.host, &mut fixture.events).await;
    fixture.host.sync_context();
    assert!(fixture.host.context_enabled());
    let second_registration = fixture.host.context.registration.clone().unwrap();
    assert_ne!(
        first_registration.host_incarnation,
        second_registration.host_incarnation
    );
    assert_eq!(
        storage.discover(&fixture.environment, false).unwrap().len(),
        1
    );

    let mut second = fixture.connect().await;
    assert_eq!(
        register(
            &mut fixture.host,
            &mut fixture.events,
            &mut second,
            &fixture.identity,
            &["editor_context_read"],
        )
        .await["type"],
        "registered"
    );
    let new_connection = *fixture.host.context.readers.keys().next().unwrap();
    assert_ne!(old_connection, new_connection);
    fixture
        .host
        .handle_context_event(Event::Closed(old_connection));
    let (reply, receiver) = tokio::sync::oneshot::channel();
    drop(receiver);
    fixture.host.handle_context_event(Event::Frame {
        connection: old_connection,
        bytes: b"ignored".to_vec(),
        reply,
    });
    assert!(fixture.host.context.readers.contains_key(&new_connection));

    drop(first);
    drop(second);
    fixture.host.shutdown_context().await.unwrap();
    assert!(
        storage
            .discover(&fixture.environment, true)
            .unwrap()
            .is_empty()
    );
}
