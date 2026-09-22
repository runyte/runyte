// SPDX-License-Identifier: MPL-2.0
//! Real native host and ConPTY frontend, kept behind bin-unit test helpers.

use super::{TerminationSignals, windows_frontend, windows_host};
use runyte::{
    launch::LaunchArguments,
    protocol::FeatureGroup,
    startup::StartupTrace,
    terminal::{
        emulator::Emulator,
        pty::{Pty, PtyEvent},
    },
    test_support::TestRuntimeRoot,
    workspace::{
        windows_endpoint::{EndpointLocation, EndpointMetadata, RegistrySet},
        windows_transport::{HostResponse, LocalServer, ServerEvent},
    },
};
use std::{
    ffi::OsString,
    fs,
    mem::size_of,
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::{
    Console::{
        CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent, GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE,
    },
    JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    },
    Threading::CREATE_NO_WINDOW,
};

const ROOT_ENV: &str = "RUNYTE_NATIVE_FRONTEND_ACCEPTANCE_ROOT";
const PARENT: &str = "windows_frontend_acceptance::fixture_parent";
const HOST: &str = "windows_frontend_acceptance::host_fixture";
const FRONTEND: &str = "windows_frontend_acceptance::frontend_fixture";
const BUSY: &str = "windows_frontend_acceptance::busy_frontend_fixture";
const LOST: &str = "windows_frontend_acceptance::lost_host_frontend_fixture";
const SIGNAL: &str = "windows_frontend_acceptance::console_break_frontend_fixture";
const STALL_SERVER: &str = "windows_frontend_acceptance::stall_server_fixture";
const STALL_FRONTEND: &str = "windows_frontend_acceptance::stall_frontend_fixture";
const TIMEOUT: Duration = Duration::from_secs(20);

fn root() -> PathBuf {
    PathBuf::from(std::env::var_os(ROOT_ENV).expect("fixture root was passed to this process"))
}

fn ready_record(root: &Path) -> PathBuf {
    root.join("project/.runyte/host/endpoint.json")
}

fn stall_ready_record(root: &Path) -> PathBuf {
    root.join("stall-endpoint/endpoint.json")
}

fn read_metadata(root: &Path) -> EndpointMetadata {
    EndpointMetadata::from_json(&fs::read(ready_record(root)).unwrap()).unwrap()
}

struct OwnedChild(Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn await_child(child: &mut OwnedChild, deadline: Instant) -> std::process::ExitStatus {
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "subprocess fixture timed out");
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Reexecute once with fixture-owned environment. ConPTY children inherit only
/// this private configuration; the concurrent test runner's environment stays
/// untouched. The host is a sibling of the ConPTY process, never its child.
#[test]
fn native_frontend_edits_resizes_reattaches_and_reports_host_loss() {
    let root = TestRuntimeRoot::new("native-frontend").unwrap();
    // The parent waits for fixture-admitted before spawning the host or a
    // ConPTY. Closing this job kills every descendant even if the parent test
    // reaches its deadline and cannot run ordinary Rust cleanup in the child.
    let raw_job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    assert!(!raw_job.is_null(), "{}", std::io::Error::last_os_error());
    let job = unsafe { OwnedHandle::from_raw_handle(raw_job) };
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    assert_ne!(
        unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
    let log = fs::File::create(root.join("fixture.log")).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", PARENT, "--ignored", "--nocapture"])
        .env(ROOT_ENV, root.path())
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
        .env_remove("XDG_RUNTIME_DIR")
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let mut child = OwnedChild(child);
    assert_ne!(
        unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.0.as_raw_handle()) },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
    fs::write(root.join("fixture-admitted"), b"ready").unwrap();
    let status = await_child(&mut child, Instant::now() + Duration::from_secs(80));
    assert!(
        status.success(),
        "{}",
        fs::read_to_string(root.join("fixture.log")).unwrap()
    );
}

#[test]
#[ignore = "reexecuted with private storage and owned native host"]
fn fixture_parent() {
    let root = root();
    let deadline = Instant::now() + TIMEOUT;
    while !root.join("fixture-admitted").exists() {
        assert!(
            Instant::now() < deadline,
            "fixture was not admitted to its job"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME"),
        Some(root.join("config").into())
    );
    assert_eq!(
        std::env::var_os("XDG_CACHE_HOME"),
        Some(root.join("cache").into())
    );
    assert!(std::env::var_os("XDG_RUNTIME_DIR").is_none());
    let project = root.join("project");
    fs::create_dir(&project).unwrap();
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(root.join("config/config.yaml"), "lsp:\n  enable: false\n").unwrap();
    let file = project.join("note.txt");
    // The marker begins beyond the 80-column viewport but fits after the
    // resize to 100 columns. Seeing it requires a newly rendered host frame;
    // resizing the test emulator alone cannot reveal text it never received.
    let long_line = format!("{}RESIZE_OK", "x".repeat(80));
    fs::write(&file, format!("original\n{long_line}\n")).unwrap();

    let host_log = fs::File::create(root.join("host.log")).unwrap();
    let host = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", HOST, "--ignored", "--nocapture"])
        .current_dir(&project)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(host_log.try_clone().unwrap())
        .stderr(host_log)
        .spawn()
        .unwrap();
    let mut host = OwnedChild(host);
    let deadline = Instant::now() + TIMEOUT;
    while !ready_record(&root).exists() {
        assert!(
            host.0.try_wait().unwrap().is_none(),
            "host exited before publication: {}",
            fs::read_to_string(root.join("host.log")).unwrap()
        );
        assert!(
            Instant::now() < deadline,
            "host did not publish: {}",
            fs::read_to_string(root.join("host.log")).unwrap()
        );
        std::thread::sleep(Duration::from_millis(15));
    }
    read_metadata(&root).validate().unwrap();

    let mut first = Console::spawn(&project, FRONTEND, 80);
    first.until_screen("original");
    first.until_screen("NOR");
    let mut busy = Console::spawn(&project, BUSY, 80);
    busy.exit("BUSY_REFUSED");

    first.send("i");
    first.until_screen("INS");
    first.send("\x1b[200~native edit \x1b[201~");
    first.until_screen("native edit original");
    assert!(
        !first.display().contains("RESIZE_OK"),
        "resize marker appeared before the ConPTY was widened"
    );
    first.resize(100, 30);
    first.until_screen("RESIZE_OK");
    first.send("\x1b");
    first.until_screen("NOR");
    first.send(":write\r");
    first.until_screen("wrote");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        format!("native edit original\n{long_line}\n")
    );
    first.send(":detach\r");
    first.exit("FRONTEND_DONE");

    let mut second = Console::spawn(&project, FRONTEND, 100);
    second.until_screen("native edit original");
    second.send(":detach\r");
    second.exit("FRONTEND_DONE");

    let mut signalled = Console::spawn(&project, SIGNAL, 100);
    signalled.until_screen("native edit original");
    fs::write(root.join("trigger-console-break"), b"ready").unwrap();
    signalled.exit("CONSOLE_BREAK_RESTORED");

    let mut third = Console::spawn(&project, LOST, 100);
    third.until_screen("native edit original");
    host.0.kill().unwrap();
    let _ = host.0.wait().unwrap();
    third.exit("HOST_LOSS_REPORTED");

    // An exact test server completes transport authentication and Welcome but
    // withholds the first frame. The frontend must leave raw mode on its own
    // bounded startup deadline, even though the peer stays connected.
    let stall_log = fs::File::create(root.join("stall.log")).unwrap();
    let stall = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", STALL_SERVER, "--ignored", "--nocapture"])
        .current_dir(&project)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(stall_log.try_clone().unwrap())
        .stderr(stall_log)
        .spawn()
        .unwrap();
    let mut stall = OwnedChild(stall);
    let deadline = Instant::now() + TIMEOUT;
    while !stall_ready_record(&root).exists() {
        assert!(
            stall.0.try_wait().unwrap().is_none(),
            "stall server exited: {}",
            fs::read_to_string(root.join("stall.log")).unwrap()
        );
        assert!(
            Instant::now() < deadline,
            "stall server did not publish: {}",
            fs::read_to_string(root.join("stall.log")).unwrap()
        );
        thread::sleep(Duration::from_millis(15));
    }
    let mut stalled = Console::spawn(&project, STALL_FRONTEND, 100);
    stalled.exit("FIRST_FRAME_TIMEOUT_RESTORED");
    let status = await_child(&mut stall, Instant::now() + TIMEOUT);
    assert!(
        status.success(),
        "{}",
        fs::read_to_string(root.join("stall.log")).unwrap()
    );
}

#[test]
#[ignore = "started as a separate host process outside ConPTY"]
fn host_fixture() {
    let root = root();
    let args: Vec<OsString> = vec![
        "--serve".into(),
        "--detached-host".into(),
        "--project-root".into(),
        root.join("project").into_os_string(),
        "--config".into(),
        root.join("config/config.yaml").into_os_string(),
        "--".into(),
        root.join("project/note.txt").into_os_string(),
    ];
    let arguments = LaunchArguments::parse_from(args).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut startup = StartupTrace::new();
    let mut termination = TerminationSignals::new().unwrap();
    runtime
        .block_on(windows_host::run(
            arguments,
            &mut startup,
            &mut termination,
            None,
        ))
        .unwrap();
}

#[test]
#[ignore = "fixture-owned exact server deliberately withholds its first frame"]
fn stall_server_fixture() {
    let root = root();
    let project = root.join("project");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let registries = RegistrySet::open(&[root.join("stall-registry")]).unwrap();
        let endpoint =
            EndpointLocation::new(&project, root.join("stall-endpoint"), registries).unwrap();
        let mut server = LocalServer::bind(endpoint.prepare(None).unwrap()).unwrap();
        let connected = tokio::time::timeout(TIMEOUT, server.recv())
            .await
            .unwrap()
            .unwrap();
        let ServerEvent::Connected {
            id,
            interactive,
            responses,
            ..
        } = connected
        else {
            panic!("expected authenticated interactive connection");
        };
        assert!(interactive);
        responses
            .send(HostResponse::Welcome {
                protocol: runyte::protocol::VERSION,
                pid: std::process::id(),
                features: vec![
                    FeatureGroup::Snapshots,
                    FeatureGroup::Input,
                    FeatureGroup::Buffers,
                    FeatureGroup::Wait,
                ],
                host_version: env!("CARGO_PKG_VERSION").into(),
            })
            .await
            .unwrap();
        // Retain the response owner and live listener until the client leaves
        // on its first-frame deadline; otherwise this is a disconnect test.
        loop {
            match tokio::time::timeout(TIMEOUT, server.recv()).await.unwrap() {
                Some(ServerEvent::Disconnected { id: ended }) if ended == id => break,
                Some(ServerEvent::TransportFailure { id: failed, .. }) if failed == id => {}
                other => panic!("unexpected stalled-server event: {other:?}"),
            }
        }
        drop(responses);
        server.shutdown().await.unwrap();
    });
}

fn input_mode() -> u32 {
    let mut flags = 0;
    let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    assert_ne!(unsafe { GetConsoleMode(input, &mut flags) }, 0);
    flags
}

fn run_frontend(expected_error: Option<&str>, marker: &str, stalled: bool, console_break: bool) {
    use std::io::Write;

    let root = root();
    let original = input_mode();
    let metadata = if stalled {
        EndpointMetadata::from_json(&fs::read(stall_ready_record(&root)).unwrap()).unwrap()
    } else {
        read_metadata(&root)
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut termination = TerminationSignals::new().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let signal = console_break.then(|| {
        let stop = Arc::clone(&stop);
        let trigger = root.join("trigger-console-break");
        thread::spawn(move || {
            let deadline = Instant::now() + TIMEOUT;
            while !trigger.exists() && !stop.load(Ordering::Acquire) {
                assert!(
                    Instant::now() < deadline,
                    "console-break fixture was not triggered"
                );
                thread::sleep(Duration::from_millis(10));
            }
            if stop.load(Ordering::Acquire) {
                return false;
            }
            // This helper alone owns its ConPTY console. Group zero cannot
            // signal the test runner or the separately spawned host.
            assert_ne!(unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0) }, 0);
            true
        })
    });
    let result = runtime.block_on(windows_frontend::attach_exact(
        &metadata,
        &mut termination,
        true,
    ));
    stop.store(true, Ordering::Release);
    if let Some(signal) = signal {
        assert!(
            signal.join().unwrap(),
            "frontend returned before console break was delivered"
        );
    }
    match expected_error {
        Some(needle) => assert!(
            result
                .as_ref()
                .is_err_and(|error| format!("{error:#}").contains(needle)),
            "expected {needle:?}, got {result:?}"
        ),
        None => result.unwrap(),
    }
    assert_eq!(
        input_mode(),
        original,
        "frontend did not restore console input mode"
    );
    println!("{marker}");
    std::io::stdout().flush().unwrap();
}

#[test]
#[ignore = "reexecuted inside ConPTY after exact host publication"]
fn frontend_fixture() {
    run_frontend(None, "FRONTEND_DONE", false, false);
}

#[test]
#[ignore = "reexecuted inside ConPTY while another TUI owns the host"]
fn busy_frontend_fixture() {
    run_frontend(
        Some("another interactive TUI is already attached"),
        "BUSY_REFUSED",
        false,
        false,
    );
}

#[test]
#[ignore = "reexecuted inside ConPTY then its host is killed"]
fn lost_host_frontend_fixture() {
    run_frontend(
        Some("native workspace host"),
        "HOST_LOSS_REPORTED",
        false,
        false,
    );
}

#[test]
#[ignore = "reexecuted inside ConPTY and signalled by its fixture parent"]
fn console_break_frontend_fixture() {
    run_frontend(Some("Ctrl+Break"), "CONSOLE_BREAK_RESTORED", false, true);
}

#[test]
#[ignore = "reexecuted inside ConPTY against a server withholding its first frame"]
fn stall_frontend_fixture() {
    run_frontend(
        Some("timed out before its first frame"),
        "FIRST_FRAME_TIMEOUT_RESTORED",
        true,
        false,
    );
}

struct Console {
    child: Pty,
    events: mpsc::Receiver<PtyEvent>,
    screen: Emulator,
    output: Vec<u8>,
}

impl Console {
    fn spawn(project: &Path, helper: &str, columns: u16) -> Self {
        let (sender, events) = mpsc::channel();
        let child = Pty::spawn(
            std::env::current_exe().unwrap().as_os_str(),
            &[
                "--exact".into(),
                helper.into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            project,
            columns,
            30,
            move |event| {
                let _ = sender.send(event);
            },
        )
        .unwrap();
        Self {
            child,
            events,
            screen: Emulator::new(columns.into(), 30),
            output: Vec::new(),
        }
    }

    fn display(&self) -> String {
        (0..self.screen.rows())
            .filter_map(|row| self.screen.grid().line(row))
            .map(|line| {
                line.iter()
                    .filter(|cell| cell.width != 0)
                    .map(|cell| cell.text())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn until_screen(&mut self, needle: &str) {
        let deadline = Instant::now() + TIMEOUT;
        while !self.display().contains(needle) {
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(PtyEvent::Output(bytes)) => {
                    self.screen.feed(&bytes);
                    self.output.extend(bytes);
                }
                other => panic!(
                    "waiting for {needle:?}, got {other:?}; screen: {}; output: {}",
                    self.display(),
                    String::from_utf8_lossy(&self.output)
                ),
            }
        }
    }

    fn send(&self, input: &str) {
        assert!(self.child.write(input.as_bytes().to_vec()));
    }

    fn resize(&mut self, columns: u16, rows: u16) {
        self.child.resize(columns, rows).unwrap();
        self.screen.resize(columns.into(), rows.into());
    }

    fn exit(&mut self, marker: &str) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(PtyEvent::Output(bytes)) => self.output.extend(bytes),
                Ok(PtyEvent::Exited(code)) => {
                    assert_eq!(code, Some(0), "{}", String::from_utf8_lossy(&self.output));
                    assert!(
                        String::from_utf8_lossy(&self.output).contains(marker),
                        "missing {marker}: {}",
                        String::from_utf8_lossy(&self.output)
                    );
                    return;
                }
                other => panic!(
                    "waiting for {marker:?}, got {other:?}: {}",
                    String::from_utf8_lossy(&self.output)
                ),
            }
        }
    }
}
