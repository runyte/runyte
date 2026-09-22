// SPDX-License-Identifier: MPL-2.0
//! Real native host and ConPTY frontend, kept behind bin-unit test helpers.

use super::{TerminationSignals, windows_frontend, windows_host};
use runyte::{
    launch::LaunchArguments,
    protocol::{ClientRequest, FeatureGroup, HostResponse as ProtocolHostResponse, WaitStatus},
    startup::StartupTrace,
    terminal::{
        emulator::Emulator,
        pty::{Pty, PtyEvent},
    },
    test_support::TestRuntimeRoot,
    workspace::{
        windows_endpoint::{EndpointLocation, EndpointMetadata, RegistrySet},
        windows_lifecycle::connect_control,
        windows_parent_identity::ForegroundParentSupervisor,
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
const SWITCH_PARENT: &str = "windows_frontend_acceptance::switch_parent_fixture";
const SWITCH_HOST: &str = "windows_frontend_acceptance::switch_host_fixture";
const SWITCH_FRONTEND: &str = "windows_frontend_acceptance::switch_frontend_fixture";
const SWITCH_BUSY_HOLDER: &str = "windows_frontend_acceptance::switch_busy_holder_fixture";
const SWITCH_LOST_ACK: &str = "windows_frontend_acceptance::switch_lost_ack_fixture";
const SWITCH_B_FRONTEND: &str = "windows_frontend_acceptance::switch_b_frontend_fixture";
const PARENT_WAIT_PARENT: &str = "windows_frontend_acceptance::parent_wait_parent_fixture";
const PARENT_WAIT_FRONTEND: &str = "windows_frontend_acceptance::parent_wait_frontend_fixture";
const PARENT_WAIT_LAUNCHER: &str = "windows_frontend_acceptance::parent_wait_launcher_fixture";
const PARENT_LOSS_LAUNCHER: &str = "windows_frontend_acceptance::parent_loss_launcher_fixture";
const PARENT_LOSS_INTERMEDIATE: &str =
    "windows_frontend_acceptance::parent_loss_intermediate_fixture";
const PARENT_WAIT_CLIENT: &str = "windows_frontend_acceptance::parent_wait_client_fixture";
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

fn runtime_ready_record(root: &Path, project: &Path) -> PathBuf {
    // Native host startup resolves the requested project before deriving its
    // workspace ID. Windows canonicalization commonly adds a verbatim prefix,
    // so hashing the fixture's authored spelling would watch a different
    // endpoint directory forever.
    let project = project.canonicalize().unwrap();
    root.join("runtime/runyte")
        .join(runyte::workspace::workspace_id(&project))
        .join("endpoint.json")
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
fn native_frontend_switches_between_exact_running_hosts() {
    let root = TestRuntimeRoot::new("native-frontend-switch").unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
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
        0
    );
    let log = fs::File::create(root.join("switch-fixture.log")).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", SWITCH_PARENT, "--ignored", "--nocapture"])
        .env(ROOT_ENV, root.path())
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_RUNTIME_DIR", runtime)
        .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let mut child = OwnedChild(child);
    assert_ne!(
        unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.0.as_raw_handle()) },
        0
    );
    fs::write(root.join("fixture-admitted"), b"ready").unwrap();
    let status = await_child(&mut child, Instant::now() + Duration::from_secs(80));
    assert!(
        status.success(),
        "{}",
        fs::read_to_string(root.join("switch-fixture.log")).unwrap()
    );
}

#[test]
fn native_parent_wait_requires_live_terminal_authority_and_survives_parent_loss() {
    let root = TestRuntimeRoot::new("native-parent-wait").unwrap();
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
    let log = fs::File::create(root.join("parent-wait-fixture.log")).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", PARENT_WAIT_PARENT, "--ignored", "--nocapture"])
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
        fs::read_to_string(root.join("parent-wait-fixture.log")).unwrap()
    );
}

#[test]
#[ignore = "reexecuted with a native host and real ConPTY parent clients"]
fn parent_wait_parent_fixture() {
    let root = root();
    let deadline = Instant::now() + TIMEOUT;
    while !root.join("fixture-admitted").exists() {
        assert!(
            Instant::now() < deadline,
            "fixture was not admitted to its job"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME"),
        Some(root.join("config").into())
    );
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(root.join("config/config.yaml"), "lsp:\n  enable: false\n").unwrap();
    let project = root.join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("note.txt"), "HOST_STILL_LIVE\n").unwrap();
    fs::write(project.join("wait.txt"), "PARENT_WAIT_TARGET\n").unwrap();
    fs::write(project.join("loss.txt"), "PARENT_LOSS_TARGET\n").unwrap();

    let host_log = fs::File::create(root.join("parent-wait-host.log")).unwrap();
    let host = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", HOST, "--ignored", "--nocapture"])
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
        .env_remove("XDG_RUNTIME_DIR")
        .current_dir(&project)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(host_log.try_clone().unwrap())
        .stderr(host_log)
        .spawn()
        .unwrap();
    let mut host = OwnedChild(host);
    let published = Instant::now() + TIMEOUT;
    while !ready_record(&root).exists() {
        assert!(
            host.0.try_wait().unwrap().is_none() && Instant::now() < published,
            "parent-wait host did not publish: {}",
            fs::read_to_string(root.join("parent-wait-host.log")).unwrap()
        );
        thread::sleep(Duration::from_millis(15));
    }

    let wait_for = |path: &Path, label: &str| {
        let deadline = Instant::now() + TIMEOUT;
        while !path.exists() {
            assert!(Instant::now() < deadline, "timed out waiting for {label}");
            thread::sleep(Duration::from_millis(15));
        }
    };
    let mut frontend = Console::spawn(&project, PARENT_WAIT_FRONTEND, 100);
    frontend.until_screen("HOST_STILL_LIVE");
    frontend.open_terminal_fixture(PARENT_WAIT_LAUNCHER);
    frontend.until_screen("PARENT_WAIT_TARGET");
    frontend.edit_and_finish_parent_wait("EDITED ");
    wait_for(
        &root.join("valid-complete"),
        "successful ParentWait completion",
    );
    assert_eq!(
        fs::read_to_string(project.join("wait.txt")).unwrap(),
        "EDITED PARENT_WAIT_TARGET\n"
    );

    fs::write(root.join("run-wrong-capability"), b"1").unwrap();
    wait_for(
        &root.join("wrong-capability-rejected"),
        "same-job wrong-capability refusal",
    );
    let marker = fs::read_to_string(root.join("parent-context")).unwrap();
    let outside_log = fs::File::create(root.join("outside-peer.log")).unwrap();
    let outside = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", PARENT_WAIT_CLIENT, "--ignored", "--nocapture"])
        .env(ROOT_ENV, &root)
        .env("RUNYTE_PARENT_WAIT_CASE", "outside-job")
        .env("RUNYTE_PARENT_WAIT_PATH", project.join("wait.txt"))
        .env("RUNYTE_PARENT_CONTEXT", marker)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env_remove("XDG_RUNTIME_DIR")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(outside_log.try_clone().unwrap())
        .stderr(outside_log)
        .spawn()
        .unwrap();
    let mut outside = OwnedChild(outside);
    let status = await_child(&mut outside, Instant::now() + TIMEOUT);
    assert!(
        status.success(),
        "{}",
        fs::read_to_string(root.join("outside-peer.log")).unwrap()
    );
    wait_for(
        &root.join("outside-job-rejected"),
        "copied-marker outside-job refusal",
    );
    fs::write(root.join("release-parent-launcher"), b"1").unwrap();
    wait_for(
        &root.join("parent-launcher-complete"),
        "first terminal launcher exit",
    );
    frontend.until_screen("NOR");

    let unrelated_path = project.join("unrelated.txt");
    fs::write(&unrelated_path, "UNRELATED_WAIT\n").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (mut unrelated, unrelated_token) = runtime.block_on(async {
        let mut client = connect_control(&read_metadata(&root)).await.unwrap();
        client
            .send(&ClientRequest::CreateWait {
                paths: vec![runyte::protocol::encode_path(&unrelated_path)],
            })
            .await
            .unwrap();
        let token = match client.recv().await.unwrap().unwrap() {
            ProtocolHostResponse::WaitCreated { token, .. } => token,
            response => panic!("unexpected unrelated-wait response: {response:?}"),
        };
        (client, token)
    });

    frontend.open_terminal_fixture(PARENT_LOSS_LAUNCHER);
    frontend.until_screen("PARENT_LOSS_TARGET");
    fs::write(root.join("release-loss-intermediate"), b"1").unwrap();
    wait_for(
        &root.join("parent-loss-cancelled"),
        "natural-parent-loss cancellation",
    );
    frontend.send(":q");
    frontend.until_screen(":q");
    frontend.send("\r");
    frontend.until_screen("INS");
    runtime.block_on(async {
        unrelated
            .send(&ClientRequest::WaitStatus {
                token: unrelated_token,
            })
            .await
            .unwrap();
        loop {
            match unrelated.recv().await.unwrap().unwrap() {
                ProtocolHostResponse::WaitState {
                    token,
                    status: WaitStatus::Pending { .. },
                    ..
                } if token == unrelated_token => break,
                ProtocolHostResponse::WaitState { .. } => {}
                response => panic!("unexpected unrelated-wait status: {response:?}"),
            }
        }
        unrelated
            .send(&ClientRequest::CancelWait {
                token: unrelated_token,
            })
            .await
            .unwrap();
    });
    fs::write(root.join("release-loss-leader"), b"1").unwrap();
    wait_for(
        &root.join("parent-loss-launcher-complete"),
        "persistent loss terminal launcher exit",
    );
    frontend.until_screen("NOR");
    frontend.send(":open note.txt\r");
    frontend.until_screen("HOST_STILL_LIVE");
    frontend.insert_and_write("AFTER_PARENT_LOSS ");
    frontend.detach();
    frontend.exit("PARENT_WAIT_FRONTEND_DONE");
    assert_eq!(
        fs::read_to_string(project.join("note.txt")).unwrap(),
        "AFTER_PARENT_LOSS HOST_STILL_LIVE\n"
    );
    host.0.kill().unwrap();
}

#[test]
#[ignore = "reexecuted inside ConPTY for native ParentWait acceptance"]
fn parent_wait_frontend_fixture() {
    run_frontend(None, "PARENT_WAIT_FRONTEND_DONE", false, false);
}

fn spawn_parent_wait_client(case: &str, path: &Path) -> Child {
    let root = root();
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", PARENT_WAIT_CLIENT, "--ignored", "--nocapture"])
        .env(ROOT_ENV, &root)
        .env("RUNYTE_PARENT_WAIT_CASE", case)
        .env("RUNYTE_PARENT_WAIT_PATH", path)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env_remove("XDG_RUNTIME_DIR")
        .spawn()
        .unwrap()
}

fn await_fixture_marker(path: &Path, label: &str) {
    let deadline = Instant::now() + TIMEOUT;
    while !path.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        thread::sleep(Duration::from_millis(15));
    }
}

#[test]
#[ignore = "integrated-terminal launcher retained while native ParentWait completes"]
fn parent_wait_launcher_fixture() {
    let root = root();
    let marker = std::env::var_os("RUNYTE_PARENT_CONTEXT").unwrap();
    fs::write(
        root.join("parent-context"),
        marker.to_string_lossy().as_bytes(),
    )
    .unwrap();
    let path = root.join("project/wait.txt");
    let mut valid = spawn_parent_wait_client("valid", &path);
    assert!(valid.wait().unwrap().success());
    await_fixture_marker(
        &root.join("run-wrong-capability"),
        "wrong-capability trigger",
    );
    let mut wrong = spawn_parent_wait_client("wrong-capability", &path);
    assert!(wrong.wait().unwrap().success());
    await_fixture_marker(
        &root.join("release-parent-launcher"),
        "parent launcher release",
    );
    fs::write(root.join("parent-launcher-complete"), b"1").unwrap();
}

#[test]
#[ignore = "persistent ConPTY leader retains the terminal while an intermediate parent exits"]
fn parent_loss_launcher_fixture() {
    let root = root();
    let mut intermediate = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            PARENT_LOSS_INTERMEDIATE,
            "--ignored",
            "--nocapture",
        ])
        .env(ROOT_ENV, &root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env_remove("XDG_RUNTIME_DIR")
        .spawn()
        .unwrap();
    assert!(intermediate.wait().unwrap().success());
    await_fixture_marker(&root.join("release-loss-leader"), "loss leader release");
    fs::write(root.join("parent-loss-launcher-complete"), b"1").unwrap();
}

#[test]
#[ignore = "disposable natural parent of the native ParentWait client"]
fn parent_loss_intermediate_fixture() {
    let root = root();
    let child = spawn_parent_wait_client("parent-loss", &root.join("project/loss.txt"));
    await_fixture_marker(
        &root.join("release-loss-intermediate"),
        "loss intermediate release",
    );
    // Dropping Child closes only this retained process handle. The waiter
    // remains in the persistent terminal leader's job and observes this
    // intermediate process exiting through its own retained parent handle.
    drop(child);
}

#[test]
#[ignore = "native ParentWait client spawned by an acceptance launcher"]
fn parent_wait_client_fixture() {
    let root = root();
    let case = std::env::var("RUNYTE_PARENT_WAIT_CASE").unwrap();
    let path = PathBuf::from(std::env::var_os("RUNYTE_PARENT_WAIT_PATH").unwrap());
    let mut context = runyte::workspace::parent::ParentContext::from_environment()
        .unwrap()
        .expect("integrated terminal supplied a parent context");
    if case == "wrong-capability" {
        let replacement = if context.capability.starts_with('0') {
            "1"
        } else {
            "0"
        };
        context.capability.replace_range(..1, replacement);
    }
    fs::write(root.join(format!("{case}-started")), b"1").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(async {
        let parent = ForegroundParentSupervisor::capture().unwrap();
        runyte::workspace::parent::run_wait(context, vec![path], &parent).await
    });
    match case.as_str() {
        "valid" => {
            result.unwrap();
            fs::write(root.join("valid-complete"), b"1").unwrap();
        }
        "parent-loss" => {
            let error = format!("{:#}", result.unwrap_err());
            assert!(
                error.contains("cancel"),
                "unexpected parent-loss result: {error}"
            );
            fs::write(root.join("parent-loss-cancelled"), b"1").unwrap();
        }
        "wrong-capability" | "outside-job" => {
            let error = format!("{:#}", result.unwrap_err());
            assert!(
                error.contains("stale") || error.contains("not owned"),
                "unexpected authority refusal: {error}"
            );
            fs::write(root.join(format!("{case}-rejected")), b"1").unwrap();
        }
        other => panic!("unknown ParentWait fixture case {other}"),
    }
}

#[test]
#[ignore = "reexecuted with two real native hosts and one ConPTY frontend"]
fn switch_parent_fixture() {
    let root = root();
    let deadline = Instant::now() + TIMEOUT;
    while !root.join("fixture-admitted").exists() {
        assert!(
            Instant::now() < deadline,
            "fixture was not admitted to its job"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME"),
        Some(root.join("config").into())
    );
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(root.join("config/config.yaml"), "lsp:\n  enable: false\n").unwrap();
    let a = root.join("project-a");
    let b = root.join("project-b");
    let busy = root.join("project-busy");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::create_dir_all(&busy).unwrap();
    fs::write(a.join("note.txt"), "WORKSPACE_A\n").unwrap();
    fs::write(b.join("note.txt"), "WORKSPACE_B\n").unwrap();
    fs::write(busy.join("note.txt"), "WORKSPACE_BUSY\n").unwrap();

    let spawn_host = |project: &Path, label: &str| {
        let inbox = root.join(format!("switch-inbox-{label}"));
        fs::create_dir(&inbox).unwrap();
        let log = fs::File::create(root.join(format!("switch-host-{label}.log"))).unwrap();
        let child = OwnedChild(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", SWITCH_HOST, "--ignored", "--nocapture"])
                .env("RUNYTE_SWITCH_PROJECT", project)
                .env("RUNYTE_TEST_NATIVE_SWITCH_INBOX", &inbox)
                .env("XDG_CONFIG_HOME", root.join("config"))
                .current_dir(project)
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::null())
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap(),
        );
        (child, inbox)
    };
    let canonical = [&a, &b, &busy]
        .map(|project| project.canonicalize().unwrap())
        .map(|project| runyte::workspace::workspace_id(&project));
    assert!(
        canonical[0] != canonical[1]
            && canonical[0] != canonical[2]
            && canonical[1] != canonical[2],
        "switch fixture projects did not produce distinct workspace identities: {canonical:?}"
    );
    let await_publication = |project: &Path, host: &mut OwnedChild, label: &str| {
        let ready = runtime_ready_record(&root, project);
        let deadline = Instant::now() + TIMEOUT;
        while !ready.exists() {
            let status = host.0.try_wait().unwrap();
            assert!(
                status.is_none() && Instant::now() < deadline,
                "host {label} did not publish {} (status {status:?}, expected {}): {}",
                project.display(),
                ready.display(),
                fs::read_to_string(root.join(format!("switch-host-{label}.log")))
                    .unwrap_or_else(|error| format!("cannot read host log: {error}")),
            );
            thread::sleep(Duration::from_millis(15));
        }
    };
    // Publication takes the one namespace registry lock by design. Serialize
    // only this fixture startup boundary; once ready, all hosts remain live and
    // discoverable together for the switching exercise.
    let (mut host_a, inbox_a) = spawn_host(&a, "a");
    await_publication(&a, &mut host_a, "a");
    let (mut host_b, inbox_b) = spawn_host(&b, "b");
    await_publication(&b, &mut host_b, "b");
    let (mut host_busy, _) = spawn_host(&busy, "busy");
    await_publication(&busy, &mut host_busy, "busy");

    let inject = |inbox: &Path, project: &Path| {
        match fs::remove_file(inbox.join("switch-stage")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to clear native switch stage: {error}"),
        }
        let pending = inbox.join("switch-target.pending");
        fs::write(
            &pending,
            fs::read(runtime_ready_record(&root, project)).unwrap(),
        )
        .unwrap();
        fs::rename(pending, inbox.join("switch-target.json")).unwrap();
    };
    let await_stage = |inbox: &Path, label: &str, expected: &str| {
        let marker = inbox.join("switch-stage");
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let stage = fs::read_to_string(&marker);
            if stage.as_ref().is_ok_and(|stage| stage == expected) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "host {label} did not reach switch stage {expected:?}; last stage {stage:?}; request remains: {}; host log: {}",
                inbox.join("switch-target.json").exists(),
                fs::read_to_string(root.join(format!("switch-host-{label}.log")))
                    .unwrap_or_else(|error| format!("cannot read host log: {error}")),
            );
            thread::sleep(Duration::from_millis(15));
        }
    };

    let mut busy_holder = Console::spawn(&busy, SWITCH_BUSY_HOLDER, 100);
    busy_holder.until_screen("WORKSPACE_BUSY");

    let mut frontend = Console::spawn(&a, SWITCH_FRONTEND, 100);
    frontend.until_screen("WORKSPACE_A");
    inject(&inbox_a, &busy);
    await_stage(&inbox_a, "a", "aborted");
    frontend.insert_and_write("RECOVERED ");
    inject(&inbox_a, &b);
    frontend.until_screen("WORKSPACE_B");
    frontend.insert_and_write("B_EDIT ");
    inject(&inbox_b, &a);
    frontend.until_screen("WORKSPACE_A");
    frontend.insert_and_write("NOOP_BEFORE ");
    inject(&inbox_a, &a);
    let deadline = Instant::now() + TIMEOUT;
    while !inbox_a.join("noop-complete").exists() {
        assert!(
            Instant::now() < deadline,
            "exact-current no-op did not complete"
        );
        thread::sleep(Duration::from_millis(15));
    }
    frontend.insert_and_write("AFTER_NOOP ");
    frontend.detach();
    frontend.exit("SWITCH_FRONTEND_DONE");
    assert!(
        fs::read_to_string(b.join("note.txt"))
            .unwrap()
            .contains("B_EDIT")
    );
    let a_text = fs::read_to_string(a.join("note.txt")).unwrap();
    assert!(a_text.contains("RECOVERED"));
    assert!(a_text.contains("NOOP_BEFORE") && a_text.contains("AFTER_NOOP"));

    fs::write(inbox_a.join("drop-commit-ack"), b"1").unwrap();
    let mut lost = Console::spawn(&a, SWITCH_LOST_ACK, 100);
    lost.until_screen("WORKSPACE_A");
    inject(&inbox_a, &b);
    lost.exit("LOST_COMMIT_ACK_BOUNDED");
    thread::sleep(Duration::from_millis(250));
    let mut destination = Console::spawn(&b, SWITCH_B_FRONTEND, 100);
    destination.until_screen("WORKSPACE_B");
    destination.detach();
    destination.exit("SWITCH_B_AVAILABLE");
    drop(busy_holder);
    host_a.0.kill().unwrap();
    host_b.0.kill().unwrap();
    host_busy.0.kill().unwrap();
}

#[test]
#[ignore = "started as one of two native switch hosts"]
fn switch_host_fixture() {
    let root = root();
    let project = PathBuf::from(std::env::var_os("RUNYTE_SWITCH_PROJECT").unwrap());
    let args: Vec<OsString> = vec![
        "--serve".into(),
        "--detached-host".into(),
        "--project-root".into(),
        project.clone().into_os_string(),
        "--config".into(),
        root.join("config/config.yaml").into_os_string(),
        "--".into(),
        project.join("note.txt").into_os_string(),
    ];
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut startup = StartupTrace::new();
        let mut termination = TerminationSignals::new().unwrap();
        windows_host::run(
            LaunchArguments::parse_from(args).unwrap(),
            &mut startup,
            &mut termination,
            None,
        )
        .await
        .unwrap();
    });
}

#[test]
#[ignore = "reexecuted inside ConPTY for exact native switching"]
fn switch_frontend_fixture() {
    run_switch_frontend("project-a", None, "SWITCH_FRONTEND_DONE");
}

#[test]
#[ignore = "holds the busy destination's interactive attachment"]
fn switch_busy_holder_fixture() {
    run_switch_frontend("project-busy", None, "BUSY_HOLDER_DONE");
}

#[test]
#[ignore = "expects the source to drop a committed switch acknowledgement"]
fn switch_lost_ack_fixture() {
    run_switch_frontend(
        "project-a",
        Some("before switch acknowledgement"),
        "LOST_COMMIT_ACK_BOUNDED",
    );
}

#[test]
#[ignore = "proves the abandoned destination accepts a later frontend"]
fn switch_b_frontend_fixture() {
    run_switch_frontend("project-b", None, "SWITCH_B_AVAILABLE");
}

fn run_switch_frontend(project: &str, expected_error: Option<&str>, marker: &str) {
    use std::io::Write;
    let root = root();
    let project = root.join(project);
    let metadata =
        EndpointMetadata::from_json(&fs::read(runtime_ready_record(&root, &project)).unwrap())
            .unwrap();
    let original = input_mode();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut termination = TerminationSignals::new().unwrap();
    let result = runtime.block_on(windows_frontend::attach_exact(
        &metadata,
        &mut termination,
        true,
    ));
    match expected_error {
        Some(needle) => assert!(
            result
                .as_ref()
                .is_err_and(|error| format!("{error:#}").contains(needle)),
            "expected {needle:?}, got {result:?}"
        ),
        None => result.unwrap(),
    }
    assert_eq!(input_mode(), original);
    println!("{marker}");
    std::io::stdout().flush().unwrap();
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

    fn open_terminal_fixture(&mut self, helper: &str) {
        let executable = std::env::current_exe().unwrap();
        let command = format!(
            ":terminal \"{}\" --exact {helper} --ignored --nocapture\r",
            executable.display()
        );
        self.send(&command);
    }

    fn edit_and_finish_parent_wait(&mut self, text: &str) {
        self.send("i");
        self.until_screen("INS");
        self.send(text);
        self.until_screen(text.trim_end());
        self.send("\x1b");
        self.until_screen("NOR");
        self.send(":wq");
        self.until_screen(":wq");
        self.send("\r");
    }

    fn insert_and_write(&mut self, text: &str) {
        self.send("i");
        self.until_screen("INS");
        self.send(text);
        self.until_screen(text.trim_end());
        self.send("\x1b");
        self.until_screen("NOR");
        self.send(":write");
        self.until_screen(":write");
        self.send("\r");
        self.until_screen("wrote");
    }

    fn detach(&mut self) {
        self.send(":detach");
        self.until_screen(":detach");
        self.send("\r");
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
