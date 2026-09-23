// SPDX-License-Identifier: MPL-2.0
//! Execute the current binary's private guard test in an isolated console.
use super::{
    ConsoleEvent, TerminationSignals, console_closed, finish_standalone_native,
    prefer_pending_console_close, reconcile_pending_console_event, terminated,
};
use runyte::{
    terminal::pty::{Pty, PtyEvent},
    test_support::TestRuntimeRoot,
};
use std::{
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::{
    Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler},
    Threading::{CREATE_NEW_CONSOLE, WaitForSingleObject},
};

const NATIVE_EVENT_HELPER: &str = "windows_console_acceptance::native_console_event_helper";
const NATIVE_EVENT_MODE: &str = "RUNYTE_NATIVE_CONSOLE_EVENT_MODE";
const NATIVE_EVENT_ROOT: &str = "RUNYTE_NATIVE_CONSOLE_EVENT_ROOT";

struct IsolatedConsoleChild {
    child: std::process::Child,
    root: Option<TestRuntimeRoot>,
}

impl Drop for IsolatedConsoleChild {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            unsafe {
                WaitForSingleObject(self.child.as_raw_handle(), 5000);
            }
            if self.child.try_wait().ok().flatten().is_none() {
                // The child may still use this storage if its exit could not
                // be proved. Keep it until the operating system reclaims it.
                std::mem::forget(self.root.take());
            }
        }
    }
}

fn run_isolated_console_event(mode: &str) {
    let root = TestRuntimeRoot::new("native-console-events").unwrap();
    let log = std::fs::File::create(root.join("event.log")).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", NATIVE_EVENT_HELPER, "--ignored", "--nocapture"])
        .env(NATIVE_EVENT_MODE, mode)
        .env(NATIVE_EVENT_ROOT, root.path())
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("RUNYTE_CONTEXT_HOME", root.join("context"))
        .creation_flags(CREATE_NEW_CONSOLE)
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let mut fixture = IsolatedConsoleChild {
        child,
        root: Some(root),
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = fixture.child.try_wait().unwrap() {
            assert!(
                status.success(),
                "isolated console event fixture failed: {}",
                std::fs::read_to_string(fixture.root.as_ref().unwrap().join("event.log")).unwrap()
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "isolated console event fixture timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(fixture.root.as_ref().unwrap().join("ready")).unwrap(),
        mode
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.as_ref().unwrap().join("received")).unwrap(),
        mode
    );
}

#[test]
fn native_ctrl_c_and_ctrl_break_keep_distinct_types_in_isolated_consoles() {
    run_isolated_console_event("ctrl-c");
    run_isolated_console_event("ctrl-break");
}

#[test]
#[ignore = "reexecuted in its own console by the parent test"]
fn native_console_event_helper() {
    let Some(mode) = std::env::var_os(NATIVE_EVENT_MODE) else {
        return;
    };
    let root = std::path::PathBuf::from(std::env::var_os(NATIVE_EVENT_ROOT).unwrap());
    let mode = mode.to_str().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut events = TerminationSignals::new().unwrap();
        let (control, expected) = match mode {
            "ctrl-c" => {
                // An inherited ignore-Ctrl+C attribute would hide a real
                // console event even though Tokio's handler is installed.
                assert_ne!(unsafe { SetConsoleCtrlHandler(None, 0) }, 0);
                (CTRL_C_EVENT, ConsoleEvent::CtrlC)
            }
            "ctrl-break" => (CTRL_BREAK_EVENT, ConsoleEvent::CtrlBreak),
            _ => panic!("unknown console event fixture mode"),
        };
        std::fs::write(root.join("ready"), mode).unwrap();
        // The helper alone owns this new console. Group zero cannot signal
        // the shared test runner's console.
        assert_ne!(unsafe { GenerateConsoleCtrlEvent(control, 0) }, 0);
        let received = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap();
        assert_eq!(received, expected);
        std::fs::write(root.join("received"), mode).unwrap();
    });
}

#[test]
fn console_close_has_separate_error_and_no_dead_console_reporting() {
    let close = terminated(ConsoleEvent::Close);
    assert!(console_closed(&close));
    assert_eq!(close.to_string(), "console closed");
    for event in [ConsoleEvent::CtrlC, ConsoleEvent::CtrlBreak] {
        assert!(!console_closed(&terminated(event)));
    }
}

#[test]
fn pending_close_overrides_startup_or_input_failure_after_joined_cleanup() {
    let startup = prefer_pending_console_close(
        Err(anyhow::anyhow!("startup failed")),
        Some(ConsoleEvent::Close),
    )
    .unwrap_err();
    assert!(console_closed(&startup));
    assert!(format!("{startup:#}").contains("startup failed"));

    let input = finish_standalone_native(
        Err(anyhow::anyhow!("input failed")),
        Err(anyhow::anyhow!("context joined with error")),
        Err(anyhow::anyhow!("catalog joined with error")),
        Err(anyhow::anyhow!("plugins joined with error")),
        None,
    )
    .unwrap_err();
    let close = prefer_pending_console_close(Err(input), Some(ConsoleEvent::Close)).unwrap_err();
    assert!(console_closed(&close));
    let detail = format!("{close:#}");
    assert!(detail.contains("input failed"));
    assert!(detail.contains("context joined with error"));
    assert!(detail.contains("catalog joined with error"));
    assert!(detail.contains("plugins joined with error"));

    let ctrl_c =
        finish_standalone_native(Ok(()), Ok(()), Ok(()), Ok(()), Some(ConsoleEvent::CtrlC))
            .unwrap_err();
    let later_close =
        prefer_pending_console_close(Err(ctrl_c), Some(ConsoleEvent::Close)).unwrap_err();
    assert!(console_closed(&later_close));
    assert_eq!(
        reconcile_pending_console_event(Ok(()), Some(ConsoleEvent::CtrlBreak))
            .unwrap_err()
            .to_string(),
        "terminated by Ctrl+Break"
    );
}

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
        .env("RUNYTE_CONTEXT_HOME", root.join("context"))
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
    assert_eq!(
        std::env::var_os("RUNYTE_CONTEXT_HOME").unwrap(),
        root.join("context")
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
        .env("RUNYTE_CONTEXT_HOME", root.join("context"))
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
    assert_eq!(
        std::env::var_os("RUNYTE_CONTEXT_HOME").unwrap(),
        root.join("context")
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
