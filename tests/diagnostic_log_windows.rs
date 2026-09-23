// SPDX-License-Identifier: MPL-2.0

#![cfg(windows)]

use runyte::{
    terminal::emulator::Emulator,
    terminal::pty::{Pty, PtyEvent},
    test_support::TestRuntimeRoot,
};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

#[test]
fn native_logging_startup_and_readonly_page() {
    let root = TestRuntimeRoot::new("native-log-startup").unwrap();
    let output = fs::File::create(root.join("fixture.log")).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "logging_fixture", "--ignored", "--nocapture"])
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("RUNYTE_CONTEXT_HOME", root.join("context"))
        .env("RUNYTE_LOG_FIXTURE", root.path())
        .stdin(Stdio::null())
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "{}",
                fs::read_to_string(root.join("fixture.log")).unwrap()
            );
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "native logging fixture timed out: {}",
                fs::read_to_string(root.join("fixture.log")).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn until(events: &mpsc::Receiver<PtyEvent>, screen: &mut Emulator, marker: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut output = Vec::new();
    while !screen.plain_text().contains(marker) {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(PtyEvent::Output(bytes)) => {
                screen.feed(&bytes);
                output.extend(bytes);
            }
            other => panic!(
                "waiting for {marker:?}: {other:?}: {}",
                String::from_utf8_lossy(&output)
            ),
        }
        assert!(output.len() < 512 * 1024, "unbounded fixture output");
    }
}

fn run_editor(root: &Path, config: &Path, degraded: bool) {
    fs::create_dir(root).unwrap();
    fs::write(root.join("file.txt"), "NATIVE_LOG_EDITOR_READY\n").unwrap();
    if degraded {
        fs::write(root.join(".runyte"), "occupied").unwrap();
    }
    let (sender, events) = mpsc::channel();
    let terminal = Pty::spawn(
        Path::new(env!("CARGO_BIN_EXE_runyte")).as_os_str(),
        &[
            "--config".into(),
            config.to_str().unwrap().into(),
            "--project-root".into(),
            root.to_str().unwrap().into(),
            "-v".into(),
            "file.txt".into(),
        ],
        root,
        120,
        30,
        move |event| {
            let _ = sender.send(event);
        },
    )
    .unwrap();
    let mut screen = Emulator::new(120, 30);
    until(&events, &mut screen, "NATIVE_LOG_EDITOR_READY");
    if !degraded {
        assert!(terminal.write(b":log-open\r".to_vec()));
        until(&events, &mut screen, "[log]");
    }
    assert!(terminal.write(b":quit!\r".to_vec()));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(PtyEvent::Output(_)) => {}
            Ok(PtyEvent::Exited(code)) => {
                assert_eq!(code, Some(0));
                break;
            }
            other => panic!("editor failed to exit: {other:?}"),
        }
    }
    if !degraded {
        let logs: Vec<_> = fs::read_dir(root.join(".runyte"))
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("standalone-")
            })
            .collect();
        assert_eq!(logs.len(), 1);
        let text = fs::read_to_string(logs[0].path()).unwrap();
        assert!(text.contains("INFO"), "{text}");
        assert!(!text.contains("NATIVE_LOG_EDITOR_READY"));
    }
}

#[test]
#[ignore = "compiled child fixture with isolated configuration and process-global logger"]
fn logging_fixture() {
    let root = std::path::PathBuf::from(std::env::var_os("RUNYTE_LOG_FIXTURE").unwrap());
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME").unwrap(),
        root.join("config")
    );
    assert_eq!(
        std::env::var_os("RUNYTE_CONTEXT_HOME").unwrap(),
        root.join("context")
    );
    fs::create_dir(root.join("config")).unwrap();
    let config = root.join("config/config.yaml");
    fs::write(
        &config,
        "workspace:\n  mode: standalone\nlsp:\n  enable: false\n",
    )
    .unwrap();
    run_editor(&root.join("normal"), &config, false);
    run_editor(&root.join("degraded"), &config, true);
    let (sender, events) = mpsc::channel();
    let _terminal = Pty::spawn(
        Path::new(env!("CARGO_BIN_EXE_runyte")).as_os_str(),
        &[
            "--config".into(),
            config.to_str().unwrap().into(),
            "--project-root".into(),
            root.join("degraded").to_str().unwrap().into(),
            "--log".into(),
            root.join("degraded/.runyte/log").to_str().unwrap().into(),
        ],
        &root.join("degraded"),
        120,
        30,
        move |event| {
            let _ = sender.send(event);
        },
    )
    .unwrap();
    until(
        &events,
        &mut Emulator::new(120, 30),
        "cannot open the diagnostic log",
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(PtyEvent::Output(_)) => {}
            Ok(PtyEvent::Exited(code)) => {
                assert_ne!(code, Some(0));
                break;
            }
            other => panic!("explicit log failure did not exit: {other:?}"),
        }
    }
}
