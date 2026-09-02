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
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use runyte::{
    app::FrameGeometry,
    input::{InputEvent, KeyCode, KeyStroke, Modifiers},
    layout::Rect,
    protocol::{CommandRequest, SnapshotRow, decode_path},
    terminal::emulator::Emulator,
    test_support::TestRuntimeRoot,
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
static TEST_RUNTIME: std::sync::OnceLock<TestRuntimeRoot> = std::sync::OnceLock::new();

extern "C" fn cleanup_test_runtime() {
    if let Some(runtime) = TEST_RUNTIME.get() {
        runtime.cleanup_if_owned();
    }
}

fn test_runtime_dir() -> &'static Path {
    TEST_RUNTIME
        .get_or_init(|| {
            let runtime = TestRuntimeRoot::new("protocol").unwrap();
            // Static values are not dropped. Register the same marker-guarded
            // cleanup for an ordinary test-process exit; abrupt termination
            // has the same unavoidable residual as any RAII fixture.
            // SAFETY: the callback has C ABI, takes no arguments, and only
            // reads a process-lifetime `OnceLock`.
            assert_eq!(unsafe { libc::atexit(cleanup_test_runtime) }, 0);
            runtime
        })
        .path()
}

fn test_cache_dir() -> &'static Path {
    static CACHE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            test_runtime_dir();
            TEST_RUNTIME
                .get()
                .unwrap()
                .create_private_dir("cache")
                .unwrap()
        })
        .as_path()
}

fn bundled_runyte() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    isolate_runyte_children(&mut command);
    command
}

fn isolate_runyte_children(command: &mut Command) {
    command
        .env(
            "RUNYTE_ALL_HOSTS_DIR",
            test_runtime_dir().join("runyte/all-hosts"),
        )
        .env("RUNYTE_TEST_SUPERVISOR_PID", std::process::id().to_string())
        .env(runyte::process_group::AUDIT_PATH_VARIABLE, process_audit());
}

/// Where every Runyte process this binary starts records the process-group
/// signals it sends.
///
/// A signal aimed at a recycled process-group number is invisible from inside
/// the process that receives it, so a failure here can only name its sender
/// if the senders wrote themselves down first. One journal for the whole
/// binary is what makes cross-process attribution possible: the record that
/// explains a killed Git child is very often written by a different process
/// from the one that hosts it.
fn process_audit() -> &'static Path {
    static AUDIT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    AUDIT
        .get_or_init(|| test_runtime_dir().join("process-audit.log"))
        .as_path()
}

/// What the audit journal says, for a failure that has to name a sender.
///
/// A child killed by a signal cannot report who sent it, so the question the
/// journal exists to answer is whether any Runyte process addressed that
/// child's process group before it died. Every record naming a
/// signal-terminated child is therefore reported in order: its spawn, any
/// group signal aimed at it, and the completion that classified it. A signal
/// record standing before that completion names a sender; none standing there
/// means nothing in Runyte killed it, and the cause lies outside the program.
fn process_audit_tail() -> String {
    const TAIL_RECORDS: usize = 20;
    let Ok(contents) = fs::read_to_string(process_audit()) else {
        return "<no process audit was recorded>".to_owned();
    };
    let records: Vec<&str> = contents.lines().collect();
    let killed: Vec<&str> = records
        .iter()
        .copied()
        .filter(|line| line.contains("event=completion") && !line.contains("signal=None"))
        .collect();
    let mut correlated = Vec::new();
    for line in &killed {
        let Some(pid) = audit_field(line, "child_pid") else {
            continue;
        };
        let history: Vec<&str> = records
            .iter()
            .copied()
            .filter(|record| audit_field(record, "child_pid").as_deref() == Some(pid.as_str()))
            .collect();
        let sent_before = history
            .iter()
            .take_while(|record| !record.contains("event=completion"))
            .filter(|record| record.contains("event=signal") && record.contains("outcome=sent"))
            .count();
        correlated.push(format!(
            "child {pid}: {sent_before} Runyte group signal(s) before completion; {history:?}"
        ));
    }
    let sent = records
        .iter()
        .filter(|line| line.contains("event=signal") && line.contains("outcome=sent"))
        .count();
    format!(
        "{} record(s), {sent} delivered group signal(s); {} signal-terminated child(ren): \
         {correlated:?}; last {} record(s): {:?}",
        records.len(),
        killed.len(),
        TAIL_RECORDS.min(records.len()),
        &records[records.len().saturating_sub(TAIL_RECORDS)..],
    )
}

/// Reads one `key=value` field out of an audit record.
fn audit_field(record: &str, key: &str) -> Option<String> {
    record.split(' ').find_map(|field| {
        field
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('='))
            .map(str::to_owned)
    })
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
    isolate_runyte_children(command);
    let (child, master, _) = spawn_in_pty_with_terminal_ownership(command, true);
    (child, master)
}

fn spawn_in_pty_with_initial_termios(command: &mut Command) -> (Child, File, libc::termios) {
    spawn_in_pty_with_terminal_ownership(command, true)
}

fn spawn_in_pty_without_hangup_signal(command: &mut Command) -> (Child, File) {
    isolate_runyte_children(command);
    let (child, master, _) = spawn_in_pty_with_terminal_ownership(command, false);
    (child, master)
}

fn spawn_in_pty_with_terminal_ownership(
    command: &mut Command,
    controlling_terminal: bool,
) -> (Child, File, libc::termios) {
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
    if controlling_terminal {
        // SAFETY: this runs in the child between fork and exec, calls only
        // async-signal-safe libc operations, and makes the PTY slave the
        // controlling terminal so closing the master exercises SIGHUP as well
        // as descriptor loss.
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
        bundled_runyte()
            .arg("--wait")
            .arg(target)
            .current_dir(root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    )
}

fn spawn_wait_in_pty_without_hangup_signal(root: &Path, target: &str) -> (Child, File) {
    spawn_in_pty_without_hangup_signal(
        bundled_runyte()
            .arg("--wait")
            .arg(target)
            .current_dir(root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    )
}

fn spawn_wait_with_redirected_stdin_in_pty(root: &Path, target: &str) -> (Child, File) {
    let mut command = bundled_runyte();
    command
        .arg("--wait")
        .arg(target)
        .current_dir(root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir());
    let mut master = -1;
    let mut slave = -1;
    let mut size = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `openpty` initializes both descriptors on success. Each is
    // immediately wrapped in a `File` with one owner.
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut size,
            )
        },
        0,
        "openpty failed: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: successful `openpty` returned fresh owned descriptors.
    let master = unsafe { File::from_raw_fd(master) };
    // SAFETY: the descriptor is live and owned by `master`.
    assert_ne!(
        unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
        -1
    );
    // SAFETY: successful `openpty` returned a fresh owned slave descriptor.
    let slave = unsafe { File::from_raw_fd(slave) };
    // SAFETY: after stdio is installed in the child, stdout names the PTY
    // slave. The calls are async-signal-safe and make it `/dev/tty` even though
    // stdin is the separate pipe under test.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .spawn()
        .unwrap();
    (child, master)
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
    git(
        &root,
        &[
            "-c",
            "init.defaultBranch=fixture-default",
            "init",
            "--quiet",
        ],
    );
    git(&root, &["symbolic-ref", "HEAD", "refs/heads/main"]);
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

const HOST_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const ASYNC_STATE_TIMEOUT: Duration = Duration::from_secs(30);
const ASYNC_STATE_POLL_INTERVAL: Duration = Duration::from_millis(25);

async fn receive_response(client: &mut LocalClient, waiting_for: &str) -> HostResponse {
    tokio::time::timeout(HOST_RESPONSE_TIMEOUT, client.recv())
        .await
        .unwrap_or_else(|error| panic!("host response timed out while {waiting_for}: {error}"))
        .unwrap_or_else(|error| panic!("host response failed while {waiting_for}: {error}"))
        .unwrap_or_else(|| panic!("host disconnected while {waiting_for}"))
}

/// Receives the next semantic reply from the host's mixed response stream.
///
/// Complete frames and terminal deltas can already be in flight when a later
/// request is handled, so neither is a correlated response to that request.
async fn receive_semantic_response(client: &mut LocalClient, waiting_for: &str) -> HostResponse {
    loop {
        let response = receive_response(client, waiting_for).await;
        if !matches!(
            response,
            HostResponse::Frame { .. } | HostResponse::TerminalDamage { .. }
        ) {
            return response;
        }
    }
}

/// Receives one host response while retaining the call site in timeout errors.
///
/// Most uses follow a request and therefore measure a local protocol round
/// trip. Asynchronous state has a separate deadline in the polling helpers
/// below, so a stalled host and state that has not settled are not conflated.
#[track_caller]
fn response(client: &mut LocalClient) -> impl Future<Output = HostResponse> + '_ {
    let caller = std::panic::Location::caller();
    async move {
        receive_response(
            client,
            &format!(
                "waiting for the request at {}:{}:{}",
                caller.file(),
                caller.line(),
                caller.column()
            ),
        )
        .await
    }
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

#[track_caller]
fn semantic_response(client: &mut LocalClient) -> impl Future<Output = HostResponse> + '_ {
    let caller = std::panic::Location::caller();
    async move {
        receive_semantic_response(
            client,
            &format!(
                "waiting for the semantic response at {}:{}:{}",
                caller.file(),
                caller.line(),
                caller.column()
            ),
        )
        .await
    }
}

/// Returns a current complete frame after asynchronous startup work settles.
///
/// Frame revisions are optimistic-concurrency tokens. Git discovery can
/// replace the first frame immediately, making a command issued against that
/// otherwise-valid startup snapshot stale before the host handles it.
async fn next_idle_frame(client: &mut LocalClient) -> runyte::protocol::HostFrame {
    loop {
        match response(client).await {
            HostResponse::Frame { frame } if frame.editor.status.long_running_action.is_none() => {
                return *frame;
            }
            HostResponse::Frame { .. } => {}
            HostResponse::TerminalDamage { .. } => {
                client.send(&ClientRequest::Resynchronize).await.unwrap();
            }
            _ => {}
        }
    }
}

async fn resynchronized_frame(
    client: &mut LocalClient,
    waiting_for: &str,
) -> runyte::protocol::HostFrame {
    client.send(&ClientRequest::Resynchronize).await.unwrap();
    loop {
        match receive_response(client, waiting_for).await {
            HostResponse::Frame { frame } => return *frame,
            HostResponse::TerminalDamage { .. } => {
                client.send(&ClientRequest::Resynchronize).await.unwrap();
            }
            response => {
                panic!("expected a resynchronized frame while {waiting_for}, got {response:?}")
            }
        }
    }
}

async fn wait_for_frame(
    client: &mut LocalClient,
    waiting_for: &str,
    matches: impl Fn(&runyte::protocol::HostFrame) -> bool,
) -> runyte::protocol::HostFrame {
    let deadline = Instant::now() + ASYNC_STATE_TIMEOUT;
    loop {
        let frame = resynchronized_frame(client, waiting_for).await;
        if matches(&frame) {
            return frame;
        }
        assert!(
            Instant::now() < deadline,
            "asynchronous state timed out after {ASYNC_STATE_TIMEOUT:?} while {waiting_for}; \
             last frame id: {:?}, mode: {:?}, active buffer: {:?}, revision: {:?}",
            frame.id,
            frame.editor.mode,
            frame.active_buffer,
            frame.active_revision,
        );
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    }
}

async fn invoke_when_current(
    client: &mut LocalClient,
    command: &str,
    mut frame: runyte::protocol::HostFrame,
) {
    let deadline = Instant::now() + ASYNC_STATE_TIMEOUT;
    loop {
        client
            .send(&ClientRequest::Invoke {
                command: CommandRequest::at(
                    command,
                    frame.id,
                    frame.active_buffer,
                    frame.active_revision,
                ),
            })
            .await
            .unwrap();
        match receive_semantic_response(client, &format!("receiving the {command} command result"))
            .await
        {
            HostResponse::CommandResult { .. } => return,
            HostResponse::Error { message } if message.starts_with("stale editor frame:") => {
                assert!(
                    Instant::now() < deadline,
                    "editor frames stayed stale for {ASYNC_STATE_TIMEOUT:?} while invoking \
                     {command}: {message}"
                );
                frame = resynchronized_frame(
                    client,
                    &format!("resynchronizing a stale frame before invoking {command}"),
                )
                .await;
            }
            response => panic!("expected the {command} command result, got {response:?}"),
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

/// What the operating system says about a process that has not exited.
///
/// A test that ran out of patience with a child can only say whether that
/// child was stuck or merely slow by asking from outside it: `ps` reports its
/// scheduling state and the CPU time it has accumulated, and Linux
/// additionally names the kernel function it is sleeping in.
fn live_process_state(pid: u32) -> String {
    let state = Command::new("ps")
        .args([
            "-o",
            "pid=,ppid=,state=,time=,command=",
            "-p",
            &pid.to_string(),
        ])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_else(|error| format!("ps failed: {error}"));
    #[cfg(target_os = "linux")]
    let state = format!(
        "{state}\n/proc stat: {}\n/proc wchan: {}",
        fs::read_to_string(format!("/proc/{pid}/stat"))
            .unwrap_or_else(|error| format!("unavailable: {error}")),
        fs::read_to_string(format!("/proc/{pid}/wchan"))
            .unwrap_or_else(|error| format!("unavailable: {error}")),
    );
    state
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

async fn wait_child_after_terminal_loss(child: &mut Child) -> ExitStatus {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    match result {
        Ok(status) => status,
        Err(_) => {
            let diagnostics = live_process_state(child.id());
            let _ = child.kill();
            let _ = child.wait();
            panic!("wait client did not exit after terminal loss:\n{diagnostics}");
        }
    }
}

async fn wait_child_after_terminal_loss_while_draining(
    child: &mut Child,
    interactive: &mut LocalClient,
) -> ExitStatus {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            tokio::select! {
                response = interactive.recv() => {
                    assert!(
                        response.unwrap().is_some(),
                        "interactive host connection closed while awaiting wait-client exit"
                    );
                }
                _ = tokio::time::sleep(Duration::from_millis(25)) => {}
            }
        }
    })
    .await;
    match result {
        Ok(status) => status,
        Err(_) => {
            let diagnostics = live_process_state(child.id());
            let _ = child.kill();
            let _ = child.wait();
            panic!("wait client did not exit while the TUI drained frames:\n{diagnostics}");
        }
    }
}

const WAIT_PARENT_HELPER_ROOT: &str = "RUNYTE_WAIT_PARENT_HELPER_ROOT";
const WAIT_PARENT_HELPER_RUNTIME: &str = "RUNYTE_WAIT_PARENT_HELPER_RUNTIME";
const WAIT_PARENT_HELPER_CACHE: &str = "RUNYTE_WAIT_PARENT_HELPER_CACHE";
const WAIT_PARENT_HELPER_INVENTORY: &str = "RUNYTE_WAIT_PARENT_HELPER_INVENTORY";

#[test]
#[ignore = "subprocess helper for wait_client_exits_when_its_launching_process_dies"]
fn wait_parent_process_helper() {
    let Some(root) = std::env::var_os(WAIT_PARENT_HELPER_ROOT).map(PathBuf::from) else {
        return;
    };
    let runtime = std::env::var_os(WAIT_PARENT_HELPER_RUNTIME).unwrap();
    let cache = std::env::var_os(WAIT_PARENT_HELPER_CACHE).unwrap();
    let inventory = std::env::var_os(WAIT_PARENT_HELPER_INVENTORY).unwrap();
    let mut waiter = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .arg("--wait")
        .arg("note.txt")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", runtime)
        .env("XDG_CACHE_HOME", cache)
        .env("RUNYTE_ALL_HOSTS_DIR", inventory)
        .spawn()
        .unwrap();
    fs::write(root.join("wait-parent.pid"), waiter.id().to_string()).unwrap();
    let _ = waiter.wait();
}

fn process_is_running(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
        return stat
            .rsplit_once(") ")
            .and_then(|(_, suffix)| suffix.chars().next())
            .is_some_and(|state| state != 'Z' && state != 'X');
    }
    // SAFETY: signal zero does not deliver a signal; `pid` came from this
    // test's freshly spawned helper child.
    (unsafe { libc::kill(pid as libc::pid_t, 0) }) == 0
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[test]
fn termination_signal_restores_the_terminal_and_preserves_its_exit_status() {
    let root = project();
    let config = default_config(&root);
    let (mut child, master, initial) = spawn_in_pty_with_initial_termios(
        bundled_runyte()
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

#[tokio::test]
async fn standalone_terminal_input_edits_and_quits_through_the_real_event_loop() {
    let root = project();
    let config = default_config(&root);
    let (child, mut terminal) = spawn_in_pty(
        bundled_runyte()
            .args(["--standalone", "--config"])
            .arg(config)
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let mut child = ChildGuard(Some(child));
    let output = capture_terminal_output(&terminal);

    wait_for_terminal_screen(&output, "Runyte").await;
    wait_for_terminal_screen(&output, "main (unborn)").await;
    type_colon_command(&mut terminal, "git-status");
    wait_for_terminal_screen(&output, "note.txt").await;
    std::io::Write::write_all(&mut terminal, b" /s").unwrap();
    std::io::Write::flush(&mut terminal).unwrap();
    wait_for_terminal_screen(&output, "workspace search:").await;
    std::io::Write::write_all(&mut terminal, b"base\r").unwrap();
    std::io::Write::flush(&mut terminal).unwrap();
    wait_for_terminal_screen(&output, "[workspace search]").await;
    type_colon_command(&mut terminal, "file-picker");
    std::io::Write::write_all(&mut terminal, b"other").unwrap();
    std::io::Write::flush(&mut terminal).unwrap();
    wait_for_terminal_screen(&output, "other.txt").await;
    std::io::Write::write_all(&mut terminal, b"\x03").unwrap();
    std::io::Write::flush(&mut terminal).unwrap();
    let deadline = Instant::now() + ASYNC_STATE_TIMEOUT;
    while output.screen_text().contains("Find · Names") {
        assert!(Instant::now() < deadline, "file picker did not close");
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    }
    tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    assert!(!output.screen_text().contains("Find · Names"));
    type_colon_command(&mut terminal, "buffer-new");
    wait_for_terminal_screen(&output, "[scratch]").await;
    std::io::Write::write_all(&mut terminal, b"ihello\x1b").unwrap();
    std::io::Write::flush(&mut terminal).unwrap();
    wait_for_terminal_screen(&output, "hello").await;
    type_colon_command(&mut terminal, "quit-all!");

    assert!(wait_child(child.0.as_mut().unwrap()).await.success());
    child.0.take();
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn standalone_event_loop_drains_integrated_terminal_output_before_quitting() {
    let root = project();
    let config = default_config(&root);
    let (child, mut terminal) = spawn_in_pty(
        bundled_runyte()
            .args(["--standalone", "--config"])
            .arg(config)
            .arg("note.txt")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let mut child = ChildGuard(Some(child));
    let output = capture_terminal_output(&terminal);

    wait_for_terminal_screen(&output, "note.txt").await;
    type_colon_command(
        &mut terminal,
        "terminal /bin/sh -c 'printf terminal-ready; read _'",
    );
    wait_for_terminal_screen(&output, "terminal-ready").await;
    std::io::Write::write_all(&mut terminal, b"\r").unwrap();
    std::io::Write::flush(&mut terminal).unwrap();
    let deadline = Instant::now() + ASYNC_STATE_TIMEOUT;
    while output.screen_text().contains("terminal-ready") {
        assert!(Instant::now() < deadline, "exited terminal stayed visible");
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    }
    type_colon_command(&mut terminal, "quit-all!");

    assert!(wait_child(child.0.as_mut().unwrap()).await.success());
    child.0.take();
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_blocked_document_open_presents_an_intentional_startup_screen() {
    use std::ffi::CString;
    use std::io::Write;

    let root = project();
    let config = default_config(&root);
    let fifo = root.join("blocking.lua");
    let fifo_name = CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
    // SAFETY: `fifo_name` is a live NUL-terminated path and the mode contains
    // only ordinary owner permissions.
    assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
    let (child, terminal) = spawn_in_pty(
        bundled_runyte()
            .args(["--standalone", "--config"])
            .arg(config)
            .arg("blocking.lua")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let mut child = ChildGuard(Some(child));
    let output = capture_terminal_output(&terminal);

    wait_for_terminal_screen(&output, "Opening workspace…").await;
    assert!(
        child.0.as_mut().unwrap().try_wait().unwrap().is_none(),
        "the editor exited instead of waiting for the startup target"
    );
    assert!(
        output.raw.lock().unwrap().len() < 256,
        "the document-free presentation alone crossed the benchmark's substantive-frame threshold"
    );
    // Startup probes once for binary data, then opens the same path for the
    // authoritative text and disk state. Feed both reads without creating or
    // executing a test program.
    let writer_path = fifo.clone();
    let writers = std::thread::spawn(move || {
        for read in 0..2 {
            let mut writer = fs::OpenOptions::new()
                .write(true)
                .open(&writer_path)
                .unwrap();
            writer.write_all(b"return 1\n").unwrap();
            drop(writer);
            if read == 0 {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
    wait_for_terminal_screen(&output, "blocking.lua").await;
    writers.join().unwrap();

    // SAFETY: `child.id()` names this test's live child process.
    assert_eq!(
        unsafe { libc::kill(child.0.as_ref().unwrap().id() as libc::pid_t, libc::SIGTERM,) },
        0
    );
    assert_eq!(
        wait_child(child.0.as_mut().unwrap()).await.code(),
        Some(128 + libc::SIGTERM)
    );
    child.0.take();
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn termination_during_a_blocked_startup_open_restores_the_terminal() {
    use std::ffi::CString;

    let root = project();
    let config = default_config(&root);
    let fifo = root.join("blocked-forever.lua");
    let fifo_name = CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
    // SAFETY: `fifo_name` is a live NUL-terminated path and the mode contains
    // only ordinary owner permissions.
    assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
    let (child, terminal, initial) = spawn_in_pty_with_initial_termios(
        bundled_runyte()
            .args(["--standalone", "--config"])
            .arg(config)
            .arg("blocked-forever.lua")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let mut child = ChildGuard(Some(child));
    let output = capture_terminal_output(&terminal);

    wait_for_terminal_screen(&output, "Opening workspace…").await;
    // SAFETY: `child.id()` names this test's live child process.
    assert_eq!(
        unsafe { libc::kill(child.0.as_ref().unwrap().id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    assert_eq!(
        wait_child(child.0.as_mut().unwrap()).await.code(),
        Some(128 + libc::SIGTERM)
    );
    child.0.take();

    let restored = terminal_attributes(terminal.as_raw_fd());
    assert_eq!(restored.c_iflag, initial.c_iflag);
    assert_eq!(restored.c_oflag, initial.c_oflag);
    assert_eq!(restored.c_cflag, initial.c_cflag);
    assert_eq!(restored.c_lflag, initial.c_lflag);
    assert_eq!(restored.c_cc, initial.c_cc);
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
    terminal_output: Option<&SharedTerminalCapture>,
    name: &str,
    needle: &str,
) {
    let mut last_text = None;
    let deadline = Instant::now() + ASYNC_STATE_TIMEOUT;
    loop {
        client.send(&ClientRequest::ListBuffers).await.unwrap();
        let buffer = match receive_response(
            client,
            &format!("polling for the {name:?} buffer to contain {needle:?}"),
        )
        .await
        {
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
            match receive_response(
                client,
                &format!("reading the {name:?} buffer while waiting for {needle:?}"),
            )
            .await
            {
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
        if Instant::now() >= deadline {
            let terminal_output = terminal_output
                .map(TerminalCapture::raw_text)
                .unwrap_or_else(|| "<not captured>".to_owned());
            panic!(
                "buffer {name:?} did not contain {needle:?} after {ASYNC_STATE_TIMEOUT:?}; \
                 last contents: {:?}; terminal output: {terminal_output}",
                last_text.as_deref().unwrap_or("<buffer was not opened>"),
            );
        }
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    }
}

async fn read_open_buffer_text(client: &mut LocalClient, name: &str) -> Option<String> {
    client.send(&ClientRequest::ListBuffers).await.unwrap();
    let buffer =
        match receive_semantic_response(client, &format!("listing buffers while reading {name:?}"))
            .await
        {
            HostResponse::Buffers { buffers } => buffers
                .into_iter()
                .find(|buffer| !buffer.closed && buffer.name == name),
            response => panic!("expected buffers while reading {name:?}, got {response:?}"),
        }?;
    client
        .send(&ClientRequest::ReadBuffer { buffer: buffer.id })
        .await
        .unwrap();
    match receive_semantic_response(client, &format!("reading {name:?}")).await {
        HostResponse::Buffer { buffer } => Some(buffer.text),
        response => panic!("expected {name:?} contents, got {response:?}"),
    }
}

struct TerminalCapture {
    screen: std::sync::Arc<std::sync::Mutex<Emulator>>,
    raw: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    at_end: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl TerminalCapture {
    fn new() -> Self {
        Self {
            screen: std::sync::Arc::new(std::sync::Mutex::new(Emulator::new(80, 24))),
            raw: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            at_end: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Whether the reader has reached the end of the terminal, so that what
    /// is captured is everything its writers produced.
    fn at_end(&self) -> bool {
        self.at_end.load(Ordering::SeqCst)
    }

    fn screen_text(&self) -> String {
        self.screen.lock().unwrap().plain_text()
    }

    fn raw_text(&self) -> String {
        String::from_utf8_lossy(&self.raw.lock().unwrap()).into_owned()
    }

    /// The end of the raw PTY stream, with the total it was taken from.
    ///
    /// A full-screen TUI redrawing for seconds writes far more than a failure
    /// message can carry, and the bytes that explain where it stopped are the
    /// last ones. The total stays in the message because a stream that never
    /// started is a different diagnosis from one that stopped part way.
    fn raw_tail(&self) -> String {
        const TAIL_BYTES: usize = 4096;
        let raw = self.raw.lock().unwrap();
        let tail = &raw[raw.len().saturating_sub(TAIL_BYTES)..];
        format!(
            "{} byte(s), last {}: {:?}",
            raw.len(),
            tail.len(),
            String::from_utf8_lossy(tail),
        )
    }
}

type SharedTerminalCapture = TerminalCapture;

fn capture_terminal_output(terminal: &File) -> SharedTerminalCapture {
    let output = TerminalCapture::new();
    let raw = std::sync::Arc::clone(&output.raw);
    let screen = std::sync::Arc::clone(&output.screen);
    let (chunks, queued) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        while let Ok(chunk) = queued.recv() {
            screen.lock().unwrap().feed(&chunk);
        }
    });
    let mut drain = terminal.try_clone().unwrap();
    let at_end = std::sync::Arc::clone(&output.at_end);
    std::thread::spawn(move || {
        let mut chunk = [0_u8; 4096];
        loop {
            match std::io::Read::read(&mut drain, &mut chunk) {
                // A PTY master whose last slave closed can report `EIO`
                // rather than an end of file, so an error is one of the two
                // ordinary ends of this stream. An interrupted read is the
                // exception: it has read nothing and the stream is still
                // there.
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    raw.lock().unwrap().extend_from_slice(&chunk[..count]);
                    let _ = chunks.send(chunk[..count].to_vec());
                }
            }
        }
        at_end.store(true, Ordering::SeqCst);
    });
    output
}

/// What a client's terminal shows, for a failure that has to say how far the
/// client got before it stopped.
///
/// The rendered screen answers whether the TUI drew anything at all; the raw
/// tail keeps the bytes that produced it. Tests that own their terminal
/// directly capture nothing, and say so rather than appearing to have found
/// an empty screen.
fn captured_terminal_state(output: Option<&SharedTerminalCapture>) -> String {
    output.map_or_else(
        || "the terminal was not captured".to_owned(),
        |output| {
            format!(
                "terminal screen: {:?}; raw output: {}",
                output.screen_text(),
                output.raw_tail(),
            )
        },
    )
}

/// The complete terminal output of a client that has exited.
///
/// The exit is what ends the stream, because the client is the last process
/// holding the terminal open: the test handed its own copies of the slave to
/// the child at spawn, and any host the client went on to start was given
/// stdio of its own rather than this terminal. Until the draining thread
/// reaches that end, bytes the client wrote on its way out can still be in
/// the PTY buffer, so a test reading the capture the instant it observes the
/// exit reads a truncated one and can miss the very message the exit was
/// about.
async fn terminal_output_at_exit(output: &SharedTerminalCapture) -> String {
    let started = Instant::now();
    while !output.at_end() {
        assert!(
            started.elapsed() < ASYNC_STATE_TIMEOUT,
            "the terminal did not reach end of file within {ASYNC_STATE_TIMEOUT:?} after its \
             client exited; {}",
            captured_terminal_state(Some(output)),
        );
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    }
    output.raw_text()
}

/// Waits for a freshly started `--wait` client's target to reach the host.
///
/// The request comes from a process that has only just been spawned, so this
/// waits through that client's whole startup rather than through a single
/// host round trip. A client that exits first ends the wait, because the
/// request it was going to make can no longer arrive.
async fn wait_for_requested_buffer(
    client: &mut LocalClient,
    waiter: &mut Child,
    path: &Path,
) -> runyte::protocol::BufferMetadata {
    let started = Instant::now();
    loop {
        client.send(&ClientRequest::ListBuffers).await.unwrap();
        if let HostResponse::Buffers { buffers } = semantic_response(client).await
            && let Some(buffer) = buffers
                .into_iter()
                .find(|buffer| buffer.path_bytes.clone().map(decode_path).as_deref() == Some(path))
        {
            return buffer;
        }
        if let Some(status) = waiter.try_wait().unwrap() {
            panic!("the wait client exited before its request reached the host: {status}");
        }
        if started.elapsed() >= ASYNC_STATE_TIMEOUT {
            let running = live_process_state(waiter.id());
            let _ = waiter.kill();
            let _ = waiter.wait();
            panic!(
                "the wait request for {path:?} did not reach the host within \
                 {ASYNC_STATE_TIMEOUT:?}; the client was still running: {running}"
            );
        }
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    }
}

/// Waits for the host to report a client's terminal as its interactive
/// attachment, holding `pending_wait_requests` durable requests at the same
/// time. `None` leaves the count out of the claim rather than expecting
/// none of them.
///
/// Attachment is asynchronous state produced by a separate process. Once the
/// host publishes its endpoint, a `--wait` client still has to finish its own
/// control handshake, create its request, and connect a second interactive
/// client before `interactive_attached` can turn true. Sharing the suite's
/// asynchronous-state deadline rests on the assumption that a loaded machine
/// lengthens that sequence rather than changing its outcome, so a failure
/// here is a stalled attachment rather than a slow machine. The message
/// carries what it takes to falsify that assumption if it is ever wrong:
/// `preceded_by` says what the caller already waited through, the host's own
/// last health report says what it believes it is holding, and the client is
/// described alive or dead.
///
/// A client that exits during the poll ends it immediately, because the
/// attachment it was going to make can no longer arrive and its exit status
/// is the diagnosis that waiting out the deadline would bury. Either failure
/// ends the client first: an editor still holding a PTY outlives the test
/// that started it and perturbs whatever runs next in the same binary.
///
/// The health it reads is the semantic one, so an interactive connection
/// carrying frames and terminal damage can ask the same question a control
/// connection asks.
async fn wait_for_interactive_attachment(
    control: &mut LocalClient,
    client: &mut Child,
    pending_wait_requests: Option<usize>,
    preceded_by: &str,
    output: Option<&SharedTerminalCapture>,
) {
    let started = Instant::now();
    loop {
        control.send(&ClientRequest::Health).await.unwrap();
        let health = semantic_response(control).await;
        if let HostResponse::Health {
            interactive_attached: true,
            pending_wait_requests: pending,
            ..
        } = health
            && pending_wait_requests.is_none_or(|expected| pending == expected)
        {
            return;
        }
        if let Some(status) = client.try_wait().unwrap() {
            panic!(
                "the client exited after {:?} without attaching its terminal: {status}; \
                 {preceded_by}; last host health: {health:?}; {}",
                started.elapsed(),
                captured_terminal_state(output),
            );
        }
        if started.elapsed() >= ASYNC_STATE_TIMEOUT {
            let running = live_process_state(client.id());
            let terminal = captured_terminal_state(output);
            let _ = client.kill();
            let _ = client.wait();
            panic!(
                "no terminal became the interactive attachment within \
                 {ASYNC_STATE_TIMEOUT:?}; {preceded_by}; last host health: {health:?}; \
                 the client was still running: {running}; {terminal}"
            );
        }
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    }
}

async fn wait_for_terminal_screen(output: &SharedTerminalCapture, needle: &str) {
    let deadline = Instant::now() + ASYNC_STATE_TIMEOUT;
    loop {
        let screen = output.screen_text();
        if screen.contains(needle) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "terminal screen did not contain {needle:?} after {ASYNC_STATE_TIMEOUT:?}; \
             last screen: {screen:?}; raw output: {:?}",
            output.raw_text(),
        );
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    }
}

/// Types and accepts a command through the real terminal.
fn type_colon_command(terminal: &mut File, command: &str) {
    std::io::Write::write_all(terminal, format!(":{command}\r").as_bytes()).unwrap();
    std::io::Write::flush(terminal).unwrap();
}

async fn start_host(root: &Path, endpoint: &LocalEndpoint) -> Option<ChildGuard> {
    start_host_opening(root, endpoint, Some("other.txt")).await
}

/// Waits for the command palette's own Git-project capability and returns a
/// current normal-mode frame.
async fn wait_for_git_command(
    interactive: &mut LocalClient,
    command: &str,
    waiting_for: &str,
) -> runyte::protocol::HostFrame {
    send_input_expect_frame(interactive, InputEvent::Key(KeyStroke::char(':'))).await;
    send_input_expect_frame(interactive, InputEvent::Text(command.to_owned())).await;
    let deadline = Instant::now() + ASYNC_STATE_TIMEOUT;
    let mut frame = resynchronized_frame(interactive, waiting_for).await;
    // Git completion publishes a frame because it changes this row. Consume
    // that event instead of repeatedly forcing the large command palette to
    // render while the Git worker is trying to finish on a loaded runner.
    loop {
        let row = frame
            .overlays
            .iter()
            .find(|overlay| overlay.title == "Commands" && overlay.query == command)
            .and_then(|overlay| {
                overlay
                    .rows
                    .iter()
                    .find(|row| row.label.contains(&format!(":{command}")))
            });
        if row.is_some_and(|row| row.available) {
            break;
        }
        let detail = row
            .map(|row| row.detail.clone())
            .unwrap_or_else(|| format!("{command} row absent"));
        let git_summary = frame.editor.status.git_summary.clone();
        let long_running_action = frame.editor.status.long_running_action.clone();
        let interaction_line = frame.editor.status.interaction_line.clone();
        let notification_counts = frame.editor.status.notification_counts;
        if detail.contains("Git repository discovery failed:") {
            let failed_frame_id = frame.id;
            invoke_when_current(interactive, "notifications", frame).await;
            let notifications = read_open_buffer_text(interactive, "[notifications]")
                .await
                .unwrap_or_else(|| "<notifications buffer was not opened>".to_owned());
            panic!(
                "Git repository discovery failed while {waiting_for}: {detail}; \
                 full notifications: {notifications:?}; last frame id: {failed_frame_id:?}, \
                 git summary: {git_summary:?}, long-running action: {long_running_action:?}, \
                 interaction line: {interaction_line:?}, notification counts: \
                 {notification_counts:?}; process audit: {}",
                process_audit_tail(),
            );
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "asynchronous state timed out after {ASYNC_STATE_TIMEOUT:?} while {waiting_for}; \
             last frame id: {:?}, row detail: {detail:?}, git summary: {git_summary:?}, \
             long-running action: {long_running_action:?}, interaction line: \
             {interaction_line:?}, notification counts: {notification_counts:?}",
            frame.id,
        );
        let response = tokio::time::timeout(remaining, interactive.recv())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "asynchronous state timed out after {ASYNC_STATE_TIMEOUT:?} while \
                     {waiting_for}; last frame id: {:?}, row detail: {detail:?}, \
                     git summary: {git_summary:?}, long-running action: \
                     {long_running_action:?}, interaction line: {interaction_line:?}, \
                     notification counts: {notification_counts:?}",
                    frame.id,
                )
            })
            .unwrap_or_else(|error| panic!("host response failed while {waiting_for}: {error}"))
            .unwrap_or_else(|| panic!("host disconnected while {waiting_for}"));
        match response {
            HostResponse::Frame { frame: next } => frame = *next,
            HostResponse::TerminalDamage { damage } => {
                if !damage.apply(&mut frame) {
                    frame = resynchronized_frame(interactive, waiting_for).await;
                }
            }
            response => panic!("expected a visual update while {waiting_for}, got {response:?}"),
        }
    }
    send_input_expect_frame(
        interactive,
        InputEvent::Key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE)),
    )
    .await;
    wait_for_frame(
        interactive,
        "waiting for the Git readiness prompt to return to Normal mode",
        |frame| frame.overlays.is_empty() && frame.editor.status.prompt_cursor_column.is_none(),
    )
    .await
}

/// Waits for Git readiness before a real PTY client needs a Git-only command.
async fn wait_for_git_before_tui(endpoint: &LocalEndpoint) {
    let mut interactive = LocalClient::connect(endpoint, tui_geometry(), true)
        .await
        .unwrap();
    assert!(matches!(
        receive_response(&mut interactive, "receiving the Git readiness welcome").await,
        HostResponse::Welcome { .. }
    ));
    let _ = wait_for_git_command(
        &mut interactive,
        "git-worktrees",
        "waiting for git-worktrees to become available before starting the real TUI",
    )
    .await;
    interactive.send(&ClientRequest::Detach).await.unwrap();
    assert!(matches!(
        semantic_response(&mut interactive).await,
        HostResponse::Detached { .. }
    ));
}

/// Starts a host, optionally on a file. Without one the host keeps the
/// scratch buffer it starts with, which is the only way to reach a scratchpad
/// from a control client.
async fn start_host_opening(
    root: &Path,
    endpoint: &LocalEndpoint,
    target: Option<&str>,
) -> Option<ChildGuard> {
    let mut command = bundled_runyte();
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
    let deadline = Instant::now() + ASYNC_STATE_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!(
                "host endpoint did not complete its Welcome handshake after \
                 {ASYNC_STATE_TIMEOUT:?}"
            );
        }
        if endpoint.metadata().exists() {
            let attempt = remaining.min(Duration::from_millis(250));
            if tokio::time::timeout(attempt, try_connect_control(endpoint))
                .await
                .is_ok_and(|connection| connection.is_ok())
            {
                return Some(child);
            }
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
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL.min(remaining)).await;
    }
}

/// Every process-group signal a host sends names a group it still owns.
///
/// A negative PID is recycled the moment its leader is reaped, so a signal
/// sent after that point is delivered to whichever unrelated process inherited
/// the number — a Git child of another test in this same binary, for
/// instance. The victim can only report that it died by a signal, never who
/// sent it, so the audit journal is where that correspondence lives. This
/// checks the journal's own invariant: a signal record says it was sent only
/// when the same record also names the anchor that made the number Runyte's.
#[tokio::test]
async fn host_process_group_signals_always_name_an_owned_group() {
    let root = project();
    git(&root, &["add", "note.txt", "other.txt"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    let endpoint = LocalEndpoint::discover_with_runtime(
        &root.join(".runyte"),
        &root,
        Some(test_runtime_dir()),
    )
    .unwrap();
    let Some(_host) = start_host(&root, &endpoint).await else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    // Repository discovery is the shortest path to a spawned, reaped, and
    // torn-down Git child inside the host.
    wait_for_git_before_tui(&endpoint).await;

    let audit = fs::read_to_string(process_audit()).unwrap_or_default();
    assert!(
        audit
            .lines()
            .any(|line| line.contains("event=spawn") && line.contains("subsystem=git")),
        "no Git spawn reached the audit journal at {:?}: {audit:?}",
        process_audit(),
    );
    for line in audit.lines().filter(|line| line.contains("event=signal")) {
        let claimed_owned = line.contains("child_state=running_leader")
            || line.contains("child_state=unreaped_leader");
        assert_eq!(
            line.contains("outcome=sent"),
            claimed_owned,
            "a signal record disagrees with its own ownership proof: {line:?}",
        );
    }

    fs::remove_dir_all(root).unwrap();
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
    match receive_semantic_response(
        &mut interactive,
        "receiving the deliberately stale Git refresh result",
    )
    .await
    {
        HostResponse::Error { message } if message.contains("stale editor frame") => {}
        response => panic!("expected a stale editor frame error, got {response:?}"),
    }
    invoke_when_current(&mut interactive, "git-refresh", command_frame).await;
    let quit_frame = resynchronized_frame(
        &mut interactive,
        "receiving a current frame before invoking quit",
    )
    .await;
    invoke_when_current(&mut interactive, "quit", quit_frame).await;
    assert_eq!(
        semantic_response(&mut interactive).await,
        HostResponse::ShuttingDown
    );

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
    // A targetless host starts on the read-only about page, exactly as a bare
    // standalone launch does, and its empty scratch buffer retires when the
    // pane leaves it. The scratchpad this test is about is therefore asked
    // for, rather than assumed to be the buffer the host happened to open on.
    let mut interactive = LocalClient::connect(&endpoint, FrameGeometry::default(), true)
        .await
        .unwrap();
    let _ = response(&mut interactive).await;
    let opening = resynchronized_frame(
        &mut interactive,
        "receiving a current frame before opening a scratchpad",
    )
    .await;
    invoke_when_current(&mut interactive, "buffer-new", opening).await;
    interactive.send(&ClientRequest::Detach).await.unwrap();
    drop(interactive);

    let mut client = connect_control(&endpoint).await;
    client.send(&ClientRequest::ListBuffers).await.unwrap();
    let scratch = match response(&mut client).await {
        HostResponse::Buffers { buffers } => buffers
            .into_iter()
            .find(|buffer| buffer.path_bytes.is_none() && !buffer.read_only && !buffer.closed)
            .expect("a host keeps the scratch buffer it was asked for"),
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
    let (mut waiter, _wait_terminal) = spawn_wait_in_pty_without_hangup_signal(&root, "note.txt");

    let requested =
        wait_for_requested_buffer(&mut interactive, &mut waiter, &root.join("note.txt")).await;
    interactive
        .send(&ClientRequest::CloseBuffer {
            buffer: requested.id,
            discard: false,
        })
        .await
        .unwrap();
    assert!(matches!(
        semantic_response(&mut interactive).await,
        HostResponse::Closed { .. }
    ));
    let status = wait_child(&mut waiter).await;
    assert!(status.success());

    interactive.send(&ClientRequest::ListBuffers).await.unwrap();
    assert!(matches!(
        semantic_response(&mut interactive).await,
        HostResponse::Buffers { buffers }
            if buffers.iter().any(|buffer| {
                !buffer.closed
                    && buffer.path_bytes.clone().map(decode_path).as_deref()
                        == Some(root.join("other.txt").as_path())
            })
    ));
    interactive.send(&ClientRequest::Detach).await.unwrap();
    let _ = semantic_response(&mut interactive).await;
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
    let mut waiter = bundled_runyte()
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
        if let HostResponse::Buffers { buffers } = semantic_response(&mut interactive).await {
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
async fn killing_the_host_fails_an_attached_persistent_tui() {
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
    let mut control = connect_control(&endpoint).await;
    let (tui, terminal) = spawn_in_pty(
        bundled_runyte()
            .arg("--persistent")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let mut tui = ChildGuard(Some(tui));
    let output = capture_terminal_output(&terminal);
    wait_for_interactive_attachment(
        &mut control,
        tui.0.as_mut().unwrap(),
        None,
        "the persistent TUI had completed its handshake",
        Some(&output),
    )
    .await;

    host.0.as_mut().unwrap().kill().unwrap();
    let _ = host.0.take().unwrap().wait();
    let status = wait_child(tui.0.as_mut().unwrap()).await;
    assert!(
        !status.success(),
        "an unannounced host disconnect was reported as a successful detach: {}; {}",
        status,
        captured_terminal_state(Some(&output)),
    );
    let _ = tui.0.take().unwrap().wait();
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
    let mut commit = Command::new("git");
    commit
        .arg("commit")
        .current_dir(&root)
        .env("GIT_EDITOR", editor)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir());
    let (mut commit, _commit_terminal) = spawn_in_pty_without_hangup_signal(&mut commit);

    let mut message = None;
    for _ in 0..200 {
        interactive.send(&ClientRequest::ListBuffers).await.unwrap();
        if let HostResponse::Buffers { buffers } = semantic_response(&mut interactive).await {
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
    assert!(
        commit.try_wait().unwrap().is_none(),
        "git commit exited while its editor-owned buffer was still open"
    );
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
        semantic_response(&mut interactive).await,
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
    let drained = capture_terminal_output(&terminal);
    let mut control = connect_control(&endpoint).await;
    wait_for_interactive_attachment(
        &mut control,
        commit.0.as_mut().unwrap(),
        None,
        "git had been asked to commit through this editor",
        Some(&drained),
    )
    .await;

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
    wait_for_terminal_screen(&drained, " INS ").await;

    // The commit message instructions this editor writes into the buffer
    // ("commit", "message", ...) are ordinary words, so word completion can
    // legitimately be showing a popup once "commit message" has been typed.
    // One Escape must both dismiss that automatic popup and leave insert
    // mode, exactly as it does when word completion is switched off.
    terminal.write_all(b"\x1b").unwrap();
    terminal.flush().unwrap();
    wait_for_terminal_screen(&drained, " NOR ").await;
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
        let output = drained.raw_text();
        let screen = drained.screen_text();
        let buffer_text = match control
            .send(&ClientRequest::ReadBuffer {
                buffer: commit_buffer,
            })
            .await
        {
            Ok(()) => match tokio::time::timeout(HOST_RESPONSE_TIMEOUT, control.recv()).await {
                Ok(Ok(Some(HostResponse::Buffer { buffer }))) => buffer.text,
                Ok(Ok(Some(other))) => format!("<unexpected response: {other:?}>"),
                Ok(Ok(None)) => "<host disconnected before the buffer reply>".to_owned(),
                Ok(Err(error)) => format!("<buffer reply failed: {error:#}>"),
                Err(error) => format!("<buffer reply timed out: {error}>"),
            },
            Err(error) => format!("<buffer request failed: {error:#}>"),
        };
        let host_status = host.0.as_mut().unwrap().try_wait().unwrap();
        let host_stderr = host_status.map_or_else(
            || "<host still running>".to_owned(),
            |_| {
                let output = host.0.take().unwrap().wait_with_output().unwrap();
                String::from_utf8_lossy(&output.stderr).into_owned()
            },
        );
        panic!(
            "Git commit failed after :wq: {status}\npty screen: {screen:?}\npty output: \
             {output:?}\ncommit buffer: {buffer_text:?}\nhost status: {host_status:?}\nhost \
             stderr: {host_stderr:?}"
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
    let mut wait_command = bundled_runyte();
    wait_command.arg("--wait").args(&wait_names);
    wait_command
        .current_dir(&nested)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir());
    let (mut waiter, _wait_terminal) = spawn_in_pty_without_hangup_signal(&mut wait_command);

    let mut requested = Vec::new();
    for _ in 0..100 {
        interactive.send(&ClientRequest::ListBuffers).await.unwrap();
        if let HostResponse::Buffers { buffers } = semantic_response(&mut interactive).await {
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
            semantic_response(&mut interactive).await,
            HostResponse::Closed { .. }
        ));
    }
    assert!(wait_child(&mut waiter).await.success());
    interactive.send(&ClientRequest::Detach).await.unwrap();
    let _ = semantic_response(&mut interactive).await;
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
    assert!(matches!(
        receive_response(&mut interactive, "receiving the interactive welcome").await,
        HostResponse::Welcome { .. }
    ));
    let initial = wait_for_git_command(
        &mut interactive,
        "git-worktrees",
        "waiting for git-worktrees capability before opening the worktree view",
    )
    .await;
    invoke_when_current(&mut interactive, "git-worktrees", initial).await;
    let linked_display = linked.to_string_lossy().into_owned();
    wait_for_buffer_text(&mut control, None, "[git worktrees]", &linked_display).await;
    let _worktree_frame = wait_for_frame(
        &mut interactive,
        "waiting for the populated worktree view to become active",
        |frame| {
            frame
                .editor
                .panes
                .iter()
                .any(|pane| pane.active && pane.title.name == "[git worktrees]")
        },
    )
    .await;
    interactive
        .send(&ClientRequest::Input {
            event: InputEvent::Key(KeyStroke::char('j')).into(),
            repeated: false,
        })
        .await
        .unwrap();
    let _selected = wait_for_frame(
        &mut interactive,
        "confirming the linked-worktree selection input was applied",
        |frame| {
            frame.editor.panes.iter().any(|pane| {
                pane.active
                    && pane.rows.iter().any(|row| {
                        matches!(
                            row,
                            SnapshotRow::Text(row)
                                if row.cursor_row && row.document_row == 1
                        )
                    })
            })
        },
    )
    .await;
    interactive
        .send(&ClientRequest::Input {
            event: InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE)).into(),
            repeated: false,
        })
        .await
        .unwrap();
    let switched = loop {
        match receive_response(
            &mut interactive,
            "waiting for the selected workspace switch response",
        )
        .await
        {
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
    wait_for_git_before_tui(&source_endpoint).await;
    let mut source = connect_control(&source_endpoint).await;
    let mut destination = connect_control(&linked_endpoint).await;
    let (switcher, mut terminal) = spawn_in_pty(
        bundled_runyte()
            .arg("--persistent")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let mut switcher = ChildGuard(Some(switcher));
    // Nothing else reads this PTY, and two full-screen TUIs render into it
    // in sequence. Once the terminal buffer fills, the attached editor blocks
    // writing a frame and stops reading input, so later keys are never seen.
    let output = capture_terminal_output(&terminal);

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
    wait_for_terminal_screen(&output, "other.txt").await;
    type_colon_command(&mut terminal, "git-worktrees");
    let linked_display = linked.to_string_lossy().into_owned();
    wait_for_buffer_text(
        &mut source,
        Some(&output),
        "[git worktrees]",
        &linked_display,
    )
    .await;
    terminal.write_all(b"j\r").unwrap();
    terminal.flush().unwrap();

    wait_for_interactive_attachment(
        &mut destination,
        switcher.0.as_mut().unwrap(),
        None,
        "the worktree picker had been asked for the destination",
        Some(&output),
    )
    .await;
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
    wait_for_git_before_tui(&source_endpoint).await;
    let mut source = connect_control(&source_endpoint).await;
    let (switcher, mut terminal) = spawn_in_pty(
        bundled_runyte()
            .arg("--persistent")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let mut switcher = ChildGuard(Some(switcher));
    let output = capture_terminal_output(&terminal);

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
    wait_for_terminal_screen(&output, "other.txt").await;
    type_colon_command(&mut terminal, "git-worktrees");
    let linked_display = linked.to_string_lossy().into_owned();
    wait_for_buffer_text(
        &mut source,
        Some(&output),
        "[git worktrees]",
        &linked_display,
    )
    .await;
    terminal.write_all(b"j\r").unwrap();
    terminal.flush().unwrap();

    wait_for_interactive_attachment(
        &mut source,
        switcher.0.as_mut().unwrap(),
        None,
        "the switch to an incompatible destination had been refused",
        Some(&output),
    )
    .await;
    wait_for_terminal_screen(&output, "E1").await;

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
    wait_for_git_before_tui(&source_endpoint).await;
    let mut source = connect_control(&source_endpoint).await;
    let (switcher, mut terminal) = spawn_in_pty(
        bundled_runyte()
            .arg("--persistent")
            .current_dir(&root)
            .env("XDG_RUNTIME_DIR", test_runtime_dir())
            .env("XDG_CACHE_HOME", test_cache_dir()),
    );
    let mut switcher = ChildGuard(Some(switcher));
    let output = capture_terminal_output(&terminal);

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
    wait_for_terminal_screen(&output, "other.txt").await;
    type_colon_command(&mut terminal, "git-worktrees");
    let root_display = root.to_string_lossy().into_owned();
    wait_for_buffer_text(&mut source, Some(&output), "[git worktrees]", &root_display).await;
    terminal
        .write_all(format!("\tncreated-from-ui\r{}\r", created.to_string_lossy()).as_bytes())
        .unwrap();
    terminal.flush().unwrap();

    // The destination host does not exist yet, so reaching it is two waits,
    // not one: the worktree has to appear and publish an endpoint before
    // there is anything to ask about an attachment. The guard around the
    // client ends it if either wait fails.
    let started = Instant::now();
    let mut destination = loop {
        if created.is_dir() {
            let canonical = created.canonicalize().unwrap();
            let endpoint = LocalEndpoint::discover_with_runtime(
                &canonical.join(".runyte"),
                &canonical,
                Some(test_runtime_dir()),
            )
            .unwrap();
            if let Ok(client) = try_connect_control(&endpoint).await {
                break client;
            }
        }
        if let Some(status) = switcher.0.as_mut().unwrap().try_wait().unwrap() {
            panic!(
                "create-and-attach TUI exited before reaching the new worktree: {status}; {}",
                captured_terminal_state(Some(&output)),
            );
        }
        assert!(
            started.elapsed() < ASYNC_STATE_TIMEOUT,
            "the created worktree did not publish a reachable host within \
             {ASYNC_STATE_TIMEOUT:?}; the client was still running: {}; {}",
            live_process_state(switcher.0.as_ref().unwrap().id()),
            captured_terminal_state(Some(&output)),
        );
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    };
    wait_for_interactive_attachment(
        &mut destination,
        switcher.0.as_mut().unwrap(),
        None,
        "the created worktree had published a reachable host",
        Some(&output),
    )
    .await;
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
    let frame = wait_for_git_command(
        &mut interactive,
        "git-log",
        "waiting for git-log to become available",
    )
    .await;
    invoke_when_current(&mut interactive, "git-log", frame).await;
    let _ = wait_for_frame(&mut interactive, "waiting for the Git log view", |frame| {
        frame
            .editor
            .panes
            .iter()
            .any(|pane| pane.active && pane.title.name == "[git log]")
    })
    .await;
    interactive
        .send(&ClientRequest::Input {
            event: InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE)).into(),
            repeated: false,
        })
        .await
        .unwrap();
    let _ = wait_for_frame(
        &mut interactive,
        "waiting for the Git commit detail view",
        |frame| {
            frame
                .editor
                .panes
                .iter()
                .any(|pane| pane.active && pane.title.name.starts_with("[git commit "))
        },
    )
    .await;
    interactive.send(&ClientRequest::Detach).await.unwrap();
    let _ = semantic_response(&mut interactive).await;
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
    // client before it observes completion from the host, and keep what it
    // drew so a failure can say how far the attachment got.
    let output = capture_terminal_output(&terminal);
    let publishing = Instant::now();
    let mut control = loop {
        if let Ok(client) = try_connect_control(&endpoint).await {
            break client;
        }
        // A restricted environment that refuses the detached host says so on
        // the client's terminal before exiting, and no host is published for
        // this test to talk to.
        if let Some(status) = waiter.try_wait().unwrap() {
            let raw = terminal_output_at_exit(&output).await;
            if raw.contains("Operation not permitted") {
                fs::remove_dir_all(root).unwrap();
                return;
            }
            panic!("--wait exited before publishing a reachable host: {status}: {raw:?}");
        }
        if publishing.elapsed() >= ASYNC_STATE_TIMEOUT {
            let running = live_process_state(waiter.id());
            let terminal = captured_terminal_state(Some(&output));
            let _ = waiter.kill();
            let _ = waiter.wait();
            panic!(
                "--wait did not publish a workspace host within {ASYNC_STATE_TIMEOUT:?}; \
                 the client was still running: {running}; {terminal}"
            );
        }
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    };
    wait_for_interactive_attachment(
        &mut control,
        &mut waiter,
        None,
        &format!(
            "the host published its endpoint after {:?}",
            publishing.elapsed()
        ),
        Some(&output),
    )
    .await;
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
    // The host, not the exited wait client, is what has to retire here, so
    // the record it failed to remove is what names the process at fault.
    let retiring = Instant::now();
    while endpoint.metadata().exists() {
        assert!(
            retiring.elapsed() < ASYNC_STATE_TIMEOUT,
            "the host left its published metadata behind {ASYNC_STATE_TIMEOUT:?} after \
             acknowledging shutdown: {:?}",
            fs::read_to_string(endpoint.metadata())
                .unwrap_or_else(|error| format!("unreadable: {error}")),
        );
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    }
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn attached_wait_client_exits_and_cancels_when_its_terminal_is_lost() {
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
    // This PTY deliberately is not the child's controlling terminal. Closing
    // it therefore tests descriptor hangup directly instead of letting SIGHUP
    // satisfy the deadline through the separate signal path.
    let (mut waiter, terminal) = spawn_wait_in_pty_without_hangup_signal(&root, "note.txt");
    let mut control = connect_control(&endpoint).await;
    wait_for_interactive_attachment(
        &mut control,
        &mut waiter,
        Some(1),
        "the host was running before the wait client started",
        None,
    )
    .await;

    drop(terminal);
    assert!(!wait_child_after_terminal_loss(&mut waiter).await.success());

    let mut released = false;
    for _ in 0..100 {
        control.send(&ClientRequest::Health).await.unwrap();
        released = matches!(
            response(&mut control).await,
            HostResponse::Health {
                interactive_attached: false,
                pending_wait_requests: 0,
                ..
            }
        );
        if released {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(released, "terminal loss left the wait request live");

    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn queued_wait_client_exits_and_cancels_when_its_terminal_is_lost() {
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
    let (mut waiter, terminal) = spawn_wait_in_pty_without_hangup_signal(&root, "note.txt");
    wait_for_interactive_attachment(
        &mut interactive,
        &mut waiter,
        Some(1),
        "a TUI was already attached when the wait client started",
        None,
    )
    .await;

    drop(terminal);
    let waiter_status =
        wait_child_after_terminal_loss_while_draining(&mut waiter, &mut interactive).await;
    assert!(!waiter_status.success());

    let mut released = false;
    for _ in 0..100 {
        interactive.send(&ClientRequest::Health).await.unwrap();
        released = matches!(
            semantic_response(&mut interactive).await,
            HostResponse::Health {
                interactive_attached: true,
                pending_wait_requests: 0,
                ..
            }
        );
        if released {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        released,
        "terminal loss left a queued wait request live or detached the existing TUI"
    );

    interactive.send(&ClientRequest::Detach).await.unwrap();
    let _ = semantic_response(&mut interactive).await;
    let mut control = connect_control(&endpoint).await;
    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn handed_off_wait_client_exits_and_cancels_when_its_terminal_is_lost() {
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
    let (mut waiter, terminal) = spawn_wait_in_pty_without_hangup_signal(&root, "note.txt");
    wait_for_interactive_attachment(
        &mut interactive,
        &mut waiter,
        Some(1),
        "a TUI was already attached when the wait client started",
        None,
    )
    .await;

    interactive.send(&ClientRequest::Detach).await.unwrap();
    let _ = semantic_response(&mut interactive).await;
    let mut control = connect_control(&endpoint).await;
    wait_for_interactive_attachment(
        &mut control,
        &mut waiter,
        Some(1),
        "the attached TUI had detached and left the wait client to take over",
        None,
    )
    .await;

    drop(terminal);
    assert!(!wait_child_after_terminal_loss(&mut waiter).await.success());

    let mut released = false;
    for _ in 0..100 {
        control.send(&ClientRequest::Health).await.unwrap();
        released = matches!(
            response(&mut control).await,
            HostResponse::Health {
                interactive_attached: false,
                pending_wait_requests: 0,
                ..
            }
        );
        if released {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        released,
        "terminal loss left a handed-off wait request live"
    );

    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn redirected_stdin_uses_dev_tty_for_terminal_loss() {
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
    let (mut waiter, terminal) = spawn_wait_with_redirected_stdin_in_pty(&root, "note.txt");
    // Close redirected stdin before the wait becomes live. Reaching the
    // durable request below is the acknowledgement that the client selected
    // `/dev/tty` and survived pipe EOF; an elapsed grace period cannot prove
    // that absence deterministically.
    drop(waiter.stdin.take());

    let _requested =
        wait_for_requested_buffer(&mut interactive, &mut waiter, &root.join("note.txt")).await;
    assert!(
        waiter.try_wait().unwrap().is_none(),
        "closing redirected stdin was mistaken for loss of /dev/tty"
    );

    drop(terminal);
    assert!(!wait_child_after_terminal_loss(&mut waiter).await.success());

    let mut released = false;
    for _ in 0..100 {
        interactive.send(&ClientRequest::Health).await.unwrap();
        released = matches!(
            semantic_response(&mut interactive).await,
            HostResponse::Health {
                interactive_attached: true,
                pending_wait_requests: 0,
                ..
            }
        );
        if released {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        released,
        "/dev/tty hangup did not release the redirected-stdin wait"
    );

    interactive.send(&ClientRequest::Detach).await.unwrap();
    let _ = semantic_response(&mut interactive).await;
    let mut control = connect_control(&endpoint).await;
    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn controlling_terminal_loss_preserves_the_hangup_exit_status() {
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
    let (mut waiter, terminal) = spawn_wait_in_pty(&root, "note.txt");
    let mut control = connect_control(&endpoint).await;
    wait_for_interactive_attachment(
        &mut control,
        &mut waiter,
        Some(1),
        "the host was running before the wait client started",
        None,
    )
    .await;

    drop(terminal);
    assert_eq!(
        wait_child_after_terminal_loss(&mut waiter).await.code(),
        Some(128 + libc::SIGHUP)
    );

    let mut released = false;
    for _ in 0..100 {
        control.send(&ClientRequest::Health).await.unwrap();
        released = matches!(
            response(&mut control).await,
            HostResponse::Health {
                interactive_attached: false,
                pending_wait_requests: 0,
                ..
            }
        );
        if released {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(released, "SIGHUP left the wait request live");

    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn durable_completion_wins_a_race_with_terminal_loss() {
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
    let (mut waiter, terminal) = spawn_wait_in_pty_without_hangup_signal(&root, "note.txt");

    let requested =
        wait_for_requested_buffer(&mut interactive, &mut waiter, &root.join("note.txt")).await;
    interactive
        .send(&ClientRequest::CloseBuffer {
            buffer: requested.id,
            discard: false,
        })
        .await
        .unwrap();
    assert!(matches!(
        semantic_response(&mut interactive).await,
        HostResponse::Closed { .. }
    ));
    drop(terminal);
    assert!(wait_child_after_terminal_loss(&mut waiter).await.success());

    interactive.send(&ClientRequest::Detach).await.unwrap();
    let _ = semantic_response(&mut interactive).await;
    let mut control = connect_control(&endpoint).await;
    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn terminal_loss_recovery_is_bounded_when_the_host_stops_responding() {
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
    let (mut waiter, terminal) = spawn_wait_in_pty_without_hangup_signal(&root, "note.txt");
    let mut control = connect_control(&endpoint).await;
    wait_for_interactive_attachment(
        &mut control,
        &mut waiter,
        Some(1),
        "the host was running before the wait client started",
        None,
    )
    .await;

    let host_pid = host.0.as_ref().unwrap().id() as libc::pid_t;
    // SAFETY: `host_pid` names this test's live child host.
    assert_eq!(unsafe { libc::kill(host_pid, libc::SIGSTOP) }, 0);
    drop(terminal);
    assert!(!wait_child_after_terminal_loss(&mut waiter).await.success());
    // SAFETY: the stopped process is still this test's owned child.
    assert_eq!(unsafe { libc::kill(host_pid, libc::SIGCONT) }, 0);

    let mut released = false;
    for _ in 0..100 {
        control.send(&ClientRequest::Health).await.unwrap();
        released = matches!(
            response(&mut control).await,
            HostResponse::Health {
                interactive_attached: false,
                pending_wait_requests: 0,
                ..
            }
        );
        if released {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        released,
        "best-effort cancellation did not reach the resumed host"
    );

    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn wait_client_exits_when_its_launching_process_dies() {
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
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            "wait_parent_process_helper",
            "--nocapture",
        ])
        .env(WAIT_PARENT_HELPER_ROOT, &root)
        .env(WAIT_PARENT_HELPER_RUNTIME, test_runtime_dir())
        .env(WAIT_PARENT_HELPER_CACHE, test_cache_dir())
        .env(
            WAIT_PARENT_HELPER_INVENTORY,
            test_runtime_dir().join("runyte/all-hosts"),
        );
    // The helper and wait client share a real PTY, but it is deliberately not
    // controlling: killing only the helper cannot produce SIGHUP or close the
    // terminal and therefore isolates launching-parent observation.
    let (mut helper, mut terminal) = spawn_in_pty_without_hangup_signal(&mut command);
    let flags = unsafe { libc::fcntl(terminal.as_raw_fd(), libc::F_GETFL) };
    assert_ne!(flags, -1);
    assert_ne!(
        unsafe {
            libc::fcntl(
                terminal.as_raw_fd(),
                libc::F_SETFL,
                flags | libc::O_NONBLOCK,
            )
        },
        -1
    );
    let marker = root.join("wait-parent.pid");
    let mut control = connect_control(&endpoint).await;
    // The launcher has to have published its child, and the host has to be
    // holding that child's attached request, before the launcher can be
    // killed. Both, not either: a parent-loss claim made about a client that
    // never attached would report this defect for the wrong reason.
    let started = Instant::now();
    let waiter_pid = loop {
        let published = fs::read_to_string(&marker)
            .ok()
            .and_then(|value| value.parse::<u32>().ok());
        control.send(&ClientRequest::Health).await.unwrap();
        let health = response(&mut control).await;
        let pending = matches!(
            health,
            HostResponse::Health {
                interactive_attached: true,
                pending_wait_requests: 1,
                ..
            }
        );
        if let Some(pid) = published
            && pending
        {
            break pid;
        }
        assert!(
            helper.try_wait().unwrap().is_none(),
            "wait launcher exited before publishing its child"
        );
        if started.elapsed() >= ASYNC_STATE_TIMEOUT {
            let running = live_process_state(helper.id());
            // The launcher owns the wait client it started, so ending it
            // here is what stops both from outliving this test.
            let _ = helper.kill();
            let _ = helper.wait();
            panic!(
                "the launched wait client was not durable and attached within \
                 {ASYNC_STATE_TIMEOUT:?}; published child: {published:?}; last host \
                 health: {health:?}; the launcher was still running: {running}"
            );
        }
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    };

    helper.kill().unwrap();
    let _ = helper.wait().unwrap();
    let mut terminal_output = Vec::new();
    let mut stopped = false;
    for _ in 0..200 {
        let mut chunk = [0_u8; 4096];
        loop {
            match terminal.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => terminal_output.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                Err(error) => panic!("failed to drain orphaned wait PTY: {error}"),
            }
        }
        stopped = !process_is_running(waiter_pid);
        if stopped {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        stopped,
        "wait client survived loss of launcher {}; output: {:?}",
        helper.id(),
        String::from_utf8_lossy(&terminal_output)
    );
    let output = String::from_utf8_lossy(&terminal_output);
    assert!(
        output.contains("wait request lost its launching process before completion"),
        "wait client did not report its nonzero parent-loss exit: {output:?}"
    );

    let mut released = false;
    for _ in 0..100 {
        control.send(&ClientRequest::Health).await.unwrap();
        released = matches!(
            response(&mut control).await,
            HostResponse::Health {
                interactive_attached: false,
                pending_wait_requests: 0,
                ..
            }
        );
        if released {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(released, "launcher loss left the wait request live");

    drop(terminal);
    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn durable_completion_wins_a_race_with_launcher_loss() {
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
    let mut launcher = Command::new("sleep").arg("120").spawn().unwrap();
    let barrier = root.join("wait-status-barrier");
    let mut wait_command = bundled_runyte();
    wait_command
        .arg("--wait")
        .arg("note.txt")
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", test_runtime_dir())
        .env("XDG_CACHE_HOME", test_cache_dir())
        .env("RUNYTE_TEST_WAIT_PARENT_PID", launcher.id().to_string())
        .env("RUNYTE_TEST_WAIT_STATUS_BARRIER", &barrier);
    let (mut waiter, _wait_terminal) = spawn_in_pty_without_hangup_signal(&mut wait_command);

    let requested =
        wait_for_requested_buffer(&mut interactive, &mut waiter, &root.join("note.txt")).await;
    let (ready, release) = runyte::test_support::wait_status_barrier_paths(&barrier);
    let started = Instant::now();
    while !ready.exists() {
        if let Some(status) = waiter.try_wait().unwrap() {
            panic!("the wait client exited before publishing its status barrier: {status}");
        }
        if started.elapsed() >= ASYNC_STATE_TIMEOUT {
            let running = live_process_state(waiter.id());
            let _ = waiter.kill();
            let _ = waiter.wait();
            panic!(
                "the wait client did not publish its in-flight status barrier within \
                 {ASYNC_STATE_TIMEOUT:?}; it was still running: {running}"
            );
        }
        tokio::time::sleep(ASYNC_STATE_POLL_INTERVAL).await;
    }
    interactive
        .send(&ClientRequest::CloseBuffer {
            buffer: requested.id,
            discard: false,
        })
        .await
        .unwrap();
    assert!(matches!(
        semantic_response(&mut interactive).await,
        HostResponse::Closed { .. }
    ));
    launcher.kill().unwrap();
    let _ = launcher.wait().unwrap();
    fs::write(release, []).unwrap();
    assert!(wait_child(&mut waiter).await.success());

    interactive.send(&ClientRequest::Detach).await.unwrap();
    let _ = semantic_response(&mut interactive).await;
    let mut control = connect_control(&endpoint).await;
    shutdown(&mut control).await;
    assert!(host.0.take().unwrap().wait().unwrap().success());
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
    let (mut waiter, terminal) = spawn_wait_in_pty(&root, "note.txt");
    // This request ends at a signal rather than at the loss of its terminal,
    // so the shared capture can hold the terminal open for the whole test and
    // keep what the client drew for a failure to report.
    let output = capture_terminal_output(&terminal);
    let mut control = connect_control(&endpoint).await;
    wait_for_interactive_attachment(
        &mut control,
        &mut waiter,
        Some(1),
        "the host was running before the wait client started",
        Some(&output),
    )
    .await;

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

    let output = bundled_runyte()
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

    let listed = bundled_runyte()
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

    let stopped = bundled_runyte()
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

    let relisted = bundled_runyte()
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
    let (mut waiter, mut terminal) = spawn_wait_in_pty(&root, "note.txt");
    let flags = unsafe { libc::fcntl(terminal.as_raw_fd(), libc::F_GETFL) };
    assert_ne!(flags, -1);
    assert_ne!(
        unsafe {
            libc::fcntl(
                terminal.as_raw_fd(),
                libc::F_SETFL,
                flags | libc::O_NONBLOCK,
            )
        },
        -1
    );
    let mut terminal_output = Vec::new();
    let mut control = None;
    for _ in 0..200 {
        let mut chunk = [0_u8; 4096];
        loop {
            match terminal.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => terminal_output.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                Err(error) => panic!("failed to drain wait PTY: {error}"),
            }
        }
        if let Ok(client) = try_connect_control(&endpoint).await {
            control = Some(client);
            break;
        }
        if let Some(status) = waiter.try_wait().unwrap() {
            let output = String::from_utf8_lossy(&terminal_output);
            if output.contains("Operation not permitted") {
                fs::remove_dir_all(root).unwrap();
                return;
            }
            panic!("--wait exited before host publication: {status}: {output:?}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut control = control.unwrap_or_else(|| {
        let diagnostics = live_process_state(waiter.id());
        panic!(
            "wait host did not become reachable; PTY output: {:?}; process: {diagnostics}",
            String::from_utf8_lossy(&terminal_output)
        )
    });
    // The hangup this test is about only means anything to an attachment
    // that exists, so the attachment is asserted rather than merely awaited:
    // a wait client that never attached would fail below for a reason this
    // test is not making a claim about.
    wait_for_interactive_attachment(
        &mut control,
        &mut waiter,
        None,
        "the wait client published its own host",
        None,
    )
    .await;
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
    let frame = next_idle_frame(&mut interactive).await;
    invoke_when_current(&mut interactive, "quit", frame).await;
    assert_eq!(
        semantic_response(&mut interactive).await,
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
        semantic_response(&mut control).await,
        // The protocol caps buffer text at a mebibyte, which is already
        // several times any local socket send buffer.
        HostResponse::Buffer { buffer } if buffer.text.len() > 512 * 1024
    ));
    assert!(matches!(
        semantic_response(&mut control).await,
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
    fs::write(root.join("source-ready.txt"), b"source\n").unwrap();
    fs::write(linked.join("linked-ready.txt"), b"linked\n").unwrap();
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
    let Some(mut source_host) =
        start_host_opening(&root, &source_endpoint, Some("source-ready.txt")).await
    else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let Some(mut linked_host) =
        start_host_opening(&linked, &linked_endpoint, Some("linked-ready.txt")).await
    else {
        let mut source = connect_control(&source_endpoint).await;
        shutdown(&mut source).await;
        let _ = source_host.0.take().unwrap().wait();
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let mut source = connect_control(&source_endpoint).await;
    let mut destination = connect_control(&linked_endpoint).await;
    let (switcher, mut terminal) = spawn_in_pty(
        bundled_runyte()
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
    let output = capture_terminal_output(&terminal);

    wait_for_interactive_attachment(
        &mut source,
        switcher.0.as_mut().unwrap(),
        None,
        "the TUI client had just started",
        Some(&output),
    )
    .await;
    wait_for_terminal_screen(&output, "source-ready.txt").await;

    // The client process stays at `root`, while `:cd` changes only the
    // editor-owned directory to `root/nested`. Resolving `../linked` against
    // the client cwd would therefore look outside this project and fail; the
    // intended destination can only be reached through the editor cwd carried
    // in the switch handoff.
    terminal
        .write_all(b":cd nested\r:session-attach ../linked\r")
        .unwrap();
    terminal.flush().unwrap();
    wait_for_interactive_attachment(
        &mut destination,
        switcher.0.as_mut().unwrap(),
        None,
        "the client was sent to the destination through a relative selector",
        Some(&output),
    )
    .await;
    // Return through the same relative selector path. Worktree-picker switching
    // has its own real-TUI coverage; coupling this process-loop regression to
    // asynchronous Git discovery let Enter arrive while that command was still
    // unavailable.
    wait_for_terminal_screen(&output, "linked-ready.txt").await;
    type_colon_command(&mut terminal, "session-attach ..");
    wait_for_interactive_attachment(
        &mut source,
        switcher.0.as_mut().unwrap(),
        None,
        "the client was sent back to the source through the same relative selector",
        Some(&output),
    )
    .await;

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
