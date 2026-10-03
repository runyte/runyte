// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::workspace::context::{
    storage::{self, HostMode, Registration},
    wire,
};
use std::{
    collections::BTreeSet,
    os::unix::fs::PermissionsExt,
    sync::{Arc, Mutex},
};
use tokio::net::UnixListener;

#[derive(Default)]
struct State {
    scopes: BTreeSet<String>,
    methods: Vec<String>,
    buffer: String,
    revision: u64,
    fail: Option<String>,
    malformed: bool,
    exhaust_inventory: Option<String>,
}
struct Host {
    _temp: crate::test_support::TestRuntimeRoot,
    root: PathBuf,
    state: Arc<Mutex<State>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Host {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Host {
    fn new() -> Self {
        let temp = crate::test_support::TestRuntimeRoot::new("mcp").unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let root = temp.path().join("context");
        let store = Storage::new(root.clone()).unwrap();
        let identity = store.identity("agent").unwrap();
        let incarnation = storage::random_token().unwrap();
        let record = Registration {
            root: project.clone(),
            workspace_id: crate::workspace::workspace_id(&project),
            host_incarnation: incarnation.clone(),
            endpoint: store.socket_path(&incarnation).unwrap(),
            mode: HostMode::Persistent,
            pid: std::process::id(),
            environment: storage::environment_fingerprint(),
        };
        let listener = UnixListener::bind(&record.endpoint).unwrap();
        std::fs::set_permissions(&record.endpoint, std::fs::Permissions::from_mode(0o600)).unwrap();
        store.register(&record).unwrap();
        let state = Arc::new(Mutex::new(State {
            scopes: wire::CAPABILITIES.iter().map(|s| s.to_string()).collect(),
            buffer: "notes: Claude Code is working on syntax\n".into(),
            revision: 1,
            ..State::default()
        }));
        let shared = state.clone();
        let task = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let record = record.clone();
                let identity = identity.clone();
                let shared = shared.clone();
                tokio::spawn(async move {
                    let mut socket = BufReader::new(socket);
                    let Ok(raw) = read_frame(&mut socket, wire::MAX_FRAME_BYTES).await else {
                        return;
                    };
                    let value = parse(&raw).unwrap();
                    if value["type"] == "probe" {
                        let _ = write_frame(
                            socket.get_mut(),
                            &json!({"type":"context_endpoint","registration":record}),
                            wire::MAX_FRAME_BYTES,
                        )
                        .await;
                        return;
                    }
                    assert_eq!(value["credential"], identity.credential());
                    let scopes = shared.lock().unwrap().scopes.clone();
                    if scopes.is_empty() {
                        let _ = write_frame(
                            socket.get_mut(),
                            &json!({"type":"registration_error","code":"capability_denied"}),
                            wire::MAX_FRAME_BYTES,
                        )
                        .await;
                        return;
                    }
                    let hello = json!({"type":"hello","version":"runyte-1","host_version":"0.3.5","features":[wire::FEATURE],"workspace":record,"limits":{"line_bytes":wire::MAX_FRAME_BYTES}});
                    if write_frame(socket.get_mut(), &hello, wire::MAX_FRAME_BYTES)
                        .await
                        .is_err()
                    {
                        return;
                    }
                    if read_frame(&mut socket, wire::MAX_FRAME_BYTES)
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let registered = json!({"type":"registered","runyte":">=0.3.0, <0.4.0","commands":[],"features":[wire::FEATURE],"capabilities":scopes,"limits":{"line_bytes":wire::MAX_FRAME_BYTES}});
                    if write_frame(socket.get_mut(), &registered, wire::MAX_FRAME_BYTES)
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let mut exhausted_inventory = None;
                    loop {
                        let Ok(raw) = read_frame(&mut socket, wire::MAX_FRAME_BYTES).await else {
                            return;
                        };
                        if raw.is_empty() {
                            return;
                        }
                        let value = parse(&raw).unwrap();
                        let method = value["method"].as_str().unwrap();
                        let params = &value["params"];
                        let response = {
                            let mut s = shared.lock().unwrap();
                            s.methods.push(method.into());
                            if let Some(method) = s.exhaust_inventory.take() {
                                exhausted_inventory = Some(method);
                            }
                            if s.fail.as_deref() == Some(method) {
                                s.fail = None;
                                return;
                            }
                            if s.malformed {
                                s.malformed = false;
                                json!({"type":"response","id":"wrong","result":{}})
                            } else if s.scopes != scopes {
                                json!({"type":"response","id":value["id"],"error":{"code":"capability_denied","message":"Revoked"}})
                            } else if exhausted_inventory.as_deref() == Some(method)
                                && params["limit"] != 1
                            {
                                // The first resource remains known, so the
                                // discovery refresh succeeds on this reader.
                                json!({"type":"response","id":value["id"],"error":{"code":"limit_exceeded","message":"Reader handle limit reached; reconnect to renew inventory"}})
                            } else {
                                let data = match method {
                                    "buffer.list" => {
                                        json!({"buffers":[{"buffer":"native:b","name":"notes.txt","revision":format!("r:{}",s.revision),"chars":s.buffer.chars().count(),"read_only":false}],"next":null})
                                    }
                                    "terminal.list" => {
                                        json!({"terminals":[{"terminal":"native:t","name":"shell","live":true,"revision":"r:1"}],"next":null})
                                    }
                                    "buffer.read" => {
                                        json!({"buffer":"native:b","revision":format!("r:{}",s.revision),"text":s.buffer.chars().skip(params["from"].as_u64().unwrap() as usize).take((params["to"].as_u64().unwrap()-params["from"].as_u64().unwrap()) as usize).collect::<String>()})
                                    }
                                    "terminal.read" => {
                                        json!({"terminal":"native:t","rows":[{"text":"Welcome to Claude Code"},{"text":"Editing Rust"}],"revision":"r:1"})
                                    }
                                    "buffer.append" => {
                                        s.buffer.push_str(params["text"].as_str().unwrap());
                                        s.revision += 1;
                                        json!({"buffer":"native:b","revision":format!("r:{}",s.revision),"to":s.buffer.chars().count()})
                                    }
                                    "buffer.edit" => {
                                        s.revision += 1;
                                        json!({"buffer":"native:b","revision":format!("r:{}",s.revision)})
                                    }
                                    "terminal.input.propose" => {
                                        json!({"proposal":"native:i","state":"pending"})
                                    }
                                    "terminal.input.status" => {
                                        json!({"proposal":"native:i","state":"pending"})
                                    }
                                    "terminal.input.cancel" => {
                                        json!({"proposal":"native:i","state":"cancelled"})
                                    }
                                    _ => panic!("unexpected method {method}"),
                                };
                                json!({"type":"response","id":value["id"],"result":data})
                            }
                        };
                        if write_frame(socket.get_mut(), &response, wire::MAX_FRAME_BYTES)
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        Self {
            _temp: temp,
            root,
            state,
            task,
        }
    }
    fn bridge(&self) -> Bridge {
        Bridge::new(
            Some(self.root.clone()),
            "agent".into(),
            Duration::from_millis(200),
        )
        .unwrap()
    }
}
fn handle(found: &Value, kind: &str) -> String {
    found["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == kind)
        .unwrap()[kind]
        .as_str()
        .unwrap()
        .into()
}

#[tokio::test]
async fn exhausted_inventory_retires_reader_until_explicit_rediscovery() {
    for kind in ["buffer", "terminal"] {
        let host = Host::new();
        let mut bridge = host.bridge();
        let found = bridge
            .call("find_resources", json!({"kind":kind}))
            .await
            .unwrap();
        let old = handle(&found, kind);
        host.state.lock().unwrap().exhaust_inventory = Some(format!("{kind}.list"));
        let exhausted = bridge
            .call("find_resources", json!({"kind":kind}))
            .await
            .unwrap();
        assert_eq!(exhausted["searches"][0]["error"], "limit_exceeded");
        assert!(exhausted["results"].as_array().unwrap().is_empty());
        assert_eq!(
            bridge
                .call(&format!("read_{kind}"), json!({(kind):old}))
                .await
                .unwrap_err()
                .code,
            "stale"
        );
        let renewed = bridge
            .call("find_resources", json!({"kind":kind}))
            .await
            .unwrap();
        let fresh = handle(&renewed, kind);
        assert_ne!(old, fresh);
        bridge
            .call(&format!("read_{kind}"), json!({(kind):fresh}))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn fuzzy_contents_find_plain_shell_and_append_in_two_tool_calls() {
    let host = Host::new();
    let mut bridge = host.bridge();
    let found = bridge
        .call(
            "find_resources",
            json!({"query":"clde","workspace":"project"}),
        )
        .await
        .unwrap();
    assert_eq!(found["results"].as_array().unwrap().len(), 2);
    for row in found["results"].as_array().unwrap() {
        assert_eq!(row["matched_on"], "content");
        assert!(row["excerpt"].as_str().unwrap().contains("Claude"));
    }
    let buffer = handle(&found, "buffer");
    let terminal = handle(&found, "terminal");
    assert!(buffer.len() < 32);
    assert!(terminal.len() < 32);
    bridge
        .call("append_buffer", json!({"buffer":buffer,"text":"hello\n"}))
        .await
        .unwrap();
    assert!(host.state.lock().unwrap().buffer.ends_with("hello\n"));
    let read = bridge
        .call("read_buffer", json!({"buffer":buffer}))
        .await
        .unwrap();
    assert!(read["data"]["text"].as_str().unwrap().ends_with("hello\n"));
    let read = bridge
        .call("read_terminal", json!({"terminal":terminal}))
        .await
        .unwrap();
    assert!(
        read["data"]["rows"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Claude")
    );
    let proposal = bridge
        .call(
            "propose_terminal_text",
            json!({"terminal":terminal,"text":"hello"}),
        )
        .await
        .unwrap();
    assert_eq!(proposal["data"]["state"], "pending");
    let id = &proposal["data"]["proposal"];
    assert_eq!(
        bridge
            .call("terminal_proposal_status", json!({"proposal":id}))
            .await
            .unwrap()["data"]["state"],
        "pending"
    );
    assert_eq!(
        bridge
            .call("cancel_terminal_proposal", json!({"proposal":id}))
            .await
            .unwrap()["data"]["state"],
        "cancelled"
    );
    bridge.call("edit_buffer",json!({"buffer":buffer,"expected_revision":"r:2","changes":[{"from":0,"to":0,"text":"x"}]})).await.unwrap();
    assert_eq!(
        bridge
            .call("read_buffer", json!({"buffer":buffer}))
            .await
            .unwrap_err()
            .code,
        "stale"
    );
}
#[tokio::test]
async fn late_permissions_refresh_without_catalog_changes_and_revocation_fails_closed() {
    let host = Host::new();
    host.state.lock().unwrap().scopes.clear();
    let mut bridge = host.bridge();
    let denied = bridge.call("find_resources", json!({})).await.unwrap();
    assert_eq!(
        denied["workspaces"][0]["unavailable_reason"],
        "capability_denied"
    );
    host.state
        .lock()
        .unwrap()
        .scopes
        .insert("editor_context_read".into());
    let found = bridge.call("find_resources", json!({})).await.unwrap();
    let old = handle(&found, "buffer");
    assert_eq!(
        bridge
            .call("append_buffer", json!({"buffer":old,"text":"x"}))
            .await
            .unwrap_err()
            .code,
        "capability_denied"
    );
    host.state
        .lock()
        .unwrap()
        .scopes
        .insert("buffer_edit".into());
    let found = bridge.call("find_resources", json!({})).await.unwrap();
    let fresh = handle(&found, "buffer");
    assert_ne!(old, fresh);
    assert_eq!(
        bridge
            .call("append_buffer", json!({"buffer":old,"text":"x"}))
            .await
            .unwrap_err()
            .code,
        "stale"
    );
    bridge
        .call("append_buffer", json!({"buffer":fresh,"text":"yes"}))
        .await
        .unwrap();
    host.state.lock().unwrap().scopes.clear();
    assert_eq!(
        bridge
            .call("read_buffer", json!({"buffer":fresh}))
            .await
            .unwrap_err()
            .code,
        "capability_denied"
    );
    assert_eq!(
        bridge
            .call("read_buffer", json!({"buffer":fresh}))
            .await
            .unwrap_err()
            .code,
        "stale"
    );
}
#[tokio::test]
async fn names_only_never_reads_contents_and_explicit_targets_reject_foreign_handles() {
    let host = Host::new();
    let mut bridge = host.bridge();
    let found = bridge
        .call("find_resources", json!({"query":"notes","match_in":"name"}))
        .await
        .unwrap();
    assert_eq!(found["results"].as_array().unwrap().len(), 1);
    assert!(
        !host
            .state
            .lock()
            .unwrap()
            .methods
            .iter()
            .any(|m| m.ends_with(".read"))
    );
    let handle = handle(&found, "buffer");
    let mut other = host.bridge();
    other.call("find_resources", json!({})).await.unwrap();
    assert_eq!(
        other
            .call("read_buffer", json!({"buffer":handle}))
            .await
            .unwrap_err()
            .code,
        "stale"
    );
    assert_eq!(
        bridge
            .call("read_terminal", json!({"terminal":handle}))
            .await
            .unwrap_err()
            .code,
        "stale"
    );
    assert!(
        bridge
            .call("find_resources", json!({"workspace":"missing"}))
            .await
            .unwrap()["results"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn uncertain_mutations_are_never_replayed_and_controls_never_reach_host() {
    let host = Host::new();
    let mut bridge = host.bridge();
    let found = bridge.call("find_resources", json!({})).await.unwrap();
    let terminal = handle(&found, "terminal");
    assert_eq!(
        bridge
            .call(
                "propose_terminal_text",
                json!({"terminal":terminal,"text":"hello\n"})
            )
            .await
            .unwrap_err()
            .code,
        "invalid_argument"
    );
    assert!(
        !host
            .state
            .lock()
            .unwrap()
            .methods
            .contains(&"terminal.input.propose".to_owned())
    );
    host.state.lock().unwrap().fail = Some("terminal.input.propose".into());
    assert_eq!(
        bridge
            .call(
                "propose_terminal_text",
                json!({"terminal":terminal,"text":"hello"})
            )
            .await
            .unwrap_err()
            .code,
        "outcome_unknown"
    );
    assert_eq!(
        bridge
            .call(
                "propose_terminal_text",
                json!({"terminal":terminal,"text":"hello"})
            )
            .await
            .unwrap_err()
            .code,
        "stale"
    );
    assert_eq!(
        host.state
            .lock()
            .unwrap()
            .methods
            .iter()
            .filter(|m| m.as_str() == "terminal.input.propose")
            .count(),
        1
    );
    let found = bridge.call("find_resources", json!({})).await.unwrap();
    let buffer = handle(&found, "buffer");
    host.state.lock().unwrap().malformed = true;
    assert_eq!(
        bridge
            .call("append_buffer", json!({"buffer":buffer,"text":"hello"}))
            .await
            .unwrap_err()
            .code,
        "outcome_unknown"
    );
}

#[tokio::test]
async fn healthy_discovery_preserves_handles_and_later_buffer_windows_are_explicit() {
    let host = Host::new();
    host.state.lock().unwrap().buffer = format!("{}\nTARGET_LATE\n", "界".repeat(9000));
    let mut bridge = host.bridge();
    let first = bridge
        .call("find_resources", json!({"kind":"buffer"}))
        .await
        .unwrap();
    let original = handle(&first, "buffer");
    let again = bridge
        .call("find_resources", json!({"kind":"buffer"}))
        .await
        .unwrap();
    assert_eq!(original, handle(&again, "buffer"));
    let missing = bridge
        .call(
            "find_resources",
            json!({"query":"tgt_lte","kind":"buffer","match_in":"content"}),
        )
        .await
        .unwrap();
    assert!(missing["results"].as_array().unwrap().is_empty());
    assert_eq!(missing["search_bounds"]["buffer_chars"], 8192);
    let found = bridge
        .call(
            "find_resources",
            json!({"query":"tgt_lte","kind":"buffer","content_from":8192}),
        )
        .await
        .unwrap();
    assert_eq!(handle(&found, "buffer"), original);
    assert!(found["results"][0]["content_partial"].as_bool().unwrap());
    assert!(
        found["results"][0]["excerpt"]
            .as_str()
            .unwrap()
            .contains("TARGET_LATE")
    );
    let read = bridge
        .call(
            "read_buffer",
            json!({"buffer":original,"from":9001,"to":9013}),
        )
        .await
        .unwrap();
    assert_eq!(read["data"]["text"], "TARGET_LATE\n");
}
