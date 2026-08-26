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
    protocol::SnapshotRow,
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
    Command::new(env!("CARGO_BIN_EXE_runyte"))
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
const TERMINAL_STATE_TIMEOUT: Duration = Duration::from_secs(30);
const TERMINAL_STATE_POLL: Duration = Duration::from_millis(250);

async fn response(client: &mut LocalClient) -> HostResponse {
    tokio::time::timeout(HOST_RESPONSE_TIMEOUT, client.recv())
        .await
        .expect("host response timed out")
        .unwrap()
        .expect("host disconnected")
}

async fn shutdown(client: &mut LocalClient, request: ClientRequest) {
    client.send(&request).await.unwrap();
    let response = tokio::time::timeout(HOST_RESPONSE_TIMEOUT, client.recv())
        .await
        .expect("host shutdown timed out")
        .unwrap();
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

fn frame_text(response: &HostResponse) -> String {
    let HostResponse::Frame { frame } = response else {
        panic!("expected frame, got {response:?}")
    };
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
    .unwrap_or_else(|| panic!("terminal frame never contained {needle:?}"))
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
            _ => {}
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
    let child = Command::new(executable)
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
    first.send(&ClientRequest::Detach).await.unwrap();
    assert_eq!(
        response(&mut first).await,
        HostResponse::Detached {
            directory_bytes: None,
        }
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
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut after_disconnect = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut after_disconnect).await,
        HostResponse::Welcome { pid, .. } if pid == host_pid
    ));
    let _ = response(&mut after_disconnect).await;
    let mut pasted = send_input(&mut after_disconnect, KeyStroke::plain(KeyCode::Char('p'))).await;
    while frame_text(&pasted).matches("word").count() < 4 {
        pasted = response(&mut after_disconnect).await;
    }
    after_disconnect.send(&ClientRequest::Detach).await.unwrap();
    assert_eq!(
        response(&mut after_disconnect).await,
        HostResponse::Detached {
            directory_bytes: None
        }
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
async fn terminal_pid_output_and_input_survive_detach_disconnect_and_reattach() {
    let root = project();
    let executable = env!("CARGO_BIN_EXE_runyte");
    let child = Command::new(executable)
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
    let initial = response(&mut client).await;
    let HostResponse::Frame { frame } = initial else {
        panic!("expected initial frame, got {initial:?}")
    };
    let mut current = Some((*frame).clone());
    let mut damage_count = 0;
    client
        .send(&ClientRequest::Invoke {
            command: runyte::protocol::CommandRequest {
                name: "terminal".to_owned(),
                argument: Some(
                    "/bin/sh -c 'printf \"token:%s\\n\" \"$$\"; sleep 0.2; printf \"detached\\n\"; while IFS= read -r line; do printf \"reply:%s\\n\" \"$line\"; done'"
                        .to_owned(),
                ),
                frame: frame.id,
                buffer: frame.active_buffer,
                revision: frame.active_revision,
            },
        })
        .await
        .unwrap();
    let first = frame_containing(&mut client, &mut current, &mut damage_count, "token:").await;
    let token = terminal_wire_frame_text(&first)
        .lines()
        .find(|line| line.contains("token:"))
        .unwrap()
        .trim()
        .to_owned();

    client.send(&ClientRequest::Detach).await.unwrap();
    while !matches!(response(&mut client).await, HostResponse::Detached { .. }) {}
    tokio::time::sleep(Duration::from_millis(350)).await;

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
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut control = LocalClient::connect(&endpoint, geometry(), false)
        .await
        .unwrap();
    let _ = response(&mut control).await;
    control.send(&ClientRequest::Health).await.unwrap();
    assert!(matches!(
        response(&mut control).await,
        HostResponse::Health {
            live_terminals: 1,
            terminal_sessions: 1,
            ..
        }
    ));
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
    let child = Command::new(env!("CARGO_BIN_EXE_runyte"))
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
    let initial = next_complete_frame(&mut client).await;
    client
        .send(&ClientRequest::Invoke {
            command: runyte::protocol::CommandRequest {
                name: "terminal".to_owned(),
                argument: Some("/bin/sh -c 'sleep 0.3; printf hidden-unread; sleep 30'".to_owned()),
                frame: initial.id,
                buffer: initial.active_buffer,
                revision: initial.active_revision,
            },
        })
        .await
        .unwrap();
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
    client
        .send(&ClientRequest::Invoke {
            command: runyte::protocol::CommandRequest {
                name: "open".to_owned(),
                argument: Some("note.txt".to_owned()),
                frame: terminal_frame.id,
                buffer: terminal_frame.active_buffer,
                revision: terminal_frame.active_revision,
            },
        })
        .await
        .unwrap();
    frame_matching(&mut client, &mut current, &mut damage_count, |frame| {
        frame
            .editor
            .panes
            .iter()
            .all(|pane| pane.terminal.is_none())
    })
    .await
    .expect("terminal pane never became hidden");
    client.send(&ClientRequest::Detach).await.unwrap();
    while !matches!(response(&mut client).await, HostResponse::Detached { .. }) {}
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut reattached = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut reattached).await,
        HostResponse::Welcome { .. }
    ));
    let frame = next_complete_frame(&mut reattached).await;
    reattached
        .send(&ClientRequest::Invoke {
            command: runyte::protocol::CommandRequest::at(
                "terminals",
                frame.id,
                frame.active_buffer,
                frame.active_revision,
            ),
        })
        .await
        .unwrap();
    let manager = loop {
        let frame = next_complete_frame(&mut reattached).await;
        if frame
            .overlays
            .iter()
            .any(|overlay| overlay.title == "Terminals")
        {
            break frame;
        }
    };
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
    let child = Command::new(executable)
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
    first.send(&ClientRequest::Detach).await.unwrap();
    assert_eq!(
        response(&mut first).await,
        HostResponse::Detached {
            directory_bytes: None,
        }
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
    reattached.send(&ClientRequest::Detach).await.unwrap();
    assert_eq!(
        response(&mut reattached).await,
        HostResponse::Detached {
            directory_bytes: None,
        }
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
        Command::new(executable)
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
        Command::new(executable)
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
    let child = Command::new(env!("CARGO_BIN_EXE_runyte"))
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
        Command::new(env!("CARGO_BIN_EXE_runyte"))
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
        Command::new(env!("CARGO_BIN_EXE_runyte"))
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
    let restart = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["--session-restart", selector.as_ref()])
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .unwrap();
    assert_cli_success(&restart);
    assert!(original.0.take().unwrap().wait().unwrap().success());
    assert!(endpoint.verify_for_connect().is_ok());

    let shutdown = Command::new(env!("CARGO_BIN_EXE_runyte"))
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
            Command::new(env!("CARGO_BIN_EXE_runyte"))
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
        Command::new(env!("CARGO_BIN_EXE_runyte"))
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

    let listing = Command::new(env!("CARGO_BIN_EXE_runyte"))
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

    let shutdown = Command::new(env!("CARGO_BIN_EXE_runyte"))
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

#[tokio::test]
async fn session_start_is_idempotent_for_current_and_explicit_workspaces() {
    let root = project();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();

    assert_cli_success(&run_cli(&root, &["--session-start"]));
    assert_cli_success(&run_cli(
        &root,
        &["--session-start", root.to_string_lossy().as_ref()],
    ));

    let listing = run_cli(&root, &["--session-list"]);
    assert_cli_success(&listing);
    let listing = String::from_utf8(listing.stdout).unwrap();
    assert!(
        listing.contains(&endpoint.id()[..ABBREVIATED_WORKSPACE_ID]),
        "{listing}"
    );
    assert!(listing.contains("running"), "{listing}");

    assert_cli_success(&run_cli(&root, &["--session-stop"]));
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

    let persistent = Command::new(env!("CARGO_BIN_EXE_runyte"))
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
    let frame = response(&mut client).await;
    let HostResponse::Frame { frame } = frame else {
        panic!("expected initial frame, got {frame:?}")
    };
    client
        .send(&ClientRequest::Invoke {
            command: runyte::protocol::CommandRequest::at(
                "quit-here",
                frame.id,
                frame.active_buffer,
                frame.active_revision,
            ),
        })
        .await
        .unwrap();
    let detached_directory = loop {
        if let HostResponse::Detached { directory_bytes } = response(&mut client).await {
            break directory_bytes.map(runyte::workspace::transport::decode_path);
        }
    };
    assert_eq!(detached_directory, Some(nested));

    assert_cli_success(&run_cli(&root, &["--session-stop"]));
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
/// on the detach, and only for a client that said it can deliver one.
#[tokio::test]
async fn quit_here_reports_its_directory_to_a_handoff_capable_client() {
    let root = project();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/deep.txt"), "deep\n").unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_runyte"))
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
    let _ = response(&mut plain).await;
    let frame = response(&mut plain).await;
    let HostResponse::Frame { frame } = &frame else {
        panic!("expected a frame, got {frame:?}")
    };
    plain
        .send(&ClientRequest::Invoke {
            command: runyte::protocol::CommandRequest::at(
                "quit-here",
                frame.id,
                frame.active_buffer,
                frame.active_revision,
            ),
        })
        .await
        .unwrap();
    assert!(matches!(
        response(&mut plain).await,
        HostResponse::CommandResult { outcome } if format!("{outcome:?}").contains("runyte()")
    ));
    plain.send(&ClientRequest::Detach).await.unwrap();
    let plain_detached = loop {
        if let HostResponse::Detached { directory_bytes } = response(&mut plain).await {
            break directory_bytes;
        }
    };
    assert_eq!(plain_detached, None);

    // A capable client receives the directory of the open file, not the root.
    let mut capable = LocalClient::connect_with_handoff(&endpoint, geometry(), true, true)
        .await
        .unwrap();
    let _ = response(&mut capable).await;
    let frame = response(&mut capable).await;
    let HostResponse::Frame { frame } = &frame else {
        panic!("expected a frame, got {frame:?}")
    };
    capable
        .send(&ClientRequest::Invoke {
            command: runyte::protocol::CommandRequest::at(
                "quit-here",
                frame.id,
                frame.active_buffer,
                frame.active_revision,
            ),
        })
        .await
        .unwrap();
    let detached = loop {
        let response = response(&mut capable).await;
        if let HostResponse::Detached { directory_bytes } = response {
            break directory_bytes;
        }
    };
    assert_eq!(
        detached.map(runyte::workspace::transport::decode_path),
        Some(root.join("nested"))
    );

    // The host survives the handoff: `:quit-here` detaches, it does not stop.
    let shutdown = run_cli(&root, &["--session-stop"]);
    assert_cli_success(&shutdown);
    assert!(child.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}
