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
                    String::from_utf8_lossy(&self.output)
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
                    String::from_utf8_lossy(&self.output)
                );
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
