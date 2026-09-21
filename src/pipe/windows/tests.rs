// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::{
    fs,
    io::{Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::Stdio,
    sync::mpsc,
};
use windows_sys::Win32::{
    Foundation::WAIT_OBJECT_0,
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
};

fn configured(text: &str, root: &Path) -> Command {
    let mut command = command(text, root).unwrap();
    command
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"));
    command
}

fn execute(text: &str, input: Vec<String>) -> Result<Vec<String>> {
    let root = crate::test_support::TestRuntimeRoot::new("native-pipe").unwrap();
    run_command(
        &configured(text, root.path()),
        input,
        &AtomicBool::new(false),
        Duration::from_secs(15),
    )
}

fn native(root: &Path, mode: &str) -> Command {
    let exe = std::env::current_exe().unwrap();
    let text = format!(
        "& '{}' --exact pipe::windows::tests::native_fixture --ignored --nocapture",
        exe.to_str().unwrap().replace('\'', "''")
    );
    let mut command = configured(&text, root);
    command
        .env("RUNYTE_PIPE_FIXTURE", mode)
        .env("RUNYTE_PIPE_FIXTURE_ROOT", root);
    command
}

fn fixture_command(root: &Path, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "pipe::windows::tests::native_fixture",
            "--ignored",
            "--nocapture",
        ])
        .current_dir(root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("RUNYTE_PIPE_FIXTURE", mode)
        .env("RUNYTE_PIPE_FIXTURE_ROOT", root);
    command
}

#[test]
fn powershell_preserves_text_and_keeps_selection_out_of_code() {
    for input in [
        "",
        "é😀\r\nnext\n\n",
        "no final newline",
        "$(throw 'selection executed'); \"quoted\" & | % ! ^ `",
    ] {
        assert_eq!(
            execute(
                "[Console]::Out.Write([Console]::In.ReadToEnd())",
                vec![input.into()]
            )
            .unwrap(),
            [input]
        );
    }
    assert_eq!(
        execute(
            "[Console]::Out.Write('é😀 ''quoted''; & | % ! ^ `')",
            vec![String::new()]
        )
        .unwrap(),
        ["é😀 'quoted'; & | % ! ^ `"]
    );
    assert_eq!(
        execute(
            "[Console]::In.ReadToEnd() | Out-Null",
            vec!["ignored".into()]
        )
        .unwrap(),
        [""]
    );
    assert_eq!(
        execute("[Console]::Out.Write(\"x`n`n\")", vec![String::new()]).unwrap(),
        ["x\n\n"]
    );
    let command = format!("[Console]::Out.Write('long'); #{}", "é".repeat(8100));
    assert!(command.len() <= 16 * 1024);
    assert_eq!(execute(&command, vec![String::new()]).unwrap(), ["long"]);
    let prefix = "[Console]::Out.Write('full ASCII command'); #";
    let text = format!("{prefix}{}", "x".repeat(16 * 1024 - prefix.len()));
    assert_eq!(text.len(), 16 * 1024);
    assert_eq!(
        execute(&text, vec![String::new()]).unwrap(),
        ["full ASCII command"]
    );
    let input = "é😀\r\n".repeat(128 * 1024);
    assert_eq!(
        execute(
            "[Console]::Out.Write([Console]::In.ReadToEnd())",
            vec![input.clone()]
        )
        .unwrap(),
        [input]
    );
}

#[test]
fn direct_native_stdin_stderr_exit_and_environment_contract() {
    let root = crate::test_support::TestRuntimeRoot::new("native-pipe-child").unwrap();
    let input = "é😀\r\nline\nno trailing newline";
    let output = run_command(
        &native(root.path(), "echo"),
        vec![input.into()],
        &AtomicBool::new(false),
        Duration::from_secs(15),
    )
    .unwrap();
    assert!(
        output[0].contains(&format!("PAYLOAD:{}", STANDARD.encode(input))),
        "{:?}",
        output
    );
    let error = run_command(
        &native(root.path(), "exit"),
        vec![String::new()],
        &AtomicBool::new(false),
        Duration::from_secs(15),
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("7") && error.contains("native diagnostic"),
        "{error}"
    );
}

#[test]
fn failures_bounds_and_whole_job_budget() {
    for (text, expected) in [
        ("throw 'terminating error'", "terminating error"),
        ("Write-Error 'cmdlet error'", "cmdlet error"),
        ("runyte_deliberately_missing_filter", "not recognized"),
        ("[Console]::Error.Write('diagnosis'); exit 7", "diagnosis"),
        (
            "[Console]::Error.Write('x' * 40000); exit 1",
            "stderr truncated",
        ),
        (
            "while ($true) { [Console]::Out.Write('x' * 8192) }",
            "8 MiB",
        ),
    ] {
        let error = execute(text, vec![String::new()]).unwrap_err().to_string();
        assert!(error.contains(expected), "{text}: {error}");
        assert!(error.len() < STDERR_BYTES + 1024);
    }
    let error = execute(
        "[Console]::Out.Write('x' * 5000000)",
        vec![String::new(), String::new()],
    )
    .unwrap_err();
    assert!(error.to_string().contains("8 MiB"));
    assert_eq!(
        execute(
            "[Console]::Out.Write([Console]::In.ReadToEnd().ToUpperInvariant())",
            vec!["a".into(), "b".into()]
        )
        .unwrap(),
        ["A", "B"]
    );
    assert!(
        command(&"x".repeat(16385), Path::new(r"C:\"))
            .unwrap_err()
            .to_string()
            .contains("16 KiB")
    );
    assert!(
        command("nul\0command", Path::new(r"C:\"))
            .unwrap_err()
            .to_string()
            .contains("NUL")
    );
}

fn live_fixture(root: &Path) -> OwnedHandle {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(pid) = fs::read_to_string(root.join("ready.pid"))
            && let Ok(pid) = pid.parse::<u32>()
        {
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            assert!(!handle.is_null(), "{}", io::Error::last_os_error());
            return unsafe { OwnedHandle::from_raw_handle(handle) };
        }
        assert!(Instant::now() < deadline, "native fixture did not start");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn invalid_native_stdout_is_refused_without_lossy_replacement() {
    let root = crate::test_support::TestRuntimeRoot::new("native-pipe-utf8").unwrap();
    let error = run_command(
        &fixture_command(root.path(), "invalid"),
        vec![String::new()],
        &AtomicBool::new(false),
        Duration::from_secs(15),
    )
    .unwrap_err();
    assert!(error.to_string().contains("stdout is not valid UTF-8"));
}

#[test]
fn continuous_stderr_remains_bounded_and_cancellable() {
    let root = crate::test_support::TestRuntimeRoot::new("native-pipe-flood").unwrap();
    let command = fixture_command(root.path(), "flood");
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = run_command(
            &command,
            vec![String::new()],
            &worker_cancel,
            Duration::from_secs(20),
        );
        let _ = sender.send(result);
    });
    // The fixture publishes only after writing more than the stderr pipe
    // quota. Readiness therefore proves the worker has drained stderr.
    let process = live_fixture(root.path());
    cancel.store(true, Ordering::Release);
    let error = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("continuous stderr prevented cancellation")
        .unwrap_err()
        .to_string();
    worker.join().unwrap();
    assert!(error.contains("cancelled"), "{error}");
    assert!(error.contains("stderr truncated"), "{error}");
    assert!(error.len() < STDERR_BYTES + 1024);
    assert_eq!(
        unsafe { WaitForSingleObject(process.as_raw_handle(), 5000) },
        WAIT_OBJECT_0
    );
}

#[test]
fn successful_leader_exit_stops_descendants_holding_both_output_pipes() {
    let root = crate::test_support::TestRuntimeRoot::new("native-pipe-tree").unwrap();
    let command = fixture_command(root.path(), "leader");
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = run_command(
            &command,
            vec![String::new()],
            &AtomicBool::new(false),
            Duration::from_secs(20),
        );
        let _ = sender.send(result);
    });
    // Capture the descendant's process handle before allowing the leader to
    // exit, avoiding PID reuse or a race with process object destruction.
    let descendant = live_fixture(root.path());
    fs::write(root.join("release"), "exit").unwrap();
    let output = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("inherited output pipes retained a completed filter")
        .unwrap();
    worker.join().unwrap();
    assert!(output[0].contains("DESCENDANT_STDOUT"), "{output:?}");
    assert_eq!(
        unsafe { WaitForSingleObject(descendant.as_raw_handle(), 5000) },
        WAIT_OBJECT_0,
        "leader completion left its descendant running"
    );
}

#[test]
fn cancellation_releases_blocked_input_and_owns_the_child() {
    let root = crate::test_support::TestRuntimeRoot::new("native-pipe-cancel").unwrap();
    let command = fixture_command(root.path(), "blocked");
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = run_command(
            &command,
            vec!["x".repeat(256 * 1024)],
            &worker_cancel,
            Duration::from_secs(20),
        );
        sender.send(result).unwrap();
    });
    let process = live_fixture(root.path());
    cancel.store(true, Ordering::Release);
    let result = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("blocked input prevented cancellation");
    worker.join().unwrap();
    assert!(result.unwrap_err().to_string().contains("cancelled"));
    assert_eq!(
        unsafe { WaitForSingleObject(process.as_raw_handle(), 5000) },
        WAIT_OBJECT_0
    );
}

#[test]
fn whole_job_deadline_stops_the_process() {
    let root = crate::test_support::TestRuntimeRoot::new("native-pipe-timeout").unwrap();
    let start = Instant::now();
    let error = run_command(
        &fixture_command(root.path(), "blocked"),
        vec!["x".repeat(256 * 1024)],
        &AtomicBool::new(false),
        Duration::from_secs(2),
    )
    .unwrap_err();
    assert!(error.to_string().contains("timed out"), "{error}");
    assert!(start.elapsed() < Duration::from_secs(5));
    if let Ok(pid) = fs::read_to_string(root.join("ready.pid")) {
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid.parse().unwrap()) };
        if !handle.is_null() {
            let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
            assert_eq!(
                unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
                WAIT_OBJECT_0
            );
        }
    }
}

#[test]
#[ignore = "compiled native child for pipe ownership and byte-stream acceptance"]
fn native_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_PIPE_FIXTURE_ROOT").unwrap());
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME").unwrap(),
        root.join("config")
    );
    assert_eq!(
        std::env::var_os("XDG_CACHE_HOME").unwrap(),
        root.join("cache")
    );
    assert!(
        std::env::var_os(COMMAND_ENV).is_none(),
        "bootstrap command leaked to native child"
    );
    match std::env::var("RUNYTE_PIPE_FIXTURE").unwrap().as_str() {
        "echo" => {
            let mut input = Vec::new();
            std::io::stdin().read_to_end(&mut input).unwrap();
            println!("PAYLOAD:{}", STANDARD.encode(input));
            eprintln!("native diagnostic");
        }
        "exit" => {
            eprintln!("native diagnostic");
            std::process::exit(7);
        }
        "blocked" => {
            use windows_sys::Win32::System::{
                Console::{GetStdHandle, STD_INPUT_HANDLE},
                Pipes::PeekNamedPipe,
            };
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let mut available = 0;
                assert_ne!(
                    unsafe {
                        PeekNamedPipe(
                            GetStdHandle(STD_INPUT_HANDLE),
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                            &mut available,
                            std::ptr::null_mut(),
                        )
                    },
                    0
                );
                // Match the production pipe quota. The child consumes no
                // bytes, and the parent's 256 KiB input exceeds that quota.
                if available >= 64 * 1024 {
                    break;
                }
                assert!(Instant::now() < deadline, "stdin never filled");
                std::thread::sleep(Duration::from_millis(5));
            }
            fs::write(root.join("ready.pid"), std::process::id().to_string()).unwrap();
            std::io::stdout().flush().unwrap();
            std::thread::sleep(Duration::from_secs(30));
        }
        "leader" => {
            let mut child = fixture_command(&root, "hold")
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !root.join("release").exists() {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("parent did not authorize leader exit");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            // The descendant deliberately outlives the successful leader;
            // only the native job owner should terminate it.
            std::process::exit(0);
        }
        "hold" => {
            std::io::stdout().write_all(b"DESCENDANT_STDOUT\n").unwrap();
            std::io::stdout().flush().unwrap();
            std::io::stderr().write_all(b"DESCENDANT_STDERR\n").unwrap();
            std::io::stderr().flush().unwrap();
            fs::write(root.join("ready.pid"), std::process::id().to_string()).unwrap();
            std::thread::sleep(Duration::from_secs(30));
        }
        "invalid" => {
            std::io::stdout().write_all(&[255]).unwrap();
        }
        "flood" => {
            for _ in 0..32 {
                std::io::stderr().write_all(&[b'x'; 8192]).unwrap();
            }
            fs::write(root.join("ready.pid"), std::process::id().to_string()).unwrap();
            loop {
                std::io::stderr().write_all(&[b'x'; 8192]).unwrap();
            }
        }
        other => panic!("unknown fixture {other}"),
    }
}
