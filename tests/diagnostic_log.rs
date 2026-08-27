// SPDX-License-Identifier: MPL-2.0

//! Durable diagnostic logging, exercised through real processes.
//!
//! The unit tests beside the module cover level filtering, rotation bounds,
//! record shape, and queue saturation without globals. These cover what only a
//! process can show: which file a role owns, that a detached host keeps
//! writing, that an attachment leaves a running host's logger alone, that a
//! client never appends to a host's file, and that no document text,
//! clipboard value, terminal output, or environment value reaches a record.

#![cfg(unix)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use runyte::{
    app::FrameGeometry,
    input::InputEvent,
    layout::Rect,
    log::{HOST_LOG_NAME, Level, MAX_LOG_BYTES, Role, Settings, Sink, default_path, previous_path},
    workspace::transport::{ClientRequest, HostResponse, LocalClient, LocalEndpoint},
};

/// A private runtime directory for every Runyte process this binary spawns, so
/// nothing publishes an endpoint or a recent workspace into the person's own.
fn test_runtime_dir() -> &'static Path {
    static RUNTIME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            use std::os::unix::fs::PermissionsExt;
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                % 1_000_000_007;
            let path = std::env::temp_dir().join(format!("rylog-{}-{unique}", std::process::id()));
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

fn project(name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "runyte-log-{name}-{}-{unique}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(root.join(".runyte")).unwrap();
    fs::write(root.join("note.txt"), "base\n").unwrap();
    root.canonicalize().unwrap()
}

fn runyte(root: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(arguments)
        .current_dir(root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
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
    LocalEndpoint::discover_with_runtime(&root.join(".runyte"), root, Some(test_runtime_dir()))
        .unwrap()
}

fn serve(root: &Path, extra: &[&str]) -> ChildGuard {
    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    command
        .arg("--serve")
        .arg("--project-root")
        .arg(root)
        .args(extra)
        .current_dir(root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
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
async fn type_command(client: &mut LocalClient, command: &str) -> HostResponse {
    let mut last = None;
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
        last = Some(response(client).await);
    }
    last.expect("one response per event")
}

fn frame_text(response: &HostResponse) -> String {
    let HostResponse::Frame { frame } = response else {
        return String::new();
    };
    frame
        .editor
        .panes
        .iter()
        .flat_map(|pane| &pane.rows)
        .filter_map(|row| match row {
            runyte::protocol::SnapshotRow::Text(row) => Some(
                row.runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
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

    fs::remove_dir_all(root).unwrap();
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
        fs::remove_dir_all(root).unwrap();
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
            Command::new(env!("CARGO_BIN_EXE_runyte"))
                .args(arguments)
                .current_dir(&root)
                .env("XDG_RUNTIME_DIR", test_runtime_dir())
                .env("XDG_CACHE_HOME", test_cache_dir())
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

    fs::remove_dir_all(root).unwrap();
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
    assert!(
        !root.join(".runyte").join("host.log").exists(),
        "a refused --log fell back to the default path"
    );

    fs::remove_dir_all(root).unwrap();
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
        fs::remove_dir_all(root).unwrap();
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
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_host_owns_host_log_and_records_client_lifecycle_while_detached() {
    let root = project("host-owned");
    let mut child = serve(&root, &["-v"]);
    let endpoint = endpoint_for(&root);
    if !wait_for_endpoint(&mut child, &endpoint).await {
        fs::remove_dir_all(root).unwrap();
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
    let _welcome = response(&mut client).await;
    assert!(
        wait_until(|| read_log(&log).contains("interactive client attached")).await,
        "{}",
        read_log(&log)
    );
    // Detachment and disconnection are separate facts the host observes.
    client.send(&ClientRequest::Detach).await.unwrap();
    assert!(
        wait_until(|| read_log(&log).contains("interactive client detached")).await,
        "{}",
        read_log(&log)
    );
    drop(client);

    let mut abandoning = LocalClient::connect(&endpoint, geometry(), true)
        .await
        .unwrap();
    let _welcome = response(&mut abandoning).await;
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
    let _welcome = response(&mut reattached).await;
    assert!(read_log(&log).contains("persistent session published"));

    let mut frame = type_command(&mut reattached, "log-open").await;
    while !frame_text(&frame).contains("host owner") {
        frame = response(&mut reattached).await;
    }
    let shown = frame_text(&frame);
    assert!(shown.contains(HOST_LOG_NAME), "{shown}");
    assert!(
        shown.contains("persistent session published"),
        "the buffer shows the host's own records:\n{shown}"
    );

    drop(reattached);
    drop(child);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn attaching_with_logging_flags_reports_the_retained_configuration() {
    let root = project("retained");
    let mut child = serve(&root, &[]);
    let endpoint = endpoint_for(&root);
    if !wait_for_endpoint(&mut child, &endpoint).await {
        fs::remove_dir_all(root).unwrap();
        return;
    }
    let log = root.join(".runyte").join(HOST_LOG_NAME);
    assert!(wait_until(|| log.exists()).await);
    let before = read_log(&log);

    let output = runyte(
        &root,
        &[
            "--session-start",
            "-v",
            "-v",
            "--log",
            root.join("elsewhere.log").to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("kept its own log level and destination"),
        "{stderr}"
    );
    assert!(stderr.contains("--session-restart"), "{stderr}");

    assert!(
        !root.join("elsewhere.log").exists(),
        "an attachment must not open a second destination for a running host"
    );
    let after = read_log(&log);
    assert_eq!(
        levels(&after)
            .into_iter()
            .filter(|level| *level == "INFO")
            .count(),
        levels(&before)
            .into_iter()
            .filter(|level| *level == "INFO")
            .count(),
        "the running host kept its own level:\n{after}"
    );

    drop(child);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn client_side_failures_never_append_to_the_host_log() {
    let root = project("client-failure");
    let mut child = serve(&root, &[]);
    let endpoint = endpoint_for(&root);
    if !wait_for_endpoint(&mut child, &endpoint).await {
        fs::remove_dir_all(root).unwrap();
        return;
    }
    let log = root.join(".runyte").join(HOST_LOG_NAME);
    assert!(wait_until(|| log.exists()).await);
    let before = read_log(&log);

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

    drop(child);
    fs::remove_dir_all(root).unwrap();
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
        fs::remove_dir_all(root).unwrap();
        return;
    }
    assert!(
        wait_until(|| previous_path(&log).exists()).await,
        "a host that inherits a full file rotates it before recording"
    );
    assert_eq!(
        fs::metadata(previous_path(&log)).unwrap().len(),
        MAX_LOG_BYTES
    );
    assert!(fs::metadata(&log).unwrap().len() < MAX_LOG_BYTES);
    assert!(read_log(&log).contains("persistent session published"));

    drop(child);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn no_document_clipboard_terminal_or_environment_value_reaches_a_record() {
    const IN_FILE: &str = "SECRETinsideTHEbuffer";
    const TYPED: &str = "SECRETtypedBYtheperson";
    const ENVIRONMENT: &str = "SECRETinTHEenvironment";
    const TERMINAL: &str = "SECRETfromTHEterminal";

    let root = project("redaction");
    fs::write(root.join("note.txt"), format!("{IN_FILE}\n")).unwrap();

    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    command
        .arg("--serve")
        .arg("--project-root")
        .arg(&root)
        // The most detailed level there is: nothing below it may leak either.
        .arg("-vvv")
        .arg("note.txt")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .env("RUNYTE_TEST_SECRET", ENVIRONMENT)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = ChildGuard(Some(command.spawn().unwrap()));
    let endpoint = endpoint_for(&root);
    if !wait_for_endpoint(&mut child, &endpoint).await {
        fs::remove_dir_all(root).unwrap();
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
    let _ = type_command(&mut client, &format!("terminal echo {TERMINAL}")).await;
    tokio::time::sleep(Duration::from_millis(750)).await;

    let text = read_log(&log);
    for secret in [IN_FILE, TYPED, ENVIRONMENT, TERMINAL] {
        assert!(
            !text.contains(secret),
            "{secret} reached a diagnostic record:\n{text}"
        );
    }
    assert!(
        text.contains("TRACE"),
        "the run must actually have recorded at the most detailed level:\n{text}"
    );

    drop(client);
    drop(child);
    fs::remove_dir_all(root).unwrap();
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
            Sink::File(PathBuf::from(destination)),
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
        .env("RUST_BACKTRACE", "0")
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

    fs::remove_dir_all(root).unwrap();
}
