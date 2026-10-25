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
    extra_terminals: Vec<Value>,
    unnumbered: bool,
    direct_insert: bool,
    /// The host advertises the agent channel.
    channel: bool,
    joins: Vec<Value>,
    posts: Vec<Value>,
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
                    let channel = shared.lock().unwrap().channel;
                    let features = if channel {
                        json!([wire::FEATURE, wire::CHANNEL_FEATURE])
                    } else {
                        json!([wire::FEATURE])
                    };
                    let hello = json!({"type":"hello","version":"runyte-1","host_version":"0.4.0","features":features,"workspace":record,"limits":{"line_bytes":wire::MAX_FRAME_BYTES}});
                    if write_frame(socket.get_mut(), &hello, wire::MAX_FRAME_BYTES)
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let Ok(registration) = read_frame(&mut socket, wire::MAX_FRAME_BYTES).await
                    else {
                        return;
                    };
                    let registration = parse(&registration).unwrap();
                    let mut negotiated = vec![json!(wire::FEATURE)];
                    negotiated.extend(
                        registration["optional_features"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|_| channel)
                            .cloned(),
                    );
                    let registered = json!({"type":"registered","runyte":">=0.3.0, <0.5.0","commands":[],"features":negotiated,"capabilities":scopes,"limits":{"line_bytes":wire::MAX_FRAME_BYTES}});
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
                                        let mut terminals = vec![
                                            json!({"terminal":"native:t","number":1,"title":"[terminal #1] shell","name":"shell","live":true,"revision":"r:1"}),
                                        ];
                                        terminals.extend(s.extra_terminals.iter().cloned());
                                        if s.unnumbered {
                                            for row in &mut terminals {
                                                let row = row.as_object_mut().unwrap();
                                                row.remove("number");
                                                row.remove("title");
                                            }
                                        }
                                        let offset = params["offset"].as_u64().unwrap() as usize;
                                        let limit = params["limit"].as_u64().unwrap() as usize;
                                        let next = (offset + limit < terminals.len())
                                            .then_some(offset + limit);
                                        let page = terminals
                                            .into_iter()
                                            .skip(offset)
                                            .take(limit)
                                            .collect::<Vec<_>>();
                                        json!({"terminals":page,"next":next})
                                    }
                                    "buffer.read" => {
                                        json!({"buffer":"native:b","revision":format!("r:{}",s.revision),"text":s.buffer.chars().skip(params["from"].as_u64().unwrap() as usize).take((params["to"].as_u64().unwrap()-params["from"].as_u64().unwrap()) as usize).collect::<String>()})
                                    }
                                    "terminal.read" => {
                                        json!({"terminal":"native:t","rows":[{"text":"Welcome to Claude Code"},{"text":"Editing Rust"},{"text":"Merged PR #19"}],"revision":"r:1"})
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
                                        let state =
                                            if s.direct_insert { "queued" } else { "pending" };
                                        json!({"proposal":"native:i","state":state})
                                    }
                                    "terminal.input.status" => {
                                        json!({"proposal":"native:i","state":"pending"})
                                    }
                                    "terminal.input.cancel" => {
                                        json!({"proposal":"native:i","state":"cancelled"})
                                    }
                                    "channel.join" => {
                                        s.joins.push(params.clone());
                                        json!({"participant":{"participant":"0c0ffee0-0000-4000-8000-000000000001","nick":"claude-1"},"resume":"secret-resume","head":4})
                                    }
                                    "channel.post" => {
                                        s.posts.push(params.clone());
                                        json!({"seq":5})
                                    }
                                    "channel.read" => {
                                        let since = params["since"].as_u64().unwrap();
                                        if since < 5 {
                                            json!({"messages":[{"seq":5,"kind":"message","from_nick":"codex-1","text":"hello"}],"next":5,"lost":false,"head":5})
                                        } else {
                                            json!({"messages":[],"next":since,"lost":false,"head":5})
                                        }
                                    }
                                    "channel.who" => json!({"participants":[]}),
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

#[tokio::test]
async fn terminal_number_finds_the_titled_terminal_on_any_page_without_reading_output() {
    let host = Host::new();
    {
        let mut s = host.state.lock().unwrap();
        // Exited terminals keep earlier slots, so #2 sits past the first page.
        for index in 0..10 {
            s.extra_terminals.push(json!({"terminal":format!("native:x{index}"),"number":null,"title":format!("[terminal] old{index}"),"name":format!("old{index}"),"live":false,"revision":"r:1"}));
        }
        s.extra_terminals.push(json!({"terminal":"native:t2","number":2,"title":"[terminal #2] codex","name":"codex","live":true,"revision":"r:1"}));
        s.buffer = "see issue #2 here\n".into();
    }
    let mut bridge = host.bridge();
    let terminal_reads = |host: &Host| {
        host.state
            .lock()
            .unwrap()
            .methods
            .iter()
            .filter(|m| *m == "terminal.read")
            .count()
    };
    for query in [
        "#2",
        "terminal 2",
        "Terminal #2",
        " [terminal #2] ",
        "[terminal #2] codex",
    ] {
        let found = bridge
            .call("find_resources", json!({"query":query,"kind":"terminal"}))
            .await
            .unwrap();
        let results = found["results"].as_array().unwrap();
        assert_eq!(results.len(), 1, "{query}");
        assert_eq!(results[0]["matched_on"], "number");
        assert_eq!(results[0]["excerpt"], "[terminal #2] codex");
    }
    assert_eq!(terminal_reads(&host), 0, "a number lookup reads no output");

    // Searching everything, the exact terminal ranks above a buffer that
    // mentions the same text.
    let found = bridge
        .call("find_resources", json!({"query":"#2"}))
        .await
        .unwrap();
    let results = found["results"].as_array().unwrap();
    assert_eq!(results[0]["matched_on"], "number");
    assert!(
        results
            .iter()
            .any(|row| row["kind"] == "buffer" && row["matched_on"] == "content")
    );
    let terminal = handle(&found, "terminal");
    let proposal = bridge
        .call(
            "propose_terminal_text",
            json!({"terminal":terminal,"text":"ls"}),
        )
        .await
        .unwrap();
    assert_eq!(proposal["data"]["state"], "pending");

    // A buffer or content search means the text, as in an issue reference.
    let found = bridge
        .call("find_resources", json!({"query":"#2","kind":"buffer"}))
        .await
        .unwrap();
    assert_eq!(found["results"][0]["matched_on"], "content");
    let before = terminal_reads(&host);
    let found = bridge
        .call(
            "find_resources",
            json!({"query":"#2","kind":"terminal","match_in":"content"}),
        )
        .await
        .unwrap();
    assert!(
        found["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["matched_on"] != "number")
    );
    assert!(terminal_reads(&host) > before);

    // No running terminal shows the number. Output that merely mentions it
    // (`PR #19`) is not offered as something to write to.
    let missing = bridge
        .call("find_resources", json!({"query":"#9","kind":"terminal"}))
        .await
        .unwrap();
    assert!(missing["results"].as_array().unwrap().is_empty());
    assert!(
        missing["searches"][0]["note"]
            .as_str()
            .unwrap()
            .starts_with("No live terminal shows this number")
    );
    let text = bridge
        .call(
            "find_resources",
            json!({"query":"#9","kind":"terminal","match_in":"content"}),
        )
        .await
        .unwrap();
    assert_eq!(text["results"][0]["matched_on"], "content");
    // A bare number is fuzzy text from the start.
    let bare = bridge
        .call("find_resources", json!({"query":"2","kind":"terminal"}))
        .await
        .unwrap();
    assert!(bare["searches"][0].get("note").is_none());

    // A host that predates numbers is named rather than silently empty.
    host.state.lock().unwrap().unnumbered = true;
    let old = bridge
        .call("find_resources", json!({"query":"#2","kind":"terminal"}))
        .await
        .unwrap();
    assert!(
        old["searches"][0]["note"]
            .as_str()
            .unwrap()
            .contains("does not report terminal numbers")
    );
}

#[tokio::test]
async fn proposal_guidance_follows_whether_the_person_skipped_review() {
    let host = Host::new();
    let mut bridge = host.bridge();
    let found = bridge
        .call("find_resources", json!({"kind":"terminal"}))
        .await
        .unwrap();
    let terminal = handle(&found, "terminal");
    let reviewed = bridge
        .call(
            "propose_terminal_text",
            json!({"terminal":terminal,"text":"ls"}),
        )
        .await
        .unwrap();
    assert!(
        reviewed["next_step"]
            .as_str()
            .unwrap()
            .starts_with("Waiting for approval")
    );
    host.state.lock().unwrap().direct_insert = true;
    let direct = bridge
        .call(
            "propose_terminal_text",
            json!({"terminal":terminal,"text":"ls"}),
        )
        .await
        .unwrap();
    assert_eq!(direct["data"]["state"], "queued");
    let next = direct["next_step"].as_str().unwrap();
    assert!(next.contains("without review") && next.contains("Enter was not sent"));
}

#[tokio::test]
async fn channel_tools_join_once_continue_reads_and_rejoin_after_a_reconnect() {
    let host = Host::new();
    let mut bridge = host.bridge();
    // A host without the channel, or a grant without it, has none to join.
    let refused = bridge
        .call(
            "join_channel",
            json!({"provider":"claude","task":"refactor"}),
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code, "capability_denied");
    assert!(
        refused.message.contains(":mcp agent"),
        "{}",
        refused.message
    );

    {
        let mut s = host.state.lock().unwrap();
        s.channel = true;
        s.scopes.insert("agent_channel".into());
    }
    let mut bridge = host.bridge();
    bridge.client = Some(crate::workspace::context::channel::Client {
        name: "claude-code".into(),
        version: "2.0".into(),
    });
    assert_eq!(
        bridge
            .call("post_message", json!({"text":"hi"}))
            .await
            .unwrap_err()
            .code,
        "capability_denied",
        "posting needs a join"
    );
    let joined = bridge
        .call(
            "join_channel",
            json!({"provider":"claude","task":"refactor"}),
        )
        .await
        .unwrap();
    assert_eq!(joined["data"]["participant"]["nick"], "claude-1");
    assert!(
        joined["data"].get("resume").is_none(),
        "the agent never sees its token"
    );
    let join = host.state.lock().unwrap().joins[0].clone();
    assert_eq!(join["client"]["name"], "claude-code");
    assert_eq!(
        join["location"]["directory"],
        std::env::current_dir().unwrap().display().to_string()
    );
    assert!(join.get("resume").is_none(), "a first join is new");

    // The first read starts with what was said before; the next continues.
    let first = bridge.call("read_messages", json!({})).await.unwrap();
    assert_eq!(first["data"]["messages"][0]["text"], "hello");
    let started = std::time::Instant::now();
    let second = bridge
        .call("read_messages", json!({"wait_seconds":1}))
        .await
        .unwrap();
    assert!(second["data"]["messages"].as_array().unwrap().is_empty());
    assert!(started.elapsed() >= Duration::from_millis(900));
    bridge
        .call("post_message", json!({"text":"done","to":"codex-1"}))
        .await
        .unwrap();
    bridge.call("list_participants", json!({})).await.unwrap();

    // A lost connection renews on the next call, which joins again as the
    // same participant before reading.
    host.state.lock().unwrap().fail = Some("channel.post".into());
    assert_eq!(
        bridge
            .call("post_message", json!({"text":"lost"}))
            .await
            .unwrap_err()
            .code,
        "outcome_unknown"
    );
    bridge.call("read_messages", json!({})).await.unwrap();
    let joins = host.state.lock().unwrap().joins.clone();
    assert_eq!(joins.len(), 2);
    assert_eq!(joins[1]["resume"], "secret-resume");
    assert_eq!(joins[1]["provider"], "claude");
}

#[tokio::test]
async fn a_channel_only_grant_keeps_its_connection_between_calls() {
    let host = Host::new();
    {
        let mut s = host.state.lock().unwrap();
        s.channel = true;
        s.scopes = ["agent_channel".to_owned()].into();
    }
    let mut bridge = host.bridge();
    bridge
        .call("join_channel", json!({"provider":"codex","task":"review"}))
        .await
        .unwrap();
    for _ in 0..3 {
        bridge
            .call("post_message", json!({"text":"still here"}))
            .await
            .unwrap();
        bridge.call("read_messages", json!({})).await.unwrap();
    }
    assert_eq!(
        host.state.lock().unwrap().joins.len(),
        1,
        "no reconnect, so no rejoin"
    );
}

#[tokio::test]
async fn a_named_directory_stays_the_agents_place_until_it_names_another() {
    let host = Host::new();
    {
        let mut s = host.state.lock().unwrap();
        s.channel = true;
        s.scopes.insert("agent_channel".into());
    }
    let mut bridge = host.bridge();
    bridge
        .call(
            "join_channel",
            json!({"provider":"claude","task":"refactor"}),
        )
        .await
        .unwrap();
    let elsewhere = std::env::temp_dir().display().to_string();
    bridge
        .call(
            "post_message",
            json!({"text":"moved","directory":elsewhere}),
        )
        .await
        .unwrap();
    bridge
        .call("post_message", json!({"text":"still there"}))
        .await
        .unwrap();
    let posts = host.state.lock().unwrap().posts.clone();
    assert_eq!(posts[0]["location"]["directory"], elsewhere);
    assert_eq!(
        posts[1]["location"]["directory"], elsewhere,
        "leaving directory out does not move the agent back"
    );
}
