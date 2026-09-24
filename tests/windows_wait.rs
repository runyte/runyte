// SPDX-License-Identifier: MPL-2.0
#![cfg(windows)]

use runyte::{
    config::Config,
    terminal::pty::{Pty, PtyEvent},
    test_support::TestRuntimeRoot,
    workspace::{
        windows_catalog::HistoryTarget,
        windows_control::ControlSnapshot,
        windows_location::{CapturedRoots, DiscoveryInputs, DiscoveryScope},
    },
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
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const FIXTURE: &str = "native_persistent_wait_fixture";
const LAUNCHER_MULTI: &str = "native_wait_launcher_multi_fixture";
const LAUNCHER_SINGLE: &str = "native_wait_launcher_single_fixture";
const FIRST_NAME: &str = "-first [note] café.txt";
const SECOND_NAME: &str = "second note.txt";
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

fn ensure_foreground_host(
    host: &mut Option<ForegroundHost>,
    root: &Path,
    project: &Path,
    config: &Path,
) {
    let project = project.canonicalize().unwrap();
    let endpoint = root
        .join("runtime/runyte")
        .join(runyte::workspace::workspace_id(&project))
        .join("endpoint.json");
    let configured_state = Config::default().workspace.state;
    let state = runyte::project_root::resolve_state_root(&project, &configured_state);
    let scope = DiscoveryScope::resolve(DiscoveryInputs {
        reserved_user_roots: vec![root.join("config"), root.join("cache")],
        roots: CapturedRoots::capture(),
    })
    .unwrap();
    let current = scope.known_read_location(&project, &state).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let ready_for = |pid| {
        let Ok(bytes) = std::fs::read(&endpoint) else {
            return false;
        };
        let Ok(metadata) = runyte::workspace::windows_endpoint::EndpointMetadata::from_json(&bytes)
        else {
            return false;
        };
        metadata.process.pid == pid
            && runtime
                .block_on(ControlSnapshot::observe_at(
                    &scope,
                    Some(&current),
                    &configured_state,
                    false,
                ))
                .is_ok_and(|snapshot| {
                    matches!(
                        snapshot.history().select(&project, Some(&project)),
                        Ok(Some(HistoryTarget::Live { publication, .. }))
                            if publication.metadata().process.pid == pid
                    )
                })
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    if let Some(current_host) = host.as_mut() {
        while current_host.0.try_wait().unwrap().is_none() {
            if ready_for(current_host.0.id()) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "prior foreground wait host remained alive without an exact live publication: {}",
                std::fs::read_to_string(root.join("host-output.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    let output = std::fs::File::create(root.join("host-output.log")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["--serve", "--project-root"])
        .arg(&project)
        .arg("-v")
        .arg("--config")
        .arg(config)
        .current_dir(&project)
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
    let pid = child.id();
    *host = Some(ForegroundHost(child));
    loop {
        if ready_for(pid) {
            return;
        }
        assert!(
            host.as_mut().unwrap().0.try_wait().unwrap().is_none() && Instant::now() < deadline,
            "foreground wait host did not publish: {}",
            std::fs::read_to_string(root.join("host-output.log")).unwrap()
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

impl FixtureChild {
    fn spawn(command: &mut Command, root: &Path) -> Self {
        let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        assert!(!raw.is_null(), "{}", std::io::Error::last_os_error());
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };
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

fn enter_breakaway_job() -> OwnedHandle {
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    assert!(!raw.is_null(), "{}", std::io::Error::last_os_error());
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_BREAKAWAY_OK;
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
    assert_ne!(
        unsafe { AssignProcessToJobObject(job.as_raw_handle(), GetCurrentProcess()) },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
    job
}

#[test]
fn windows_wait_uses_a_persistent_session_until_requested_buffers_complete() {
    let root = TestRuntimeRoot::new("windows-wait").unwrap();
    root.create_private_dir("runtime").unwrap();
    let output = std::fs::File::create(root.join("output.log")).unwrap();
    let mut fixture = FixtureChild::spawn(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("RUNYTE_CONTEXT_HOME", root.join("context"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
            .env("RUNYTE_WAIT_ROOT", root.path())
            .env_remove("RUNYTE_INTERNAL_WINDOWS_LAYOUT")
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output),
        root.path(),
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
            "native wait fixture timed out: {}",
            std::fs::read_to_string(root.join("output.log")).unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct Console {
    child: Pty,
    events: mpsc::Receiver<PtyEvent>,
    screen: runyte::terminal::emulator::Emulator,
    transcript: Vec<u8>,
}

impl Console {
    fn spawn(root: &Path, config: &Path, targets: &[&str]) -> Self {
        let (sender, events) = mpsc::channel();
        assert_eq!(config, root.parent().unwrap().join("config/config.yaml"));
        let launcher = match targets {
            [FIRST_NAME, SECOND_NAME] => LAUNCHER_MULTI,
            [SECOND_NAME] => LAUNCHER_SINGLE,
            _ => panic!("unexpected wait launcher targets"),
        };
        let args = [
            "--exact".to_owned(),
            launcher.to_owned(),
            "--ignored".to_owned(),
            "--nocapture".to_owned(),
        ];
        let child = Pty::spawn_in_context(
            std::env::current_exe().unwrap().as_os_str(),
            &args,
            root,
            120,
            30,
            Some(""), // external console, without an integrated-terminal marker
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
                assert!(self.child.finished().is_none());
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

    fn exit(&mut self) -> Option<i32> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(PtyEvent::Output(bytes)) => self.output(&bytes),
                Ok(PtyEvent::Exited(code)) => return code,
                error => panic!("{error:?}: {}", String::from_utf8_lossy(&self.transcript)),
            }
        }
    }
}

#[test]
#[ignore = "compiled fixture relaunched with isolated configuration and cache"]
fn native_persistent_wait_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_WAIT_ROOT").unwrap());
    let admitted = Instant::now() + Duration::from_secs(20);
    while !root.join("fixture-admitted").exists() {
        assert!(
            Instant::now() < admitted,
            "wait fixture was not admitted to its job"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let _breakaway = enter_breakaway_job();
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
    let project = root.join("project");
    let config = root.join("config/config.yaml");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    // An explicit --wait selects the same persistent workspace regardless of
    // the bare-launch mode preference.
    std::fs::write(
        &config,
        "lsp:\n  enable: false\nworkspace:\n  mode: persistent\n",
    )
    .unwrap();
    let first = FIRST_NAME;
    let second = SECOND_NAME;
    std::fs::write(project.join(first), "WAIT_FIRST_MARKER\n").unwrap();
    std::fs::write(project.join(second), "WAIT_SECOND_MARKER\n").unwrap();

    let mut host = None;
    ensure_foreground_host(&mut host, &root, &project, &config);
    let mut editor = Console::spawn(&project, &config, &[first, second]);
    let first_marker = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        editor.until("WAIT_FIRST_MARKER");
    }));
    if let Err(panic) = first_marker {
        eprintln!(
            "wait fixture: foreground host pid={} status={:?} after editor failure\nhost log:\n{}",
            host.as_ref().unwrap().0.id(),
            host.as_mut().unwrap().0.try_wait().unwrap(),
            std::fs::read_to_string(root.join("host-output.log")).unwrap_or_default()
        );
        std::panic::resume_unwind(panic);
    }
    editor.until("NOR");
    editor.send("i");
    editor.until("INS");
    editor.send("\x1b[200~saved \x1b[201~");
    editor.until("saved WAIT_FIRST_MARKER");
    editor.send("\x1b");
    editor.until("NOR");
    editor.send(":quit\r");
    editor.until("unsaved changes");
    assert_eq!(
        std::fs::read_to_string(project.join(first)).unwrap(),
        "WAIT_FIRST_MARKER\n"
    );
    // Closing one requested buffer leaves the multi-file wait pending.
    editor.send(":wbc\r");
    editor.until("[about]");
    editor.send(":open \"second note.txt\"\r");
    let second_marker = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        editor.until("WAIT_SECOND_MARKER");
    }));
    if let Err(panic) = second_marker {
        eprintln!(
            "wait fixture: foreground host pid={} status={:?} after second marker failure\nhost log:\n{}",
            host.as_ref().unwrap().0.id(),
            host.as_mut().unwrap().0.try_wait().unwrap(),
            std::fs::read_to_string(root.join("host-output.log")).unwrap_or_default()
        );
        std::panic::resume_unwind(panic);
    }
    assert_eq!(
        std::fs::read_to_string(project.join(first)).unwrap(),
        "saved WAIT_FIRST_MARKER\n"
    );
    editor.send(":wbc\r");
    assert_eq!(editor.exit(), Some(0));
    // Clean quit commits the attached wait and can stop the host in the same
    // loop turn. Its creator must receive Completed before pipe shutdown.
    ensure_foreground_host(&mut host, &root, &project, &config);
    let mut editor = Console::spawn(&project, &config, &[second]);
    editor.until("WAIT_SECOND_MARKER");
    editor.send(":q\r");
    assert_eq!(editor.exit(), Some(0));

    ensure_foreground_host(&mut host, &root, &project, &config);
    let mut editor = Console::spawn(&project, &config, &[second]);
    editor.until("WAIT_SECOND_MARKER");
    editor.send("i");
    editor.until("INS");
    editor.send("\x1b[200~discard \x1b[201~");
    editor.until("discard WAIT_SECOND_MARKER");
    editor.send("\x1b");
    editor.until("NOR");
    editor.send(":q!\r");
    assert_ne!(editor.exit(), Some(0));
    assert_eq!(
        std::fs::read_to_string(project.join(second)).unwrap(),
        "WAIT_SECOND_MARKER\n"
    );

    ensure_foreground_host(&mut host, &root, &project, &config);
    let mut editor = Console::spawn(&project, &config, &[second]);
    editor.until("WAIT_SECOND_MARKER");
    editor.child.terminate();
    assert_ne!(editor.exit(), Some(0));

    // Invalid launch requests fail before entering an editor, also without a
    // terminal. Each process gets explicit fixture-owned environment roots.
    for args in [
        vec!["--wait"],
        vec!["--wait", "+2", second],
        vec!["--wait", "--standalone", second],
        vec!["--wait", "--config", "missing.yaml", second],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_runyte"))
            .args(args)
            .current_dir(&project)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("RUNYTE_CONTEXT_HOME", root.join("context"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
            .env_remove("RUNYTE_INTERNAL_WINDOWS_LAYOUT")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
    let stop = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["--session-stop", "--force"])
        .arg(&project)
        .arg("--config")
        .arg(&config)
        .current_dir(&project)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env("RUNYTE_CONTEXT_HOME", root.join("context"))
        .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
        .env_remove("RUNYTE_INTERNAL_WINDOWS_LAYOUT")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        stop.status.success(),
        "{}{}",
        String::from_utf8_lossy(&stop.stdout),
        String::from_utf8_lossy(&stop.stderr)
    );
    if let Some(mut host) = host {
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.0.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "foreground wait host ignored stop"
            );
            std::thread::sleep(Duration::from_millis(15));
        }
    }
}

fn launch_wait_editor(targets: &[&str]) -> ! {
    let root = PathBuf::from(std::env::var_os("RUNYTE_WAIT_ROOT").unwrap());
    let project = root.join("project");
    let config = root.join("config/config.yaml");
    let status = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["--wait", "--config"])
        .arg(config)
        .arg("--project-root")
        .arg(&project)
        .arg("--")
        .args(targets)
        .current_dir(project)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env("RUNYTE_CONTEXT_HOME", root.join("context"))
        .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
        .env_remove("RUNYTE_INTERNAL_WINDOWS_LAYOUT")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .unwrap();
    std::process::exit(status.code().unwrap_or(1));
}

#[test]
#[ignore = "compiled ConPTY launcher keeps the --wait process's natural parent alive"]
fn native_wait_launcher_multi_fixture() {
    launch_wait_editor(&[FIRST_NAME, SECOND_NAME]);
}

#[test]
#[ignore = "compiled ConPTY launcher keeps the --wait process's natural parent alive"]
fn native_wait_launcher_single_fixture() {
    launch_wait_editor(&[SECOND_NAME]);
}
