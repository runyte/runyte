// SPDX-License-Identifier: MPL-2.0

//! Language server client lifecycle, exercised against an in-process mock.
//!
//! The mock speaks real JSON-RPC over in-memory pipes, so the handshake, a
//! crash, a malformed frame, and a cancelled request all take the same code
//! path they would with a real server — without needing one installed.
//!
//! Tests that need a real server are `#[ignore]`d and opt-in.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use runyte::{
    config::{LanguageServerConfig, LspConfig},
    lsp::{
        ChangeSync, DocumentSync, Encoding, Launch, LspCommand, LspEvent, LspHandle,
        PENDING_CAPACITY, RequestKind, Response, Severity, path_to_uri, spawn, spawn_with,
        transport::{self, Incoming},
    },
};
use serde_json::{Value, json};
use tokio::{
    io::{BufReader, DuplexStream, WriteHalf},
    sync::mpsc,
};

const PIPE: usize = 64 * 1024;

/// How a mock server answers one request method.
type Reply = Box<dyn Fn(&Value) -> Value + Send + Sync>;

#[derive(Default)]
struct Script {
    replies: HashMap<String, Reply>,
    errors: HashMap<String, Value>,
    initialize_result: Option<Value>,
    ignore_initialize: bool,
    ignored: HashSet<String>,
    /// Frames pushed verbatim, before any reply, once `initialize` is answered.
    on_initialized: Vec<Value>,
    /// Frames pushed before the initialize response, while the client must
    /// still reject server requests that require ready editor state.
    before_initialize: Vec<Value>,
    /// Frames sent after shutdown begins but before its response.
    on_shutdown: Vec<Value>,
    /// Raw bytes written instead of a framed reply, to simulate a broken server.
    raw_after_initialize: Option<Vec<u8>>,
    /// Close the connection instead of answering this method.
    die_on: Option<String>,
}

impl Script {
    fn reply(
        mut self,
        method: &str,
        reply: impl Fn(&Value) -> Value + Send + Sync + 'static,
    ) -> Self {
        self.replies.insert(method.to_owned(), Box::new(reply));
        self
    }

    fn result(self, method: &str, result: Value) -> Self {
        self.reply(method, move |_| result.clone())
    }

    fn error(mut self, method: &str, code: i64, message: &str) -> Self {
        self.errors
            .insert(method.to_owned(), json!({"code": code, "message": message}));
        self
    }

    fn push(mut self, message: Value) -> Self {
        self.on_initialized.push(message);
        self
    }

    fn push_before_initialize(mut self, message: Value) -> Self {
        self.before_initialize.push(message);
        self
    }

    fn push_on_shutdown(mut self, message: Value) -> Self {
        self.on_shutdown.push(message);
        self
    }

    fn die_on(mut self, method: &str) -> Self {
        self.die_on = Some(method.to_owned());
        self
    }

    fn raw_after_initialize(mut self, bytes: &[u8]) -> Self {
        self.raw_after_initialize = Some(bytes.to_vec());
        self
    }

    fn initialize_result(mut self, result: Value) -> Self {
        self.initialize_result = Some(result);
        self
    }

    fn ignore_initialize(mut self) -> Self {
        self.ignore_initialize = true;
        self
    }

    fn ignore(mut self, method: &str) -> Self {
        self.ignored.insert(method.to_owned());
        self
    }
}

/// Everything a test needs to drive the manager and inspect what the server
/// was told.
struct Harness {
    handle: LspHandle,
    events: mpsc::Receiver<LspEvent>,
    /// Every message the client sent, in order.
    sent: Arc<Mutex<Vec<Value>>>,
}

impl Harness {
    async fn next(&mut self) -> LspEvent {
        tokio::time::timeout(Duration::from_secs(5), self.events.recv())
            .await
            .expect("the manager should answer within five seconds")
            .expect("the manager should still be running")
    }

    /// Drains events until one matches, so an unrelated status message cannot
    /// make a test flaky.
    async fn next_matching<T>(&mut self, mut pick: impl FnMut(&LspEvent) -> Option<T>) -> T {
        for _ in 0..32 {
            let event = self.next().await;
            if let Some(found) = pick(&event) {
                return found;
            }
        }
        panic!("no matching event arrived");
    }

    async fn ready(&mut self) -> Encoding {
        self.next_matching(|event| match event {
            LspEvent::Ready { encoding, .. } => Some(*encoding),
            _ => None,
        })
        .await
    }

    async fn response(&mut self) -> Response {
        self.next_matching(|event| match event {
            LspEvent::Response { response, .. } => Some(response.clone()),
            _ => None,
        })
        .await
    }

    fn request(&self, kind: RequestKind) {
        assert!(self.handle.send(LspCommand::Request {
            token: 1,
            language: "rust".to_owned(),
            path: PathBuf::from("/tmp/runyte-lsp/a.rs"),
            kind: Box::new(kind),
        }));
    }

    /// Messages the client sent whose `method` matches.
    fn sent_with(&self, method: &str) -> Vec<Value> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|message| message.get("method").and_then(Value::as_str) == Some(method))
            .cloned()
            .collect()
    }

    async fn wait_for_sent(&self, mut predicate: impl FnMut(&Value) -> bool) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(message) = self
                    .sent
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|message| predicate(message))
                    .cloned()
                {
                    return message;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the client should send the expected message within five seconds")
    }

    async fn settle(&self) {
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
}

fn config() -> LspConfig {
    let mut servers = HashMap::new();
    servers.insert(
        "rust".to_owned(),
        LanguageServerConfig {
            command: PathBuf::from("mock-analyzer"),
            args: Vec::new(),
            initialization_options: None,
        },
    );
    LspConfig {
        enable: true,
        servers,
    }
}

/// Starts a manager whose only server is the scripted mock.
fn harness(script: Script) -> Harness {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&sent);
    let script = Arc::new(script);
    let launch: Launch = Box::new(move |language, generation, _settings, _root, inbox| {
        // Two pipes: one per direction, so the client and the mock never read
        // their own writes.
        let (client_reader, server_writer) = tokio::io::duplex(PIPE);
        let (server_reader, client_writer) = tokio::io::duplex(PIPE);
        let connection = transport::connect(
            language.to_owned(),
            generation,
            client_reader,
            client_writer,
            inbox as mpsc::Sender<(String, u64, Incoming)>,
        );
        tokio::spawn(mock_server(
            Arc::clone(&script),
            Arc::clone(&recorded),
            server_reader,
            server_writer,
        ));
        Ok(connection)
    });
    let (handle, events) = spawn_with(config(), PathBuf::from("/tmp/runyte-lsp"), launch);
    Harness {
        handle,
        events,
        sent,
    }
}

async fn mock_server(
    script: Arc<Script>,
    sent: Arc<Mutex<Vec<Value>>>,
    reader: DuplexStream,
    writer: DuplexStream,
) {
    let mut reader = BufReader::new(reader);
    let (_, mut writer) = tokio::io::split(writer);
    while let Some(message) = transport::read_framed(&mut reader).await {
        sent.lock().unwrap().push(message.clone());
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if script.die_on.as_deref() == Some(method.as_str()) {
            return;
        }
        let Some(id) = message.get("id").cloned() else {
            continue;
        };

        if method == "initialize" {
            if script.ignore_initialize {
                continue;
            }
            for message in &script.before_initialize {
                let _ = transport::write_message(&mut writer, message).await;
            }
            respond(
                &mut writer,
                id,
                script.initialize_result.clone().unwrap_or_else(|| {
                    json!({
                        "capabilities": {
                            "positionEncoding": "utf-8",
                            "textDocumentSync": 2,
                        },
                        "serverInfo": { "name": "mock-analyzer", "version": "1" },
                    })
                }),
            )
            .await;
            if let Some(bytes) = &script.raw_after_initialize {
                use tokio::io::AsyncWriteExt;
                let _ = writer.write_all(bytes).await;
                let _ = writer.flush().await;
            }
            for message in &script.on_initialized {
                let _ = transport::write_message(&mut writer, message).await;
            }
            continue;
        }

        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if method == "shutdown" {
            for message in &script.on_shutdown {
                let _ = transport::write_message(&mut writer, message).await;
            }
        }
        if script.ignored.contains(&method) {
            continue;
        }
        if let Some(error) = script.errors.get(&method) {
            let _ = transport::write_message(
                &mut writer,
                &json!({"jsonrpc": "2.0", "id": id, "error": error}),
            )
            .await;
        } else {
            match script.replies.get(&method) {
                Some(reply) => respond(&mut writer, id, reply(&params)).await,
                None => respond(&mut writer, id, Value::Null).await,
            }
        }
    }
}

async fn respond(writer: &mut WriteHalf<DuplexStream>, id: Value, result: Value) {
    let _ = transport::write_message(
        writer,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    )
    .await;
}

fn open(handle: &LspHandle) {
    assert!(handle.send(LspCommand::Open {
        language: "rust".to_owned(),
        path: PathBuf::from("/tmp/runyte-lsp/a.rs"),
        version: 1,
        text: "fn main() {}\n".to_owned(),
    }));
}

fn location(line: u32) -> Value {
    json!({
        "uri": path_to_uri(std::path::Path::new("/tmp/runyte-lsp/b.rs")).unwrap(),
        "range": {
            "start": {"line": line, "character": 3},
            "end": {"line": line, "character": 9},
        },
    })
}

// -- Lifecycle -------------------------------------------------------------

#[tokio::test]
async fn a_language_server_that_cannot_start_is_stopped_with_an_actionable_reason() {
    let root = std::env::temp_dir().join(format!(
        "runyte-missing-language-server-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let missing = root.join("missing-language-server");
    let mut configuration = config();
    configuration.servers.get_mut("rust").unwrap().command = missing.clone();
    let (handle, mut events) = spawn(configuration, root.clone());

    assert!(handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    let stopped = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.expect("the manager remains available");
            if let LspEvent::Stopped { language, message } = event {
                break (language, message);
            }
        }
    })
    .await
    .expect("the failed launch should be reported");

    assert_eq!(stopped.0, "rust");
    assert!(stopped.1.contains("cannot start"), "{}", stopped.1);
    assert!(
        stopped
            .1
            .contains(missing.file_name().unwrap().to_string_lossy().as_ref()),
        "{}",
        stopped.1
    );

    assert!(handle.send(LspCommand::Status));
    let status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.expect("the manager remains available");
            if let LspEvent::Status { message, error } = event
                && !error
                && message.contains("cannot start")
            {
                break message;
            }
        }
    })
    .await
    .expect("status should retain the recorded launch failure");
    assert!(
        status.contains(missing.file_name().unwrap().to_string_lossy().as_ref()),
        "{status}"
    );
    drop(handle);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn the_handshake_negotiates_an_encoding_and_gates_document_notifications() {
    let mut harness = harness(Script::default());
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));

    assert_eq!(harness.ready().await, Encoding::Utf8);
    open(&harness.handle);
    harness.settle().await;

    let sent = harness.sent.lock().unwrap().clone();
    let methods: Vec<&str> = sent
        .iter()
        .map(|message| message.get("method").and_then(Value::as_str).unwrap_or(""))
        .collect();
    assert_eq!(methods.first(), Some(&"initialize"));
    let initialized = methods
        .iter()
        .position(|method| *method == "initialized")
        .unwrap();
    let configured = methods
        .iter()
        .position(|method| *method == "workspace/didChangeConfiguration")
        .unwrap();
    let opened = methods
        .iter()
        .position(|method| *method == "textDocument/didOpen")
        .unwrap();
    assert!(
        initialized < configured && configured < opened,
        "documents must not be announced before the handshake completes: {methods:?}"
    );
    assert_eq!(
        sent[configured].get("params"),
        Some(&json!({ "settings": {} })),
        "servers such as Pyright wait for an initial configuration notification"
    );
}

#[tokio::test]
async fn a_malformed_initialize_result_stops_and_records_the_server() {
    let mut harness = harness(Script::default().initialize_result(json!({"not": "initialize"})));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    let stopped = harness
        .next_matching(|event| match event {
            LspEvent::Stopped { message, .. } => Some(message.clone()),
            _ => None,
        })
        .await;
    assert!(stopped.contains("failed to initialize"), "{stopped}");

    assert!(harness.handle.send(LspCommand::Status));
    let status = harness
        .next_matching(|event| match event {
            LspEvent::Status { message, .. } => Some(message.clone()),
            _ => None,
        })
        .await;
    assert!(status.contains("stopped"), "{status}");
}

#[tokio::test]
async fn an_unknown_position_encoding_stops_the_server() {
    let mut harness = harness(Script::default().initialize_result(json!({
        "capabilities": {"positionEncoding": "utf-7"}
    })));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    let message = harness
        .next_matching(|event| match event {
            LspEvent::Stopped { message, .. } => Some(message.clone()),
            _ => None,
        })
        .await;
    assert!(
        message.contains("unsupported position encoding"),
        "{message}"
    );
}

#[tokio::test]
async fn an_unknown_text_sync_kind_stops_the_server() {
    let mut harness = harness(Script::default().initialize_result(json!({
        "capabilities": {"textDocumentSync": 99}
    })));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    let message = harness
        .next_matching(|event| match event {
            LspEvent::Stopped { message, .. } => Some(message.clone()),
            _ => None,
        })
        .await;
    assert!(
        message.contains("unsupported text document sync kind"),
        "{message}"
    );
}

#[tokio::test]
async fn unsupported_document_notifications_are_not_sent() {
    let mut harness = harness(Script::default().initialize_result(json!({
        "capabilities": {
            "positionEncoding": "utf-8",
            "textDocumentSync": {"openClose": false, "change": 0, "save": false}
        }
    })));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    let sync = harness
        .next_matching(|event| match event {
            LspEvent::Ready { sync, .. } => Some(*sync),
            _ => None,
        })
        .await;
    assert_eq!(sync, DocumentSync::default());
    open(&harness.handle);
    assert!(harness.handle.send(LspCommand::Change {
        language: "rust".to_owned(),
        path: PathBuf::from("/tmp/runyte-lsp/a.rs"),
        version: 2,
        changes: vec![],
    }));
    assert!(harness.handle.send(LspCommand::Save {
        language: "rust".to_owned(),
        path: PathBuf::from("/tmp/runyte-lsp/a.rs"),
        text: "changed".to_owned(),
    }));
    assert!(harness.handle.send(LspCommand::Close {
        language: "rust".to_owned(),
        path: PathBuf::from("/tmp/runyte-lsp/a.rs"),
    }));
    harness.settle().await;
    for method in [
        "textDocument/didOpen",
        "textDocument/didChange",
        "textDocument/didSave",
        "textDocument/didClose",
    ] {
        assert!(harness.sent_with(method).is_empty(), "unexpected {method}");
    }
}

#[tokio::test]
async fn save_text_is_omitted_when_the_server_did_not_request_it() {
    let mut harness = harness(Script::default().initialize_result(json!({
        "capabilities": {
            "positionEncoding": "utf-8",
            "textDocumentSync": {
                "openClose": true,
                "change": 1,
                "save": {"includeText": false}
            }
        }
    })));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    let sync = harness
        .next_matching(|event| match event {
            LspEvent::Ready { sync, .. } => Some(*sync),
            _ => None,
        })
        .await;
    assert_eq!(sync.change, ChangeSync::Full);
    assert_eq!(sync.save, Some(false));
    assert!(harness.handle.send(LspCommand::Save {
        language: "rust".to_owned(),
        path: PathBuf::from("/tmp/runyte-lsp/a.rs"),
        text: "secret text".to_owned(),
    }));
    harness.settle().await;
    let saves = harness.sent_with("textDocument/didSave");
    assert_eq!(saves.len(), 1);
    assert!(saves[0]["params"].get("text").is_none());
}

#[tokio::test]
async fn pre_handshake_document_updates_are_bounded_and_stop_a_wedged_server() {
    let mut harness = harness(Script::default().ignore_initialize());
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    for version in 1..=(transport::OUTGOING_CAPACITY as i32 + 1) {
        assert!(harness.handle.send(LspCommand::Open {
            language: "rust".to_owned(),
            path: PathBuf::from(format!("/tmp/runyte-lsp/{version}.rs")),
            version,
            text: String::new(),
        }));
    }
    let stopped = harness
        .next_matching(|event| match event {
            LspEvent::Stopped { message, .. } => Some(message.clone()),
            _ => None,
        })
        .await;
    assert!(
        stopped.contains("not accepting document updates"),
        "{stopped}"
    );
}

#[tokio::test]
async fn cancelling_releases_protocol_correlation_and_notifies_the_server() {
    let mut harness = harness(Script::default().ignore("textDocument/hover"));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    assert!(harness.handle.send(LspCommand::Request {
        token: 77,
        language: "rust".to_owned(),
        path: PathBuf::from("/tmp/runyte-lsp/a.rs"),
        kind: Box::new(RequestKind::Hover(runyte::lsp::LspPosition::new(0, 0))),
    }));
    harness.settle().await;
    assert!(harness.handle.send(LspCommand::Cancel { token: 77 }));
    harness.settle().await;

    let cancelled = harness.sent_with("$/cancelRequest");
    assert_eq!(cancelled.len(), 1);
    assert!(cancelled[0]["params"]["id"].is_i64());
}

#[tokio::test]
async fn cancellation_that_overtakes_its_request_drops_the_request() {
    let mut harness = harness(Script::default().ignore("textDocument/hover"));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    assert!(harness.handle.send(LspCommand::Request {
        token: 78,
        language: "rust".to_owned(),
        path: PathBuf::from("/tmp/runyte-lsp/a.rs"),
        kind: Box::new(RequestKind::Hover(runyte::lsp::LspPosition::new(0, 0))),
    }));
    assert!(harness.handle.send(LspCommand::Cancel { token: 78 }));
    harness.settle().await;

    assert!(harness.sent_with("textDocument/hover").is_empty());
    assert!(harness.sent_with("$/cancelRequest").is_empty());
}

#[tokio::test]
async fn unanswered_requests_are_bounded_per_server() {
    let mut harness = harness(Script::default().ignore("textDocument/hover"));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;

    for index in 0..=PENDING_CAPACITY {
        let command = LspCommand::Request {
            token: 10_000 + index as u64,
            language: "rust".to_owned(),
            path: PathBuf::from("/tmp/runyte-lsp/a.rs"),
            kind: Box::new(RequestKind::Hover(runyte::lsp::LspPosition::new(0, 0))),
        };
        while !harness.handle.send(command.clone()) {
            tokio::task::yield_now().await;
        }
    }

    let (token, reason) = harness
        .next_matching(|event| match event {
            LspEvent::Response {
                token,
                response: Response::Failed(reason),
            } => Some((*token, reason.clone())),
            _ => None,
        })
        .await;
    assert_eq!(token, 10_000 + PENDING_CAPACITY as u64);
    assert!(reason.contains("not responding"), "{reason}");
}

#[tokio::test]
async fn incremental_changes_are_sent_in_the_order_a_server_can_apply() {
    let mut harness = harness(Script::default());
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    open(&harness.handle);

    // Two edits in one transaction, described against the pre-edit document.
    assert!(harness.handle.send(LspCommand::Change {
        language: "rust".to_owned(),
        path: PathBuf::from("/tmp/runyte-lsp/a.rs"),
        version: 2,
        changes: vec![
            runyte::lsp::TextDocumentContentChangeEvent {
                range: Some(runyte::lsp::LspRange::new(
                    runyte::lsp::LspPosition::new(0, 9),
                    runyte::lsp::LspPosition::new(0, 9),
                )),
                range_length: None,
                text: "b".to_owned(),
            },
            runyte::lsp::TextDocumentContentChangeEvent {
                range: Some(runyte::lsp::LspRange::new(
                    runyte::lsp::LspPosition::new(0, 3),
                    runyte::lsp::LspPosition::new(0, 3),
                )),
                range_length: None,
                text: "a".to_owned(),
            },
        ],
    }));
    harness.settle().await;

    let changes = harness.sent_with("textDocument/didChange");
    assert_eq!(changes.len(), 1);
    let content = changes[0]["params"]["contentChanges"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    let first = content[0]["range"]["start"]["character"].as_u64().unwrap();
    let second = content[1]["range"]["start"]["character"].as_u64().unwrap();
    assert!(
        first > second,
        "later positions must be sent first so earlier edits cannot shift them"
    );
    assert_eq!(changes[0]["params"]["textDocument"]["version"], 2);
}

#[tokio::test]
async fn navigation_results_keep_the_sending_servers_encoding() {
    let mut harness =
        harness(Script::default().result("textDocument/definition", json!([location(0)])));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    assert_eq!(harness.ready().await, Encoding::Utf8);
    harness.request(RequestKind::Definition(runyte::lsp::LspPosition::new(0, 0)));
    let Response::Locations(locations) = harness.response().await else {
        panic!("expected locations");
    };
    assert_eq!(locations[0].encoding, Encoding::Utf8);
}

#[tokio::test]
async fn every_advertised_request_round_trips_through_its_wire_shape() {
    let edit = json!({
        "range": {
            "start": {"line": 0, "character": 0},
            "end": {"line": 0, "character": 1}
        },
        "newText": "replacement"
    });
    let script = Script::default()
        .initialize_result(json!({
            "capabilities": {
                "positionEncoding": "utf-8",
                "textDocumentSync": 2,
                "definitionProvider": true,
                "declarationProvider": true,
                "typeDefinitionProvider": true,
                "implementationProvider": true,
                "referencesProvider": true,
                "hoverProvider": true,
                "completionProvider": {},
                "signatureHelpProvider": {},
                "documentSymbolProvider": true,
                "workspaceSymbolProvider": true,
                "renameProvider": true,
                "codeActionProvider": {"resolveProvider": true},
                "executeCommandProvider": {"commands": ["mock.command"]},
                "documentFormattingProvider": true
            },
            "serverInfo": {"name": "mock-analyzer", "version": "1"}
        }))
        .result("textDocument/definition", json!([location(1)]))
        .result("textDocument/declaration", location(2))
        .result("textDocument/typeDefinition", json!([location(3)]))
        .result("textDocument/implementation", json!([location(4)]))
        .result("textDocument/references", json!([location(5)]))
        .result("textDocument/hover", json!({"contents": "documentation"}))
        .result(
            "textDocument/completion",
            json!({"isIncomplete": false, "items": [{"label": "candidate", "kind": 3}]}),
        )
        .result(
            "textDocument/signatureHelp",
            json!({"signatures": [{"label": "item(value)", "parameters": [{"label": "value"}]}]}),
        )
        .result(
            "textDocument/documentSymbol",
            json!([{
                "name": "item",
                "kind": 12,
                "location": {"uri": "file:///tmp/runyte-lsp/a.rs", "range": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 4}
                }}
            }]),
        )
        .result(
            "workspace/symbol",
            json!([{
                "name": "item",
                "kind": 12,
                "location": {"uri": "file:///tmp/runyte-lsp/a.rs"}
            }]),
        )
        .result(
            "textDocument/rename",
            json!({"changes": {"file:///tmp/runyte-lsp/a.rs": [edit.clone()]}}),
        )
        .result(
            "textDocument/codeAction",
            json!([{"title": "fix it", "command": "mock.command"}]),
        )
        .result(
            "codeAction/resolve",
            json!({
                "title": "fix it",
                "edit": {"changes": {"file:///tmp/runyte-lsp/a.rs": [edit.clone()]}}
            }),
        )
        .result("workspace/executeCommand", json!({}))
        .result("textDocument/formatting", json!([edit]));
    let mut harness = harness(script);
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;

    let position = runyte::lsp::LspPosition::new(3, 7);
    let range = runyte::lsp::LspRange::new(position, runyte::lsp::LspPosition::new(3, 9));
    let action = serde_json::from_value::<lsp_types::CodeAction>(json!({
        "title": "fix it",
        "kind": "quickfix"
    }))
    .unwrap();
    let command = lsp_types::Command {
        title: "run fix".to_owned(),
        command: "mock.command".to_owned(),
        arguments: Some(vec![json!("argument")]),
    };
    let requests = vec![
        RequestKind::Definition(position),
        RequestKind::Declaration(position),
        RequestKind::TypeDefinition(position),
        RequestKind::Implementation(position),
        RequestKind::References(position),
        RequestKind::Hover(position),
        RequestKind::Completion(position),
        RequestKind::SignatureHelp {
            position,
            context: runyte::lsp::SignatureContext::default(),
        },
        RequestKind::DocumentSymbols,
        RequestKind::WorkspaceSymbols("needle".to_owned()),
        RequestKind::Rename {
            position,
            new_name: "renamed".to_owned(),
        },
        RequestKind::CodeActions {
            range,
            diagnostics: Vec::new(),
        },
        RequestKind::ResolveCodeAction(Box::new(action)),
        RequestKind::ExecuteCommand(Box::new(command)),
        RequestKind::Format {
            tab_size: 4,
            insert_spaces: true,
        },
    ];
    let methods = [
        "textDocument/definition",
        "textDocument/declaration",
        "textDocument/typeDefinition",
        "textDocument/implementation",
        "textDocument/references",
        "textDocument/hover",
        "textDocument/completion",
        "textDocument/signatureHelp",
        "textDocument/documentSymbol",
        "workspace/symbol",
        "textDocument/rename",
        "textDocument/codeAction",
        "codeAction/resolve",
        "workspace/executeCommand",
        "textDocument/formatting",
    ];

    for (index, (request, method)) in requests.into_iter().zip(methods).enumerate() {
        harness.request(request);
        let response = harness.response().await;
        match (index, response) {
            (0..=4, Response::Locations(locations)) => {
                assert_eq!(locations.len(), 1);
                assert_eq!(locations[0].range.start.line, index as u32 + 1);
                assert_eq!(locations[0].encoding, Encoding::Utf8);
            }
            (5, Response::Hover(text)) => assert!(text.contains("documentation"), "{text}"),
            (6, Response::Completions(items)) => assert_eq!(items[0].label, "candidate"),
            (7, Response::Signatures(signatures)) => {
                assert_eq!(signatures[0].label, "item(value)")
            }
            (8 | 9, Response::Symbols(symbols)) => assert_eq!(symbols[0].name, "item"),
            (
                10 | 14,
                Response::Edits {
                    edits,
                    skipped: 0,
                    encoding: Encoding::Utf8,
                },
            ) => {
                assert_eq!(edits.len(), 1);
                let expected_path = if index == 10 {
                    PathBuf::from("/tmp/runyte-lsp/a.rs")
                } else {
                    PathBuf::new()
                };
                assert_eq!(edits[0].path, expected_path);
                assert_eq!(edits[0].edits.len(), 1);
            }
            (11, Response::Actions(actions)) => assert_eq!(actions[0].title, "fix it"),
            (
                12,
                Response::ActionEdits {
                    edits,
                    skipped: 0,
                    encoding: Encoding::Utf8,
                    command: None,
                },
            ) => assert_eq!(edits.len(), 1),
            (13, Response::Empty) => {}
            (_, response) => panic!("request {index} returned {response:?}"),
        }
        assert_eq!(
            harness.sent_with(method).len(),
            1,
            "{method} was not consumed exactly once"
        );
    }

    harness.settle().await;
    assert_eq!(
        harness.sent_with("textDocument/references")[0]["params"]["context"]["includeDeclaration"],
        json!(true)
    );
    assert_eq!(
        harness.sent_with("workspace/symbol")[0]["params"]["query"],
        json!("needle")
    );
    assert_eq!(
        harness.sent_with("textDocument/formatting")[0]["params"]["options"]["tabSize"],
        json!(4)
    );
}

#[tokio::test]
async fn diagnostics_are_published_and_cleared() {
    let uri = path_to_uri(std::path::Path::new("/tmp/runyte-lsp/a.rs")).unwrap();
    let script = Script::default()
        .push(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": uri,
                "diagnostics": [{
                    "range": {
                        "start": {"line": 2, "character": 4},
                        "end": {"line": 2, "character": 9},
                    },
                    "severity": 1,
                    "source": "mock",
                    "message": "mismatched types",
                }],
            },
        }))
        .push(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {"uri": uri, "diagnostics": []},
        }));
    let mut harness = harness(script);
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));

    let diagnostics = harness
        .next_matching(|event| match event {
            LspEvent::Diagnostics { diagnostics, .. } => Some(diagnostics.clone()),
            _ => None,
        })
        .await;
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].severity, Severity::Error);
    assert_eq!(diagnostics[0].message, "mismatched types");
    assert_eq!(diagnostics[0].row(), 2);
    let cleared = harness
        .next_matching(|event| match event {
            LspEvent::Diagnostics { diagnostics, .. } if diagnostics.is_empty() => Some(()),
            _ => None,
        })
        .await;
    assert_eq!(cleared, ());
}

#[tokio::test]
async fn only_actionable_show_messages_reach_the_editor_status() {
    let script = Script::default()
        .push(json!({
            "jsonrpc": "2.0",
            "method": "window/logMessage",
            "params": {"type": 1, "message": "private server log"},
        }))
        .push(json!({
            "jsonrpc": "2.0",
            "method": "window/showMessage",
            "params": {"type": 3, "message": "routine information"},
        }))
        .push(json!({
            "jsonrpc": "2.0",
            "method": "window/showMessage",
            "params": {"type": 2, "message": "index is incomplete"},
        }));
    let mut harness = harness(script);
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    let mut statuses = Vec::new();
    loop {
        let event = harness.next().await;
        if let LspEvent::Status { message, .. } = event {
            let done = message == "rust: index is incomplete";
            statuses.push(message);
            if done {
                break;
            }
        }
    }
    assert_eq!(statuses, ["rust: index is incomplete"]);
}

#[tokio::test]
async fn a_crash_fails_every_outstanding_request_and_reports_the_stop() {
    let mut harness = harness(Script::default().die_on("textDocument/definition"));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    harness.request(RequestKind::Definition(runyte::lsp::LspPosition::new(0, 0)));

    // Nothing may be left waiting on a server that will never answer.
    let failure = harness
        .next_matching(|event| match event {
            LspEvent::Response {
                response: Response::Failed(reason),
                token,
            } => Some((*token, reason.clone())),
            _ => None,
        })
        .await;
    assert_eq!(failure.0, 1);
    assert!(failure.1.contains("stopped"), "{}", failure.1);

    let stopped = harness
        .next_matching(|event| match event {
            LspEvent::Stopped { language, .. } => Some(language.clone()),
            _ => None,
        })
        .await;
    assert_eq!(stopped, "rust");
}

#[tokio::test]
async fn a_request_to_a_missing_server_fails_instead_of_hanging() {
    // No `Ensure`, so nothing is running for this language.
    let mut harness = harness(Script::default());
    assert!(harness.handle.send(LspCommand::Request {
        token: 7,
        language: "python".to_owned(),
        path: PathBuf::from("/tmp/runyte-lsp/a.py"),
        kind: Box::new(RequestKind::Hover(runyte::lsp::LspPosition::new(0, 0))),
    }));

    let event = harness.next().await;
    let LspEvent::Response {
        token,
        response: Response::Failed(reason),
    } = event
    else {
        panic!("expected a failed response, got {event:?}");
    };
    assert_eq!(token, 7);
    assert!(reason.contains("python"), "{reason}");
}

#[tokio::test]
async fn a_malformed_frame_is_reported_and_the_session_survives() {
    let script = Script::default()
        .raw_after_initialize(b"Content-Length: 5\r\n\r\n{ bad")
        .result("textDocument/definition", json!([location(4)]));
    let mut harness = harness(script);
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;

    let complaint = harness
        .next_matching(|event| match event {
            LspEvent::Status { message, error } if *error => Some(message.clone()),
            _ => None,
        })
        .await;
    assert!(complaint.contains("unreadable"), "{complaint}");

    // The stream stayed aligned, so the next request still works.
    harness.request(RequestKind::Definition(runyte::lsp::LspPosition::new(0, 0)));
    let Response::Locations(locations) = harness.response().await else {
        panic!("expected locations after a malformed frame");
    };
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].range.start.line, 4);
}

#[tokio::test]
async fn a_server_error_response_fails_only_that_request() {
    let script = Script::default().error("textDocument/hover", -32603, "backend failed");
    let mut harness = harness(script);
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;

    harness.request(RequestKind::Hover(runyte::lsp::LspPosition::new(0, 0)));
    let Response::Failed(message) = harness.response().await else {
        panic!("the server error was not preserved");
    };
    assert!(message.contains("documentation"), "{message}");
    assert!(message.contains("backend failed"), "{message}");

    // The server is still usable afterwards.
    harness.request(RequestKind::DocumentSymbols);
    assert!(matches!(harness.response().await, Response::Empty));
}

#[tokio::test]
async fn concurrent_requests_keep_their_editor_tokens() {
    let script = Script::default().result("textDocument/definition", json!([location(1)]));
    let mut harness = harness(script);
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;

    for token in [11, 12] {
        assert!(harness.handle.send(LspCommand::Request {
            token,
            language: "rust".to_owned(),
            path: PathBuf::from("/tmp/runyte-lsp/a.rs"),
            kind: Box::new(RequestKind::Definition(runyte::lsp::LspPosition::new(0, 0))),
        }));
    }

    let mut seen = Vec::new();
    for _ in 0..2 {
        seen.push(
            harness
                .next_matching(|event| match event {
                    LspEvent::Response { token, .. } => Some(*token),
                    _ => None,
                })
                .await,
        );
    }
    seen.sort_unstable();
    assert_eq!(seen, vec![11, 12], "each token gets exactly its own answer");
}

#[tokio::test]
async fn restarting_clears_a_failure_so_the_next_edit_starts_a_server_again() {
    let mut harness = harness(Script::default().die_on("textDocument/definition"));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    harness.request(RequestKind::Definition(runyte::lsp::LspPosition::new(0, 0)));
    harness
        .next_matching(|event| match event {
            LspEvent::Stopped { .. } => Some(()),
            _ => None,
        })
        .await;

    // A request while stopped reports the recorded failure rather than hanging.
    harness.request(RequestKind::Hover(runyte::lsp::LspPosition::new(0, 0)));
    assert!(matches!(harness.response().await, Response::Failed(_)));

    assert!(
        harness
            .handle
            .send(LspCommand::Restart(Some("rust".to_owned())))
    );
    harness
        .next_matching(|event| match event {
            LspEvent::Status { message, .. } if message.contains("restart") => Some(()),
            _ => None,
        })
        .await;
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    assert_eq!(harness.ready().await, Encoding::Utf8);
}

#[tokio::test]
async fn a_retired_connection_cannot_stop_its_replacement() {
    let mut harness = harness(Script::default());
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    assert!(
        harness
            .handle
            .send(LspCommand::Restart(Some("rust".to_owned())))
    );
    harness
        .next_matching(|event| match event {
            LspEvent::Status { message, .. } if message.contains("restart") => Some(()),
            _ => None,
        })
        .await;
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    harness.settle().await;

    assert!(harness.handle.send(LspCommand::Status));
    let status = harness
        .next_matching(|event| match event {
            LspEvent::Status { message, .. } => Some(message.clone()),
            _ => None,
        })
        .await;
    assert!(status.contains("ready"), "{status}");
    assert!(!status.contains("stopped"), "{status}");
}

#[tokio::test]
async fn a_retiring_server_cannot_apply_an_edit_during_shutdown() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 903,
        "method": "workspace/applyEdit",
        "params": {"edit": {"changes": {}}},
    });
    let mut harness = harness(Script::default().push_on_shutdown(request));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    assert!(
        harness
            .handle
            .send(LspCommand::Restart(Some("rust".to_owned())))
    );
    harness
        .next_matching(|event| matches!(event, LspEvent::Restarted { .. }).then_some(()))
        .await;
    harness.settle().await;
    while let Ok(event) = harness.events.try_recv() {
        assert!(!matches!(event, LspEvent::ApplyEdit { .. }), "{event:?}");
    }
}

#[tokio::test]
async fn an_unknown_server_request_is_answered_rather_than_ignored() {
    // A server that blocks on an unanswered request would look like a hang.
    let script = Script::default().push(json!({
        "jsonrpc": "2.0",
        "id": 900,
        "method": "window/workDoneProgress/create",
        "params": {"token": "x"},
    }));
    let mut harness = harness(script);
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    harness.settle().await;

    let answered = harness
        .sent
        .lock()
        .unwrap()
        .iter()
        .any(|message| message.get("id") == Some(&json!(900)));
    assert!(answered, "the server request went unanswered");
}

#[tokio::test]
async fn invalid_json_rpc_requests_are_rejected_before_dispatch() {
    let invalid = json!({
        "id": 901,
        "method": "workspace/applyEdit",
        "params": {"edit": {"changes": {}}},
    });
    let mut harness = harness(Script::default().push(invalid));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    harness.settle().await;

    let rejected = harness.sent.lock().unwrap().iter().any(|message| {
        message.get("id") == Some(&json!(901)) && message["error"]["code"] == json!(-32600)
    });
    assert!(rejected, "invalid request was not rejected");
}

#[tokio::test]
async fn malformed_workspace_edits_receive_an_explicit_refusal() {
    let invalid = json!({
        "jsonrpc": "2.0",
        "id": 905,
        "method": "workspace/applyEdit",
        "params": {"edit": "not a workspace edit"},
    });
    let mut harness = harness(Script::default().push(invalid));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    let refused = harness
        .wait_for_sent(|message| {
            message.get("id") == Some(&json!(905))
                && message["result"]["applied"] == Value::Bool(false)
        })
        .await;
    assert_eq!(refused["result"]["applied"], Value::Bool(false));
}

#[tokio::test]
async fn apply_edit_before_initialization_is_answered_without_editor_state() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 902,
        "method": "workspace/applyEdit",
        "params": {"edit": {"changes": {}}},
    });
    let mut harness = harness(Script::default().push_before_initialize(request));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    harness.settle().await;

    let refused = harness.sent.lock().unwrap().iter().any(|message| {
        message.get("id") == Some(&json!(902)) && message["result"]["applied"] == Value::Bool(false)
    });
    assert!(refused, "pre-ready workspace edit was left unanswered");
}

#[tokio::test]
async fn apply_edit_reaches_the_editor_and_is_acknowledged() {
    let script = Script::default().push(json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "workspace/applyEdit",
        "params": {
            "edit": {
                "changes": {
                    path_to_uri(std::path::Path::new("/tmp/runyte-lsp/a.rs")).unwrap().as_str(): [{
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 2},
                        },
                        "newText": "pub fn",
                    }],
                },
            },
        },
    }));
    let mut harness = harness(script);
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;

    let (generation, id, edits) = harness
        .next_matching(|event| match event {
            LspEvent::ApplyEdit {
                generation,
                id,
                edits,
                ..
            } => Some((*generation, id.clone(), edits.clone())),
            _ => None,
        })
        .await;
    assert_eq!(id, json!(42));
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].path, PathBuf::from("/tmp/runyte-lsp/a.rs"));

    assert!(harness.handle.send(LspCommand::EditApplied {
        language: "rust".to_owned(),
        generation,
        id: json!(42),
        applied: true,
    }));
    harness.settle().await;
    let acknowledged = harness
        .sent
        .lock()
        .unwrap()
        .iter()
        .any(|message| message.get("id") == Some(&json!(42)));
    assert!(acknowledged, "workspace/applyEdit was never answered");
}

#[tokio::test]
async fn a_duplicate_outstanding_server_request_id_is_rejected() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 55,
        "method": "workspace/applyEdit",
        "params": {"edit": {"changes": {}}},
    });
    let mut harness = harness(Script::default().push(request.clone()).push(request));
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    let _ = harness
        .next_matching(|event| matches!(event, LspEvent::ApplyEdit { .. }).then_some(()))
        .await;
    harness.settle().await;

    let rejected = harness.sent.lock().unwrap().iter().any(|message| {
        message.get("id") == Some(&json!(55)) && message["error"]["code"] == json!(-32600)
    });
    assert!(rejected, "the duplicate server request was not rejected");
}

#[tokio::test]
async fn shutdown_stops_the_manager() {
    let mut harness = harness(Script::default());
    assert!(harness.handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));
    harness.ready().await;
    assert!(harness.handle.send(LspCommand::Shutdown));
    harness.settle().await;
    let sent = harness.sent.lock().unwrap().clone();
    let shutdown = sent
        .iter()
        .position(|message| message.get("method") == Some(&json!("shutdown")))
        .expect("shutdown request");
    let exit = sent
        .iter()
        .position(|message| message.get("method") == Some(&json!("exit")))
        .expect("exit notification");
    assert!(shutdown < exit, "exit must follow the shutdown response");
    // The manager has stopped, so its command queue no longer accepts work.
    // Sending must fail rather than block.
    let mut refused = false;
    for _ in 0..8 {
        if !harness.handle.send(LspCommand::Status) {
            refused = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(refused, "the handle kept accepting work after shutdown");
}

// -- Opt-in, against a real server -----------------------------------------

/// Drives a real `rust-analyzer` over this repository: handshake, document
/// sync, diagnostics, and a goto-definition that has to resolve through the
/// crate's own source.
///
/// Requires `rust-analyzer` on PATH and `RUNYTE_SMOKE_LSP=1`.
#[tokio::test]
#[ignore = "requires RUNYTE_SMOKE_LSP=1 and rust-analyzer on PATH"]
async fn rust_analyzer_answers_a_real_goto_definition() {
    if std::env::var("RUNYTE_SMOKE_LSP").ok().as_deref() != Some("1") {
        return;
    }
    let root = std::env::current_dir().unwrap();
    let file = root.join("src/lsp/diagnostics.rs");
    let text = std::fs::read_to_string(&file).unwrap();
    let (handle, mut events) = runyte::lsp::spawn(LspConfig::default(), root);
    assert!(handle.send(LspCommand::Ensure {
        language: "rust".to_owned()
    }));

    let encoding = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            match events.recv().await {
                Some(LspEvent::Ready { encoding, .. }) => return encoding,
                Some(LspEvent::Stopped { message, .. }) => panic!("{message}"),
                Some(_) => continue,
                None => panic!("the manager stopped before the handshake"),
            }
        }
    })
    .await
    .expect("rust-analyzer did not finish its handshake in two minutes");

    assert!(handle.send(LspCommand::Open {
        language: "rust".to_owned(),
        path: file.clone(),
        version: 1,
        text: text.clone(),
    }));

    // `DiagnosticStore` is declared in this file and used again in
    // `impl DiagnosticStore`; asking about the use must resolve to the
    // declaration.
    let document = runyte::text::Text::from_str(&text);
    let declaration = text
        .find("pub struct DiagnosticStore")
        .expect("the fixture must declare DiagnosticStore");
    let use_byte = declaration
        + 1
        + text[declaration + 1..]
            .find("DiagnosticStore")
            .expect("the fixture must use DiagnosticStore again");
    let use_site = text[..use_byte].chars().count();
    let position = runyte::lsp::to_lsp_position(&document, use_site, encoding);

    // rust-analyzer answers immediately but only meaningfully once it has
    // indexed the crate, so ask until it has something rather than racing it.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    let mut token = 0;
    let locations = loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "rust-analyzer never resolved a definition"
        );
        token += 1;
        assert!(handle.send(LspCommand::Request {
            token,
            language: "rust".to_owned(),
            path: file.clone(),
            kind: Box::new(RequestKind::Definition(position)),
        }));
        let response = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                match events.recv().await {
                    Some(LspEvent::Response { response, .. }) => return response,
                    Some(LspEvent::Stopped { message, .. }) => panic!("{message}"),
                    Some(_) => continue,
                    None => panic!("the manager stopped before answering"),
                }
            }
        })
        .await
        .expect("rust-analyzer stopped answering");
        match response {
            Response::Locations(locations) if !locations.is_empty() => break locations,
            // Not indexed yet.
            Response::Locations(_) | Response::Empty => {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            other => panic!("unexpected response: {other:?}"),
        }
    };

    assert_eq!(locations[0].path, file);
    let (from, _) = runyte::lsp::from_lsp_range(&document, locations[0].range, encoding);
    assert_eq!(
        document.slice_string(from, from + "DiagnosticStore".chars().count()),
        "DiagnosticStore",
        "the definition must land on the declared name"
    );
    handle.send(LspCommand::Shutdown);
}
