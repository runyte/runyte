// SPDX-License-Identifier: MPL-2.0
#![cfg(windows)]

use runyte::{
    terminal::{
        emulator::Emulator,
        pty::{Pty, PtyEvent},
    },
    test_support::TestRuntimeRoot,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::{JobObjects::IsProcessInJob, Threading::GetCurrentProcess};

const FIXTURE: &str = "public_persistent_attachment_fixture";
const WAIT_FIXTURE: &str = "public_parent_wait_fixture";
const MARKER_HELPER: &str = "public_parent_wait_marker_helper";
const WAIT_HELPER: &str = "public_parent_wait_launch_helper";
const VISIT_FIXTURE: &str = "public_manager_visit_fixture";

fn running_in_job() -> bool {
    let mut member = 0;
    assert_ne!(
        unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut member) },
        0
    );
    member != 0
}

#[test]
fn direct_native_attachment_starts_retains_and_refuses_takeover() {
    let root = TestRuntimeRoot::new("public-native-attach").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let config_dir = root.create_private_dir("config").unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
    let cache = root.create_private_dir("cache").unwrap();
    let context = root.create_private_dir("context").unwrap();
    let config = config_dir.join("config.yaml");
    fs::write(
        &config,
        "lsp:\n  enable: false\nworkspace:\n  mode: persistent\n  idle_retirement_minutes: 0\n",
    )
    .unwrap();
    fs::write(project.join("note.txt"), "original\n").unwrap();
    fs::create_dir(project.join("nested")).unwrap();
    fs::create_dir(project.join("nested/.runyte")).unwrap();
    let _cleanup = HostCleanup {
        root: root.path().to_path_buf(),
        project: project.clone(),
        config: config.clone(),
    };
    let output = fs::File::create(root.join("fixture-output")).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
        .env("RUNYTE_PUBLIC_ATTACH_ROOT", root.path())
        .env("XDG_CONFIG_HOME", config_dir)
        .env("XDG_CACHE_HOME", cache)
        .env("XDG_RUNTIME_DIR", runtime)
        .env("RUNYTE_CONTEXT_HOME", context)
        .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
        .env_remove("RUNYTE_PARENT_CONTEXT")
        .env("PATH", "")
        .stdin(Stdio::null())
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!(
                "native attachment timed out: {}",
                fs::read_to_string(root.join("fixture-output")).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.success(),
        "{}",
        fs::read_to_string(root.join("fixture-output")).unwrap()
    );
}

#[test]
fn integrated_terminal_wait_uses_exact_parent_and_completes_each_file() {
    let root = TestRuntimeRoot::new("public-native-parent-wait").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let config_dir = root.create_private_dir("config").unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
    let cache = root.create_private_dir("cache").unwrap();
    let context = root.create_private_dir("context").unwrap();
    let config = config_dir.join("config.yaml");
    fs::write(
        &config,
        "lsp:\n  enable: false\nworkspace:\n  mode: persistent\n",
    )
    .unwrap();
    fs::write(project.join("first.txt"), "FIRST_WAIT_MARKER\n").unwrap();
    fs::write(project.join("second.txt"), "SECOND_WAIT_MARKER\n").unwrap();
    let _cleanup = HostCleanup {
        root: root.path().to_path_buf(),
        project,
        config,
    };
    let output = fs::File::create(root.join("wait-fixture-output")).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", WAIT_FIXTURE, "--ignored", "--nocapture"])
        .env("RUNYTE_PUBLIC_ATTACH_ROOT", root.path())
        .env("XDG_CONFIG_HOME", config_dir)
        .env("XDG_CACHE_HOME", cache)
        .env("XDG_RUNTIME_DIR", runtime)
        .env("RUNYTE_CONTEXT_HOME", context)
        .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
        .env_remove("RUNYTE_PARENT_CONTEXT")
        .env("PATH", "")
        .stdin(Stdio::null())
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!(
                "native parent wait timed out: {}",
                fs::read_to_string(root.join("wait-fixture-output")).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.success(),
        "{}",
        fs::read_to_string(root.join("wait-fixture-output")).unwrap()
    );
}

#[test]
fn native_manager_visits_selected_live_publication_and_refuses_stale_row() {
    let root = TestRuntimeRoot::new("public-native-manager-visit").unwrap();
    let source = root.create_private_dir("visit-source").unwrap();
    let destination = root.create_private_dir("visit-destination").unwrap();
    let config_dir = root.create_private_dir("config").unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
    let cache = root.create_private_dir("cache").unwrap();
    let context = root.create_private_dir("context").unwrap();
    let config = config_dir.join("config.yaml");
    fs::write(
        &config,
        "lsp:\n  enable: false\nworkspace:\n  mode: persistent\n",
    )
    .unwrap();
    fs::write(source.join("source.txt"), "SOURCE_VISIT_MARKER\n").unwrap();
    fs::write(
        destination.join("destination.txt"),
        "DESTINATION_VISIT_MARKER\n",
    )
    .unwrap();
    let _source_cleanup = HostCleanup {
        root: root.path().to_path_buf(),
        project: source,
        config: config.clone(),
    };
    let _destination_cleanup = HostCleanup {
        root: root.path().to_path_buf(),
        project: destination,
        config,
    };
    let output = fs::File::create(root.join("manager-visit-output")).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", VISIT_FIXTURE, "--ignored", "--nocapture"])
        .env("RUNYTE_PUBLIC_ATTACH_ROOT", root.path())
        .env("XDG_CONFIG_HOME", config_dir)
        .env("XDG_CACHE_HOME", cache)
        .env("XDG_RUNTIME_DIR", runtime)
        .env("RUNYTE_CONTEXT_HOME", context)
        .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
        .env_remove("RUNYTE_PARENT_CONTEXT")
        .env("PATH", "")
        .stdin(Stdio::null())
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!(
                "manager visit timed out: {}",
                fs::read_to_string(root.join("manager-visit-output")).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.success(),
        "{}",
        fs::read_to_string(root.join("manager-visit-output")).unwrap()
    );
}

struct Console {
    child: Pty,
    events: mpsc::Receiver<PtyEvent>,
    screen: Emulator,
    output: Vec<u8>,
}

struct HostCleanup {
    root: PathBuf,
    project: PathBuf,
    config: PathBuf,
}
struct ForegroundHost(std::process::Child);
impl Drop for ForegroundHost {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Drop for HostCleanup {
    fn drop(&mut self) {
        let Ok(mut child) = Command::new(env!("CARGO_BIN_EXE_runyte"))
            .args(["--session-stop", "--force"])
            .arg(&self.project)
            .arg("--config")
            .arg(&self.config)
            .current_dir(&self.project)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("XDG_RUNTIME_DIR", self.root.join("runtime"))
            .env("RUNYTE_CONTEXT_HOME", self.root.join("context"))
            .env("RUNYTE_ALL_HOSTS_DIR", self.root.join("inventory"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
}
impl Console {
    fn spawn(args: &[String], cwd: &Path) -> Self {
        let (sender, events) = mpsc::channel();
        let child = Pty::spawn_in_context(
            Path::new(env!("CARGO_BIN_EXE_runyte")).as_os_str(),
            args,
            cwd,
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
            screen: Emulator::new(120, 30),
            output: Vec::new(),
        }
    }
    fn send(&self, text: &str) {
        assert!(self.child.write(text.as_bytes().to_vec()));
    }
    fn screen_text(&self) -> String {
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
    fn output_tail(&self) -> String {
        String::from_utf8_lossy(&self.output[self.output.len().saturating_sub(1024)..]).into_owned()
    }
    fn event(&mut self, deadline: Instant, stage: &str) -> Option<i32> {
        match self.events.recv_timeout(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(50)),
        ) {
            Ok(PtyEvent::Output(bytes)) => {
                self.screen.feed(&bytes);
                self.output.extend_from_slice(&bytes);
                None
            }
            Ok(PtyEvent::Exited(code)) => Some(code.unwrap_or(-1)),
            other => {
                assert!(
                    Instant::now() < deadline,
                    "{stage}: {other:?}; screen: {}; output: {}",
                    self.screen_text(),
                    self.output_tail()
                );
                None
            }
        }
    }
    fn until(&mut self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.screen_text().replace('\n', "").contains(text) {
            if let Some(code) = self.event(deadline, text) {
                panic!(
                    "editor exited before {text} with code {code}; screen: {}; output: {}",
                    self.screen_text(),
                    self.output_tail()
                );
            }
        }
    }
    fn until_screen_after(&mut self, stage: &str, matches: impl Fn(&str) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(code) = self.event(deadline, stage) {
                panic!(
                    "editor exited before {stage} with code {code}; screen: {}; output: {}",
                    self.screen_text(),
                    self.output_tail()
                );
            }
            if matches(&self.screen_text()) {
                return;
            }
        }
    }
    fn exit(&mut self, expected: i32) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(code) = self.event(deadline, "editor exit") {
                assert_eq!(code, expected, "{}", self.screen_text());
                return;
            }
        }
    }
}

#[test]
#[ignore = "reexecuted with fixture-owned storage and a real native console"]
fn public_persistent_attachment_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_PUBLIC_ATTACH_ROOT").unwrap());
    let project = root.join("project");
    let config = root.join("config/config.yaml");
    // ConPTY test children belong to a non-breakaway job. Exercise the public
    // missing-host startup from an ordinary process, then use ConPTY to verify
    // the interactive attachment to its exact published host.
    let start_output = fs::File::create(root.join("start-output")).unwrap();
    let mut starter = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["-a", "--config"])
        .arg(&config)
        .current_dir(&project)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(start_output)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if starter.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            starter.kill().unwrap();
            starter.wait().unwrap();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let listing = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["--session-list", "--config"])
        .arg(&config)
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(
        listing.status.success(),
        "{}",
        String::from_utf8_lossy(&listing.stderr)
    );
    let published = String::from_utf8_lossy(&listing.stdout).contains("running")
        && String::from_utf8_lossy(&listing.stdout).contains(&*project.to_string_lossy());
    let start_error = fs::read_to_string(root.join("start-output")).unwrap();
    let _foreground = if published {
        None
    } else {
        assert!(
            running_in_job()
                && start_error
                    .contains("cannot create detached host under the current process/job policy")
                && start_error
                    .contains("CreateProcessW could not create the detached inheritance parent")
                && start_error.contains("(os error 5)"),
            "public startup failed for an unexpected reason: {start_error}; list: {}",
            String::from_utf8_lossy(&listing.stdout)
        );
        assert!(
            !String::from_utf8_lossy(&listing.stdout).contains(&*project.to_string_lossy()),
            "refused startup left a project publication: {}",
            String::from_utf8_lossy(&listing.stdout)
        );
        let host_output = fs::File::create(root.join("host-output")).unwrap();
        let host = Command::new(env!("CARGO_BIN_EXE_runyte"))
            .args(["--serve", "--project-root"])
            .arg(&project)
            .arg("--config")
            .arg(&config)
            .current_dir(&project)
            .stdin(Stdio::null())
            .stdout(host_output.try_clone().unwrap())
            .stderr(host_output)
            .spawn()
            .unwrap();
        let mut host = ForegroundHost(host);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let listing = Command::new(env!("CARGO_BIN_EXE_runyte"))
                .args(["--session-list", "--config"])
                .arg(&config)
                .current_dir(&project)
                .output()
                .unwrap();
            if listing.status.success()
                && String::from_utf8_lossy(&listing.stdout).contains("running")
                && String::from_utf8_lossy(&listing.stdout).contains(&*project.to_string_lossy())
            {
                break;
            }
            assert!(
                host.0.try_wait().unwrap().is_none() && Instant::now() < deadline,
                "foreground fixture host failed: {}; list: {}; error: {}",
                fs::read_to_string(root.join("host-output")).unwrap(),
                String::from_utf8_lossy(&listing.stdout),
                String::from_utf8_lossy(&listing.stderr)
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        Some(host)
    };
    let args = vec!["-a".into(), "--config".into(), config.display().to_string()];
    let mut first = Console::spawn(&args, &project);
    first.until("NOR");
    first.send(":open note.txt\r");
    first.until("original");
    first.send("iX\x1b");
    first.until("Xoriginal");

    let standalone = vec![
        "--standalone".into(),
        "--config".into(),
        config.display().to_string(),
    ];
    let mut overridden = Console::spawn(&standalone, &project);
    overridden.until("NOR");
    overridden.send(":quit\r");
    overridden.exit(0);

    let target = vec![
        "note.txt".into(),
        "--config".into(),
        config.display().to_string(),
    ];
    let mut target_editor = Console::spawn(&target, &project);
    target_editor.until("original");
    target_editor.send(":quit\r");
    target_editor.exit(0);

    let occupied = vec![
        "--persistent".into(),
        project.display().to_string(),
        "-v".into(),
        "--config".into(),
        config.display().to_string(),
    ];
    let mut second = Console::spawn(&occupied, &root);
    second.exit(1);
    assert!(
        String::from_utf8_lossy(&second.output).contains("kept its own log level and destination"),
        "retained host did not report ignored logging: {}",
        String::from_utf8_lossy(&second.output)
    );
    first.send(":detach\r");
    first.exit(0);

    let resumed_args = vec![
        "-a".into(),
        "--project-root".into(),
        project.display().to_string(),
        "--config".into(),
        config.display().to_string(),
    ];
    let mut resumed = Console::spawn(&resumed_args, &project.join("nested"));
    resumed.until("Xoriginal");
    resumed.send(":detach\r");
    resumed.exit(0);

    let mut automatic =
        Console::spawn(&["--config".into(), config.display().to_string()], &project);
    automatic.until("Xoriginal");
    automatic.send(":detach\r");
    automatic.exit(0);

    let undiscovered = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .arg("--config")
        .arg(&config)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(!undiscovered.status.success());
    assert!(
        String::from_utf8_lossy(&undiscovered.stderr)
            .contains("workspace.mode: persistent requires a discoverable project"),
        "{}",
        String::from_utf8_lossy(&undiscovered.stderr)
    );
    assert!(!root.join(".runyte").exists());

    let stop = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["--session-stop", "--force"])
        .arg(&project)
        .arg("--config")
        .arg(&config)
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
}

fn wait_for_file(path: &Path, stage: &str) {
    let deadline = Instant::now() + Duration::from_secs(12);
    while !path.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for {stage}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn publish_file(path: &Path, contents: &str) {
    let pending = path.with_extension("pending");
    fs::write(&pending, contents).unwrap();
    fs::rename(pending, path).unwrap();
}

fn fixture_environment(command: &mut Command, root: &Path) {
    command
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env("RUNYTE_CONTEXT_HOME", root.join("context"))
        .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"));
}

#[test]
#[ignore = "reexecuted with fixture-owned storage and a real native console"]
fn public_parent_wait_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_PUBLIC_ATTACH_ROOT").unwrap());
    let project = root.join("project");
    let config = root.join("config/config.yaml");
    let host_output = fs::File::create(root.join("wait-host-output")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    command
        .args(["--serve", "--project-root"])
        .arg(&project)
        .arg("--config")
        .arg(&config)
        .current_dir(&project)
        .stdin(Stdio::null())
        .stdout(host_output.try_clone().unwrap())
        .stderr(host_output);
    fixture_environment(&mut command, &root);
    let host = command.spawn().unwrap();
    let mut host = ForegroundHost(host);
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
        command
            .args(["--session-list", "--config"])
            .arg(&config)
            .current_dir(&project);
        fixture_environment(&mut command, &root);
        let listing = command.output().unwrap();
        if listing.status.success()
            && String::from_utf8_lossy(&listing.stdout).contains("running")
            && String::from_utf8_lossy(&listing.stdout).contains(&*project.to_string_lossy())
        {
            break;
        }
        assert!(
            host.0.try_wait().unwrap().is_none() && Instant::now() < deadline,
            "parent wait host failed: {}; listing: {}",
            fs::read_to_string(root.join("wait-host-output")).unwrap(),
            String::from_utf8_lossy(&listing.stderr)
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let args = vec!["-a".into(), "--config".into(), config.display().to_string()];
    let mut editor = Console::spawn(&args, &project);
    editor.until("NOR");
    let fixture_executable = std::env::current_exe().unwrap();
    editor.send(&format!(
        ":terminal \"{}\" --exact {MARKER_HELPER} --ignored --nocapture\r",
        fixture_executable.display()
    ));
    let marker_path = root.join("live-parent-marker");
    wait_for_file(&marker_path, "live integrated-terminal marker");
    editor.until("INS");
    let marker = fs::read_to_string(&marker_path).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    command
        .args(["--wait", "copied.txt"])
        .env("RUNYTE_PARENT_CONTEXT", &marker)
        .current_dir(&project);
    fixture_environment(&mut command, &root);
    let copied = command.output().unwrap();
    assert!(
        !copied.status.success(),
        "copied marker outside terminal job unexpectedly admitted parent wait"
    );
    assert!(!project.join("copied.txt").exists());
    publish_file(&root.join("release-marker-helper"), "release");
    editor.until("NOR");
    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    command
        .args(["--wait", "stale.txt"])
        .env("RUNYTE_PARENT_CONTEXT", &marker)
        .current_dir(&project);
    fixture_environment(&mut command, &root);
    let stale = command.output().unwrap();
    assert!(!stale.status.success(), "stale marker admitted parent wait");
    assert!(!project.join("stale.txt").exists());

    editor.send(&format!(
        ":terminal \"{}\" --exact {WAIT_HELPER} --ignored --nocapture\r",
        fixture_executable.display()
    ));
    wait_for_file(
        &root.join("wait-client-started"),
        "real wait client startup",
    );
    editor.until("FIRST_WAIT_MARKER");
    editor.send(":wbc\r");
    editor.until("INS");
    editor.send("\x1c"); // Ctrl-\\ leaves Terminal Insert without sending child input.
    editor.until("NOR");
    assert!(
        !root.join("wait-client-exit").exists(),
        "first buffer completion released a two-file wait"
    );
    editor.send(":open second.txt\r");
    editor.until("SECOND_WAIT_MARKER");
    editor.send(":wbc\r");
    editor.until("INS");
    wait_for_file(&root.join("wait-client-exit"), "two-file wait completion");
    assert_eq!(
        fs::read_to_string(root.join("wait-client-exit")).unwrap(),
        "0"
    );
    publish_file(&root.join("release-wait-helper"), "release");
    editor.until("NOR");
    assert!(host.0.try_wait().unwrap().is_none());
    editor.send(":detach\r");
    editor.exit(0);
}

#[test]
#[ignore = "compiled child launched inside a real integrated terminal"]
fn public_parent_wait_marker_helper() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_PUBLIC_ATTACH_ROOT").unwrap());
    let marker = std::env::var("RUNYTE_PARENT_CONTEXT").unwrap();
    publish_file(&root.join("live-parent-marker"), &marker);
    wait_for_file(&root.join("release-marker-helper"), "marker release");
}

#[test]
#[ignore = "compiled child launches the public CLI inside a real terminal job"]
fn public_parent_wait_launch_helper() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_PUBLIC_ATTACH_ROOT").unwrap());
    let project = root.join("project");
    let error = fs::File::create(root.join("wait-client-error")).unwrap();
    let mut client = Command::new(env!("CARGO_BIN_EXE_runyte"))
        .args(["--wait", "first.txt", "second.txt"])
        .current_dir(&project)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env("RUNYTE_CONTEXT_HOME", root.join("context"))
        .env("RUNYTE_ALL_HOSTS_DIR", root.join("inventory"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(error)
        .spawn()
        .unwrap();
    publish_file(&root.join("wait-client-started"), "started");
    let status = client.wait().unwrap();
    publish_file(
        &root.join("wait-client-exit"),
        &status.code().unwrap_or(-1).to_string(),
    );
    wait_for_file(&root.join("release-wait-helper"), "wait helper release");
    assert!(
        status.success(),
        "{}",
        fs::read_to_string(root.join("wait-client-error")).unwrap()
    );
}

fn start_visit_host(root: &Path, project: &Path, config: &Path, label: &str) -> ForegroundHost {
    let output = fs::File::create(root.join(format!("{label}-host-output"))).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    command
        .args(["--serve", "--project-root"])
        .arg(project)
        .arg("--config")
        .arg(config)
        .current_dir(project)
        .stdin(Stdio::null())
        .stdout(output.try_clone().unwrap())
        .stderr(output);
    fixture_environment(&mut command, root);
    let mut host = ForegroundHost(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
        command
            .args(["--session-list", "--config"])
            .arg(config)
            .current_dir(project);
        fixture_environment(&mut command, root);
        let listing = command.output().unwrap();
        if listing.status.success()
            && String::from_utf8_lossy(&listing.stdout).contains("running")
            && String::from_utf8_lossy(&listing.stdout).contains(&*project.to_string_lossy())
        {
            return host;
        }
        assert!(
            host.0.try_wait().unwrap().is_none() && Instant::now() < deadline,
            "{label} host failed: {}; listing: {}",
            fs::read_to_string(root.join(format!("{label}-host-output"))).unwrap(),
            String::from_utf8_lossy(&listing.stderr)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn stop_visit_host(root: &Path, project: &Path, config: &Path, host: &mut ForegroundHost) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_runyte"));
    command
        .args(["--session-stop", "--force"])
        .arg(project)
        .arg("--config")
        .arg(config)
        .current_dir(project);
    fixture_environment(&mut command, root);
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while host.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "stopped visit host did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "reexecuted with fixture-owned hosts and a real native frontend"]
fn public_manager_visit_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_PUBLIC_ATTACH_ROOT").unwrap());
    let source = root.join("visit-source");
    let destination = root.join("visit-destination");
    let config = root.join("config/config.yaml");
    let mut source_host = start_visit_host(&root, &source, &config, "source");
    let mut destination_host = start_visit_host(&root, &destination, &config, "destination");
    let args = vec!["-a".into(), "--config".into(), config.display().to_string()];

    let mut destination_editor = Console::spawn(&args, &destination);
    destination_editor.until("NOR");
    destination_editor.send(":open destination.txt\r");
    destination_editor.until("DESTINATION_VISIT_MARKER");
    destination_editor.send(":detach\r");
    destination_editor.exit(0);

    let mut source_editor = Console::spawn(&args, &source);
    source_editor.until("NOR");
    source_editor.send(":open source.txt\r");
    source_editor.until("SOURCE_VISIT_MARKER");
    source_editor.send("i");
    source_editor.until("INS");
    source_editor.send("\x1b[200~dirty \x1b[201~");
    source_editor.until("dirty SOURCE_VISIT_MARKER");
    source_editor.send("\x1b");
    source_editor.until("NOR");
    source_editor.send(":session-list\r");
    source_editor.until("Enter visit running");
    source_editor.send("visit-destination\r");
    source_editor.until("DESTINATION_VISIT_MARKER");
    assert!(source_host.0.try_wait().unwrap().is_none());
    assert!(destination_host.0.try_wait().unwrap().is_none());
    source_editor.send(":detach\r");
    source_editor.exit(0);

    let mut source_editor = Console::spawn(&args, &source);
    source_editor.until("dirty SOURCE_VISIT_MARKER");
    source_editor.send(":session-list\r");
    source_editor.until("Enter visit running");
    source_editor.send("visit-destination");
    stop_visit_host(&root, &destination, &config, &mut destination_host);
    let mut replacement = start_visit_host(&root, &destination, &config, "replacement");
    // A catalog poll may already have marked the row lost. In that case
    // Enter refuses locally and leaves the manager open; Escape dismisses it.
    // If Enter reached the host, its exact-publication refusal already closes
    // the manager and Escape is harmless in the source editor's Normal mode.
    source_editor.send("\r\x1b");
    source_editor.until_screen_after("manager closes after stale visit", |screen| {
        screen.contains("dirty SOURCE_VISIT_MARKER")
            && screen.contains("NOR")
            && !screen.contains("Enter visit running")
    });
    // The interaction line can be replaced by a catalog refresh. The host
    // retains the exact-publication rejection in the notification buffer.
    source_editor.send(":notifications\r");
    source_editor.until("selected session changed");
    assert!(source_host.0.try_wait().unwrap().is_none());
    assert!(replacement.0.try_wait().unwrap().is_none());
    source_editor.send(":open source.txt\r");
    source_editor.until("dirty SOURCE_VISIT_MARKER");
    source_editor.send(":detach\r");
    source_editor.exit(0);
}
