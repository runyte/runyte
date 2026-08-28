// SPDX-License-Identifier: MPL-2.0
#![cfg(unix)]

#[cfg(not(target_os = "macos"))]
use std::os::unix::ffi::OsStringExt;
use std::{
    fs::{self, File},
    io::Read,
    os::fd::{AsRawFd, FromRawFd},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use runyte::{
    app::FrameGeometry,
    input::{InputEvent, KeyCode, KeyStroke, Modifiers},
    layout::Rect,
    protocol::{CommandRequest, decode_path},
    workspace::transport::{
        ClientRequest, HostResponse, LocalClient, LocalEndpoint, PROTOCOL_VERSION, TransportChange,
        encode_path,
    },
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

fn spawn_in_pty(command: &mut Command) -> (Child, File) {
    let (child, master, _) = spawn_in_pty_with_initial_termios(command);
    (child, master)
}

fn spawn_in_pty_with_initial_termios(command: &mut Command) -> (Child, File, libc::termios) {
    let mut master = -1;
    let mut slave = -1;
    // `openpty` takes `*mut` for both of the trailing arguments on Apple
    // platforms and `*const` on Linux. Raw `*mut` pointers coerce to either,
    // so one spelling compiles on both. Taking `&size` instead builds only on
    // Linux, and clippy will suggest exactly that if given a reference here.
    let mut size = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `openpty` initializes both owned descriptors on success. They
    // are immediately wrapped in `File`, and each descriptor has one owner.
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut size,
        )
    };
    assert_eq!(
        result,
        0,
        "openpty failed: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: successful `openpty` returned fresh descriptors owned here.
    let master = unsafe { File::from_raw_fd(master) };
    // SAFETY: the descriptor is live and owned by `master`; setting CLOEXEC
    // prevents the child from retaining the PTY master and masking hangup.
    assert_ne!(
        unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
        -1
    );
    // SAFETY: successful `openpty` returned fresh descriptors owned here.
    let slave = unsafe { File::from_raw_fd(slave) };
    let initial = terminal_attributes(slave.as_raw_fd());
    // SAFETY: this runs in the child between fork and exec, calls only
    // async-signal-safe libc operations, and makes the PTY slave controlling
    // terminal so closing the master exercises a real terminal hangup.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .spawn()
        .unwrap();
    (child, master, initial)
}

fn terminal_attributes(descriptor: std::os::fd::RawFd) -> libc::termios {
    let mut attributes = std::mem::MaybeUninit::uninit();
    // SAFETY: `attributes` points to writable storage for one termios value,
    // and callers pass a live PTY descriptor.
    assert_eq!(
        unsafe { libc::tcgetattr(descriptor, attributes.as_mut_ptr()) },
        0
    );
    // SAFETY: successful `tcgetattr` initialized the complete value.
    unsafe { attributes.assume_init() }
}

fn spawn_wait_in_pty(root: &Path, target: &str) -> (Child, File) {
    spawn_in_pty(
        Command::new(env!("CARGO_BIN_EXE_runyte"))
            .arg("--wait")
            .arg(target)
            .current_dir(root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    )
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} in {} failed with {}: {}{}",
        root.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
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
        "runyte-local-protocol-{}-{unique}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("note.txt"), "base\n").unwrap();
    fs::write(root.join("other.txt"), "other\n").unwrap();
    git(&root, &["init", "--quiet"]);
    git(&root, &["config", "user.name", "Runyte Test"]);
    git(&root, &["config", "user.email", "runyte@example.invalid"]);
    root.canonicalize().unwrap()
}

fn tui_geometry() -> FrameGeometry {
    FrameGeometry {
        screen: Rect {
            width: 80,
            height: 24,
            ..Rect::default()
        },
        editor: Rect {
            width: 80,
            height: 22,
            ..Rect::default()
        },
        status: Rect {
            y: 22,
            width: 80,
            height: 1,
            ..Rect::default()
        },
        message: Rect {
            y: 23,
            width: 80,
            height: 1,
            ..Rect::default()
        },
    }
}

async fn response(client: &mut LocalClient) -> HostResponse {
    tokio::time::timeout(Duration::from_secs(5), client.recv())
        .await
        .expect("host response timed out")
        .unwrap()
        .expect("host disconnected")
}

async fn shutdown(client: &mut LocalClient) {
    client.send(&ClientRequest::Shutdown).await.unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), client.recv())
        .await
        .expect("host shutdown timed out")
        .unwrap();
    assert!(
        matches!(response, Some(HostResponse::ShuttingDown) | None),
        "expected host shutdown, got {response:?}"
    );
}

async fn response_ignoring_frames(client: &mut LocalClient) -> HostResponse {
    loop {
        let response = response(client).await;
        if !matches!(response, HostResponse::Frame { .. }) {
            return response;
        }
    }
}

async fn send_input_expect_frame(client: &mut LocalClient, event: InputEvent) {
    client
        .send(&ClientRequest::Input {
            event: event.into(),
            repeated: false,
        })
        .await
        .unwrap();
    assert!(matches!(response(client).await, HostResponse::Frame { .. }));
}

async fn wait_child(child: &mut Child) -> ExitStatus {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("child process timed out")
}

#[test]
fn termination_signal_restores_the_terminal_and_preserves_its_exit_status() {
    let root = project();
    let config = default_config(&root);
    let (mut child, master, initial) = spawn_in_pty_with_initial_termios(
        Command::new(env!("CARGO_BIN_EXE_runyte"))
            .args(["--standalone", "--config"])
            .arg(config)
            .arg("note.txt")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let mut output = master.try_clone().unwrap();
    let _output_drain = std::thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        while output.read(&mut buffer).is_ok_and(|read| read != 0) {}
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let current = terminal_attributes(master.as_raw_fd());
        if current.c_lflag & (libc::ICANON | libc::ECHO) == 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "editor did not enter terminal raw mode"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // SAFETY: `child.id()` names this test's live child process.
    assert_eq!(
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    let exit_deadline = std::time::Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() >= exit_deadline {
            let current = terminal_attributes(master.as_raw_fd());
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "editor did not exit after SIGTERM; terminal lflag was {:#x}, initial {:#x}",
                current.c_lflag, initial.c_lflag
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(128 + libc::SIGTERM));

    let restored = terminal_attributes(master.as_raw_fd());
    assert_eq!(restored.c_iflag, initial.c_iflag);
    assert_eq!(restored.c_oflag, initial.c_oflag);
    assert_eq!(restored.c_cflag, initial.c_cflag);
    assert_eq!(restored.c_lflag, initial.c_lflag);
    assert_eq!(restored.c_cc, initial.c_cc);
    // SAFETY: both pointers refer to initialized termios values.
    assert_eq!(unsafe { libc::cfgetispeed(&restored) }, unsafe {
        libc::cfgetispeed(&initial)
    });
    // SAFETY: both pointers refer to initialized termios values.
    assert_eq!(unsafe { libc::cfgetospeed(&restored) }, unsafe {
        libc::cfgetospeed(&initial)
    });
    fs::remove_dir_all(root).unwrap();
}

async fn connect_control(endpoint: &LocalEndpoint) -> LocalClient {
    try_connect_control(endpoint).await.unwrap()
}

async fn try_connect_control(endpoint: &LocalEndpoint) -> anyhow::Result<LocalClient> {
    let mut client = LocalClient::connect(endpoint, FrameGeometry::default(), false).await?;
    let welcome = tokio::time::timeout(Duration::from_secs(5), client.recv())
        .await??
        .ok_or_else(|| anyhow::anyhow!("host disconnected during control handshake"))?;
    anyhow::ensure!(
        matches!(welcome, HostResponse::Welcome { .. }),
        "unexpected control handshake response: {welcome:?}"
    );
    Ok(client)
}

async fn wait_for_buffer_text(
    client: &mut LocalClient,
    terminal_output: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    name: &str,
    needle: &str,
) {
    let mut last_text = None;
    for _ in 0..400 {
        client.send(&ClientRequest::ListBuffers).await.unwrap();
        let buffer = match response(client).await {
            HostResponse::Buffers { buffers } => buffers
                .into_iter()
                .find(|buffer| !buffer.closed && buffer.name == name),
            response => panic!("expected buffers while waiting for {name:?}, got {response:?}"),
        };
        if let Some(buffer) = buffer {
            client
                .send(&ClientRequest::ReadBuffer { buffer: buffer.id })
                .await
                .unwrap();
            match response(client).await {
                HostResponse::Buffer { buffer } => {
                    if buffer.text.contains(needle) {
                        return;
                    }
                    last_text = Some(buffer.text);
                }
                response => {
                    panic!("expected {name:?} contents while waiting, got {response:?}")
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "buffer {name:?} did not contain {needle:?}; last contents: {:?}; terminal output: {}",
        last_text.as_deref().unwrap_or("<buffer was not opened>"),
        String::from_utf8_lossy(&terminal_output.lock().unwrap())
    );
}

async fn wait_for_terminal_output(
    output: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    needle: &str,
) {
    for _ in 0..400 {
        if String::from_utf8_lossy(&output.lock().unwrap()).contains(needle) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "terminal output did not contain {needle:?}: {}",
        String::from_utf8_lossy(&output.lock().unwrap())
    );
}

/// Types and accepts a command through the real terminal.
fn type_colon_command(terminal: &mut File, command: &str) {
    std::io::Write::write_all(terminal, format!(":{command}\r").as_bytes()).unwrap();
    std::io::Write::flush(terminal).unwrap();
}

async fn start_host(root: &Path, endpoint: &LocalEndpoint) -> Option<ChildGuard> {
    start_host_opening(root, endpoint, Some("other.txt")).await
}

/// Starts a host, optionally on a file. Without one the host keeps the
/// scratch buffer it starts with, which is the only way to reach a scratchpad
/// from a control client.
async fn start_host_opening(
    root: &Path,
    endpoint: &LocalEndpoint,
    target: Option<&str>,
) -> Option<ChildGuard> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    command.arg("--serve");
    if let Some(target) = target {
        command.arg(target);
    }
    let child = command
        .current_dir(root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(Some(child));
    for _ in 0..100 {
        if endpoint.metadata().exists()
            && LocalClient::connect(endpoint, FrameGeometry::default(), false)
                .await
                .is_ok()
        {
            return Some(child);
        }
        if let Some(status) = child.0.as_mut().unwrap().try_wait().unwrap() {
            use std::io::Read;
            let mut stderr = String::new();
            if let Some(pipe) = child.0.as_mut().unwrap().stderr.as_mut() {
                let _ = pipe.read_to_string(&mut stderr);
            }
            if stderr.contains("Operation not permitted") {
                return None;
            }
            panic!("workspace host exited during startup with {status}: {stderr}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("host endpoint did not become ready");
}

#[tokio::test]
async fn revision_protocol_is_stale_safe_undoable_and_bounded() {
    let root = project();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut host) = start_host(&root, &endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut client = connect_control(&endpoint).await;
    client.send(&ClientRequest::ListBuffers).await.unwrap();
    let buffers = match response(&mut client).await {
        HostResponse::Buffers { buffers } => buffers,
        response => panic!("expected buffers, got {response:?}"),
    };
    let other = buffers
        .iter()
        .find(|buffer| {
            buffer.path_bytes.clone().map(decode_path).as_deref()
                == Some(root.join("other.txt").as_path())
        })
        .unwrap();
    let id = other.id;
    let original = other.revision;
    client
        .send(&ClientRequest::ApplyTransaction {
            buffer: id,
            expected: original,
            changes: vec![TransportChange {
                from: 0,
                to: 0,
                text: "changed ".to_owned(),
            }],
        })
        .await
        .unwrap();
    let revision = match response(&mut client).await {
        HostResponse::TransactionApplied { revision, .. } => revision,
        response => panic!("expected applied transaction, got {response:?}"),
    };
    client
        .send(&ClientRequest::ApplyTransaction {
            buffer: id,
            expected: original,
            changes: vec![TransportChange {
                from: 0,
                to: 0,
                text: "stale ".to_owned(),
            }],
        })
        .await
        .unwrap();
    assert!(matches!(
        response(&mut client).await,
        HostResponse::StaleRevision { actual, .. } if actual == revision
    ));
    client
        .send(&ClientRequest::ReadBuffer { buffer: id })
        .await
        .unwrap();
    assert!(matches!(
        response(&mut client).await,
        HostResponse::Buffer { buffer } if buffer.text == "changed other\n"
    ));
    let mut interactive = LocalClient::connect(&endpoint, FrameGeometry::default(), true)
        .await
        .unwrap();
    let _ = response(&mut interactive).await;
    let initial_frame = match response(&mut interactive).await {
        HostResponse::Frame { frame } => *frame,
        response => panic!("expected initial frame, got {response:?}"),
    };
    interactive
        .send(&ClientRequest::Input {
            event: InputEvent::Key(KeyStroke::char('u')).into(),
            repeated: false,
        })
        .await
        .unwrap();
    let command_frame = match response(&mut interactive).await {
        HostResponse::Frame { frame } => *frame,
        response => panic!("expected updated frame, got {response:?}"),
    };
    client
        .send(&ClientRequest::ReadBuffer { buffer: id })
        .await
        .unwrap();
    assert!(matches!(
        response(&mut client).await,
        HostResponse::Buffer { buffer } if buffer.text == "other\n"
    ));
    interactive
        .send(&ClientRequest::Invoke {
            command: CommandRequest::at(
                "git-refresh",
                initial_frame.id,
                initial_frame.active_buffer,
                initial_frame.active_revision,
            ),
        })
        .await
        .unwrap();
    assert!(matches!(
        response(&mut interactive).await,
        HostResponse::Error { message } if message.contains("stale editor frame")
    ));
    interactive
        .send(&ClientRequest::Invoke {
            command: CommandRequest::at(
                "git-refresh",
                command_frame.id,
                command_frame.active_buffer,
                command_frame.active_revision,
            ),
        })
        .await
        .unwrap();
    assert!(matches!(
        response(&mut interactive).await,
        HostResponse::CommandResult { .. }
    ));
    let quit_frame = match response(&mut interactive).await {
        HostResponse::Frame { frame } => *frame,
        response => panic!("expected command frame, got {response:?}"),
    };
    interactive
        .send(&ClientRequest::Invoke {
            command: CommandRequest::at(
                "quit",
                quit_frame.id,
                quit_frame.active_buffer,
                quit_frame.active_revision,
            ),
        })
        .await
        .unwrap();
    assert!(matches!(
        response(&mut interactive).await,
        HostResponse::CommandResult { .. }
    ));
    assert_eq!(response(&mut interactive).await, HostResponse::ShuttingDown);

    let status = tokio::task::spawn_blocking(move || host.0.take().unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn an_edited_scratchpad_leaves_a_workspace_clean_enough_to_stop() {
    let root = project();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut host) = start_host_opening(&root, &endpoint, None).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut client = connect_control(&endpoint).await;
    client.send(&ClientRequest::ListBuffers).await.unwrap();
    let scratch = match response(&mut client).await {
        HostResponse::Buffers { buffers } => buffers
            .into_iter()
            .find(|buffer| buffer.path_bytes.is_none() && !buffer.read_only && !buffer.closed)
            .expect("a host keeps a scratch buffer"),
        response => panic!("expected buffers, got {response:?}"),
    };
    client
        .send(&ClientRequest::ApplyTransaction {
            buffer: scratch.id,
            expected: scratch.revision,
            changes: vec![TransportChange {
                from: 0,
                to: 0,
                text: "a note to self".to_owned(),
            }],
        })
        .await
        .unwrap();
    assert!(matches!(
        response(&mut client).await,
        HostResponse::TransactionApplied { .. }
    ));

    // The scratchpad reports itself dirty, because the pane still marks it so.
    client.send(&ClientRequest::ListBuffers).await.unwrap();
    assert!(matches!(
        response(&mut client).await,
        HostResponse::Buffers { buffers }
            if buffers.iter().any(|buffer| buffer.id == scratch.id && buffer.dirty)
    ));
    // The workspace is clean all the same: nothing here can be saved in place.
    client.send(&ClientRequest::Health).await.unwrap();
    assert!(matches!(
        response(&mut client).await,
        HostResponse::Health {
            unsaved_buffers: 0,
            ..
        }
    ));
    shutdown(&mut client).await;
    let status = tokio::task::spawn_blocking(move || host.0.take().unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn wait_cli_completes_without_stopping_host_or_unrelated_buffers() {
    let root = project();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut host) = start_host(&root, &endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut interactive = LocalClient::connect(&endpoint, FrameGeometry::default(), true)
        .await
        .unwrap();
    let _ = response(&mut interactive).await;
    let _ = response(&mut interactive).await;
    let mut waiter = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .arg("--wait")
        .arg("note.txt")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let mut requested = None;
    for _ in 0..100 {
        interactive.send(&ClientRequest::ListBuffers).await.unwrap();
        if let HostResponse::Buffers { buffers } = response_ignoring_frames(&mut interactive).await
        {
            requested = buffers.into_iter().find(|buffer| {
                buffer.path_bytes.clone().map(decode_path).as_deref()
                    == Some(root.join("note.txt").as_path())
            });
        }
        if requested.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let requested = requested.expect("wait request did not open its target");
    interactive
        .send(&ClientRequest::CloseBuffer {
            buffer: requested.id,
            discard: false,
        })
        .await
        .unwrap();
    assert!(matches!(
        response_ignoring_frames(&mut interactive).await,
        HostResponse::Closed { .. }
    ));
    let status = wait_child(&mut waiter).await;
    assert!(status.success());

    interactive.send(&ClientRequest::ListBuffers).await.unwrap();
    assert!(matches!(
        response_ignoring_frames(&mut interactive).await,
        HostResponse::Buffers { buffers }
            if buffers.iter().any(|buffer| {
                !buffer.closed
                    && buffer.path_bytes.clone().map(decode_path).as_deref()
                        == Some(root.join("other.txt").as_path())
            })
    ));
    interactive.send(&ClientRequest::Detach).await.unwrap();
    let _ = response(&mut interactive).await;
    let mut control = connect_control(&endpoint).await;
    shutdown(&mut control).await;
    let status = tokio::task::spawn_blocking(move || host.0.take().unwrap().wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn killing_the_host_fails_an_outstanding_wait_process() {
    let root = project();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut host) = start_host(&root, &endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut interactive = LocalClient::connect(&endpoint, FrameGeometry::default(), true)
        .await
        .unwrap();
    let _ = response(&mut interactive).await;
    let _ = response(&mut interactive).await;
    let mut waiter = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .arg("--wait")
        .arg("note.txt")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let mut opened = false;
    for _ in 0..100 {
        interactive.send(&ClientRequest::ListBuffers).await.unwrap();
        if let HostResponse::Buffers { buffers } = response_ignoring_frames(&mut interactive).await
        {
            opened = buffers.iter().any(|buffer| {
                buffer.path_bytes.clone().map(decode_path).as_deref()
                    == Some(root.join("note.txt").as_path())
            });
        }
        if opened {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(opened, "wait request did not become live");
    host.0.as_mut().unwrap().kill().unwrap();
    let _ = host.0.take().unwrap().wait();
    assert!(!wait_child(&mut waiter).await.success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn git_commit_wait_closes_its_buffer_without_detaching_an_existing_tui() {
    let root = project();
    git(&root, &["add", "note.txt", "other.txt"]);
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut host) = start_host(&root, &endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut interactive = LocalClient::connect(&endpoint, FrameGeometry::default(), true)
        .await
        .unwrap();
    let _ = response(&mut interactive).await;
    let _ = response(&mut interactive).await;
    // An editor request arriving in an existing persistent session must not
    // inherit Insert mode from the buffer that was active before Git opened
    // its message. The `i` sent after the wait appears below should therefore
    // enter Insert rather than becoming part of the commit subject.
    send_input_expect_frame(&mut interactive, InputEvent::Key(KeyStroke::char('i'))).await;
    let editor = format!("{} --wait", env!("CARGO_BIN_EXE_runyte"));
    let mut commit = Command::new("git")
        .arg("commit")
        .current_dir(&root)
        .env("GIT_EDITOR", editor)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let mut message = None;
    for _ in 0..200 {
        interactive.send(&ClientRequest::ListBuffers).await.unwrap();
        if let HostResponse::Buffers { buffers } = response_ignoring_frames(&mut interactive).await
        {
            message = buffers.into_iter().find(|buffer| {
                buffer
                    .path_bytes
                    .clone()
                    .map(decode_path)
                    .is_some_and(|path| path.ends_with("COMMIT_EDITMSG"))
            });
        }
        if message.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let _message = message.expect("Git editor wait request did not open COMMIT_EDITMSG");
    assert!(commit.try_wait().unwrap().is_none());
    send_input_expect_frame(&mut interactive, InputEvent::Key(KeyStroke::char('i'))).await;
    send_input_expect_frame(
        &mut interactive,
        InputEvent::Text("host-owned commit message".to_owned()),
    )
    .await;
    send_input_expect_frame(
        &mut interactive,
        InputEvent::Key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE)),
    )
    .await;
    send_input_expect_frame(&mut interactive, InputEvent::Key(KeyStroke::char(':'))).await;
    send_input_expect_frame(&mut interactive, InputEvent::Text("wbc".to_owned())).await;
    interactive
        .send(&ClientRequest::Input {
            event: InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE)).into(),
            repeated: false,
        })
        .await
        .unwrap();
    assert!(matches!(
        response(&mut interactive).await,
        HostResponse::Frame { .. }
    ));
    assert!(wait_child(&mut commit).await.success());
    interactive.send(&ClientRequest::Health).await.unwrap();
    assert!(matches!(
        response_ignoring_frames(&mut interactive).await,
        HostResponse::Health {
            interactive_attached: true,
            ..
        }
    ));
    let subject = Command::new("git")
        .args(["log", "-1", "--pretty=%s"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(subject.status.success());
    assert_eq!(
        String::from_utf8(subject.stdout).unwrap().trim(),
        "host-owned commit message"
    );

    let mut control = connect_control(&endpoint).await;
    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn git_commit_wait_tui_completes_through_write_quit() {
    use std::io::Write;

    let root = project();
    git(&root, &["add", "note.txt", "other.txt"]);
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut host) = start_host(&root, &endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let editor = format!("{} --wait", env!("CARGO_BIN_EXE_runyte"));
    let (commit, mut terminal) = spawn_in_pty(
        Command::new("git")
            .arg("commit")
            .current_dir(&root)
            .env("GIT_EDITOR", editor)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let mut commit = ChildGuard(Some(commit));
    // Nothing else reads this PTY, and the buffer we're about to fill (an
    // inserted commit message plus the `:wq` command line, which opens a
    // 71-entry command palette) is enough to fill it. Once that happens the
    // attached editor's frame write fails outright rather than just
    // blocking, which git reports as "there was a problem with the editor"
    // and exits 1 — the same failure text this test used to see from a
    // completely different cause. Drain continuously, but keep what was
    // drained so a real failure can still be diagnosed from it below.
    let drained = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let drain_sink = std::sync::Arc::clone(&drained);
    let mut drain = terminal.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut chunk = [0_u8; 4096];
        loop {
            match std::io::Read::read(&mut drain, &mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(count) => drain_sink
                    .lock()
                    .unwrap()
                    .extend_from_slice(&chunk[..count]),
            }
        }
    });
    let mut control = connect_control(&endpoint).await;
    let mut attached = false;
    for _ in 0..200 {
        control.send(&ClientRequest::Health).await.unwrap();
        attached = matches!(
            response(&mut control).await,
            HostResponse::Health {
                interactive_attached: true,
                ..
            }
        );
        if attached {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(attached, "Git's wait editor did not attach its TUI");

    // Attachment being observed does not mean the commit buffer exists yet;
    // wait for it explicitly rather than trusting `interactive_attached`
    // alone, the same way the sibling protocol-driven test does.
    let mut commit_buffer = None;
    for _ in 0..200 {
        control.send(&ClientRequest::ListBuffers).await.unwrap();
        if let HostResponse::Buffers { buffers } = response(&mut control).await {
            commit_buffer = buffers.into_iter().find(|buffer| {
                buffer
                    .path_bytes
                    .clone()
                    .map(decode_path)
                    .is_some_and(|path| path.ends_with("COMMIT_EDITMSG"))
            });
        }
        if commit_buffer.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let commit_buffer = commit_buffer
        .expect("Git editor wait request did not open COMMIT_EDITMSG")
        .id;

    terminal.write_all(b"iPTY commit message").unwrap();
    terminal.flush().unwrap();
    let mut inserted = false;
    for _ in 0..200 {
        control
            .send(&ClientRequest::ReadBuffer {
                buffer: commit_buffer,
            })
            .await
            .unwrap();
        if let HostResponse::Buffer { buffer } = response(&mut control).await {
            inserted = buffer.text.contains("PTY commit message");
        }
        if inserted {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(inserted, "typed commit message did not reach the buffer");

    // Escape has no buffer-visible effect, so unlike the insert above there is
    // no host state to poll here: a `Buffer` read taken right after writing
    // it would trivially match on the first attempt and confirm nothing.
    // What actually needs guarding against is the raw byte stream, not the
    // host: if the escape and the following `:wq\r` land in the same read on
    // the editor's terminal input parser, a bare ESC immediately followed by
    // `:` is Alt/Meta-sequence-shaped and can be swallowed as a modified key
    // instead of a standalone Escape, leaving insert mode active so `:wq`
    // never reaches the command line. Guarantee real separation between the
    // two writes, then use the buffer read only as a sanity check that
    // nothing was corrupted in the meantime.
    //
    // The commit message instructions this editor writes into the buffer
    // ("commit", "message", ...) are ordinary words, so word completion can
    // legitimately be showing a popup once "commit message" has been typed.
    // One Escape must both dismiss that automatic popup and leave insert
    // mode, exactly as it does when word completion is switched off.
    terminal.write_all(b"\x1b").unwrap();
    terminal.flush().unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    control
        .send(&ClientRequest::ReadBuffer {
            buffer: commit_buffer,
        })
        .await
        .unwrap();
    let unchanged = matches!(
        response(&mut control).await,
        HostResponse::Buffer { buffer } if buffer.text.contains("PTY commit message")
    );
    assert!(unchanged, "commit buffer changed unexpectedly after escape");

    terminal.write_all(b":wq\r").unwrap();
    terminal.flush().unwrap();

    let status = wait_child(commit.0.as_mut().unwrap()).await;
    if !status.success() {
        // Give the drain thread a moment to catch up with whatever the
        // exiting process wrote last, so the diagnostic below is complete.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let output = String::from_utf8_lossy(&drained.lock().unwrap()).into_owned();
        control
            .send(&ClientRequest::ReadBuffer {
                buffer: commit_buffer,
            })
            .await
            .unwrap();
        let buffer_text = match response(&mut control).await {
            HostResponse::Buffer { buffer } => buffer.text,
            other => format!("<unavailable: {other:?}>"),
        };
        panic!(
            "Git commit failed after :wq: {status}\npty output: {output:?}\ncommit buffer: {buffer_text:?}"
        );
    }
    let _ = commit.0.take().unwrap().wait();
    let subject = Command::new("git")
        .args(["log", "-1", "--pretty=%s"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(subject.status.success());
    assert_eq!(
        String::from_utf8(subject.stdout).unwrap().trim(),
        "PTY commit message"
    );

    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn wait_paths_are_resolved_in_the_callers_directory_without_utf8_loss() {
    let root = project();
    let nested = root.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("note.txt"), "nested\n").unwrap();
    let wait_names = vec![std::ffi::OsString::from("note.txt")];
    // macOS rejects non-UTF-8 path components with EILSEQ. It still covers
    // caller-relative wait targets here; Unix filesystems that can represent
    // arbitrary bytes additionally cover lossless path transport.
    #[cfg(not(target_os = "macos"))]
    let wait_names = {
        let mut wait_names = wait_names;
        let non_utf8_name = std::ffi::OsString::from_vec(b"odd-\xff.txt".to_vec());
        fs::write(nested.join(&non_utf8_name), "encoded\n").unwrap();
        wait_names.push(non_utf8_name);
        wait_names
    };
    let expected_paths = wait_names
        .iter()
        .map(|name| nested.join(name))
        .collect::<Vec<_>>();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut host) = start_host(&root, &endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut interactive = LocalClient::connect(&endpoint, FrameGeometry::default(), true)
        .await
        .unwrap();
    let _ = response(&mut interactive).await;
    let _ = response(&mut interactive).await;
    let mut wait_command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    wait_command.arg("--wait").args(&wait_names);
    let mut waiter = wait_command
        .current_dir(&nested)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let mut requested = Vec::new();
    for _ in 0..100 {
        interactive.send(&ClientRequest::ListBuffers).await.unwrap();
        if let HostResponse::Buffers { buffers } = response_ignoring_frames(&mut interactive).await
        {
            requested = buffers
                .into_iter()
                .filter(|buffer| {
                    buffer
                        .path_bytes
                        .clone()
                        .map(decode_path)
                        .is_some_and(|path| expected_paths.contains(&path))
                })
                .collect();
        }
        if requested.len() == expected_paths.len() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        requested.len(),
        expected_paths.len(),
        "caller-relative wait targets were not opened"
    );
    for buffer in requested {
        interactive
            .send(&ClientRequest::CloseBuffer {
                buffer: buffer.id,
                discard: false,
            })
            .await
            .unwrap();
        assert!(matches!(
            response_ignoring_frames(&mut interactive).await,
            HostResponse::Closed { .. }
        ));
    }
    assert!(wait_child(&mut waiter).await.success());
    interactive.send(&ClientRequest::Detach).await.unwrap();
    let _ = response_ignoring_frames(&mut interactive).await;
    let mut control = connect_control(&endpoint).await;
    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn persistent_worktree_switch_detaches_to_a_new_root_without_retargeting_the_host() {
    let root = project();
    git(&root, &["add", "note.txt", "other.txt"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    git(
        &root,
        &["worktree", "add", "--quiet", "-b", "linked", "linked"],
    );
    let linked = root.join("linked").canonicalize().unwrap();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut host) = start_host(&root, &endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut control = connect_control(&endpoint).await;
    control.send(&ClientRequest::ListBuffers).await.unwrap();
    let (dirty_buffer, dirty_revision) = match response(&mut control).await {
        HostResponse::Buffers { buffers } => {
            let buffer = buffers
                .into_iter()
                .find(|buffer| {
                    buffer.path_bytes.clone().map(decode_path).as_deref()
                        == Some(root.join("other.txt").as_path())
                })
                .expect("startup buffer was not hosted");
            (buffer.id, buffer.revision)
        }
        response => panic!("expected hosted buffers, got {response:?}"),
    };
    control
        .send(&ClientRequest::ApplyTransaction {
            buffer: dirty_buffer,
            expected: dirty_revision,
            changes: vec![TransportChange {
                from: 0,
                to: 0,
                text: "preserved ".to_owned(),
            }],
        })
        .await
        .unwrap();
    let dirty_revision = match response(&mut control).await {
        HostResponse::TransactionApplied { revision, .. } => revision,
        response => panic!("expected dirty transaction, got {response:?}"),
    };
    let mut interactive = LocalClient::connect(&endpoint, tui_geometry(), true)
        .await
        .unwrap();
    let _ = response(&mut interactive).await;
    let mut initial = match response(&mut interactive).await {
        HostResponse::Frame { frame } => *frame,
        response => panic!("expected initial frame, got {response:?}"),
    };
    while initial
        .editor
        .status
        .git_summary
        .as_deref()
        .is_none_or(|summary| summary.contains(":git-cancel"))
    {
        initial = match response(&mut interactive).await {
            HostResponse::Frame { frame } => *frame,
            response => panic!("expected Git discovery frame, got {response:?}"),
        };
    }
    interactive
        .send(&ClientRequest::Invoke {
            command: CommandRequest::at(
                "git-worktrees",
                initial.id,
                initial.active_buffer,
                initial.active_revision,
            ),
        })
        .await
        .unwrap();
    assert!(matches!(
        response(&mut interactive).await,
        HostResponse::CommandResult { .. }
    ));
    let mut worktree_frame = None;
    for _ in 0..100 {
        if let HostResponse::Frame { frame } = response(&mut interactive).await
            && frame
                .editor
                .panes
                .iter()
                .any(|pane| pane.active && pane.title.name == "[git worktrees]")
        {
            worktree_frame = Some(*frame);
            break;
        }
    }
    let _ = worktree_frame.expect("worktree service did not open its view");
    interactive
        .send(&ClientRequest::Input {
            event: InputEvent::Key(KeyStroke::char('j')).into(),
            repeated: false,
        })
        .await
        .unwrap();
    let mut selected = None;
    while let Ok(Ok(Some(HostResponse::Frame { frame }))) =
        tokio::time::timeout(Duration::from_millis(100), interactive.recv()).await
    {
        selected = Some(*frame);
    }
    let _selected = selected.expect("selection input did not publish a frame");
    interactive
        .send(&ClientRequest::Input {
            event: InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE)).into(),
            repeated: false,
        })
        .await
        .unwrap();
    let switched = loop {
        match response(&mut interactive).await {
            HostResponse::SwitchWorkspace {
                selector_bytes,
                working_directory_bytes,
            } => {
                assert_eq!(decode_path(working_directory_bytes), root);
                break decode_path(selector_bytes);
            }
            HostResponse::Frame { .. } => {}
            response => panic!("expected workspace switch, got {response:?}"),
        }
    };
    assert_eq!(switched, linked);

    control.send(&ClientRequest::Health).await.unwrap();
    assert!(matches!(
        response(&mut control).await,
        HostResponse::Health {
            interactive_attached: false,
            ..
        }
    ));
    control.send(&ClientRequest::ListBuffers).await.unwrap();
    assert!(matches!(
        response(&mut control).await,
        HostResponse::Buffers { buffers }
            if buffers.iter().any(|buffer| {
                !buffer.closed
                    && buffer.path_bytes.clone().map(decode_path).as_deref()
                        == Some(root.join("other.txt").as_path())
            })
    ));
    control
        .send(&ClientRequest::ReadBuffer {
            buffer: dirty_buffer,
        })
        .await
        .unwrap();
    assert!(matches!(
        response(&mut control).await,
        HostResponse::Buffer { buffer }
            if buffer.text == "preserved other\n" && buffer.metadata.revision == dirty_revision
    ));
    control.send(&ClientRequest::Shutdown).await.unwrap();
    assert!(matches!(
        response(&mut control).await,
        HostResponse::Refused { message } if message.contains("1 unsaved buffer")
    ));
    control
        .send(&ClientRequest::SaveBuffer {
            buffer: dirty_buffer,
        })
        .await
        .unwrap();
    assert!(matches!(
        response(&mut control).await,
        HostResponse::Saved { buffer, .. } if buffer == dirty_buffer
    ));
    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn worktree_switch_reuses_the_destination_host_through_the_real_tui_launcher() {
    use std::io::Write;

    let root = project();
    git(&root, &["add", "note.txt", "other.txt"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    git(
        &root,
        &["worktree", "add", "--quiet", "-b", "linked", "linked"],
    );
    let linked = root.join("linked").canonicalize().unwrap();
    let source_endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let linked_endpoint = LocalEndpoint::discover_with_runtime(
        &linked.join(".runyte"),
        &linked,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut source_host) = start_host(&root, &source_endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let Some(mut linked_host) = start_host(&linked, &linked_endpoint).await else {
        let mut source = connect_control(&source_endpoint).await;
        shutdown(&mut source).await;
        let _ = source_host.0.take().unwrap().wait();
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut source = connect_control(&source_endpoint).await;
    let mut destination = connect_control(&linked_endpoint).await;
    let (switcher, mut terminal) = spawn_in_pty(
        Command::new(env!("CARGO_BIN_EXE_runyte"))
            .arg("--persistent")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let mut switcher = ChildGuard(Some(switcher));
    // Nothing else reads this PTY, and two full-screen TUIs render into it
    // in sequence. Once the terminal buffer fills, the attached editor blocks
    // writing a frame and stops reading input, so later keys are never seen.
    let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = std::sync::Arc::clone(&output);
    let drain = terminal.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut drain = drain;
        let mut sink = [0_u8; 4096];
        while let Ok(count) = std::io::Read::read(&mut drain, &mut sink) {
            if count == 0 {
                break;
            }
            captured.lock().unwrap().extend_from_slice(&sink[..count]);
        }
    });

    for _ in 0..200 {
        source.send(&ClientRequest::Health).await.unwrap();
        if matches!(
            response(&mut source).await,
            HostResponse::Health {
                interactive_attached: true,
                ..
            }
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    wait_for_terminal_output(&output, "other").await;
    wait_for_terminal_output(&output, "│ master").await;
    type_colon_command(&mut terminal, "git-worktrees");
    let linked_display = linked.to_string_lossy().into_owned();
    wait_for_buffer_text(&mut source, &output, "[git worktrees]", &linked_display).await;
    terminal.write_all(b"j\r").unwrap();
    terminal.flush().unwrap();

    let mut attached_to_destination = false;
    for _ in 0..200 {
        destination.send(&ClientRequest::Health).await.unwrap();
        attached_to_destination = matches!(
            response(&mut destination).await,
            HostResponse::Health {
                interactive_attached: true,
                ..
            }
        );
        if attached_to_destination {
            break;
        }
        if let Some(status) = switcher.0.as_mut().unwrap().try_wait().unwrap() {
            panic!("worktree-switching TUI exited before reattaching: {status}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(attached_to_destination, "destination host was not reused");
    source.send(&ClientRequest::Health).await.unwrap();
    assert!(matches!(
        response(&mut source).await,
        HostResponse::Health {
            interactive_attached: false,
            ..
        }
    ));

    terminal.write_all(b":detach\r").unwrap();
    terminal.flush().unwrap();
    assert!(wait_child(switcher.0.as_mut().unwrap()).await.success());
    let _ = switcher.0.take().unwrap().wait();
    shutdown(&mut source).await;
    shutdown(&mut destination).await;
    assert!(source_host.0.take().unwrap().wait().unwrap().success());
    assert!(linked_host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn incompatible_worktree_host_returns_the_tui_to_its_source() {
    use std::io::Write;

    let root = project();
    git(&root, &["add", "note.txt", "other.txt"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    git(
        &root,
        &["worktree", "add", "--quiet", "-b", "linked", "linked"],
    );
    let linked = root.join("linked").canonicalize().unwrap();
    let Some((incompatible_endpoint, incompatible_listener, _)) =
        publish_incompatible_endpoint(&linked, std::process::id())
    else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let source_endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut source_host) = start_host(&root, &source_endpoint).await else {
        drop(incompatible_listener);
        incompatible_endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut source = connect_control(&source_endpoint).await;
    let (switcher, mut terminal) = spawn_in_pty(
        Command::new(env!("CARGO_BIN_EXE_runyte"))
            .arg("--persistent")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let mut switcher = ChildGuard(Some(switcher));
    let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = std::sync::Arc::clone(&output);
    let drain = terminal.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut drain = drain;
        let mut sink = [0_u8; 4096];
        while let Ok(count) = std::io::Read::read(&mut drain, &mut sink) {
            if count == 0 {
                break;
            }
            captured.lock().unwrap().extend_from_slice(&sink[..count]);
        }
    });

    for _ in 0..200 {
        source.send(&ClientRequest::Health).await.unwrap();
        if matches!(
            response(&mut source).await,
            HostResponse::Health {
                interactive_attached: true,
                ..
            }
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    wait_for_terminal_output(&output, "other").await;
    wait_for_terminal_output(&output, "│ master").await;
    type_colon_command(&mut terminal, "git-worktrees");
    let linked_display = linked.to_string_lossy().into_owned();
    wait_for_buffer_text(&mut source, &output, "[git worktrees]", &linked_display).await;
    terminal.write_all(b"j\r").unwrap();
    terminal.flush().unwrap();

    let mut recovered = false;
    for _ in 0..200 {
        source.send(&ClientRequest::Health).await.unwrap();
        let attached = matches!(
            response(&mut source).await,
            HostResponse::Health {
                interactive_attached: true,
                ..
            }
        );
        let reported = String::from_utf8_lossy(&output.lock().unwrap()).contains("E1");
        if attached && reported {
            recovered = true;
            break;
        }
        if let Some(status) = switcher.0.as_mut().unwrap().try_wait().unwrap() {
            panic!("workspace switch error terminated the TUI: {status}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        recovered,
        "incompatible destination did not return to source; terminal output: {}",
        String::from_utf8_lossy(&output.lock().unwrap())
    );

    terminal.write_all(b":detach\r").unwrap();
    terminal.flush().unwrap();
    assert!(wait_child(switcher.0.as_mut().unwrap()).await.success());
    let _ = switcher.0.take().unwrap().wait();
    shutdown(&mut source).await;
    assert!(source_host.0.take().unwrap().wait().unwrap().success());
    drop(incompatible_listener);
    incompatible_endpoint.cleanup().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn creating_a_worktree_starts_and_attaches_its_persistent_session() {
    use std::io::Write;

    let root = project();
    git(&root, &["add", "note.txt", "other.txt"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    let created = root.join("created-from-ui");
    let source_endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut source_host) = start_host(&root, &source_endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut source = connect_control(&source_endpoint).await;
    let (switcher, mut terminal) = spawn_in_pty(
        Command::new(env!("CARGO_BIN_EXE_runyte"))
            .arg("--persistent")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let mut switcher = ChildGuard(Some(switcher));
    let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = std::sync::Arc::clone(&output);
    let drain = terminal.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut drain = drain;
        let mut sink = [0_u8; 4096];
        while let Ok(count) = std::io::Read::read(&mut drain, &mut sink) {
            if count == 0 {
                break;
            }
            captured.lock().unwrap().extend_from_slice(&sink[..count]);
        }
    });

    for _ in 0..200 {
        source.send(&ClientRequest::Health).await.unwrap();
        if matches!(
            response(&mut source).await,
            HostResponse::Health {
                interactive_attached: true,
                ..
            }
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    wait_for_terminal_output(&output, "other").await;
    wait_for_terminal_output(&output, "│ master").await;
    type_colon_command(&mut terminal, "git-worktrees");
    let root_display = root.to_string_lossy().into_owned();
    wait_for_buffer_text(&mut source, &output, "[git worktrees]", &root_display).await;
    terminal
        .write_all(format!("\tNcreated-from-ui\r{}\r", created.to_string_lossy()).as_bytes())
        .unwrap();
    terminal.flush().unwrap();

    let mut destination = None;
    for _ in 0..400 {
        if created.is_dir() {
            let canonical = created.canonicalize().unwrap();
            let endpoint = LocalEndpoint::discover_with_runtime(
                &canonical.join(".runyte"),
                &canonical,
                Some(test_runtime_dir()),
            )
            .unwrap();
            if let Ok(mut client) = try_connect_control(&endpoint).await {
                let health = if client.send(&ClientRequest::Health).await.is_ok() {
                    tokio::time::timeout(Duration::from_secs(1), client.recv())
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .flatten()
                } else {
                    None
                };
                if matches!(
                    health,
                    Some(HostResponse::Health {
                        interactive_attached: true,
                        ..
                    })
                ) {
                    destination = Some(client);
                    break;
                }
            }
        }
        if let Some(status) = switcher.0.as_mut().unwrap().try_wait().unwrap() {
            panic!("create-and-attach TUI exited before reaching the new worktree: {status}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut destination = destination.expect("new worktree session was not started and attached");
    source.send(&ClientRequest::Health).await.unwrap();
    assert!(matches!(
        response(&mut source).await,
        HostResponse::Health {
            interactive_attached: false,
            ..
        }
    ));
    let branch = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&created)
        .output()
        .unwrap();
    assert!(branch.status.success());
    assert_eq!(
        String::from_utf8_lossy(&branch.stdout).trim(),
        "created-from-ui"
    );

    terminal.write_all(b":detach\r").unwrap();
    terminal.flush().unwrap();
    assert!(wait_child(switcher.0.as_mut().unwrap()).await.success());
    let _ = switcher.0.take().unwrap().wait();
    shutdown(&mut source).await;
    shutdown(&mut destination).await;
    assert!(source_host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn persistent_tui_opens_async_log_and_shared_commit_detail() {
    let root = project();
    for (index, subject) in ["first", "second", "third"].into_iter().enumerate() {
        fs::write(root.join("note.txt"), format!("{index}\n")).unwrap();
        git(&root, &["add", "note.txt"]);
        git(&root, &["commit", "--quiet", "-m", subject]);
    }
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut host) = start_host(&root, &endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut interactive = LocalClient::connect(&endpoint, tui_geometry(), true)
        .await
        .unwrap();
    let _ = response(&mut interactive).await;
    let mut frame = match response(&mut interactive).await {
        HostResponse::Frame { frame } => *frame,
        response => panic!("expected initial history frame, got {response:?}"),
    };
    while frame
        .editor
        .status
        .git_summary
        .as_deref()
        .is_none_or(|summary| summary.contains(":git-cancel"))
    {
        frame = match response(&mut interactive).await {
            HostResponse::Frame { frame } => *frame,
            response => panic!("expected Git discovery frame, got {response:?}"),
        };
    }
    interactive
        .send(&ClientRequest::Invoke {
            command: CommandRequest::at(
                "git-log",
                frame.id,
                frame.active_buffer,
                frame.active_revision,
            ),
        })
        .await
        .unwrap();
    loop {
        match response(&mut interactive).await {
            HostResponse::CommandResult { .. } => break,
            HostResponse::Frame { .. } => {}
            response => panic!("expected Git log command result, got {response:?}"),
        }
    }
    loop {
        if let HostResponse::Frame { frame } = response(&mut interactive).await
            && frame
                .editor
                .panes
                .iter()
                .any(|pane| pane.active && pane.title.name == "[git log]")
        {
            break;
        }
    }
    interactive
        .send(&ClientRequest::Input {
            event: InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE)).into(),
            repeated: false,
        })
        .await
        .unwrap();
    loop {
        if let HostResponse::Frame { frame } = response(&mut interactive).await
            && frame
                .editor
                .panes
                .iter()
                .any(|pane| pane.active && pane.title.name.starts_with("[git commit "))
        {
            break;
        }
    }
    interactive.send(&ClientRequest::Detach).await.unwrap();
    let _ = response_ignoring_frames(&mut interactive).await;
    let mut control = connect_control(&endpoint).await;
    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn wait_without_a_host_starts_one_and_attaches_the_invoking_terminal() {
    let root = project();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let (mut waiter, terminal) = spawn_wait_in_pty(&root, "note.txt");
    // The attached TUI renders full frames while the wait is pending. Keep
    // draining its PTY so a platform's smaller PTY buffer cannot block the
    // client before it observes completion from the host.
    let terminal_output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = std::sync::Arc::clone(&terminal_output);
    std::thread::spawn(move || {
        use std::io::Read;

        let mut terminal = terminal;
        let mut chunk = [0_u8; 4096];
        while let Ok(count) = terminal.read(&mut chunk) {
            if count == 0 {
                break;
            }
            captured.lock().unwrap().extend_from_slice(&chunk[..count]);
        }
    });
    let mut control = None;
    for _ in 0..200 {
        if let Ok(client) = try_connect_control(&endpoint).await {
            control = Some(client);
            break;
        }
        if let Some(status) = waiter.try_wait().unwrap() {
            let output = String::from_utf8_lossy(&terminal_output.lock().unwrap()).into_owned();
            if output.contains("Operation not permitted") {
                fs::remove_dir_all(root).unwrap();
                return;
            }
            panic!("--wait exited before publishing a reachable host: {status}: {output:?}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut control = control.expect("--wait did not publish a workspace host");
    let mut attached = false;
    for _ in 0..100 {
        control.send(&ClientRequest::Health).await.unwrap();
        attached = matches!(
            response(&mut control).await,
            HostResponse::Health {
                interactive_attached: true,
                ..
            }
        );
        if attached {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        attached,
        "the no-host wait request never attached its terminal"
    );
    control.send(&ClientRequest::ListBuffers).await.unwrap();
    let note = match response(&mut control).await {
        HostResponse::Buffers { buffers } => buffers
            .into_iter()
            .find(|buffer| {
                buffer.path_bytes.clone().map(decode_path).as_deref()
                    == Some(root.join("note.txt").as_path())
            })
            .expect("requested buffer is not reachable through the host"),
        response => panic!("expected buffers, got {response:?}"),
    };
    control
        .send(&ClientRequest::CloseBuffer {
            buffer: note.id,
            discard: false,
        })
        .await
        .unwrap();
    assert!(matches!(
        response(&mut control).await,
        HostResponse::Closed { .. }
    ));
    assert!(wait_child(&mut waiter).await.success());

    shutdown(&mut control).await;
    for _ in 0..100 {
        if !endpoint.metadata().exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(!endpoint.metadata().exists());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn signalling_a_wait_client_cancels_its_durable_request() {
    let root = project();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut host) = start_host(&root, &endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let (mut waiter, mut terminal) = spawn_wait_in_pty(&root, "note.txt");
    let _output_drain = std::thread::spawn(move || {
        let mut chunk = [0_u8; 4096];
        while terminal.read(&mut chunk).is_ok_and(|read| read != 0) {}
    });
    let mut control = connect_control(&endpoint).await;
    let mut pending = false;
    for _ in 0..100 {
        control.send(&ClientRequest::Health).await.unwrap();
        pending = matches!(
            response(&mut control).await,
            HostResponse::Health {
                interactive_attached: true,
                pending_wait_requests: 1,
                ..
            }
        );
        if pending {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(pending, "wait request did not become durable and attached");

    // SAFETY: `waiter.id()` names this test's live child process.
    assert_eq!(
        unsafe { libc::kill(waiter.id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    assert_eq!(
        wait_child(&mut waiter).await.code(),
        Some(128 + libc::SIGTERM)
    );

    let mut cancelled = false;
    for _ in 0..100 {
        control.send(&ClientRequest::Health).await.unwrap();
        cancelled = matches!(
            response(&mut control).await,
            HostResponse::Health {
                interactive_attached: false,
                pending_wait_requests: 0,
                ..
            }
        );
        if cancelled {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        cancelled,
        "signalled wait request remained protected host state"
    );

    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

/// Publishes the endpoint a host of an older protocol would leave in a
/// project, recording `pid` as the process holding it. `None` means this
/// platform refused the socket, and the caller skips.
///
/// The metadata is written by hand because no build that speaks that protocol
/// exists here to write it. Its shape is the historical one: it predates the
/// registered identity and the optional name, which is exactly why the host
/// registry cannot account for such an endpoint.
fn publish_incompatible_endpoint(
    root: &Path,
    pid: u32,
) -> Option<(LocalEndpoint, tokio::net::UnixListener, u32)> {
    use std::os::unix::fs::PermissionsExt;

    let endpoint =
        LocalEndpoint::discover_with_runtime(&root.join(".runyte"), root, Some(test_runtime_dir()))
            .unwrap();
    let directory = endpoint.metadata().parent().unwrap();
    fs::create_dir_all(directory).unwrap();
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    let listener = match tokio::net::UnixListener::bind(endpoint.socket()) {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            fs::remove_dir_all(directory).unwrap();
            return None;
        }
        Err(error) => panic!("cannot bind test workspace host: {error}"),
    };
    fs::set_permissions(endpoint.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    let old_protocol = PROTOCOL_VERSION.checked_sub(1).unwrap();
    let metadata = serde_json::json!({
        "protocol": old_protocol,
        "pid": pid,
        "project_root_bytes": encode_path(root),
        "socket_bytes": encode_path(endpoint.socket()),
    });
    fs::write(endpoint.metadata(), serde_json::to_vec(&metadata).unwrap()).unwrap();
    fs::set_permissions(endpoint.metadata(), fs::Permissions::from_mode(0o600)).unwrap();
    Some((endpoint, listener, old_protocol))
}

/// Records a workspace in the recents this test binary's cache holds, which is
/// where `--session-list` looks for workspaces no registry entry covers.
fn record_recent(root: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let cache = test_cache_dir().join("runyte");
    fs::create_dir_all(&cache).unwrap();
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    let path = cache.join("workspaces.json");
    // Every Runyte process this binary spawns shares one recents file, and a
    // client records its own workspace on startup. Those writes take the
    // advisory lock beside the file, so this read-modify-write has to take it
    // too: without it a client that read the file first overwrites the entry
    // inserted here and the workspace vanishes from the listing under test.
    let _lock = RecentsLock::acquire(&path);
    let mut entries: Vec<serde_json::Value> = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    entries.insert(
        0,
        serde_json::json!({ "project_root_bytes": encode_path(root), "name": null }),
    );
    fs::write(&path, serde_json::to_vec(&entries).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
}

/// The same exclusive advisory lock the editor takes over the recents file,
/// held for as long as the value lives. The kernel drops it when the
/// descriptor closes, so a panicking test cannot strand it.
struct RecentsLock(File);

impl RecentsLock {
    fn acquire(recents: &Path) -> Self {
        use std::os::unix::fs::OpenOptionsExt;

        let file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            // The lock file is a rendezvous point, never a payload; its
            // contents are irrelevant and must survive being opened.
            .truncate(false)
            .mode(0o600)
            .open(recents.with_extension("lock"))
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        Self(file)
    }
}

impl Drop for RecentsLock {
    fn drop(&mut self) {
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// Writes a configuration so a spawned client reads this test's settings
/// rather than the settings of whoever is running the suite.
///
/// It lives beside the test's other private state and not inside the project,
/// because a configuration below a project root would make that root per-user
/// configuration storage, which may not also hold project state.
fn default_config(root: &Path) -> PathBuf {
    let name = root.file_name().unwrap().to_string_lossy();
    let path = test_runtime_dir().join(format!("{name}.yaml"));
    fs::write(&path, "workspace:\n  state: .runyte\n").unwrap();
    path
}

#[tokio::test]
async fn wait_preserves_the_error_from_a_live_incompatible_host() {
    let root = project();
    let Some((endpoint, listener, old_protocol)) =
        publish_incompatible_endpoint(&root, std::process::id())
    else {
        fs::remove_dir_all(root).unwrap();
        return;
    };

    let output = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .arg("--wait")
        .arg("note.txt")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains(&format!(
            "workspace host protocol {old_protocol} is incompatible with client protocol {PROTOCOL_VERSION}"
        )),
        "{stderr}"
    );
    assert!(!stderr.contains("already listening"), "{stderr}");
    // The error has to lead somewhere: the host holding this workspace cannot
    // be spoken to, so naming the process and the command that ends it is the
    // only way out of the loop it otherwise puts the caller in.
    assert!(stderr.contains("--session-stop"), "{stderr}");
    assert!(
        stderr.contains(&format!("process {} is still running", std::process::id())),
        "{stderr}"
    );

    drop(listener);
    endpoint.cleanup().unwrap();
    fs::remove_dir_all(root).unwrap();
}

/// A host of an older protocol is running and cannot be asked anything. It
/// still has to be visible and explicitly force-stoppable: it holds the
/// endpoint every client in that project resolves to, but an incompatible
/// client cannot know whether killing it would lose terminal sessions.
#[tokio::test]
async fn a_live_incompatible_host_is_listed_and_can_be_force_stopped() {
    let root = project();
    let config = default_config(&root);
    let mut host = Command::new("sleep").arg("120").spawn().unwrap();
    let host_pid = host.id();
    // A host in the field is nobody's child and is reaped by init. Here it is
    // this test's child, and a zombie still answers `kill(pid, 0)`, so the
    // wait has to run beside the stop rather than after it.
    let reaper = std::thread::spawn(move || {
        let _ = host.wait();
    });
    let Some((endpoint, listener, old_protocol)) = publish_incompatible_endpoint(&root, host_pid)
    else {
        let _ = Command::new("kill").arg(host_pid.to_string()).status();
        reaper.join().unwrap();
        fs::remove_dir_all(root).unwrap();
        return;
    };
    record_recent(&root);

    let listed = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["--config".as_ref(), config.as_os_str(), "-l".as_ref()])
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .output()
        .unwrap();
    let listing = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .find(|line| line.contains(&root.display().to_string()))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("workspace missing from listing: {listed:?}"));
    assert!(
        listing.contains(&format!("running (protocol {old_protocol})")),
        "{listing}"
    );

    let stopped = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args([
            "--config".as_ref(),
            config.as_os_str(),
            "--session-stop".as_ref(),
            root.as_os_str(),
            "--force".as_ref(),
        ])
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&stopped.stderr);
    assert!(stopped.status.success(), "{stderr}");
    // Force-stopping a host that cannot be asked about its protected state is
    // explicit and is not reported as if it had cooperated.
    assert!(
        stderr.contains("force-stopped persistent session"),
        "{stderr}"
    );
    assert!(
        stderr.contains("protected live state was discarded"),
        "{stderr}"
    );
    reaper.join().unwrap();
    assert!(!endpoint.metadata().exists());
    assert!(!endpoint.socket().exists());

    let relisted = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["--config".as_ref(), config.as_os_str(), "-l".as_ref()])
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .output()
        .unwrap();
    let relisting = String::from_utf8_lossy(&relisted.stdout)
        .lines()
        .find(|line| line.contains(&root.display().to_string()))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("workspace missing from listing: {relisted:?}"));
    assert!(relisting.contains("stopped"), "{relisting}");

    drop(listener);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn wait_terminal_hangup_is_not_reported_as_success() {
    let root = project();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let (mut waiter, terminal) = spawn_wait_in_pty(&root, "note.txt");
    let mut control = None;
    for _ in 0..200 {
        if let Ok(client) = try_connect_control(&endpoint).await {
            control = Some(client);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut control = control.expect("wait host did not become reachable");
    for _ in 0..100 {
        control.send(&ClientRequest::Health).await.unwrap();
        if matches!(
            response(&mut control).await,
            HostResponse::Health {
                interactive_attached: true,
                ..
            }
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    drop(terminal);
    assert!(!wait_child(&mut waiter).await.success());
    if control.send(&ClientRequest::Shutdown).await.is_ok() {
        let _ = tokio::time::timeout(Duration::from_secs(1), control.recv()).await;
    }
    fs::remove_dir_all(root).unwrap();
}

/// A host that is shutting down owns the last message on every connection it
/// holds. Exiting with one still in flight truncates it, and the client reads
/// a message that ends inside itself: a transport error for what is an
/// ordinary end of session.
#[tokio::test]
async fn an_interactive_quit_flushes_its_shutdown_response_without_a_control_client() {
    let root = project();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut host) = start_host(&root, &endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut interactive = LocalClient::connect(&endpoint, FrameGeometry::default(), true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut interactive).await,
        HostResponse::Welcome { .. }
    ));
    let frame = match response(&mut interactive).await {
        HostResponse::Frame { frame } => *frame,
        response => panic!("expected initial frame, got {response:?}"),
    };
    interactive
        .send(&ClientRequest::Invoke {
            command: CommandRequest::at(
                "quit",
                frame.id,
                frame.active_buffer,
                frame.active_revision,
            ),
        })
        .await
        .unwrap();
    assert!(matches!(
        response_ignoring_frames(&mut interactive).await,
        HostResponse::CommandResult { .. }
    ));
    assert_eq!(
        response_ignoring_frames(&mut interactive).await,
        HostResponse::ShuttingDown
    );
    assert!(
        end_of_stream(&mut interactive).await,
        "the interactive shutdown response was truncated"
    );
    assert!(wait_child(host.0.as_mut().unwrap()).await.success());
    fs::remove_dir_all(root).unwrap();
}

/// A control response can be much larger than a socket buffer, so shutdown
/// also keeps control connections alive until every queued semantic reply is
/// complete.
#[tokio::test]
async fn a_shutting_down_host_finishes_its_last_message_before_exiting() {
    let root = project();
    // Larger than any local socket send buffer, so the reply cannot be handed
    // to the kernel in one write and is certain to still be in flight when
    // the shutdown that follows it is answered.
    fs::write(
        root.join("large.txt"),
        "abcdefghijklmnopqrstuvwxyz\n".repeat(80_000),
    )
    .unwrap();
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut host) = start_host_opening(&root, &endpoint, Some("large.txt")).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut control = connect_control(&endpoint).await;
    control.send(&ClientRequest::ListBuffers).await.unwrap();
    let buffers = match response(&mut control).await {
        HostResponse::Buffers { buffers } => buffers,
        response => panic!("expected buffers, got {response:?}"),
    };
    let large = buffers
        .iter()
        .find(|buffer| {
            buffer.path_bytes.clone().map(decode_path).as_deref()
                == Some(root.join("large.txt").as_path())
        })
        .expect("host did not open the large file")
        .id;

    // Both requests are sent before either reply is read, so the host has
    // queued the whole buffer and then `ShuttingDown` behind it by the time
    // it leaves its loop.
    control
        .send(&ClientRequest::ReadBuffer { buffer: large })
        .await
        .unwrap();
    control.send(&ClientRequest::Shutdown).await.unwrap();
    assert!(matches!(
        response_ignoring_frames(&mut control).await,
        // The protocol caps buffer text at a mebibyte, which is already
        // several times any local socket send buffer.
        HostResponse::Buffer { buffer } if buffer.text.len() > 512 * 1024
    ));
    assert!(matches!(
        response_ignoring_frames(&mut control).await,
        HostResponse::ShuttingDown
    ));
    assert!(
        end_of_stream(&mut control).await,
        "the connection did not end on a message boundary"
    );
    assert!(wait_child(host.0.as_mut().unwrap()).await.success());
    fs::remove_dir_all(root).unwrap();
}

/// Reads until the host closes the connection, reporting whether it did so
/// cleanly. A message that ends inside itself is an error rather than an end
/// of stream, so this distinguishes a flushed shutdown from a truncated one.
async fn end_of_stream(client: &mut LocalClient) -> bool {
    loop {
        let response = tokio::time::timeout(Duration::from_secs(5), client.recv())
            .await
            .expect("host did not close the connection");
        match response {
            Ok(None) => return true,
            Ok(Some(_)) => {}
            Err(error) => panic!("connection ended uncleanly: {error}"),
        }
    }
}

/// Switching between workspaces must stay in one process. The previous
/// arrangement spawned a child `runyte --persistent` and blocked on it, so moving
/// from one workspace to another and back again stacked processes and quitting
/// unwound a stack instead of ending the session.
#[tokio::test]
async fn relative_workspace_attach_uses_editor_cwd_and_keeps_one_client_process() {
    use std::io::Write;

    let root = project();
    git(&root, &["add", "note.txt", "other.txt"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    git(
        &root,
        &["worktree", "add", "--quiet", "-b", "linked", "linked"],
    );
    fs::create_dir(root.join("nested")).unwrap();
    let linked = root.join("linked").canonicalize().unwrap();
    let source_endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let linked_endpoint = LocalEndpoint::discover_with_runtime(
        &linked.join(".runyte"),
        &linked,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(mut source_host) = start_host(&root, &source_endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let Some(mut linked_host) = start_host(&linked, &linked_endpoint).await else {
        let mut source = connect_control(&source_endpoint).await;
        shutdown(&mut source).await;
        let _ = source_host.0.take().unwrap().wait();
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut source = connect_control(&source_endpoint).await;
    let mut destination = connect_control(&linked_endpoint).await;
    let (switcher, mut terminal) = spawn_in_pty(
        Command::new(env!("CARGO_BIN_EXE_runyte"))
            .arg("--persistent")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let client_pid = switcher.id();
    let mut switcher = ChildGuard(Some(switcher));
    // Nothing else reads this PTY, and two full-screen TUIs render into it in
    // sequence. Once the terminal buffer fills, the attached editor blocks
    // writing a frame and stops reading input, so later keys are never seen.
    let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = std::sync::Arc::clone(&output);
    let drain = terminal.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut drain = drain;
        let mut sink = [0_u8; 4096];
        while let Ok(count) = std::io::Read::read(&mut drain, &mut sink) {
            if count == 0 {
                break;
            }
            captured.lock().unwrap().extend_from_slice(&sink[..count]);
        }
    });

    let mut attached_to_source = false;
    for _ in 0..200 {
        source.send(&ClientRequest::Health).await.unwrap();
        attached_to_source = matches!(
            response(&mut source).await,
            HostResponse::Health {
                interactive_attached: true,
                ..
            }
        );
        if attached_to_source {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(attached_to_source, "the source host never received the TUI");
    let root_display = root.to_string_lossy().into_owned();
    wait_for_terminal_output(&output, "other").await;

    // The client process stays at `root`, while `:cd` changes only the
    // editor-owned directory to `root/nested`. Resolving `../linked` against
    // the client cwd would therefore look outside this project and fail; the
    // intended destination can only be reached through the editor cwd carried
    // in the switch handoff.
    terminal
        .write_all(b":cd nested\r:session-attach ../linked\r")
        .unwrap();
    terminal.flush().unwrap();
    let mut attached_to_destination = false;
    for _ in 0..200 {
        destination.send(&ClientRequest::Health).await.unwrap();
        attached_to_destination = matches!(
            response(&mut destination).await,
            HostResponse::Health {
                interactive_attached: true,
                ..
            }
        );
        if attached_to_destination {
            break;
        }
        if let Some(status) = switcher.0.as_mut().unwrap().try_wait().unwrap() {
            panic!("the relative-switching client exited before reattaching: {status}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        attached_to_destination,
        "the relative selector did not reach the destination host"
    );
    wait_for_terminal_output(&output, "linked/other.txt").await;

    // Return through the worktree list to retain the original regression that
    // switching both ways stays inside one client process. The linked
    // worktree is the second row, so the main worktree is immediately above it.
    wait_for_terminal_output(&output, "│ linked").await;
    type_colon_command(&mut terminal, "git-worktrees");
    wait_for_buffer_text(&mut destination, &output, "[git worktrees]", &root_display).await;
    terminal.write_all(b"k\r").unwrap();
    terminal.flush().unwrap();
    let mut returned_to_source = false;
    for _ in 0..200 {
        source.send(&ClientRequest::Health).await.unwrap();
        returned_to_source = matches!(
            response(&mut source).await,
            HostResponse::Health {
                interactive_attached: true,
                ..
            }
        );
        if returned_to_source {
            break;
        }
        if let Some(status) = switcher.0.as_mut().unwrap().try_wait().unwrap() {
            panic!("the switching client exited instead of returning: {status}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        returned_to_source,
        "the client did not return to the source"
    );

    // The client that came back is the one that started.
    assert_eq!(switcher.0.as_ref().unwrap().id(), client_pid);
    // The assertion that actually distinguishes a loop from the re-exec it
    // replaced: waiting on a child would keep this process's PID too, so the
    // absence of a child is what proves nothing was stacked behind it. Only
    // Linux publishes this, and an unreadable file would assert nothing, so the
    // check is explicit about where it runs.
    #[cfg(target_os = "linux")]
    {
        let children =
            std::fs::read_to_string(format!("/proc/{client_pid}/task/{client_pid}/children"))
                .expect("Linux publishes a process's children");
        assert!(
            children.trim().is_empty(),
            "the switching client kept a child process: {children:?}"
        );
    }

    terminal.write_all(b":detach\r").unwrap();
    terminal.flush().unwrap();
    assert!(wait_child(switcher.0.as_mut().unwrap()).await.success());
    let _ = switcher.0.take().unwrap().wait();
    shutdown(&mut source).await;
    shutdown(&mut destination).await;
    assert!(source_host.0.take().unwrap().wait().unwrap().success());
    assert!(linked_host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}
