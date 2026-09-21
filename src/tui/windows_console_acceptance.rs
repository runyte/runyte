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

#[test]
fn console_control_key_transport() {
    let root = TestRuntimeRoot::new("console-key-transport").unwrap();
    let log = std::fs::File::create(root.join("output.log")).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "windows_console_acceptance::transport_parent",
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
            let _ = child.kill();
            let _ = child.wait();
            panic!("transport fixture timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "isolated configuration and native console transport fixture"]
fn transport_parent() {
    let root = std::path::PathBuf::from(std::env::var_os("RUNYTE_CONSOLE_TEST_ROOT").unwrap());
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME").unwrap(),
        root.join("config")
    );
    let (sender, events) = mpsc::channel();
    let child = Pty::spawn(
        std::env::current_exe().unwrap().as_os_str(),
        &[
            "--exact".into(),
            "windows_console_acceptance::transport_capture".into(),
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
    let mut sent = 0;
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(PtyEvent::Output(bytes)) => {
                output.extend(bytes);
                if String::from_utf8_lossy(&output).contains(&format!("READY{sent}")) && sent < 3 {
                    assert!(child.write(b"\x1b[72;35;8;1;8;1_\x1b[74;36;10;1;8;1_\x1b[8;14;8;1;0;1_\x1b[13;28;13;1;0;1_\x1b[200~hello\r\n\x08\x0a\x1b[201~\x1b[123;88;0;1;0;1_".to_vec()));
                    sent += 1;
                }
            }
            Ok(PtyEvent::Exited(code)) => {
                assert_eq!(code, Some(0), "{}", String::from_utf8_lossy(&output));
                assert_eq!(sent, 3);
                break;
            }
            other => panic!("{other:?}: {}", String::from_utf8_lossy(&output)),
        }
    }
}

#[test]
#[ignore = "native ConPTY input and guard restoration fixture"]
fn transport_capture() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use std::io::Write;
    use windows_sys::Win32::System::Console::*;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    {
        let _guard = super::TerminalGuard::enter(true).unwrap();
        let mut events = runyte::tui::windows_input::EventStream::new().unwrap();
        println!("READY0");
        std::io::stdout().flush().unwrap();
        runtime.block_on(async {
            for expected in [
                Event::Key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL)),
                Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)),
                Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
                Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                // ConPTY normalizes a raw BS byte to its Backspace/DEL unit.
                Event::Paste("hello\r\n\x7f\n".into()),
                Event::Key(KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE)),
            ] {
                loop {
                    let actual = tokio::time::timeout(Duration::from_secs(3), events.next())
                        .await
                        .unwrap()
                        .unwrap()
                        .unwrap();
                    if matches!(
                        actual,
                        Event::Resize(..) | Event::FocusGained | Event::FocusLost
                    ) {
                        continue;
                    }
                    assert_eq!(actual, expected);
                    break;
                }
            }
        });
    }
    // After normal exit and panic, a fresh VT-only reader must receive legacy
    // bytes again. Console mode restoration alone cannot verify DEC mode 9001.
    for stage in 1..3 {
        if stage == 2 {
            assert!(
                std::panic::catch_unwind(|| {
                    let _guard = super::TerminalGuard::enter(false).unwrap();
                    panic!("exercise guard unwind");
                })
                .is_err()
            );
        }
        let mode = runyte::tui::windows_input::ConsoleMode::capture().unwrap();
        crossterm::terminal::enable_raw_mode().unwrap();
        mode.enable_vt().unwrap();
        println!("READY{stage}");
        std::io::stdout().flush().unwrap();
        let mut units = Vec::new();
        loop {
            let mut record: INPUT_RECORD = unsafe { std::mem::zeroed() };
            let mut count = 0;
            assert_ne!(
                unsafe {
                    ReadConsoleInputW(GetStdHandle(STD_INPUT_HANDLE), &mut record, 1, &mut count)
                },
                0
            );
            if record.EventType as u32 == KEY_EVENT {
                let key = unsafe { record.Event.KeyEvent };
                if key.bKeyDown == 0 {
                    continue;
                }
                units.push(unsafe { key.uChar.UnicodeChar });
                if units.ends_with(&[27, 91, 50, 52, 126]) {
                    break;
                }
            }
        }
        assert!(units.starts_with(&[8, 10, 127, 13]), "{units:?}");
        drop(mode);
    }
}
