// SPDX-License-Identifier: MPL-2.0
#![cfg(windows)]

use runyte::{
    terminal::pty::{Pty, PtyEvent},
    test_support::TestRuntimeRoot,
};
use std::{
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    ptr,
    sync::mpsc,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};

const FIXTURE: &str = "native_powershell_handoff_fixture";
const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;

struct FixtureChild {
    child: Child,
    job: OwnedHandle,
}

struct ForegroundHost(Child);
impl Drop for ForegroundHost {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
impl FixtureChild {
    fn spawn(command: &mut Command, root: &Path) -> Self {
        let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        assert!(!job.is_null(), "{}", std::io::Error::last_os_error());
        let job = unsafe { OwnedHandle::from_raw_handle(job) };
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
        let fixture = Self {
            child: command.spawn().unwrap(),
            job,
        };
        // The compiled helper waits for this gate before it can spawn anything.
        // Admission therefore covers its entire subsequent descendant tree.
        assert_ne!(
            unsafe {
                AssignProcessToJobObject(fixture.job.as_raw_handle(), fixture.child.as_raw_handle())
            },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        std::fs::write(root.join("fixture-admitted"), b"ready").unwrap();
        fixture
    }
}
impl Drop for FixtureChild {
    fn drop(&mut self) {
        unsafe {
            TerminateJobObject(self.job.as_raw_handle(), 1);
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

#[test]
fn powershell_wrapper_keeps_literal_paths_and_returns_to_its_caller() {
    let root = TestRuntimeRoot::new("powershell-handoff").unwrap();
    // This fixture is allocated on a local drive. Supply the identity-equivalent
    // ordinary spelling to .NET Framework and the PowerShell filesystem provider.
    let root_path = PathBuf::from(root.path().to_str().unwrap().strip_prefix(r"\\?\").unwrap());
    assert_eq!(root_path.canonicalize().unwrap(), root.path());
    let temporary = root_path.join("handoff-temp");
    std::fs::create_dir(&temporary).unwrap();
    for name in ["runtime", "cache", "context", "inventory"] {
        root.create_private_dir(name).unwrap();
    }
    let binary = Path::new(env!("CARGO_BIN_EXE_runyte"));
    let mut search_path = vec![binary.parent().unwrap().to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        search_path.extend(std::env::split_paths(&existing));
    }
    let output = std::fs::File::create(root.join("output.log")).unwrap();
    let mut fixture = FixtureChild::spawn(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
            .env("XDG_CONFIG_HOME", root_path.join("config"))
            .env("XDG_CACHE_HOME", root_path.join("cache"))
            .env("XDG_RUNTIME_DIR", root_path.join("runtime"))
            .env("RUNYTE_CONTEXT_HOME", root_path.join("context"))
            .env("RUNYTE_ALL_HOSTS_DIR", root_path.join("inventory"))
            .env("RUNYTE_HANDOFF_ROOT", &root_path)
            .env(
                "RUNYTE_HANDOFF_WRAPPER",
                Path::new(env!("CARGO_MANIFEST_DIR")).join("contrib/runyte.ps1"),
            )
            .env(
                "RUNYTE_HANDOFF_NATIVE_FIXTURE",
                std::env::current_exe().unwrap(),
            )
            .env("TEMP", &temporary)
            .env("TMP", &temporary)
            .env("PATH", std::env::join_paths(search_path).unwrap())
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output),
        &root_path,
    );
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if let Some(status) = fixture.child.try_wait().unwrap() {
            assert!(
                status.success(),
                "{}",
                std::fs::read_to_string(root.join("output.log")).unwrap()
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "PowerShell acceptance timed out: {}",
            std::fs::read_to_string(root.join("output.log")).unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn powershell() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe")
}

fn start_foreground_host(
    root: &Path,
    project: &Path,
    target: &Path,
    config: &Path,
    label: &str,
) -> ForegroundHost {
    let output = std::fs::File::create(root.join(format!("{label}-host.log"))).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["--serve", "--project-root"])
        .arg(project)
        .arg("--config")
        .arg(config)
        .arg("--log")
        .arg(root.join(format!("{label}-runyte.log")))
        .arg("--")
        .arg(target)
        .current_dir(project)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env("RUNYTE_CONTEXT_HOME", root.join("context"))
        .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
        .env_remove("RUNYTE_INTERNAL_WINDOWS_LAYOUT")
        .stdin(Stdio::null())
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn()
        .unwrap();
    let mut host = ForegroundHost(child);
    let canonical = project.canonicalize().unwrap();
    let ready = root
        .join("runtime/runyte")
        .join(runyte::workspace::workspace_id(&canonical))
        .join("endpoint.json");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(bytes) = std::fs::read(&ready)
            && let Ok(metadata) =
                runyte::workspace::windows_endpoint::EndpointMetadata::from_json(&bytes)
            && metadata.process.pid == host.0.id()
        {
            return host;
        }
        assert!(
            host.0.try_wait().is_ok_and(|status| status.is_none()) && Instant::now() < deadline,
            "{label} persistent host did not publish at {ready:?}; process={:?}; output={}; host log={}; inventory entries={:?}; state entries={:?}; runtime entries={:?}",
            host.0.try_wait(),
            std::fs::read_to_string(root.join(format!("{label}-host.log"))).unwrap_or_default(),
            std::fs::read_to_string(root.join(format!("{label}-runyte.log"))).unwrap_or_default(),
            std::fs::read_dir(root.join("inventory"))
                .ok()
                .map(|entries| entries
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .collect::<Vec<_>>()),
            std::fs::read_dir(project.join(".runyte"))
                .ok()
                .map(|entries| entries
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .collect::<Vec<_>>()),
            std::fs::read_dir(root.join("runtime/runyte"))
                .ok()
                .map(|entries| entries
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .collect::<Vec<_>>()),
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn script_arguments(mode: &str) -> Vec<String> {
    vec![
        "-NoLogo".into(),
        "-NoProfile".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-File".into(),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/windows_handoff.ps1")
            .to_str()
            .unwrap()
            .to_owned(),
        "-Mode".into(),
        mode.into(),
    ]
}

struct Console {
    child: Pty,
    events: mpsc::Receiver<PtyEvent>,
    screen: runyte::terminal::emulator::Emulator,
    transcript: Vec<u8>,
}

impl Console {
    fn spawn(root: &Path, mode: &str) -> Self {
        let (sender, events) = mpsc::channel();
        let child = Pty::spawn(
            powershell().as_os_str(),
            &script_arguments(mode),
            root,
            120,
            30,
            move |event| {
                let _ = sender.send(event);
            },
        )
        .unwrap();
        Self {
            child,
            events,
            screen: runyte::terminal::emulator::Emulator::new(120, 30),
            transcript: Vec::new(),
        }
    }

    fn output(&mut self, bytes: &[u8]) {
        assert!(self.transcript.len() + bytes.len() <= OUTPUT_LIMIT);
        self.transcript.extend_from_slice(bytes);
        self.screen.feed(bytes);
    }

    fn until(&mut self, marker: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let screen = (0..self.screen.rows())
                .filter_map(|row| self.screen.grid().line(row))
                .map(|line| {
                    line.iter()
                        .filter(|cell| cell.width != 0)
                        .map(|cell| cell.text())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            if screen.contains(marker) {
                return;
            }
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(PtyEvent::Output(bytes)) => self.output(&bytes),
                event => panic!(
                    "waiting for {marker:?}, got {event:?}: {}",
                    String::from_utf8_lossy(&self.transcript)
                ),
            }
        }
    }

    fn send(&self, text: &str) {
        assert!(self.child.write(text.as_bytes().to_vec()));
    }

    fn exit(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(PtyEvent::Output(bytes)) => self.output(&bytes),
                Ok(PtyEvent::Exited(code)) => {
                    assert_eq!(
                        code,
                        Some(0),
                        "{}",
                        String::from_utf8_lossy(&self.transcript)
                    );
                    return;
                }
                event => panic!("{event:?}: {}", String::from_utf8_lossy(&self.transcript)),
            }
        }
    }
}

#[derive(serde::Deserialize)]
struct ShellResult {
    code: i32,
    directory: PathBuf,
    remaining: usize,
}

fn result(root: &Path) -> ShellResult {
    serde_json::from_slice(&std::fs::read(root.join("result.json")).unwrap()).unwrap()
}

#[test]
#[ignore = "compiled fixture relaunched with private config and cache"]
fn native_powershell_handoff_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_HANDOFF_ROOT").unwrap());
    let admission_deadline = Instant::now() + Duration::from_secs(15);
    while !root.join("fixture-admitted").try_exists().unwrap() {
        assert!(
            Instant::now() < admission_deadline,
            "fixture job admission timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let project = root.join("project");
    let directory = project.join("literal [brackets] café ' $tick`");
    let target = directory.join("note.txt");
    let config = root.join("config/config.yaml");
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME").unwrap(),
        root.join("config")
    );
    assert_eq!(
        std::env::var_os("XDG_CACHE_HOME").unwrap(),
        root.join("cache")
    );
    assert_eq!(
        std::env::var_os("RUNYTE_CONTEXT_HOME").unwrap(),
        root.join("context")
    );
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "lsp:\n  enable: false\n").unwrap();
    std::fs::write(&target, "HANDOFF_TARGET_MARKER\n").unwrap();

    // Each PowerShell child and its editor inherit explicit isolated paths from
    // this command builder; no process-global environment is changed.
    let run_script = |mode: &str| {
        Command::new(powershell())
            .args(script_arguments(mode))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .env("RUNYTE_CONTEXT_HOME", root.join("context"))
            .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
            .env("RUNYTE_HANDOFF_CONFIG", &config)
            .env("RUNYTE_HANDOFF_TARGET", &target)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let helpers = run_script("Helpers");
    assert!(
        helpers.status.success(),
        "{}",
        String::from_utf8_lossy(&helpers.stderr)
    );
    let failure = run_script("Failure");
    assert!(
        failure.status.success(),
        "{}",
        String::from_utf8_lossy(&failure.stderr)
    );
    let failed = result(&root);
    assert_ne!(failed.code, 0);
    assert_eq!(
        failed.directory.canonicalize().unwrap(),
        project.canonicalize().unwrap()
    );
    assert_eq!(failed.remaining, 0);
    for mode in ["ReadFailure", "LocationFailure"] {
        let output = run_script(mode);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("POWERSHELL_CALLER_CONTINUED"));
        let failed = result(&root);
        assert_ne!(failed.code, 0, "{mode} incorrectly reported success");
        assert_eq!(
            failed.directory.canonicalize().unwrap(),
            project.canonicalize().unwrap()
        );
        assert_eq!(failed.remaining, 0);
    }

    // ConPTY's environment comes from a second compiled helper, so the editor
    // targets stay fixture-owned without mutating this process's environment.
    let run_console = |operation: &str| {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "handoff_console_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .env("RUNYTE_CONTEXT_HOME", root.join("context"))
            .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
            .env("RUNYTE_HANDOFF_CONFIG", &config)
            .env("RUNYTE_HANDOFF_TARGET", &target)
            .env("RUNYTE_HANDOFF_OPERATION", operation)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    for operation in ["quit", "quit-here", "force"] {
        let output = run_console(operation);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let completed = result(&root);
        assert_eq!(completed.code, 0);
        assert_eq!(completed.remaining, 0);
        let expected = if operation == "quit" {
            &project
        } else {
            &directory
        };
        assert_eq!(
            completed.directory.canonicalize().unwrap(),
            expected.canonicalize().unwrap()
        );
    }

    // The physical PowerShell caller owns the same private handoff file while
    // its TUI moves from project A to a separate live project B. The host
    // reports B's chosen directory; only that caller publishes the record.
    std::fs::write(project.join("a.txt"), "HANDOFF_A_MARKER\n").unwrap();
    let mut host_a = start_foreground_host(&root, &project, &project.join("a.txt"), &config, "a");
    let mut host_b = start_foreground_host(&root, &directory, &target, &config, "b");
    for operation in [
        "persistent-detach",
        "persistent-no-handoff",
        "persistent-quit-here",
        "persistent-quit",
    ] {
        let output = run_console(operation);
        assert!(
            output.status.success(),
            "{operation}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let completed = result(&root);
        assert_eq!(completed.code, 0, "{operation}");
        assert_eq!(completed.remaining, 0, "{operation}");
        let expected = if operation == "persistent-quit-here" {
            &directory
        } else {
            &project
        };
        assert_eq!(
            completed.directory.canonicalize().unwrap(),
            expected.canonicalize().unwrap(),
            "{operation}"
        );
        if operation != "persistent-quit" {
            assert!(
                host_a.0.try_wait().unwrap().is_none(),
                "source host A stopped after {operation}"
            );
        }
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while host_b.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "quit-here did not stop host B");
        std::thread::sleep(Duration::from_millis(15));
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while host_a.0.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "ordinary quit did not stop host A"
        );
        std::thread::sleep(Duration::from_millis(15));
    }
    assert_eq!(
        std::fs::read_to_string(target).unwrap(),
        "HANDOFF_TARGET_MARKER\n"
    );
}

#[test]
#[ignore = "compiled native console fixture"]
fn handoff_console_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_HANDOFF_ROOT").unwrap());
    let operation = std::env::var("RUNYTE_HANDOFF_OPERATION").unwrap();
    let mode = if operation == "persistent-no-handoff" {
        "PersistentDirect"
    } else if operation.starts_with("persistent") {
        "Persistent"
    } else {
        "Editor"
    };
    let mut editor = Console::spawn(&root, mode);
    if operation.starts_with("persistent") {
        editor.until("HANDOFF_A_MARKER");
        editor.until("NOR");
    }
    if operation == "persistent-quit-here" {
        editor.send(":open-session-directory\r");
        editor.until("Open this directory");
        let directory = root.join("project/literal [brackets] café ' $tick`");
        editor.send(&format!("{}\\", directory.display()));
        editor.until("literal [brackets]");
        editor.send("\r");
    }
    if !operation.starts_with("persistent") || operation == "persistent-quit-here" {
        editor.until("HANDOFF_TARGET_MARKER");
        editor.until("NOR");
    }
    if operation == "force" || operation == "persistent-quit-here" {
        editor.send("i");
        editor.until("INS");
        editor.send("\x1b[200~discard \x1b[201~");
        editor.until("discard HANDOFF_TARGET_MARKER");
        editor.send("\x1b");
        editor.until("NOR");
        editor.send(":qh\r");
        editor.until("unsaved changes");
        editor.send(":qh!\r");
    } else if operation == "persistent-no-handoff" {
        editor.send(":qh\r");
        editor.until("requires the runyte()");
        editor.send(":detach\r");
    } else if operation == "persistent-detach" {
        editor.send(":detach\r");
    } else if operation == "persistent-quit" {
        editor.send(":qa\r");
    } else {
        editor.send(&format!(":{operation}\r"));
    }
    editor.exit();
    assert!(String::from_utf8_lossy(&editor.transcript).contains("POWERSHELL_CALLER_CONTINUED"));
}

#[test]
#[ignore = "compiled argument fidelity fixture"]
fn handoff_argument_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_HANDOFF_ROOT").unwrap());
    std::fs::write(
        root.join("arguments.json"),
        serde_json::to_vec(&std::env::args().skip(1).collect::<Vec<_>>()).unwrap(),
    )
    .unwrap();
}
