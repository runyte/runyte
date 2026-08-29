// SPDX-License-Identifier: MPL-2.0
#![cfg(unix)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use runyte::{
    app::FrameGeometry,
    input::{InputEvent, KeyCode, KeyStroke, Modifiers},
    layout::Rect,
    protocol::{HostFrame, SnapshotRow},
    workspace::ABBREVIATED_WORKSPACE_ID,
    workspace::lifecycle::{HostStartup, start_detached_host},
    workspace::transport::{ClientRequest, HostResponse, LocalClient, LocalEndpoint},
};

/// A private runtime directory for every Runyte process this test binary
/// spawns. Tests must not publish host endpoints into the person's real
/// `XDG_RUNTIME_DIR`, which `LocalEndpoint::discover` prefers by default.
/// One directory per test binary is enough: each test uses a distinct
/// project root, and the endpoint key is derived from that root.
fn test_runtime_dir() -> &'static Path {
    static RUNTIME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            use std::os::unix::fs::PermissionsExt;
            // Unix socket paths are capped near 100 bytes and the endpoint
            // adds "/runyte/<32 hex>/workspace.sock" below this, so the base
            // name has to stay short.
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                % 1_000_000_007;
            let path = std::env::temp_dir().join(format!("ryt-{}-{unique}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            path
        })
        .as_path()
}

fn test_cache_dir() -> &'static Path {
    static CACHE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            use std::os::unix::fs::PermissionsExt;
            let path = test_runtime_dir().join("cache");
            fs::create_dir_all(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            path
        })
        .as_path()
}

fn isolated_runyte(executable: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(executable);
    command
        .env(
            "RUNYTE_ALL_HOSTS_DIR",
            test_runtime_dir().join("runyte/all-hosts"),
        )
        .env("RUNYTE_TEST_SUPERVISOR_PID", std::process::id().to_string());
    command
}

fn bundled_runyte() -> Command {
    isolated_runyte(Path::new(env!("CARGO_BIN_EXE_runyte")))
}

struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn git(root: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success());
}

fn run_cli(root: &Path, arguments: &[&str]) -> std::process::Output {
    bundled_runyte()
        .args(arguments)
        .current_dir(root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .output()
        .unwrap()
}

fn assert_cli_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "command failed with {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn project() -> PathBuf {
    static NEXT_PROJECT: AtomicU64 = AtomicU64::new(0);
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = NEXT_PROJECT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "runyte-persistent-host-{}-{unique}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("note.txt"), "base\n").unwrap();
    git(&root, &["init", "--quiet"]);
    git(&root, &["config", "user.name", "Runyte Test"]);
    git(&root, &["config", "user.email", "runyte@example.invalid"]);
    git(&root, &["add", "note.txt"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    root.canonicalize().unwrap()
}

/// A project directory with neither marker discovery looks for, standing in
/// for a plain directory a person confirmed at the non-Git prompt.
fn plain_project() -> PathBuf {
    let root = project();
    fs::remove_dir_all(root.join(".git")).unwrap();
    root
}

const HOST_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const EDITOR_STATE_TIMEOUT: Duration = Duration::from_secs(30);
const EDITOR_STATE_POLL: Duration = Duration::from_millis(250);
const TERMINAL_STATE_TIMEOUT: Duration = Duration::from_secs(30);
const TERMINAL_STATE_POLL: Duration = Duration::from_millis(250);

async fn response(client: &mut LocalClient) -> HostResponse {
    tokio::time::timeout(HOST_RESPONSE_TIMEOUT, client.recv())
        .await
        .expect("host response timed out")
        .unwrap()
        .expect("host disconnected")
}

async fn semantic_response_after(
    client: &mut LocalClient,
    mut first: Option<HostResponse>,
    waiting_for: &str,
) -> HostResponse {
    tokio::time::timeout(HOST_RESPONSE_TIMEOUT, async {
        loop {
            let response = match first.take() {
                Some(response) => response,
                None => client
                    .recv()
                    .await
                    .unwrap_or_else(|error| {
                        panic!("host response failed while {waiting_for}: {error}")
                    })
                    .unwrap_or_else(|| panic!("host disconnected while {waiting_for}")),
            };
            if !matches!(
                response,
                HostResponse::Frame { .. } | HostResponse::TerminalDamage { .. }
            ) {
                return response;
            }
        }
    })
    .await
    .unwrap_or_else(|error| panic!("host response timed out while {waiting_for}: {error}"))
}

async fn await_detached(
    client: &mut LocalClient,
    first: Option<HostResponse>,
    waiting_for: &str,
) -> Option<Vec<u8>> {
    match semantic_response_after(client, first, waiting_for).await {
        HostResponse::Detached { directory_bytes } => directory_bytes,
        response => panic!("expected host detach while {waiting_for}, got {response:?}"),
    }
}

async fn detach(client: &mut LocalClient, waiting_for: &str) -> Option<Vec<u8>> {
    client.send(&ClientRequest::Detach).await.unwrap();
    await_detached(client, None, waiting_for).await
}

async fn connect_interactive_when_available(
    endpoint: &LocalEndpoint,
    geometry: FrameGeometry,
) -> (LocalClient, HostResponse) {
    let deadline = Instant::now() + EDITOR_STATE_TIMEOUT;
    loop {
        let last_observation = match LocalClient::connect(endpoint, geometry, true).await {
            Ok(mut client) => {
                match tokio::time::timeout(HOST_RESPONSE_TIMEOUT, client.recv()).await {
                    Ok(Ok(Some(welcome @ HostResponse::Welcome { .. }))) => {
                        return (client, welcome);
                    }
                    Ok(Ok(Some(response))) => format!("host replied {response:?}"),
                    Ok(Ok(None)) => "host disconnected".to_owned(),
                    Ok(Err(error)) => format!("receive failed: {error:#}"),
                    Err(error) => format!("welcome timed out: {error}"),
                }
            }
            Err(error) => format!("connect failed: {error:#}"),
        };
        assert!(
            Instant::now() < deadline,
            "interactive attachment was not released after {EDITOR_STATE_TIMEOUT:?}: \
             {last_observation}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_session_preview(
    client: &mut LocalClient,
    waiting_for: &str,
    matches: impl Fn(&runyte::protocol::SessionPreview) -> bool,
) -> runyte::protocol::SessionPreview {
    let deadline = Instant::now() + TERMINAL_STATE_TIMEOUT;
    loop {
        client.send(&ClientRequest::SessionPreview).await.unwrap();
        let preview = match response(client).await {
            HostResponse::SessionPreview { preview } => preview,
            response => panic!("expected session preview while {waiting_for}, got {response:?}"),
        };
        if matches(&preview) {
            return preview;
        }
        assert!(
            Instant::now() < deadline,
            "session preview did not show {waiting_for} after {TERMINAL_STATE_TIMEOUT:?}; \
             last preview: {preview:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn shutdown(client: &mut LocalClient, request: ClientRequest) {
    client.send(&request).await.unwrap();
    let response = tokio::time::timeout(HOST_RESPONSE_TIMEOUT, async {
        loop {
            match client.recv().await.unwrap() {
                Some(HostResponse::Frame { .. } | HostResponse::TerminalDamage { .. }) => {}
                response => return response,
            }
        }
    })
    .await
    .expect("host shutdown timed out");
    assert!(
        matches!(response, Some(HostResponse::ShuttingDown) | None),
        "expected host shutdown, got {response:?}"
    );
}

async fn send_input(client: &mut LocalClient, event: impl Into<InputEvent>) -> HostResponse {
    let event: InputEvent = event.into();
    client
        .send(&ClientRequest::Input {
            event: event.into(),
            repeated: false,
        })
        .await
        .unwrap();
    response(client).await
}

fn geometry() -> FrameGeometry {
    FrameGeometry {
        screen: Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        },
        editor: Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 22,
        },
        status: Rect {
            x: 0,
            y: 22,
            width: 80,
            height: 1,
        },
        message: Rect {
            x: 0,
            y: 23,
            width: 80,
            height: 1,
        },
    }
}

async fn wait_for_endpoint(child: &mut ChildGuard, endpoint: &LocalEndpoint) -> bool {
    for _ in 0..100 {
        if endpoint.metadata().exists() {
            return true;
        }
        if child.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            let output = child.0.take().unwrap().wait_with_output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Operation not permitted") {
                return false;
            }
            panic!("host exited before endpoint discovery: {}", stderr);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("host endpoint was not published");
}

fn process_is_running(pid: u32) -> bool {
    // SAFETY: signal zero only asks the kernel whether this positive process
    // identifier is still observable; it does not deliver a signal.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

async fn wait_for_process_exit(pid: u32) {
    let deadline = Instant::now() + HOST_RESPONSE_TIMEOUT;
    while process_is_running(pid) {
        assert!(
            Instant::now() < deadline,
            "host process {pid} remained live after its shutdown acknowledgement"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn frame_text(response: &HostResponse) -> String {
    let HostResponse::Frame { frame } = response else {
        panic!("expected frame, got {response:?}")
    };
    editor_frame_text(frame)
}

fn editor_frame_text(frame: &HostFrame) -> String {
    frame
        .editor
        .panes
        .iter()
        .flat_map(|pane| &pane.rows)
        .filter_map(|row| match row {
            SnapshotRow::Text(row) => Some(
                row.runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
            ),
            SnapshotRow::Placeholder | SnapshotRow::Padding | SnapshotRow::Filler => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn frame_diagnostic(frame: Option<&HostFrame>) -> String {
    frame.map_or_else(
        || "last complete frame: none".to_owned(),
        |frame| {
            format!(
                "last complete frame id: {:?}, active buffer: {:?}, revision: {:?}, text:\n{}",
                frame.id,
                frame.active_buffer,
                frame.active_revision,
                editor_frame_text(frame),
            )
        },
    )
}

/// Waits for an observable editor transition without attributing a frame to
/// the input that happened to precede it on the transport.
///
/// Visual responses are asynchronous and replaceable. The response already
/// received by the caller can therefore be current, or it can be an older
/// frame or terminal damage. Resynchronization converges either case on a
/// complete snapshot while the absolute deadline bounds the whole state wait.
async fn wait_for_editor_frame(
    client: &mut LocalClient,
    first: HostResponse,
    waiting_for: &str,
    matches: impl Fn(&HostFrame) -> bool,
) -> HostFrame {
    let deadline = Instant::now() + EDITOR_STATE_TIMEOUT;
    let mut pending = Some(first);
    let mut last_complete = None;
    let mut next_resynchronize = Instant::now();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "editor state timed out after {EDITOR_STATE_TIMEOUT:?} while {waiting_for}; {}",
            frame_diagnostic(last_complete.as_ref()),
        );

        let message = if let Some(message) = pending.take() {
            message
        } else {
            match tokio::time::timeout(remaining.min(EDITOR_STATE_POLL), client.recv()).await {
                Ok(Ok(Some(message))) => message,
                Ok(Ok(None)) => panic!(
                    "host disconnected while {waiting_for}; {}",
                    frame_diagnostic(last_complete.as_ref()),
                ),
                Ok(Err(error)) => panic!(
                    "host protocol failed while {waiting_for}: {error}; {}",
                    frame_diagnostic(last_complete.as_ref()),
                ),
                Err(_) if Instant::now() < deadline => {
                    client.send(&ClientRequest::Resynchronize).await.unwrap();
                    next_resynchronize = Instant::now() + EDITOR_STATE_POLL;
                    continue;
                }
                Err(_) => panic!(
                    "editor state timed out after {EDITOR_STATE_TIMEOUT:?} while {waiting_for}; {}",
                    frame_diagnostic(last_complete.as_ref()),
                ),
            }
        };

        match message {
            HostResponse::Frame { frame } => {
                if matches(&frame) {
                    return *frame;
                }
                last_complete = Some(*frame);
            }
            HostResponse::TerminalDamage { .. } => {}
            response => panic!(
                "unexpected host response while {waiting_for}: {response:?}; {}",
                frame_diagnostic(last_complete.as_ref()),
            ),
        }

        if Instant::now() >= next_resynchronize {
            client.send(&ClientRequest::Resynchronize).await.unwrap();
            next_resynchronize = Instant::now() + EDITOR_STATE_POLL;
        }
    }
}

fn terminal_frame_text(response: &HostResponse) -> String {
    let HostResponse::Frame { frame } = response else {
        return String::new();
    };
    frame
        .editor
        .panes
        .iter()
        .filter_map(|pane| pane.terminal.as_ref())
        .flat_map(|terminal| &terminal.rows)
        .map(|row| {
            row.iter()
                .filter(|cell| cell.width != 0)
                .map(|cell| cell.character)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn terminal_wire_frame_text(frame: &runyte::protocol::HostFrame) -> String {
    terminal_frame_text(&HostResponse::Frame {
        frame: Box::new(frame.clone()),
    })
}

async fn frame_containing(
    client: &mut LocalClient,
    current: &mut Option<runyte::protocol::HostFrame>,
    damage_count: &mut usize,
    needle: &str,
) -> runyte::protocol::HostFrame {
    frame_matching(client, current, damage_count, |frame| {
        terminal_wire_frame_text(frame).contains(needle)
    })
    .await
    .unwrap_or_else(|| {
        let last_frame = current.as_ref().map(|frame| frame.id);
        let last_screen = current
            .as_ref()
            .map(terminal_wire_frame_text)
            .unwrap_or_else(|| "<no complete terminal frame>".to_owned());
        panic!(
            "terminal frame never contained {needle:?}; last frame: {last_frame:?}; \
             damage messages: {damage_count}; last screen:\n{last_screen}"
        )
    })
}

/// Waits for terminal-owned state without weakening the host response budget.
///
/// A loaded scheduler can delay the external program even while the host is
/// responsive. Polling with `Resynchronize` makes that distinction observable:
/// every poll must still receive a host response within five seconds, while
/// the terminal process gets a separate bounded deadline to change its state.
async fn frame_matching(
    client: &mut LocalClient,
    current: &mut Option<runyte::protocol::HostFrame>,
    damage_count: &mut usize,
    matches: impl Fn(&runyte::protocol::HostFrame) -> bool,
) -> Option<runyte::protocol::HostFrame> {
    let deadline = Instant::now() + TERMINAL_STATE_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let message =
            match tokio::time::timeout(remaining.min(TERMINAL_STATE_POLL), client.recv()).await {
                Ok(response) => response
                    .unwrap()
                    .expect("host disconnected while waiting for terminal state"),
                Err(_) => {
                    client.send(&ClientRequest::Resynchronize).await.unwrap();
                    response(client).await
                }
            };
        match message {
            HostResponse::Frame { frame } => *current = Some(*frame),
            HostResponse::TerminalDamage { damage } => {
                *damage_count += 1;
                if !current.as_mut().is_some_and(|frame| damage.apply(frame)) {
                    client.send(&ClientRequest::Resynchronize).await.unwrap();
                }
            }
            HostResponse::Error { message } => {
                panic!("host rejected a request while waiting for terminal state: {message}")
            }
            HostResponse::Refused { message } => {
                panic!("host refused a request while waiting for terminal state: {message}")
            }
            response => {
                panic!("unexpected host response while waiting for terminal state: {response:?}")
            }
        }
        if current.as_ref().is_some_and(&matches) {
            return current.clone();
        }
    }
}

async fn next_complete_frame(client: &mut LocalClient) -> runyte::protocol::HostFrame {
    loop {
        match response(client).await {
            HostResponse::Frame { frame } => return *frame,
            HostResponse::TerminalDamage { .. } => {
                client.send(&ClientRequest::Resynchronize).await.unwrap();
            }
            _ => {}
        }
    }
}

/// Returns a current semantic frame after startup services have settled.
///
/// A frame is an optimistic-concurrency token. Using the first one while Git
/// discovery is still running lets its completion make an immediate command
/// stale before the host reads it.
async fn next_idle_frame(client: &mut LocalClient) -> runyte::protocol::HostFrame {
    loop {
        let frame = next_complete_frame(client).await;
        if frame.editor.status.long_running_action.is_none() {
            return frame;
        }
    }
}

/// Invokes a semantic command against the newest frame until the optimistic
/// concurrency check accepts it. Ambient services may publish another frame
/// after the caller observed one but before the host reads the request.
async fn invoke_when_current(
    client: &mut LocalClient,
    command: &str,
    frame: runyte::protocol::HostFrame,
) -> HostResponse {
    invoke_with_argument_when_current(client, command, None, frame).await
}

async fn invoke_with_argument_when_current(
    client: &mut LocalClient,
    command: &str,
    argument: Option<&str>,
    mut frame: runyte::protocol::HostFrame,
) -> HostResponse {
    let deadline = Instant::now() + EDITOR_STATE_TIMEOUT;
    loop {
        client
            .send(&ClientRequest::Invoke {
                command: runyte::protocol::CommandRequest {
                    name: command.to_owned(),
                    argument: argument.map(str::to_owned),
                    frame: frame.id,
                    buffer: frame.active_buffer,
                    revision: frame.active_revision,
                },
            })
            .await
            .unwrap();
        loop {
            match response(client).await {
                HostResponse::Frame { .. } | HostResponse::TerminalDamage { .. } => {}
                HostResponse::Error { message } if message.starts_with("stale editor frame:") => {
                    assert!(
                        Instant::now() < deadline,
                        "editor frames stayed stale while invoking {command}: {message}"
                    );
                    client.send(&ClientRequest::Resynchronize).await.unwrap();
                    frame = next_complete_frame(client).await;
                    break;
                }
                response @ (HostResponse::CommandResult { .. } | HostResponse::Detached { .. }) => {
                    return response;
                }
                response => panic!("expected {command} result, got {response:?}"),
            }
        }
    }
}

fn selection_count(response: &HostResponse) -> usize {
    let HostResponse::Frame { frame } = response else {
        panic!("expected frame, got {response:?}")
    };
    frame.editor.status.selection_count
}

fn unread_errors(response: &HostResponse) -> usize {
    let HostResponse::Frame { frame } = response else {
        panic!("expected frame, got {response:?}")
    };
    frame.editor.status.notification_counts.errors
}

#[tokio::test]
async fn detach_reattach_preserves_live_editor_and_refuses_a_second_tui() {
    let root = project();
    let executable = env!("CARGO_BIN_EXE_runyte");
    let child = isolated_runyte(executable)
        .arg("--serve")
        .arg("note.txt")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(Some(child));
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    if !wait_for_endpoint(&mut child, &endpoint).await {
        fs::remove_dir_all(root).unwrap();
        return;
    }

    let mut first = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    let host_pid = match response(&mut first).await {
        HostResponse::Welcome { pid, .. } => pid,
        response => panic!("expected welcome, got {response:?}"),
    };
    let _initial = response(&mut first).await;

    let mut second = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut second).await,
        HostResponse::Refused { message } if message.contains("already attached")
    ));

    let _ = send_input(
        &mut first,
        KeyStroke::new(KeyCode::Char('i'), Modifiers::NONE),
    )
    .await;
    let mut edited = send_input(&mut first, InputEvent::Text("word word ".to_owned())).await;
    while !frame_text(&edited).contains("word word base") {
        edited = response(&mut first).await;
    }
    let _ = send_input(&mut first, KeyStroke::new(KeyCode::Escape, Modifiers::NONE)).await;
    let _ = send_input(&mut first, KeyStroke::plain(KeyCode::Char('b'))).await;
    let selections = send_input(&mut first, KeyStroke::plain(KeyCode::Char('*'))).await;
    assert_eq!(selection_count(&selections), 2);
    assert_eq!(
        detach(&mut first, "detaching the edited client").await,
        None
    );

    let mut reattached = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut reattached).await,
        HostResponse::Welcome { pid, .. } if pid == host_pid
    ));
    let frame = response(&mut reattached).await;
    assert!(frame_text(&frame).contains("word word base"));
    assert_eq!(selection_count(&frame), 2);
    assert_eq!(fs::read_to_string(root.join("note.txt")).unwrap(), "base\n");
    let _ = send_input(&mut reattached, KeyStroke::plain(KeyCode::Char('y'))).await;
    drop(reattached);
    let (mut after_disconnect, welcome) =
        connect_interactive_when_available(&endpoint, geometry()).await;
    assert!(matches!(
        welcome,
        HostResponse::Welcome { pid, .. } if pid == host_pid
    ));
    let _ = response(&mut after_disconnect).await;
    let mut pasted = send_input(&mut after_disconnect, KeyStroke::plain(KeyCode::Char('p'))).await;
    while frame_text(&pasted).matches("word").count() < 4 {
        pasted = response(&mut after_disconnect).await;
    }
    assert_eq!(
        detach(&mut after_disconnect, "detaching the reconnected client").await,
        None
    );

    let mut control = LocalClient::connect(&endpoint, geometry(), false)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut control).await,
        HostResponse::Welcome { .. }
    ));
    control.send(&ClientRequest::SessionPreview).await.unwrap();
    let preview = match response(&mut control).await {
        HostResponse::SessionPreview { preview } => preview,
        response => panic!("expected semantic session preview, got {response:?}"),
    };
    assert_eq!(preview.layout_panes, 1);
    assert!(preview.panes[0].active);
    assert!(preview.panes[0].title.ends_with("note.txt"));
    assert!(
        preview.panes[0]
            .lines
            .iter()
            .any(|line| line.contains("word word")),
        "{preview:?}"
    );
    control.send(&ClientRequest::ListBuffers).await.unwrap();
    let dirty = match response(&mut control).await {
        HostResponse::Buffers { buffers } => buffers
            .into_iter()
            .filter(|buffer| buffer.dirty && !buffer.closed)
            .map(|buffer| buffer.id)
            .collect::<Vec<_>>(),
        response => panic!("expected buffers before shutdown, got {response:?}"),
    };
    for buffer in dirty {
        control
            .send(&ClientRequest::SaveBuffer { buffer })
            .await
            .unwrap();
        assert!(matches!(
            response(&mut control).await,
            HostResponse::Saved { buffer: saved, .. } if saved == buffer
        ));
    }
    shutdown(&mut control, ClientRequest::Shutdown).await;
    let status = tokio::task::spawn_blocking(move || child.0.take().unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    assert!(!endpoint.metadata().exists());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn tutorial_persistent_lesson_completes_across_a_real_client_reattachment() {
    let root = project();
    let executable = env!("CARGO_BIN_EXE_runyte");
    let child = isolated_runyte(executable)
        .arg("--serve")
        .arg("note.txt")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(Some(child));
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    if !wait_for_endpoint(&mut child, &endpoint).await {
        fs::remove_dir_all(root).unwrap();
        return;
    }

    let mut first = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut first).await,
        HostResponse::Welcome { .. }
    ));
    let _ = response(&mut first).await;
    let _ = send_input(&mut first, KeyStroke::plain(KeyCode::Char(':'))).await;
    let _ = send_input(&mut first, InputEvent::Text("tutorial sessions".to_owned())).await;
    let tutorial = send_input(&mut first, KeyStroke::plain(KeyCode::Enter)).await;
    let _tutorial = wait_for_editor_frame(
        &mut first,
        tutorial,
        "waiting for the persistent-session tutorial lesson",
        |frame| {
            let text = editor_frame_text(frame);
            text.contains("PERSISTENT SESSIONS") && text.contains("persistent tutorial token")
        },
    )
    .await;

    let _ = send_input(&mut first, KeyStroke::plain(KeyCode::Char(':'))).await;
    let _ = send_input(&mut first, InputEvent::Text("detach".to_owned())).await;
    let first_response = send_input(&mut first, KeyStroke::plain(KeyCode::Enter)).await;
    assert_eq!(
        await_detached(
            &mut first,
            Some(first_response),
            "detaching from the tutorial"
        )
        .await,
        None
    );

    let mut reattached = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut reattached).await,
        HostResponse::Welcome { .. }
    ));
    let completed = response(&mut reattached).await;
    let _completed = wait_for_editor_frame(
        &mut reattached,
        completed,
        "waiting for the tutorial to complete after reattachment",
        |frame| {
            let text = editor_frame_text(frame);
            text.contains("NEXT STEPS") && text.contains("persistent tutorial token")
        },
    )
    .await;

    shutdown(&mut reattached, ClientRequest::ForceShutdown).await;
    let status = tokio::task::spawn_blocking(move || child.0.take().unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn terminal_pid_output_and_input_survive_detach_disconnect_and_reattach() {
    let root = project();
    let executable = env!("CARGO_BIN_EXE_runyte");
    let child = isolated_runyte(executable)
        .arg("--serve")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(Some(child));
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    if !wait_for_endpoint(&mut child, &endpoint).await {
        fs::remove_dir_all(root).unwrap();
        return;
    }

    let mut client = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    let host_pid = match response(&mut client).await {
        HostResponse::Welcome { pid, .. } => pid,
        other => panic!("expected welcome, got {other:?}"),
    };
    let frame = next_idle_frame(&mut client).await;
    let mut damage_count = 0;
    let command = "/bin/sh -c 'printf \"token:%s\\n\" \"$$\"; sleep 0.2; printf \"detached\\n\"; while IFS= read -r line; do printf \"reply:%s\\n\" \"$line\"; done'";
    let outcome =
        invoke_with_argument_when_current(&mut client, "terminal", Some(command), frame).await;
    assert!(
        matches!(outcome, HostResponse::CommandResult { .. }),
        "expected terminal command result, got {outcome:?}"
    );
    let mut current = None;
    let first = frame_containing(&mut client, &mut current, &mut damage_count, "token:").await;
    let token = terminal_wire_frame_text(&first)
        .lines()
        .find(|line| line.contains("token:"))
        .unwrap()
        .trim()
        .to_owned();

    assert_eq!(
        detach(&mut client, "detaching the terminal client").await,
        None
    );
    let mut detached_control = LocalClient::connect(&endpoint, geometry(), false)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut detached_control).await,
        HostResponse::Welcome { .. }
    ));
    wait_for_session_preview(
        &mut detached_control,
        "terminal output produced while detached",
        |preview| {
            preview
                .panes
                .iter()
                .flat_map(|pane| &pane.lines)
                .any(|line| line.contains("detached"))
        },
    )
    .await;
    drop(detached_control);

    let mut resized = geometry();
    resized.screen.width = 96;
    resized.editor.width = 96;
    resized.status.width = 96;
    resized.message.width = 96;
    let mut reattached = LocalClient::connect(&endpoint, resized, true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut reattached).await,
        HostResponse::Welcome { pid, .. } if pid == host_pid
    ));
    let mut current = None;
    let detached =
        frame_containing(&mut reattached, &mut current, &mut damage_count, "detached").await;
    let detached_text = terminal_wire_frame_text(&detached);
    assert!(detached_text.contains(&token));

    reattached
        .send(&ClientRequest::Input {
            event: InputEvent::Text("hello\n".to_owned()).into(),
            repeated: false,
        })
        .await
        .unwrap();
    let replied = frame_containing(
        &mut reattached,
        &mut current,
        &mut damage_count,
        "reply:hello",
    )
    .await;
    assert!(terminal_wire_frame_text(&replied).contains(&token));
    assert!(damage_count > 0, "terminal output never used row damage");

    // Losing the socket is a detach too. The same terminal remains protected.
    drop(reattached);
    let mut control = LocalClient::connect(&endpoint, geometry(), false)
        .await
        .unwrap();
    let _ = response(&mut control).await;
    let deadline = Instant::now() + EDITOR_STATE_TIMEOUT;
    loop {
        control.send(&ClientRequest::Health).await.unwrap();
        let health = response(&mut control).await;
        if matches!(
            health,
            HostResponse::Health {
                interactive_attached: false,
                live_terminals: 1,
                terminal_sessions: 1,
                ..
            }
        ) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "socket loss was not observed after {EDITOR_STATE_TIMEOUT:?}; last health: {health:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(control);

    let refused = run_cli(&root, &["--session-stop"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("live terminal"));
    let refused_restart = run_cli(&root, &["--session-restart"]);
    assert!(!refused_restart.status.success());
    assert!(String::from_utf8_lossy(&refused_restart.stderr).contains("live terminal"));
    assert_cli_success(&run_cli(&root, &["--session-stop", "--force"]));
    assert!(child.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn hidden_terminal_output_while_detached_is_unread_after_reattach() {
    let root = project();
    let child = bundled_runyte()
        .arg("--serve")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(Some(child));
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    if !wait_for_endpoint(&mut child, &endpoint).await {
        fs::remove_dir_all(root).unwrap();
        return;
    }

    let mut client = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut client).await,
        HostResponse::Welcome { .. }
    ));
    let initial = next_idle_frame(&mut client).await;
    let terminal_command =
        "/bin/sh -c 'sleep 0.3; printf \"hidden-unread\\033]2;hidden-ready\\007\"; sleep 30'";
    let outcome =
        invoke_with_argument_when_current(&mut client, "terminal", Some(terminal_command), initial)
            .await;
    assert!(
        matches!(outcome, HostResponse::CommandResult { .. }),
        "expected terminal command result, got {outcome:?}"
    );
    let mut current = None;
    let mut damage_count = 0;
    let terminal_frame = frame_matching(&mut client, &mut current, &mut damage_count, |frame| {
        frame
            .editor
            .panes
            .iter()
            .any(|pane| pane.terminal.is_some())
    })
    .await
    .expect("terminal pane never opened");
    // Opening the pane's document hides the terminal without making this
    // persistence test depend on the 1.2-second `Space t q` prefix deadline.
    // Prefix expiry under scheduler contention belongs to key-hint behavior;
    // the state needed here is only a live terminal with no attached view.
    let outcome =
        invoke_with_argument_when_current(&mut client, "open", Some("note.txt"), terminal_frame)
            .await;
    assert!(
        matches!(outcome, HostResponse::CommandResult { .. }),
        "expected open command result, got {outcome:?}"
    );
    current = None;
    frame_matching(&mut client, &mut current, &mut damage_count, |frame| {
        frame
            .editor
            .panes
            .iter()
            .all(|pane| pane.terminal.is_none())
    })
    .await
    .expect("terminal pane never became hidden");
    assert_eq!(
        detach(&mut client, "detaching with a hidden terminal").await,
        None
    );
    let mut detached_control = LocalClient::connect(&endpoint, geometry(), false)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut detached_control).await,
        HostResponse::Welcome { .. }
    ));
    wait_for_session_preview(
        &mut detached_control,
        "the hidden terminal's processed output marker",
        |preview| {
            preview
                .other_resources
                .iter()
                .any(|resource| resource.contains("hidden-ready"))
        },
    )
    .await;
    drop(detached_control);

    let mut reattached = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut reattached).await,
        HostResponse::Welcome { .. }
    ));
    let frame = next_idle_frame(&mut reattached).await;
    let outcome = invoke_when_current(&mut reattached, "terminals", frame).await;
    assert!(
        matches!(outcome, HostResponse::CommandResult { .. }),
        "expected terminals command result, got {outcome:?}"
    );
    let first = response(&mut reattached).await;
    let manager = wait_for_editor_frame(
        &mut reattached,
        first,
        "waiting for the terminal manager",
        |frame| {
            frame
                .overlays
                .iter()
                .any(|overlay| overlay.title == "Terminals")
        },
    )
    .await;
    let terminals = manager
        .overlays
        .iter()
        .find(|overlay| overlay.title == "Terminals")
        .unwrap();
    assert!(
        terminals
            .rows
            .iter()
            .any(|row| row.detail.contains("unread")),
        "detached output was marked viewed without an attached observer"
    );

    // Keep a visual response deliberately in flight before shutdown. The
    // interactive protocol multiplexes replaceable frames with semantic
    // replies, so shutdown must drain this frame before its acknowledgement.
    reattached
        .send(&ClientRequest::Resynchronize)
        .await
        .unwrap();
    shutdown(&mut reattached, ClientRequest::ForceShutdown).await;
    let status = tokio::task::spawn_blocking(move || child.0.take().unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn detach_reattach_preserves_notification_history_and_unread_state() {
    let root = project();
    let executable = env!("CARGO_BIN_EXE_runyte");
    let child = isolated_runyte(executable)
        .arg("--serve")
        .arg("note.txt")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(Some(child));
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    if !wait_for_endpoint(&mut child, &endpoint).await {
        fs::remove_dir_all(root).unwrap();
        return;
    }

    let mut first = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut first).await,
        HostResponse::Welcome { .. }
    ));
    let _ = response(&mut first).await;
    let missing = root.join("directory-that-does-not-exist");
    let _ = send_input(&mut first, KeyStroke::plain(KeyCode::Char(':'))).await;
    let _ = send_input(
        &mut first,
        InputEvent::Text(format!("cd {}", missing.display())),
    )
    .await;
    let mut failed = send_input(&mut first, KeyStroke::plain(KeyCode::Enter)).await;
    while unread_errors(&failed) == 0 {
        failed = response(&mut first).await;
    }
    assert_eq!(
        detach(&mut first, "detaching after a failed command").await,
        None
    );

    let mut reattached = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut reattached).await,
        HostResponse::Welcome { .. }
    ));
    let retained = response(&mut reattached).await;
    assert_eq!(unread_errors(&retained), 1);

    let _ = send_input(&mut reattached, KeyStroke::plain(KeyCode::Char(':'))).await;
    let _ = send_input(&mut reattached, InputEvent::Text("not".to_owned())).await;
    let notifications = send_input(&mut reattached, KeyStroke::plain(KeyCode::Enter)).await;
    assert!(frame_text(&notifications).contains("ERROR"));
    assert_eq!(unread_errors(&notifications), 0);
    assert_eq!(
        detach(&mut reattached, "detaching after reading notifications").await,
        None
    );

    let mut control = LocalClient::connect(&endpoint, geometry(), false)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut control).await,
        HostResponse::Welcome { .. }
    ));
    shutdown(&mut control, ClientRequest::Shutdown).await;
    let status = tokio::task::spawn_blocking(move || child.0.take().unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn killed_host_leaves_files_intact_and_its_endpoint_is_recoverable() {
    let root = project();
    let executable = env!("CARGO_BIN_EXE_runyte");
    let spawn = || {
        isolated_runyte(executable)
            .arg("--serve")
            .arg("note.txt")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let mut first_host = ChildGuard(Some(spawn()));
    if !wait_for_endpoint(&mut first_host, &endpoint).await {
        fs::remove_dir_all(root).unwrap();
        return;
    }
    let mut client = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    let _ = response(&mut client).await;
    let _ = response(&mut client).await;
    let _ = send_input(&mut client, KeyStroke::plain(KeyCode::Char('i'))).await;
    let _ = send_input(&mut client, InputEvent::Text("unsaved ".to_owned())).await;
    drop(client);
    first_host.0.as_mut().unwrap().kill().unwrap();
    first_host.0.as_mut().unwrap().wait().unwrap();
    first_host.0 = None;
    assert_eq!(fs::read_to_string(root.join("note.txt")).unwrap(), "base\n");

    let mut replacement = ChildGuard(Some(spawn()));
    assert!(wait_for_endpoint(&mut replacement, &endpoint).await);
    let mut control = None;
    for _ in 0..100 {
        match LocalClient::connect(&endpoint, geometry(), false).await {
            Ok(client) => {
                control = Some(client);
                break;
            }
            Err(_)
                if replacement
                    .0
                    .as_mut()
                    .unwrap()
                    .try_wait()
                    .unwrap()
                    .is_none() =>
            {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => panic!("replacement host did not recover its endpoint: {error:#}"),
        }
    }
    let mut control = control.expect("replacement host endpoint did not become ready");
    assert!(matches!(
        response(&mut control).await,
        HostResponse::Welcome { .. }
    ));
    shutdown(&mut control, ClientRequest::Shutdown).await;
    let status = tokio::task::spawn_blocking(move || replacement.0.take().unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    assert_eq!(fs::read_to_string(root.join("note.txt")).unwrap(), "base\n");
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn sessions_list_rename_restart_and_resolve_by_id_name_or_directory() {
    let root = project();
    let cwd_file = root.join("shell-cwd");
    fs::write(&cwd_file, []).unwrap();
    let cwd_file = cwd_file.to_str().unwrap();
    let executable = env!("CARGO_BIN_EXE_runyte");
    let spawn = || {
        isolated_runyte(executable)
            .arg("--serve")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let display_id = endpoint.id()[..12].to_owned();
    let listed_id = endpoint.id()[..ABBREVIATED_WORKSPACE_ID].to_owned();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        % 1_000_000_007;
    let first_name = format!("managed-{unique}");
    let second_name = format!("restarted-{unique}");
    let third_name = format!("directory-{unique}");

    let mut original = ChildGuard(Some(spawn()));
    assert!(
        wait_for_endpoint(&mut original, &endpoint).await,
        "host did not become ready"
    );
    assert_cli_success(&run_cli(
        &root,
        &[
            "--cwd-file",
            cwd_file,
            "--session-rename",
            &display_id,
            &first_name,
        ],
    ));
    let listing = run_cli(&root, &["--session-list"]);
    assert_cli_success(&listing);
    let listing = String::from_utf8(listing.stdout).unwrap();
    assert!(listing.contains("ID"));
    assert!(listing.contains("NAME"));
    assert!(listing.contains("DIRECTORY"));
    assert!(listing.contains(&listed_id), "{listing}");
    assert!(!listing.contains(endpoint.id()), "{listing}");
    assert!(listing.contains(&first_name));
    assert!(listing.contains(root.to_string_lossy().as_ref()));

    assert_cli_success(&run_cli(
        &root,
        &["--session-rename", &first_name, &second_name],
    ));
    assert_cli_success(&run_cli(
        &root,
        &[
            "--session-rename",
            root.to_string_lossy().as_ref(),
            &third_name,
        ],
    ));
    assert_cli_success(&run_cli(&root, &["--session-restart", &third_name]));
    let status = original.0.take().unwrap().wait().unwrap();
    assert!(status.success());
    let listing = run_cli(&root, &["--cwd-file", cwd_file, "-l"]);
    assert_cli_success(&listing);
    let listing = String::from_utf8(listing.stdout).unwrap();
    assert!(listing.contains(&third_name));
    assert!(listing.contains(&listed_id), "{listing}");
    assert!(fs::read(cwd_file).unwrap().is_empty());

    // A host can remain live while its catalog rows are lost. Directory
    // selectors should still reach the endpoint owned by the project.
    for registry in [
        test_runtime_dir().join("runyte/hosts"),
        test_cache_dir().join("runyte/hosts"),
    ] {
        let registration = registry.join(format!("{}.json", endpoint.id()));
        let _ = fs::remove_file(registration);
    }

    assert_cli_success(&run_cli(
        &root,
        &["--session-stop", root.to_string_lossy().as_ref()],
    ));
    for _ in 0..200 {
        if !endpoint.metadata().exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(!endpoint.metadata().exists());

    let mut by_name = ChildGuard(Some(spawn()));
    assert!(wait_for_endpoint(&mut by_name, &endpoint).await);
    assert_eq!(
        endpoint.verify_for_connect().unwrap().name.as_deref(),
        Some(third_name.as_str())
    );
    assert_cli_success(&run_cli(&root, &["--session-stop", &third_name]));
    assert!(by_name.0.take().unwrap().wait().unwrap().success());

    let mut by_id = ChildGuard(Some(spawn()));
    assert!(wait_for_endpoint(&mut by_id, &endpoint).await);
    assert_cli_success(&run_cli(&root, &["--session-stop", &display_id]));
    assert!(by_id.0.take().unwrap().wait().unwrap().success());

    let stopped_listing = run_cli(&root, &["--session-list"]);
    assert_cli_success(&stopped_listing);
    let stopped_listing = String::from_utf8(stopped_listing.stdout).unwrap();
    assert!(stopped_listing.contains("stopped"), "{stopped_listing}");
    assert!(
        !stopped_listing.contains('?'),
        "unavailable workspace fields should be blank: {stopped_listing}"
    );

    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_new_workspace_is_listed_and_resolved_by_its_default_directory_name() {
    let root = project();
    let name = root.file_name().unwrap().to_str().unwrap().to_owned();
    let child = bundled_runyte()
        .arg("--serve")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(Some(child));
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    assert!(wait_for_endpoint(&mut child, &endpoint).await);
    assert_eq!(
        endpoint.verify_for_connect().unwrap().name.as_deref(),
        Some(name.as_str())
    );

    let listing = run_cli(&root, &["-l"]);
    assert_cli_success(&listing);
    let listing = String::from_utf8(listing.stdout).unwrap();
    let row = listing
        .lines()
        .find(|line| line.contains(root.to_string_lossy().as_ref()))
        .unwrap_or_else(|| panic!("new workspace missing from listing:\n{listing}"));
    assert_eq!(row.split_whitespace().nth(1), Some(name.as_str()));

    assert_cli_success(&run_cli(&root, &["--session-stop", &name]));
    assert!(child.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn restart_keeps_a_fallback_host_on_its_original_endpoint() {
    let root = project();
    let cache = root.join("xdg-cache");
    let mut original = ChildGuard(Some(
        bundled_runyte()
            .arg("--serve")
            .current_dir(&root)
            .env_remove("XDG_RUNTIME_DIR")
            .env("XDG_CACHE_HOME", &cache)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let endpoint = LocalEndpoint::new(&root.join(".runyte"), &root).unwrap();
    assert!(
        wait_for_endpoint(&mut original, &endpoint).await,
        "fallback host did not become ready"
    );

    let mut duplicate = ChildGuard(Some(
        bundled_runyte()
            .arg("--serve")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", &cache)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let mut duplicate_status = None;
    for _ in 0..200 {
        duplicate_status = duplicate.0.as_mut().unwrap().try_wait().unwrap();
        if duplicate_status.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let status = duplicate_status.expect("second endpoint accepted a duplicate project host");
    assert!(!status.success());
    duplicate.0.take();

    let selector = root.to_string_lossy();
    let restart = bundled_runyte()
        .args(["--session-restart", selector.as_ref()])
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .unwrap();
    assert_cli_success(&restart);
    assert!(original.0.take().unwrap().wait().unwrap().success());
    let replacement_pid = endpoint.verify_for_connect().unwrap().pid;

    let shutdown = bundled_runyte()
        .args(["--session-stop", selector.as_ref()])
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .unwrap();
    assert_cli_success(&shutdown);
    for _ in 0..200 {
        if !endpoint.metadata().exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(!endpoint.metadata().exists());
    // The endpoint is unpublished before the detached process flushes its
    // connections, diagnostic log, and registry cleanup. Wait for that
    // explicit process lifecycle boundary before deleting its project.
    wait_for_process_exit(replacement_pid).await;
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn an_exact_name_wins_over_another_hosts_id_prefix() {
    let named_root = project();
    let prefixed_root = project();
    let endpoint = |root: &Path| {
        LocalEndpoint::discover_with_runtime(&root.join(".runyte"), root, Some(test_runtime_dir()))
            .unwrap()
    };
    let named_endpoint = endpoint(&named_root);
    let prefixed_endpoint = endpoint(&prefixed_root);
    let name = &prefixed_endpoint.id()[..1];
    let spawn = |root: &Path| {
        ChildGuard(Some(
            bundled_runyte()
                .arg("--serve")
                .current_dir(root)
                .env("XDG_RUNTIME_DIR", test_runtime_dir())
                .env("XDG_CACHE_HOME", test_cache_dir())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        ))
    };
    let mut named = spawn(&named_root);
    let mut prefixed = spawn(&prefixed_root);
    assert!(wait_for_endpoint(&mut named, &named_endpoint).await);
    assert!(wait_for_endpoint(&mut prefixed, &prefixed_endpoint).await);
    assert_cli_success(&run_cli(
        &named_root,
        &["--session-rename", named_endpoint.id(), name],
    ));

    assert_cli_success(&run_cli(&named_root, &["--session-stop", name]));
    assert!(named.0.take().unwrap().wait().unwrap().success());
    assert!(prefixed_endpoint.verify_for_connect().is_ok());
    assert_cli_success(&run_cli(
        &prefixed_root,
        &["--session-stop", prefixed_endpoint.id()],
    ));
    assert!(prefixed.0.take().unwrap().wait().unwrap().success());

    fs::remove_dir_all(named_root).unwrap();
    fs::remove_dir_all(prefixed_root).unwrap();
}

#[tokio::test]
async fn an_unusable_cache_registry_falls_back_to_the_runtime_registry() {
    let root = project();
    let unusable_cache = root.join("cache-is-a-file");
    fs::write(&unusable_cache, b"not a directory").unwrap();
    let mut host = ChildGuard(Some(
        bundled_runyte()
            .arg("--serve")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", &unusable_cache)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    assert!(
        wait_for_endpoint(&mut host, &endpoint).await,
        "host did not fall back to its runtime registry"
    );

    let listing = bundled_runyte()
        .arg("--session-list")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", &unusable_cache)
        .output()
        .unwrap();
    assert_cli_success(&listing);
    assert!(
        String::from_utf8(listing.stdout)
            .unwrap()
            .contains(&endpoint.id()[..ABBREVIATED_WORKSPACE_ID])
    );

    let shutdown = bundled_runyte()
        .arg("--session-stop")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", &unusable_cache)
        .output()
        .unwrap();
    assert_cli_success(&shutdown);
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

/// Two callers may ask for a host at the same endpoint at once. The loser's
/// child exits immediately, because the winner already holds the endpoint's
/// identity lock, so a readiness poll that inspected the child before trying to
/// connect would report that race as a failure even though a host is serving.
#[tokio::test]
async fn racing_starts_for_one_workspace_both_reach_the_winning_host() {
    let root = project();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let startup = || {
        HostStartup::new(env!("CARGO_BIN_EXE_runyte"), "raced")
            .with_env("XDG_CACHE_HOME", test_cache_dir())
    };

    let (first, second) = tokio::join!(
        start_detached_host(&endpoint, startup()),
        start_detached_host(&endpoint, startup()),
    );
    let outcome = first.and(second);

    // Shut the host down before asserting, so a failure cannot leave a stray
    // host holding this test's endpoint.
    let shutdown = run_cli(&root, &["--session-stop"]);
    outcome.unwrap();
    assert_cli_success(&shutdown);
    fs::remove_dir_all(root).unwrap();
}

/// `--persistent` means "keep this workspace alive and show its TUI", which is
/// answerable whether or not one is running, so it starts the missing host
/// itself rather than failing at connect. This runs under the default
/// standalone `workspace.mode`, where the start used to be skipped entirely.
///
/// The persistent launch cannot reach its editor here: this test's stdio is a
/// pipe, so entering raw mode fails. That failure comes after the host is
/// started, which is what the assertion is about, and it is also what lets the
/// test run without a pseudoterminal.
#[tokio::test]
async fn persistent_mode_starts_the_missing_workspace_before_it_reaches_a_terminal() {
    let root = project();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();

    let persistent = bundled_runyte()
        .arg("--persistent")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let listing = run_cli(&root, &["--session-list"]);

    // Stop before asserting, so a failing assertion cannot leave a stray host
    // holding this test's endpoint.
    let shutdown = run_cli(&root, &["--session-stop"]);
    assert_cli_success(&listing);
    let listed = String::from_utf8(listing.stdout).unwrap();
    assert!(
        listed.contains(&endpoint.id()[..ABBREVIATED_WORKSPACE_ID]) && listed.contains("running"),
        "--persistent did not leave a running workspace\nlisting: {listed}\nstderr: {}",
        String::from_utf8_lossy(&persistent.stderr)
    );
    assert_cli_success(&shutdown);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn detached_host_keeps_the_requested_editor_directory_below_the_project_root() {
    let root = project();
    let nested = root.join("nested");
    fs::create_dir(&nested).unwrap();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let startup = HostStartup::new(env!("CARGO_BIN_EXE_runyte"), "nested")
        .with_working_directory(&nested)
        .with_env("XDG_CACHE_HOME", test_cache_dir());
    if let Err(error) = start_detached_host(&endpoint, startup).await {
        if format!("{error:#}").contains("Operation not permitted") {
            fs::remove_dir_all(root).unwrap();
            return;
        }
        panic!("detached host did not start: {error:#}");
    }

    let mut client = LocalClient::connect_with_handoff(&endpoint, geometry(), true, true)
        .await
        .unwrap();
    let _ = response(&mut client).await;
    let frame = next_idle_frame(&mut client).await;
    let outcome = invoke_when_current(&mut client, "quit-here", frame).await;
    let detached_directory = match outcome {
        HostResponse::Detached { directory_bytes } => {
            directory_bytes.map(runyte::workspace::transport::decode_path)
        }
        HostResponse::CommandResult { .. } => {
            await_detached(&mut client, None, "waiting for quit-here to detach")
                .await
                .map(runyte::workspace::transport::decode_path)
        }
        response => panic!("expected quit-here result, got {response:?}"),
    };
    assert_eq!(detached_directory, Some(nested));

    let stopped = tokio::time::timeout(HOST_RESPONSE_TIMEOUT, async {
        while endpoint.metadata().exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if stopped.is_err() {
        let _ = run_cli(&root, &["--session-stop", "--force"]);
        panic!(":quit-here did not stop the persistent session");
    }
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn detached_host_rejects_a_working_directory_outside_its_project_before_spawn() {
    let root = project();
    let outside = root.with_extension("outside");
    fs::create_dir(&outside).unwrap();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let startup =
        HostStartup::new(root.join("missing-runyte"), "outside").with_working_directory(&outside);

    let error = start_detached_host(&endpoint, startup).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("workspace host working directory")
    );
    assert!(error.to_string().contains("is outside project root"));
    assert!(!endpoint.metadata().exists());
    assert!(!endpoint.socket().exists());

    fs::remove_dir_all(outside).unwrap();
    fs::remove_dir_all(root).unwrap();
}

/// A workspace whose root cannot be derived from the filesystem — no Git root
/// and no state directory — is resolved once, by the process that has a
/// terminal to ask on. A detached host is spawned with its stdin closed, so
/// rediscovering the project there would reach a prompt nothing can answer.
#[tokio::test]
async fn detached_host_serves_a_project_it_could_not_have_discovered() {
    let root = plain_project();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    assert!(!root.join(".git").exists());
    assert!(!root.join(".runyte").exists());
    let startup = HostStartup::new(env!("CARGO_BIN_EXE_runyte"), "undiscoverable")
        .with_env("XDG_CACHE_HOME", test_cache_dir());
    if let Err(error) = start_detached_host(&endpoint, startup).await {
        if format!("{error:#}").contains("Operation not permitted") {
            fs::remove_dir_all(root).unwrap();
            return;
        }
        panic!("detached host did not start: {error:#}");
    }

    let mut client = LocalClient::connect_with_handoff(&endpoint, geometry(), true, false)
        .await
        .unwrap();
    let _ = response(&mut client).await;
    let frame = response(&mut client).await;
    assert!(
        matches!(frame, HostResponse::Frame { .. }),
        "expected initial frame, got {frame:?}"
    );

    // The selector is given rather than left to the current directory: a
    // management command resolves its own project the same way the host would
    // have, and this one is deliberately standing in a directory that cannot
    // be resolved that way.
    assert_cli_success(&run_cli(&root, &["--session-stop", root.to_str().unwrap()]));
    fs::remove_dir_all(root).unwrap();
}

/// `:quit-here` chooses a directory inside the host, while the file a shell
/// wrapper reads belongs to the client. The directory therefore has to travel
/// in a detach-shaped response even though a successful quit also stops the
/// host, and only for a client that said it can deliver one.
#[tokio::test]
async fn quit_here_reports_its_directory_to_a_handoff_capable_client() {
    let root = project();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/deep.txt"), "deep\n").unwrap();
    let child = bundled_runyte()
        .arg("--serve")
        .arg("nested/deep.txt")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(Some(child));
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    if !wait_for_endpoint(&mut child, &endpoint).await {
        fs::remove_dir_all(root).unwrap();
        return;
    }

    // A client without the capability is told the wrapper is required, and the
    // host stays attached rather than detaching with nothing to report.
    let mut plain = LocalClient::connect_with_handoff(&endpoint, geometry(), true, false)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut plain).await,
        HostResponse::Welcome { .. }
    ));
    let frame = next_idle_frame(&mut plain).await;
    let outcome = match invoke_when_current(&mut plain, "quit-here", frame).await {
        HostResponse::CommandResult { outcome } => outcome,
        response => panic!("expected quit-here command result, got {response:?}"),
    };
    assert!(
        format!("{outcome:?}").contains("runyte()"),
        "unexpected quit-here outcome: {outcome:?}"
    );
    let plain_detached = detach(&mut plain, "detaching the client without handoff").await;
    assert_eq!(plain_detached, None);

    // A capable client receives the directory of the open file, not the root.
    let mut capable = LocalClient::connect_with_handoff(&endpoint, geometry(), true, true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut capable).await,
        HostResponse::Welcome { .. }
    ));
    let frame = next_idle_frame(&mut capable).await;
    assert!(matches!(
        invoke_when_current(&mut capable, "quit-here", frame).await,
        HostResponse::CommandResult { .. }
    ));
    let detached = await_detached(
        &mut capable,
        None,
        "waiting for the handoff-capable client to detach",
    )
    .await;
    assert_eq!(
        detached.map(runyte::workspace::transport::decode_path),
        Some(root.join("nested"))
    );

    // The response performs the client-owned handoff, then the ordinary quit
    // lifecycle ends the clean persistent session.
    assert!(child.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}
