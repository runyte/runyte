// SPDX-License-Identifier: MPL-2.0

//! Durable diagnostic logging, exercised through real processes.
//!
//! The unit tests beside the module cover level filtering, rotation bounds,
//! record shape, and queue saturation without globals. These cover what only a
//! process can show: which file a role owns, that a detached host keeps
//! writing, that an attachment leaves a running host's logger alone, that a
//! client never appends to a host's file, that an explicit destination has one
//! live owner, that framing failures reach the host log, and that no document
//! text, clipboard value, terminal output, environment value, or propagated
//! top-level error chain reaches a record.

#![cfg(unix)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

use runyte::{
    app::FrameGeometry,
    input::InputEvent,
    layout::Rect,
    log::{HOST_LOG_NAME, Level, MAX_LOG_BYTES, Role, Settings, Sink, default_path, previous_path},
    test_support::TestRuntimeRoot,
    workspace::transport::{
        CLIENT_VERSION, ClientKind, ClientRequest, ClientRole, FeatureGroup, HostResponse,
        LocalClient, LocalEndpoint, PROTOCOL_VERSION, encode_path,
    },
};
use tokio::{io::AsyncWriteExt, net::UnixStream};

/// A private, short process directory for one test workspace.
///
/// The test binary runs several real hosts in parallel. Giving each workspace
/// its own runtime registry and cache prevents their endpoint cleanup and
/// recent-workspace writes from contending with unrelated assertions. The
/// abbreviated identity also keeps macOS Unix socket paths below their limit.
fn test_process_dir(root: &Path, kind: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = root
        .parent()
        .expect("a diagnostic project has an owning test root")
        .join(kind);
    fs::create_dir_all(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn test_runtime_dir(root: &Path) -> PathBuf {
    test_process_dir(root, "run")
}

fn test_cache_dir(root: &Path) -> PathBuf {
    test_process_dir(root, "cache")
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

/// A project directory that is removed even when an assertion unwinds past
/// it. Explicit removal at the end of each test left one tree per failure
/// behind in the temporary directory.
struct Project {
    root: PathBuf,
    _owner: TestRuntimeRoot,
}

impl std::ops::Deref for Project {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.root
    }
}

impl AsRef<Path> for Project {
    fn as_ref(&self) -> &Path {
        &self.root
    }
}

fn project(name: &str) -> Project {
    let owner = TestRuntimeRoot::new(name).unwrap();
    let root = owner.join("project");
    fs::create_dir_all(root.join(".runyte")).unwrap();
    fs::write(root.join("note.txt"), "base\n").unwrap();
    Project {
        root: root.canonicalize().unwrap(),
        _owner: owner,
    }
}

fn bundled_runyte(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    command
        .env(
            "RUNYTE_ALL_HOSTS_DIR",
            test_runtime_dir(root).join("runyte/all-hosts"),
        )
        .env("RUNYTE_TEST_SUPERVISOR_PID", std::process::id().to_string());
    command
}

fn runyte(root: &Path, arguments: &[&str]) -> std::process::Output {
    bundled_runyte(root)
        .args(arguments)
        .current_dir(root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir(root))
        .env("XDG_CACHE_HOME", test_cache_dir(root))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

async fn runyte_bounded(root: &Path, arguments: &[&str]) -> std::process::Output {
    let mut command = bundled_runyte(root);
    command
        .args(arguments)
        .current_dir(root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir(root))
        .env("XDG_CACHE_HOME", test_cache_dir(root))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ChildGuard(Some(command.spawn().unwrap()));

    for _ in 0..200 {
        if child.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            return child.0.take().unwrap().wait_with_output().unwrap();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let mut process = child.0.take().unwrap();
    let _ = process.kill();
    let output = process.wait_with_output().unwrap();
    panic!(
        "Runyte subprocess did not exit within five seconds\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Runs a standalone editor that fails before it can enter the terminal.
///
/// Two binary startup targets are refused by name, so the process reaches
/// exactly the boundaries logging cares about — startup, then the failure that
/// ends it — on every platform and without a controlling terminal.
fn standalone_that_fails(root: &Path, extra: &[&str]) -> std::process::Output {
    fs::write(root.join("first.bin"), [0u8, 1, 2, 3]).unwrap();
    fs::write(root.join("second.bin"), [0u8, 4, 5, 6]).unwrap();
    let mut arguments = vec!["--standalone", "--project-root", root.to_str().unwrap()];
    arguments.extend_from_slice(extra);
    arguments.push("first.bin");
    arguments.push("second.bin");
    let output = runyte(root, &arguments);
    assert!(
        !output.status.success(),
        "the launch was expected to fail: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn read_log(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn levels(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect()
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

fn endpoint_for(root: &Path) -> LocalEndpoint {
    LocalEndpoint::discover_with_runtime(&root.join(".runyte"), root, Some(&test_runtime_dir(root)))
        .unwrap()
}

fn serve(root: &Path, extra: &[&str]) -> ChildGuard {
    let mut command = bundled_runyte(root);
    command
        .arg("--serve")
        .arg("--project-root")
        .arg(root)
        .args(extra)
        .current_dir(root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir(root))
        .env("XDG_CACHE_HOME", test_cache_dir(root))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    ChildGuard(Some(command.spawn().unwrap()))
}

async fn wait_for_endpoint(child: &mut ChildGuard, endpoint: &LocalEndpoint) -> bool {
    for _ in 0..200 {
        if endpoint.metadata().exists() {
            return true;
        }
        if child.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            let output = child.0.take().unwrap().wait_with_output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Operation not permitted") {
                return false;
            }
            panic!("host exited before publishing an endpoint: {stderr}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("host endpoint was not published");
}

async fn wait_until(mut ready: impl FnMut() -> bool) -> bool {
    for _ in 0..200 {
        if ready() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

/// Runs a colon command the way a person would, through the command prompt.
async fn type_command(client: &mut LocalClient, command: &str) {
    for event in [
        InputEvent::from(runyte::input::KeyStroke::new(
            runyte::input::KeyCode::Char(':'),
            runyte::input::Modifiers::NONE,
        )),
        InputEvent::Text(command.to_owned()),
        InputEvent::from(runyte::input::KeyStroke::new(
            runyte::input::KeyCode::Enter,
            runyte::input::Modifiers::NONE,
        )),
    ] {
        client
            .send(&ClientRequest::Input {
                event: event.into(),
                repeated: false,
            })
            .await
            .unwrap();
    }
}

async fn response_ignoring_visuals(client: &mut LocalClient) -> HostResponse {
    loop {
        let response = response(client).await;
        if !matches!(
            response,
            HostResponse::Frame { .. } | HostResponse::TerminalDamage { .. }
        ) {
            return response;
        }
    }
}

async fn wait_for_git_discovery(endpoint: &LocalEndpoint) {
    let mut client = LocalClient::connect(endpoint, geometry(), true)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut client).await,
        HostResponse::Welcome { .. }
    ));
    for event in [
        InputEvent::from(runyte::input::KeyStroke::new(
            runyte::input::KeyCode::Char(':'),
            runyte::input::Modifiers::NONE,
        )),
        InputEvent::Text("git-worktrees".to_owned()),
    ] {
        client
            .send(&ClientRequest::Input {
                event: event.into(),
                repeated: false,
            })
            .await
            .unwrap();
    }
    loop {
        match response(&mut client).await {
            HostResponse::Frame { frame } => {
                let row = frame
                    .overlays
                    .iter()
                    .find(|overlay| overlay.title == "Commands")
                    .and_then(|overlay| {
                        overlay
                            .rows
                            .iter()
                            .find(|row| row.label.contains(":git-worktrees"))
                    });
                if row.is_some_and(|row| {
                    row.available || !row.detail.contains("discovery is still in progress")
                }) {
                    break;
                }
            }
            HostResponse::TerminalDamage { .. } => {
                client.send(&ClientRequest::Resynchronize).await.unwrap();
            }
            response => panic!("expected a Git discovery frame, got {response:?}"),
        }
    }
    client.send(&ClientRequest::Detach).await.unwrap();
    assert!(matches!(
        response_ignoring_visuals(&mut client).await,
        HostResponse::Detached { .. }
    ));
}

async fn response(client: &mut LocalClient) -> HostResponse {
    tokio::time::timeout(Duration::from_secs(5), client.recv())
        .await
        .expect("host response timed out")
        .unwrap()
        .expect("host disconnected")
}

#[test]
fn a_standalone_process_keeps_warnings_and_errors_and_omits_the_rest() {
    let root = project("default-level");
    standalone_that_fails(&root, &[]);

    let directory = root.join(".runyte");
    let logs = fs::read_dir(&directory)
        .unwrap()
        .filter_map(|entry| {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            name.starts_with("standalone-").then_some(name)
        })
        .collect::<Vec<_>>();
    assert_eq!(logs.len(), 1, "a standalone process owns one log: {logs:?}");
    assert!(logs[0].ends_with(".log"));

    let text = read_log(&directory.join(&logs[0]));
    let levels = levels(&text);
    assert!(
        levels.contains(&"ERROR"),
        "the failure that ended the process is retained:\n{text}"
    );
    for absent in ["INFO", "DEBUG", "TRACE"] {
        assert!(
            !levels.contains(&absent),
            "{absent} was retained at the default level:\n{text}"
        );
    }
    assert!(text.contains("standalone["), "{text}");
}

#[test]
fn repeated_verbosity_raises_the_level_and_stops_at_trace() {
    for (flags, expected, absent) in [
        (vec!["-v"], vec!["ERROR", "INFO"], vec!["DEBUG", "TRACE"]),
        (
            vec!["-v", "-v"],
            vec!["ERROR", "INFO", "DEBUG"],
            vec!["TRACE"],
        ),
        (
            vec!["-v", "-v", "-v"],
            vec!["ERROR", "INFO", "DEBUG", "TRACE"],
            vec![],
        ),
        // A fourth occurrence cannot raise the level past trace.
        (
            vec!["-v", "-v", "-v", "-v", "-v"],
            vec!["ERROR", "INFO", "DEBUG", "TRACE"],
            vec![],
        ),
    ] {
        let root = project("verbosity");
        let destination = root.join("explicit.log");
        let mut arguments = flags.clone();
        arguments.push("--log");
        arguments.push(destination.to_str().unwrap());
        standalone_that_fails(&root, &arguments);

        let text = read_log(&destination);
        let levels = levels(&text);
        for level in &expected {
            assert!(levels.contains(level), "{flags:?} lost {level}:\n{text}");
        }
        for level in &absent {
            assert!(!levels.contains(level), "{flags:?} kept {level}:\n{text}");
        }
        drop(root);
    }
}

#[test]
fn concurrent_standalone_processes_never_share_a_writable_log() {
    let root = project("concurrent");
    fs::write(root.join("first.bin"), [0u8, 1, 2, 3]).unwrap();
    fs::write(root.join("second.bin"), [0u8, 4, 5, 6]).unwrap();
    let arguments = [
        "--standalone",
        "--project-root",
        root.to_str().unwrap(),
        "first.bin",
        "second.bin",
    ];
    let children = (0..2)
        .map(|_| {
            bundled_runyte(&root)
                .args(arguments)
                .current_dir(&root)
                .env("XDG_RUNTIME_DIR", test_runtime_dir(&root))
                .env("XDG_CACHE_HOME", test_cache_dir(&root))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect::<Vec<_>>();
    for mut child in children {
        let _ = child.wait();
    }

    let mut logs = fs::read_dir(root.join(".runyte"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("standalone-"))
        .collect::<Vec<_>>();
    logs.sort();
    assert_eq!(
        logs.len(),
        2,
        "each standalone process must own its own file: {logs:?}"
    );
    assert_ne!(logs[0], logs[1]);
    for name in &logs {
        assert!(read_log(&root.join(".runyte").join(name)).contains("ERROR"));
    }
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn a_second_process_is_refused_when_an_explicit_log_is_owned() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let root = project("explicit-owner");
    let destination = root.join("shared.log");
    // Opening this FIFO for writing blocks after logger initialization. That
    // gives the test one real Runyte process which demonstrably holds the
    // explicit destination without needing a terminal or local socket.
    let trace = root.join("hold-open.fifo");
    let trace_bytes = CString::new(trace.as_os_str().as_bytes()).unwrap();
    // SAFETY: `trace_bytes` is a live, NUL-terminated path and the mode is a
    // conventional private FIFO mode.
    assert_eq!(unsafe { libc::mkfifo(trace_bytes.as_ptr(), 0o600) }, 0);
    let mut command = bundled_runyte(&root);
    command
        .args([
            "--standalone",
            "--project-root",
            root.to_str().unwrap(),
            "-v",
            "--log",
            destination.to_str().unwrap(),
        ])
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir(&root))
        .env("XDG_CACHE_HOME", test_cache_dir(&root))
        .env("RUNYTE_INPUT_TRACE", &trace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut owner = ChildGuard(Some(command.spawn().unwrap()));
    assert!(
        wait_until(|| destination.exists() && read_log(&destination).contains("runyte ")).await,
        "the first process must demonstrably own and write the destination"
    );
    assert!(
        owner.0.as_mut().unwrap().try_wait().unwrap().is_none(),
        "the first owner exited before the competing process started"
    );

    let output = standalone_that_fails(&root, &["--log", destination.to_str().unwrap()]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already owned by another running Runyte process"),
        "{stderr}"
    );
    assert!(stderr.contains("choose a different --log path"), "{stderr}");

    let text = read_log(&destination);
    let owner_pid = owner.0.as_ref().unwrap().id();
    assert!(
        text.contains(&format!("standalone[{owner_pid}]")),
        "the owning process left no record:\n{text}"
    );
    assert!(
        text.lines()
            .all(|line| line.contains(&format!("standalone[{owner_pid}]"))),
        "the refused process wrote to the shared destination:\n{text}"
    );

    drop(owner);
}

#[test]
fn an_invalid_explicit_destination_fails_startup_clearly() {
    let root = project("explicit-failure");
    let blocker = root.join("blocker");
    fs::write(&blocker, "not a directory").unwrap();

    let output = runyte(
        &root,
        &[
            "--standalone",
            "--project-root",
            root.to_str().unwrap(),
            "--log",
            blocker.join("host.log").to_str().unwrap(),
        ],
    );

    assert!(!output.status.success(), "an unusable --log must not start");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("diagnostic log"), "{stderr}");
    assert!(stderr.contains("blocker"), "{stderr}");
    // A refused explicit destination must not silently become another file.
    // The default this process would otherwise have chosen is the standalone
    // one, so that is the name a fallback would leave behind.
    let fallbacks = fs::read_dir(root.join(".runyte"))
        .unwrap()
        .filter(|entry| {
            let name = entry.as_ref().unwrap().file_name();
            let name = name.to_string_lossy();
            name.starts_with("standalone-") || name == HOST_LOG_NAME
        })
        .count();
    assert_eq!(fallbacks, 0, "a refused --log fell back to a default path");
}

#[tokio::test]
async fn an_unusable_default_destination_leaves_the_host_serving() {
    let root = project("degraded");
    // A directory where the file belongs: unwritable for every user, root
    // included, and portable in a way a permission bit is not.
    fs::create_dir_all(root.join(".runyte").join(HOST_LOG_NAME)).unwrap();

    let mut child = serve(&root, &[]);
    let endpoint = endpoint_for(&root);
    if !wait_for_endpoint(&mut child, &endpoint).await {
        return;
    }

    let mut client = LocalClient::connect(&endpoint, geometry(), false)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut client).await,
        HostResponse::Welcome { .. }
    ));
    client.send(&ClientRequest::Health).await.unwrap();
    assert!(
        matches!(response(&mut client).await, HostResponse::Health { .. }),
        "a host with no usable log still serves its workspace"
    );

    drop(client);
    drop(child);
}

#[tokio::test]
async fn a_host_owns_host_log_and_records_client_lifecycle_while_detached() {
    let root = project("host-owned");
    let mut child = serve(&root, &["-v"]);
    let endpoint = endpoint_for(&root);
    if !wait_for_endpoint(&mut child, &endpoint).await {
        return;
    }

    let log = default_path(&root.join(".runyte"), Role::Host, 0);
    assert_eq!(log.file_name().unwrap(), HOST_LOG_NAME);
    assert!(
        wait_until(|| log.exists()).await,
        "a persistent host owns host.log"
    );

    let mut client = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    let welcome = response(&mut client).await;
    assert!(
        matches!(welcome, HostResponse::Welcome { .. }),
        "expected the first attachment welcome, got {welcome:?}"
    );
    assert!(
        wait_until(|| read_log(&log).contains("interactive client attached")).await,
        "{}",
        read_log(&log)
    );
    // Detachment and disconnection are separate facts the host observes.
    client.send(&ClientRequest::Detach).await.unwrap();
    assert_eq!(
        response_ignoring_visuals(&mut client).await,
        HostResponse::Detached {
            directory_bytes: None,
        }
    );
    assert!(
        wait_until(|| read_log(&log).contains("interactive client detached")).await,
        "{}",
        read_log(&log)
    );
    drop(client);

    let mut abandoning = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    let welcome = response(&mut abandoning).await;
    assert!(
        matches!(welcome, HostResponse::Welcome { .. }),
        "expected the abandoning attachment welcome, got {welcome:?}"
    );
    drop(abandoning);
    assert!(
        wait_until(|| read_log(&log).contains("interactive client disconnected")).await,
        "{}",
        read_log(&log)
    );
    let detached = read_log(&log);
    assert!(
        detached.contains("host["),
        "every record names the owning host role:\n{detached}"
    );
    assert!(
        !detached.contains("standalone["),
        "nothing else appends to a host's log:\n{detached}"
    );

    // A client that arrives later opens the same file the host has been
    // writing the whole time, through the ordinary read-only buffer.
    let mut reattached = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    let welcome = response(&mut reattached).await;
    assert!(
        matches!(welcome, HostResponse::Welcome { .. }),
        "expected the final attachment welcome, got {welcome:?}"
    );
    assert!(read_log(&log).contains("persistent session published"));

    type_command(&mut reattached, "log-open").await;
    reattached.send(&ClientRequest::ListBuffers).await.unwrap();
    let log_buffer = match response_ignoring_visuals(&mut reattached).await {
        HostResponse::Buffers { buffers } => {
            buffers
                .into_iter()
                .find(|buffer| !buffer.closed && buffer.name == "[log]")
                .unwrap_or_else(|| panic!("the log command did not open [log]"))
                .id
        }
        response => panic!("expected buffers after log-open, got {response:?}"),
    };
    reattached
        .send(&ClientRequest::ReadBuffer { buffer: log_buffer })
        .await
        .unwrap();
    let text = match response_ignoring_visuals(&mut reattached).await {
        HostResponse::Buffer { buffer } => buffer.text,
        response => panic!("expected [log] contents, got {response:?}"),
    };
    assert!(text.contains("host owner"), "{text}");
    assert!(text.contains(HOST_LOG_NAME), "{text}");
    assert!(
        text.contains("persistent session published"),
        "the buffer shows the host's own records:\n{text}"
    );

    drop(reattached);
    drop(child);
}

#[tokio::test]
async fn a_malformed_frame_is_recorded_in_host_log_at_the_default_level() {
    let root = project("malformed-frame");
    let mut child = serve(&root, &[]);
    let endpoint = endpoint_for(&root);
    if !wait_for_endpoint(&mut child, &endpoint).await {
        return;
    }
    let log = root.join(".runyte").join(HOST_LOG_NAME);
    assert!(wait_until(|| log.exists()).await);

    let mut stream = UnixStream::connect(endpoint.socket()).await.unwrap();
    let hello = ClientRequest::Hello {
        protocol: PROTOCOL_VERSION,
        directory_handoff: false,
        features: vec![
            FeatureGroup::Control,
            FeatureGroup::Buffers,
            FeatureGroup::Wait,
        ],
        project_root_bytes: encode_path(&root),
        client_kind: ClientKind::Control,
        client_version: CLIENT_VERSION.to_owned(),
        role: ClientRole::Control,
        geometry: geometry().into(),
    };
    let mut frame = serde_json::to_vec(&hello).unwrap();
    frame.push(b'\n');
    stream.write_all(&frame).await.unwrap();
    stream.write_all(b"{not-json}\n").await.unwrap();
    stream.shutdown().await.unwrap();

    assert!(
        wait_until(|| {
            read_log(&log).lines().any(|line| {
                line.contains("WARN")
                    && line.contains("transport: client connection failed")
                    && line.contains("malformed workspace transport message")
            })
        })
        .await,
        "the framing failure was not retained at the default level:\n{}",
        read_log(&log)
    );

    drop(child);
}

#[tokio::test]
async fn attaching_with_logging_flags_reports_the_retained_configuration() {
    let root = project("retained");
    // The host runs at the default level; the wait client asks for debug. A
    // host that had adopted the client's flags would start emitting DEBUG
    // records into this same file.
    let mut child = serve(&root, &[]);
    let endpoint = endpoint_for(&root);
    if !wait_for_endpoint(&mut child, &endpoint).await {
        return;
    }
    let log = root.join(".runyte").join(HOST_LOG_NAME);
    assert!(wait_until(|| log.exists()).await);
    wait_for_git_discovery(&endpoint).await;
    let before = read_log(&log);

    // A rejected wait request reaches the existing host and reports its
    // retained logger without entering the TUI. Piped stdio is not enough to
    // prevent Crossterm from reopening /dev/tty when the test runner owns a
    // controlling terminal.
    fs::write(root.join("attachment.bin"), [0_u8, 1, 2, 3]).unwrap();
    let output = runyte_bounded(
        &root,
        &[
            "--wait",
            "-v",
            "-v",
            "--log",
            root.join("elsewhere.log").to_str().unwrap(),
            "attachment.bin",
        ],
    )
    .await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "a binary wait target was unexpectedly accepted"
    );
    assert!(
        stderr.contains("binary files cannot be opened through the workspace protocol"),
        "wait client did not report the intended bounded refusal: {stderr}"
    );
    assert!(
        stderr.contains("kept its own log level and destination"),
        "{stderr}"
    );
    assert!(stderr.contains("--session-restart"), "{stderr}");

    assert!(
        !root.join("elsewhere.log").exists(),
        "an attachment must not open a second destination for a running host"
    );
    // Provoke records the raised level would have admitted, then confirm the
    // host still admits none of them.
    let mut client = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    let welcome = response(&mut client).await;
    assert!(matches!(welcome, HostResponse::Welcome { .. }));
    client.send(&ClientRequest::Detach).await.unwrap();
    assert_eq!(
        response_ignoring_visuals(&mut client).await,
        HostResponse::Detached {
            directory_bytes: None,
        }
    );
    drop(client);

    // `Detached` is the host's acknowledgement that it handled the client
    // transition. At the retained Warn level, no attach/detach record is
    // admitted, so there is no elapsed-time barrier to wait through.
    let after = read_log(&log);
    assert_eq!(after, before, "the running session kept its own level");
    for raised in ["INFO", "DEBUG", "TRACE"] {
        assert!(
            !levels(&after).contains(&raised),
            "the attachment's flags reached the running session:\n{after}"
        );
    }

    drop(child);
}

#[tokio::test]
async fn client_side_failures_never_append_to_the_host_log() {
    let root = project("client-failure");
    // `-v` so the host has records of its own: comparing an empty file with an
    // empty file would prove nothing about what a client can append.
    let mut child = serve(&root, &["-v"]);
    let endpoint = endpoint_for(&root);
    if !wait_for_endpoint(&mut child, &endpoint).await {
        return;
    }
    let log = root.join(".runyte").join(HOST_LOG_NAME);
    assert!(wait_until(|| read_log(&log).contains("persistent session published")).await);
    let before = read_log(&log);
    assert!(!before.is_empty(), "the host must have records to preserve");

    // A client command that fails entirely on its own side.
    let output = runyte(&root, &["--session-stop", "no-such-workspace"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no-such-workspace"), "{stderr}");

    assert_eq!(
        read_log(&log),
        before,
        "a client's own failure must not reach the host's log"
    );
    assert!(
        !read_log(&log).contains("standalone["),
        "only the owning host appends to its log"
    );

    drop(child);
}

#[tokio::test]
async fn rotation_bounds_the_host_log_across_a_restart() {
    let root = project("restart-rotation");
    let log = root.join(".runyte").join(HOST_LOG_NAME);
    fs::File::create(&log)
        .unwrap()
        .set_len(MAX_LOG_BYTES)
        .unwrap();

    let mut child = serve(&root, &["-v"]);
    let endpoint = endpoint_for(&root);
    if !wait_for_endpoint(&mut child, &endpoint).await {
        return;
    }
    let mut client = LocalClient::connect(&endpoint, geometry(), false)
        .await
        .unwrap();
    assert!(matches!(
        response(&mut client).await,
        HostResponse::Welcome { .. }
    ));
    client.send(&ClientRequest::Shutdown).await.unwrap();
    assert!(matches!(
        response_ignoring_visuals(&mut client).await,
        HostResponse::ShuttingDown
    ));
    drop(client);
    let status = child.0.take().unwrap().wait().unwrap();
    assert!(status.success(), "host did not flush and shut down cleanly");

    assert!(
        previous_path(&log).exists(),
        "a host that inherits a full file rotates it before recording"
    );
    assert_eq!(
        fs::metadata(previous_path(&log)).unwrap().len(),
        MAX_LOG_BYTES
    );
    assert!(fs::metadata(&log).unwrap().len() < MAX_LOG_BYTES);
    assert!(read_log(&log).contains("persistent session published"));
}

#[tokio::test]
async fn no_document_clipboard_terminal_or_environment_value_reaches_a_record() {
    const IN_FILE: &str = "SECRETinsideTHEbuffer";
    const TYPED: &str = "SECRETtypedBYtheperson";
    const ENVIRONMENT: &str = "SECRETinTHEenvironment";
    const TERMINAL: &str = "SECRETfromTHEterminal";
    const SERVER_STDERR: &str = "SECRETonSERVERstderr";

    let root = project("redaction");
    fs::write(root.join("note.txt"), format!("{IN_FILE}\n")).unwrap();
    // A "language server" that dies immediately with a secret on its stderr.
    // A real one is not needed: what matters is that the stderr tail the
    // editor retains for `:lsp-status` never reaches the durable file.
    fs::write(root.join("leak.rs"), format!("// {IN_FILE}\n")).unwrap();
    // Outside the project: a config directory is reserved storage, and one
    // containing the project root would be refused as overlapping it.
    let config_directory = test_runtime_dir(&root).join("config");
    fs::create_dir_all(&config_directory).unwrap();
    let config = config_directory.join("config.yaml");
    fs::write(
        &config,
        format!(
            "lsp:\n  rust:\n    command: /bin/sh\n    args: [\"-c\", \"echo {SERVER_STDERR} >&2; exit 3\"]\n"
        ),
    )
    .unwrap();

    let mut command = bundled_runyte(&root);
    command
        .arg("--serve")
        .arg("--project-root")
        .arg(&*root)
        .arg("--config")
        .arg(&config)
        // The most detailed level there is: nothing below it may leak either.
        .arg("-vvv")
        .arg("leak.rs")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir(&root))
        .env("XDG_CACHE_HOME", test_cache_dir(&root))
        .env("RUNYTE_TEST_SECRET", ENVIRONMENT)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = ChildGuard(Some(command.spawn().unwrap()));
    let endpoint = endpoint_for(&root);
    if !wait_for_endpoint(&mut child, &endpoint).await {
        return;
    }
    let log = root.join(".runyte").join(HOST_LOG_NAME);
    assert!(wait_until(|| log.exists()).await);

    let mut client = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    let _welcome = response(&mut client).await;
    for event in [
        InputEvent::from(runyte::input::KeyStroke::new(
            runyte::input::KeyCode::Char('i'),
            runyte::input::Modifiers::NONE,
        )),
        InputEvent::Text(TYPED.to_owned()),
        InputEvent::from(runyte::input::KeyStroke::new(
            runyte::input::KeyCode::Escape,
            runyte::input::Modifiers::NONE,
        )),
    ] {
        client
            .send(&ClientRequest::Input {
                event: event.into(),
                repeated: false,
            })
            .await
            .unwrap();
        let _ = response(&mut client).await;
    }

    // A terminal child whose output is emulator state, never a record. The
    // command itself is typed into the prompt, so the prompt text is covered
    // by the same assertion.
    type_command(&mut client, &format!("terminal echo {TERMINAL}")).await;

    // Yanking puts the secret in a register, which is where a clipboard value
    // would also live.
    for key in ['%', 'y'] {
        client
            .send(&ClientRequest::Input {
                event: InputEvent::from(runyte::input::KeyStroke::new(
                    runyte::input::KeyCode::Char(key),
                    runyte::input::Modifiers::NONE,
                ))
                .into(),
                repeated: false,
            })
            .await
            .unwrap();
        let _ = response(&mut client).await;
    }

    // The configured server has had every chance to start, fail, and have its
    // stderr tail composed into the editor's stop message.
    assert!(
        wait_until(|| read_log(&log).contains("language server stopped")).await,
        "the language server never reported a stop:\n{}",
        read_log(&log)
    );
    client.send(&ClientRequest::ForceShutdown).await.unwrap();
    assert!(matches!(
        response_ignoring_visuals(&mut client).await,
        HostResponse::ShuttingDown
    ));
    drop(client);
    let status = child.0.take().unwrap().wait().unwrap();
    assert!(status.success(), "host did not flush and shut down cleanly");

    // Process exit follows the diagnostic logger's bounded flush, so the file
    // now contains every record this host can emit. This makes the redaction
    // checks final rather than an absence claim after an arbitrary delay.
    let text = read_log(&log);
    for secret in [IN_FILE, TYPED, ENVIRONMENT, TERMINAL, SERVER_STDERR] {
        assert!(
            !text.contains(secret),
            "{secret} reached a diagnostic record:\n{text}"
        );
    }
    assert!(
        text.contains("TRACE"),
        "the run must actually have recorded at the most detailed level:\n{text}"
    );
}

#[cfg(debug_assertions)]
#[test]
fn the_top_level_failure_record_never_carries_a_propagated_error_chain() {
    const SECRET: &str = "SECRET_FROM_RUNYTE_INPUT_TRACE";

    let root = project("top-level-redaction");
    let destination = root.join("explicit.log");
    let trace = root.join(SECRET).join("input.trace");
    let output = bundled_runyte(&root)
        .args([
            "--standalone",
            "--project-root",
            root.to_str().unwrap(),
            "--log",
            destination.to_str().unwrap(),
        ])
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir(&root))
        .env("XDG_CACHE_HOME", test_cache_dir(&root))
        .env("RUNYTE_INPUT_TRACE", &trace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "the invalid trace path must fail startup"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(SECRET),
        "the test did not provoke the propagated path-bearing chain:\n{stderr}"
    );

    let text = read_log(&destination);
    assert!(
        text.lines()
            .any(|line| line.ends_with("process: runyte exited with an error")),
        "the generic process failure record is missing:\n{text}"
    );
    assert!(
        !text.contains(SECRET),
        "an environment-derived path reached the durable log:\n{text}"
    );
}

/// A panic in the process that owns editor state leaves its thread, location,
/// and message in the log, and still fails the process the ordinary way.
///
/// The child is this test binary re-run against one test name: it is the only
/// way to reach a real panic hook in a real process without an injection point
/// inside the editor.
#[test]
fn a_panic_leaves_its_location_and_message_without_changing_process_failure() {
    const CHILD: &str = "RUNYTE_LOG_PANIC_CHILD";

    if let Some(destination) = std::env::var_os(CHILD) {
        let logger = runyte::log::Logger::start(
            Settings::new(Level::Warn, Role::Host),
            Sink::file(PathBuf::from(destination)),
        )
        .unwrap();
        runyte::log::install(logger);
        runyte::log::install_panic_hook();
        panic!("host loop stopped unexpectedly");
    }

    let root = project("panic");
    let destination = root.join("host.log");
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "a_panic_leaves_its_location_and_message_without_changing_process_failure",
            "--exact",
            "--nocapture",
        ])
        .env(CHILD, &destination)
        // `Backtrace::capture` prefers RUST_LIB_BACKTRACE, so clearing only
        // RUST_BACKTRACE would still capture one for anybody who exports it.
        .env("RUST_BACKTRACE", "0")
        .env("RUST_LIB_BACKTRACE", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "the panic must still fail the process"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("host loop stopped unexpectedly"),
        "the ordinary panic output stays on stderr:\n{stderr}"
    );

    let text = read_log(&destination);
    assert!(text.contains("panic: thread"), "{text}");
    assert!(text.contains("host loop stopped unexpectedly"), "{text}");
    assert!(text.contains("tests/diagnostic_log.rs:"), "{text}");
    assert_eq!(text.lines().count(), 1, "one panic, one record:\n{text}");
}
