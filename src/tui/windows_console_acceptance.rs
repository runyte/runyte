// SPDX-License-Identifier: MPL-2.0
//! Execute the current binary's private guard test in an isolated console.
use runyte::{
    terminal::pty::{Pty, PtyEvent},
    test_support::TestRuntimeRoot,
};
use std::{
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

#[test]
fn console_guard_runs_in_conpty() {
    let root = TestRuntimeRoot::new("console-guard").unwrap();
    let log = std::fs::File::create(root.join("output.log")).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "windows_console_acceptance::console_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("RUNYTE_CONSOLE_TEST_ROOT", root.path())
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "{}",
                std::fs::read_to_string(root.join("output.log")).unwrap()
            );
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("console fixture timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "reexecuted with fixture-owned configuration by the parent test"]
fn console_fixture() {
    let root = std::path::PathBuf::from(std::env::var_os("RUNYTE_CONSOLE_TEST_ROOT").unwrap());
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME").unwrap(),
        root.join("config")
    );
    let (sender, events) = mpsc::channel();
    let _child = Pty::spawn(
        std::env::current_exe().unwrap().as_os_str(),
        &[
            "--exact".into(),
            "tests::windows_console_paste_and_restoration".into(),
            "--ignored".into(),
            "--nocapture".into(),
        ],
        &root,
        120,
        30,
        move |event| {
            let _ = sender.send(event);
        },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut output = Vec::new();
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(PtyEvent::Output(bytes)) => output.extend(bytes),
            Ok(PtyEvent::Exited(code)) => {
                assert_eq!(code, Some(0), "{}", String::from_utf8_lossy(&output));
                assert!(String::from_utf8_lossy(&output).contains("1 passed"));
                break;
            }
            other => panic!("{other:?}: {}", String::from_utf8_lossy(&output)),
        }
    }
}
