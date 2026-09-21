// SPDX-License-Identifier: MPL-2.0
use super::*;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

const FIXTURE: &str = "terminal::pty::tests::native_console_fixture";

fn system_cmd() -> std::ffi::OsString {
    std::path::PathBuf::from(std::env::var_os("SystemRoot").expect("Windows system directory"))
        .join("System32/cmd.exe")
        .into_os_string()
}

/// The already-compiled test executable is the native subprocess fixture.
#[test]
#[ignore = "launched only inside the ConPTY tests"]
fn native_console_fixture() {
    use std::io::BufRead;
    for argument in
        std::env::args().filter_map(|argument| argument.strip_prefix("probe-").map(str::to_owned))
    {
        println!("ARGUMENT:{argument}:END");
    }
    println!("READY");
    io::stdout().flush().unwrap();
    let mut descendants = Vec::new();
    for line in io::stdin().lock().lines() {
        match line.unwrap().as_str() {
            "size" => {
                let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { zeroed() };
                assert_ne!(
                    unsafe {
                        GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info)
                    },
                    0
                );
                println!("SIZE {} {}", info.dwSize.X, info.dwSize.Y);
            }
            "descendant" => {
                let child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap();
                println!("DESCENDANT {}", child.id());
                descendants.push(child);
            }
            "spam" => loop {
                println!("{}", "x".repeat(4096));
            },
            "exit" => break,
            value => println!("RECEIVED {value}"),
        }
        io::stdout().flush().unwrap();
    }
    // Deliberately retain descendants on exit: the PTY's job owns cleanup.
    std::mem::forget(descendants);
}

fn fixture() -> (Pty, mpsc::Receiver<PtyEvent>) {
    let (sender, receiver) = mpsc::channel();
    let child = Pty::spawn(
        std::env::current_exe().unwrap().as_os_str(),
        &[
            "--exact".into(),
            FIXTURE.into(),
            "--ignored".into(),
            "--nocapture".into(),
        ],
        &std::env::temp_dir(),
        80,
        24,
        move |event| {
            let _ = sender.send(event);
        },
    )
    .unwrap();
    read_until(&receiver, "READY");
    (child, receiver)
}

fn read_until(events: &mpsc::Receiver<PtyEvent>, expected: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = Vec::new();
    while Instant::now() < deadline {
        match events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output)))
        {
            PtyEvent::Output(bytes) => {
                output.extend(bytes);
                if String::from_utf8_lossy(&output).contains(expected) {
                    return String::from_utf8(output).unwrap();
                }
            }
            PtyEvent::Exited(code) => panic!(
                "unexpected exit {code:?}: {}",
                String::from_utf8_lossy(&output)
            ),
        }
    }
    panic!("output did not contain {expected}");
}

#[test]
fn native_resize_reaches_console_and_close_kills_descendants() {
    let (child, events) = fixture();
    child.resize(117, 39).unwrap();
    // Resize is asynchronous. Query through native console APIs until applied.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(child.write(b"size\r".to_vec()));
        let output = read_until(&events, "SIZE ");
        if output.contains("SIZE 117 39") {
            break;
        }
        assert!(Instant::now() < deadline, "{output}");
    }
    assert!(child.write(b"descendant\r".to_vec()));
    let output = read_until(&events, "DESCENDANT ");
    let pid = output
        .split("DESCENDANT ")
        .nth(1)
        .unwrap()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse::<u32>()
        .unwrap();
    let descendant = owned(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) }).unwrap();
    drop(child);
    assert_eq!(
        unsafe { WaitForSingleObject(descendant.as_raw_handle(), 5000) },
        WAIT_OBJECT_0
    );
}

fn native_shell(arguments: &[&str]) -> (Pty, mpsc::Receiver<PtyEvent>) {
    let (sender, receiver) = mpsc::channel();
    let child = Pty::spawn(
        &system_cmd(),
        &arguments
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>(),
        &std::env::temp_dir(),
        80,
        24,
        move |event| {
            let _ = sender.send(event);
        },
    )
    .unwrap();
    (child, receiver)
}
#[test]
fn native_shell_drains_unicode_output_before_exit() {
    let (mut child, events) =
        native_shell(&["/d", "/c", "chcp 65001 >nul & echo café & exit /b 7"]);
    let mut output = Vec::new();
    loop {
        match events.recv_timeout(Duration::from_secs(10)).unwrap() {
            PtyEvent::Output(bytes) => output.extend(bytes),
            PtyEvent::Exited(code) => {
                assert_eq!(code, Some(7));
                break;
            }
        }
    }
    assert!(
        String::from_utf8_lossy(&output).contains("café"),
        "{}",
        String::from_utf8_lossy(&output)
    );
    assert_eq!(child.finished(), Some(Some(7)));
}

#[test]
fn cmd_shell_payload_preserves_nested_quotes_and_batch_path_spaces() {
    let batch =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/windows shell arguments.cmd");
    for (payload, expected) in [
        (String::from("echo \"a b\""), String::from("\"a b\"")),
        (
            format!("\"{}\" \"argument with spaces\"", batch.display()),
            String::from("BATCH-ARG:argument with spaces"),
        ),
    ] {
        let (_child, events) = native_shell(&["/d", "/c", &payload]);
        let mut bytes = Vec::new();
        while let PtyEvent::Output(chunk) = events.recv_timeout(Duration::from_secs(10)).unwrap() {
            bytes.extend(chunk);
        }
        let output = String::from_utf8_lossy(&bytes);
        assert!(output.contains(&expected), "{output}");
        assert!(!output.contains("\\\""), "{output}");
    }
}
#[test]
fn independent_shells_accept_input_and_resize_without_cross_talk() {
    let (first, first_events) = native_shell(&["/d", "/q"]);
    let (second, second_events) = native_shell(&["/d", "/q"]);
    assert_ne!(first.process_id(), second.process_id());
    first.resize(120, 40).unwrap();
    assert!(first.write(b"echo FIRST-SHELL\r\nexit\r\n".to_vec()));
    assert!(second.write(b"echo SECOND-SHELL\r\nexit\r\n".to_vec()));
    for (events, own, other) in [
        (first_events, "FIRST-SHELL", "SECOND-SHELL"),
        (second_events, "SECOND-SHELL", "FIRST-SHELL"),
    ] {
        let mut output = Vec::new();
        while let PtyEvent::Output(bytes) = events.recv_timeout(Duration::from_secs(10)).unwrap() {
            output.extend(bytes);
        }
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains(own), "{text}");
        assert!(!text.contains(other));
    }
}
#[test]
fn queued_input_and_close_remain_bounded() {
    let (child, events) = native_shell(&["/d", "/c", "pause >nul"]);
    assert!(!child.write(vec![b'x'; MAX_INPUT_BYTES + 1]));
    let start = Instant::now();
    for _ in 0..32 {
        child.write(vec![b'x'; MAX_INPUT_BYTES]);
    }
    drop(child);
    assert!(start.elapsed() < Duration::from_secs(2));
    while !matches!(
        events.recv_timeout(Duration::from_secs(10)).unwrap(),
        PtyEvent::Exited(_)
    ) {}
}
#[test]
fn failed_setup_owns_and_terminates_suspended_child() {
    for stage in [
        SpawnCheckpoint::ChildOwned,
        SpawnCheckpoint::ReaderStarted,
        SpawnCheckpoint::WriterStarted,
        SpawnCheckpoint::LifecycleStarted,
    ] {
        let process = Mutex::new(None);
        let result = Pty::spawn_checked(
            &system_cmd(),
            &["/d".into()],
            &std::env::temp_dir(),
            80,
            24,
            |_| {},
            |current, pid| {
                if current != stage {
                    return Ok(());
                }
                *process.lock().unwrap() = Some(
                    owned(unsafe {
                        OpenProcess(
                            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                            0,
                            pid,
                        )
                    })
                    .unwrap(),
                );
                Err(io::Error::other("injected setup failure"))
            },
        );
        assert!(result.is_err());
        let process = process.into_inner().unwrap().unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(process.as_raw_handle(), 5000) },
            WAIT_OBJECT_0
        );
    }
}

#[test]
fn failed_process_creation_closes_undrained_console() {
    let root = crate::test_support::TestRuntimeRoot::new("failed-console").unwrap();
    // An installed DLL is an existing native image that CreateProcess rejects;
    // no executable or script is written by the fixture.
    let library = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/kernel32.dll");
    let start = Instant::now();
    assert!(Pty::spawn(library.as_os_str(), &[], root.path(), 80, 24, |_| {}).is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn native_child_receives_arguments_from_requested_directory() {
    let executable = std::env::current_exe().unwrap();
    // CI can put TEMP and the build on different drives. Hard links require
    // one volume; keep this owned temporary fixture beside the compiled image.
    let root = crate::test_support::TestRuntimeRoot::new_in(
        "native-arguments",
        executable.parent().unwrap(),
    )
    .unwrap();
    std::fs::hard_link(&executable, root.join("native helper.exe")).unwrap();
    let values = [
        r"C:\folder with spaces\",
        "embedded \"quotes\"",
        "café 😀",
        "",
    ];
    let mut arguments = vec![
        "--exact".into(),
        FIXTURE.into(),
        "--ignored".into(),
        "--nocapture".into(),
    ];
    for value in values {
        arguments.extend(["--skip".into(), format!("probe-{value}")]);
    }
    let (sender, events) = mpsc::channel();
    let child = Pty::spawn(
        OsStr::new(r".\native helper.exe"),
        &arguments,
        root.path(),
        120,
        24,
        move |event| {
            let _ = sender.send(event);
        },
    )
    .unwrap();
    let cleanup = child.cleanup_waiter();
    let output = read_until(&events, "READY");
    for value in values {
        assert!(
            output.contains(&format!("ARGUMENT:{value}:END")),
            "{output}"
        );
    }
    child.write(b"exit\r".to_vec());
    while !matches!(
        events.recv_timeout(Duration::from_secs(10)).unwrap(),
        PtyEvent::Exited(_)
    ) {}
    drop(child);
    cleanup();
}

#[test]
fn closing_a_saturated_terminal_wakes_all_workers() {
    let (child, events) = fixture();
    let control = child.control.clone();
    child.write(b"spam\r".to_vec());
    read_until(&events, "xxxxxxxx");
    drop(child);
    assert_eq!(
        unsafe { WaitForSingleObject(control.completed.as_raw_handle(), 5000) },
        WAIT_OBJECT_0
    );
    assert!(control.input.lock().unwrap().is_empty());
}

#[test]
fn manager_close_wakes_a_reader_blocked_on_its_output_queue() {
    use crate::terminal::{TerminalRequest, TerminalSessions};
    let mut terminals = TerminalSessions::new();
    let id = terminals
        .open(
            TerminalRequest {
                program: std::env::current_exe().unwrap().into_os_string(),
                arguments: vec![
                    "--exact".into(),
                    FIXTURE.into(),
                    "--ignored".into(),
                    "--nocapture".into(),
                ],
                directory: std::env::temp_dir(),
                label: "fixture".into(),
            },
            80,
            24,
        )
        .unwrap();
    let events = terminals.take_events().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = String::new();
    while !output.contains("READY") {
        if let Ok(crate::terminal::TerminalOutput::Bytes { bytes, .. }) = events.try_recv() {
            output.push_str(&String::from_utf8_lossy(&bytes));
        } else {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(Instant::now() < deadline);
    }
    let pty = terminals.get(id).unwrap().pty.as_ref().unwrap();
    let control = pty.control.clone();
    assert!(pty.write(b"spam\r".to_vec()));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let full = terminals.events.0.state.lock().unwrap().sessions[&id]
            .bytes
            .len()
            >= crate::terminal::PER_SESSION_OUTPUT_QUEUE;
        if full {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let started = Instant::now();
    terminals.close_all();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        unsafe { WaitForSingleObject(control.completed.as_raw_handle(), 5000) },
        WAIT_OBJECT_0
    );
}
