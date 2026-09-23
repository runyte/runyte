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

const FIXTURE: &str = "terminal_mode_fixture";

#[test]
fn native_ctrl_backslash_switches_live_terminal_modes() {
    let root = TestRuntimeRoot::new("windows-terminal-mode").unwrap();
    let config_dir = root.create_private_dir("config").unwrap();
    let cache = root.create_private_dir("cache").unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
    let context = root.create_private_dir("context").unwrap();
    let project = root.create_private_dir("project").unwrap();
    fs::create_dir(project.join(".git")).unwrap();
    fs::write(project.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let config = config_dir.join("config.yaml");
    fs::write(&config, "lsp:\n  enable: false\n").unwrap();
    let output = fs::File::create(root.join("fixture-output")).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
        .env("RUNYTE_TERMINAL_MODE_ROOT", root.path())
        .env("XDG_CONFIG_HOME", config_dir)
        .env("XDG_CACHE_HOME", cache)
        .env("XDG_RUNTIME_DIR", runtime)
        .env("RUNYTE_CONTEXT_HOME", context)
        .env_remove("RUNYTE_PARENT_CONTEXT")
        .stdin(Stdio::null())
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "{}",
                fs::read_to_string(root.join("fixture-output")).unwrap()
            );
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "terminal mode fixture timed out: {}",
                fs::read_to_string(root.join("fixture-output")).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(project.is_dir());
}

#[test]
#[ignore = "reexecuted with fixture-owned storage and a real ConPTY editor"]
fn terminal_mode_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_TERMINAL_MODE_ROOT").unwrap());
    let project = root.join("project");
    let config = root.join("config/config.yaml");
    let (sender, events) = mpsc::channel();
    let editor = Pty::spawn_in_context(
        Path::new(env!("CARGO_BIN_EXE_runyte")).as_os_str(),
        &[
            "--standalone".into(),
            "--config".into(),
            config.display().to_string(),
        ],
        &project,
        120,
        30,
        Some(""),
        move |event| {
            let _ = sender.send(event);
        },
    )
    .unwrap();
    let mut screen = Emulator::new(120, 30);
    let mut output = Vec::new();
    let mut until = |needle: &str, absent: Option<&str>| {
        let deadline = Instant::now() + Duration::from_secs(15);
        // A previous frame can still contain the requested mode. Require new
        // PTY output after each input before accepting the next transition.
        let mut consumed_output = false;
        loop {
            let current = (0..screen.rows())
                .filter_map(|row| screen.grid().line(row))
                .map(|line| {
                    line.iter()
                        .filter(|cell| cell.width != 0)
                        .map(|cell| cell.text())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            if consumed_output
                && current.contains(needle)
                && absent.is_none_or(|excluded| !current.contains(excluded))
            {
                return;
            }
            match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(PtyEvent::Output(bytes)) => {
                    screen.feed(&bytes);
                    output.extend(bytes);
                    consumed_output = true;
                }
                other => panic!(
                    "waiting for {needle}: {other:?}; screen: {current}; output tail: {}",
                    String::from_utf8_lossy(&output[output.len().saturating_sub(1024)..])
                ),
            }
        }
    };
    until("NOR", None);
    assert!(editor.write(b":terminal cmd.exe /d /q\r".to_vec()));
    until("INS", None);
    assert!(editor.write(vec![0x1c]));
    until("NOR", Some("[review]"));
    assert!(editor.write(vec![0x1c]));
    until("[review]", None);
    assert!(editor.write(b"i".to_vec()));
    until("INS", Some("[review]"));
    assert!(editor.write(b"exit\r".to_vec()));
    until("NOR", None);
    assert!(editor.write(b":quit\r".to_vec()));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(PtyEvent::Output(bytes)) => {
                screen.feed(&bytes);
                output.extend(bytes);
            }
            Ok(PtyEvent::Exited(code)) => {
                assert_eq!(code, Some(0));
                break;
            }
            other => panic!(
                "editor exit: {other:?}; output tail: {}",
                String::from_utf8_lossy(&output[output.len().saturating_sub(1024)..])
            ),
        }
    }
}
