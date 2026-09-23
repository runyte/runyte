// SPDX-License-Identifier: MPL-2.0
// A harness-free executable provides real framed stdio without libtest banners.
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    native::main();
}

#[cfg(windows)]
mod native {
    use runyte::{
        lsp::transport::{self, Connection, Incoming},
        test_support::TestRuntimeRoot,
    };
    use serde_json::{Value, json};
    use std::{
        fs,
        io::{BufRead, Write},
        os::windows::{
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
            process::CommandExt,
        },
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };
    use tokio::{runtime::Runtime, sync::mpsc};
    use windows_sys::Win32::{
        Foundation::WAIT_OBJECT_0,
        System::Threading::{
            CREATE_NO_WINDOW, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
        },
    };

    const CASES: &[&str] = &[
        "framing_arguments_directory_and_stderr",
        "blocked_stdin_stop_and_restart",
        "full_inbox_and_connection_drop",
        "leader_exit_ends_descendants",
        "runtime_shutdown_owns_the_connection",
        "unpolled_runtime_shutdown_owns_the_connection",
    ];

    pub fn main() {
        let arguments: Vec<_> = std::env::args().collect();
        match arguments.get(1).map(String::as_str) {
            Some("--fixture") => fixture(&arguments[2], Path::new(&arguments[3]), &arguments[4..]),
            Some("--case") => run_case(&arguments[2], Path::new(&arguments[3])),
            _ => {
                for case in CASES {
                    let root = TestRuntimeRoot::new("native-lsp-acceptance").unwrap();
                    let output = fs::File::create(root.join("case-output")).unwrap();
                    let mut process = Process(
                        Command::new(std::env::current_exe().unwrap())
                            .args(["--case", case])
                            .arg(root.path())
                            .env("XDG_CONFIG_HOME", root.join("config"))
                            .creation_flags(CREATE_NO_WINDOW)
                            .stdin(Stdio::null())
                            .stdout(output.try_clone().unwrap())
                            .stderr(output)
                            .spawn()
                            .unwrap(),
                    );
                    let deadline = Instant::now() + Duration::from_secs(30);
                    let status = loop {
                        if let Some(status) = process.0.try_wait().unwrap() {
                            break status;
                        }
                        assert!(
                            Instant::now() < deadline,
                            "{case} timed out: {}",
                            fs::read_to_string(root.join("case-output")).unwrap()
                        );
                        std::thread::sleep(Duration::from_millis(10));
                    };
                    assert!(
                        status.success(),
                        "{case}: {}",
                        fs::read_to_string(root.join("case-output")).unwrap()
                    );
                    println!("test {case} ... ok");
                }
                println!("native LSP transport: {} passed", CASES.len());
            }
        }
    }

    struct Process(Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            unsafe { WaitForSingleObject(self.0.as_raw_handle(), 5000) };
        }
    }

    fn process(pid: u32) -> OwnedHandle {
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(
            !handle.is_null(),
            "open fixture process: {}",
            std::io::Error::last_os_error()
        );
        unsafe { OwnedHandle::from_raw_handle(handle) }
    }

    fn ended(process: &OwnedHandle) {
        assert_eq!(
            unsafe { WaitForSingleObject(process.as_raw_handle(), 5000) },
            WAIT_OBJECT_0,
            "fixture remained alive"
        );
    }

    fn frame(value: &Value) {
        let body = serde_json::to_vec(value).unwrap();
        let mut stdout = std::io::stdout().lock();
        write!(stdout, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
        stdout.write_all(&body).unwrap();
        stdout.flush().unwrap();
    }

    fn receive(reader: &mut impl BufRead) -> Option<Value> {
        let mut length = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                return None;
            }
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length: ") {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
        }
        let mut body = vec![0; length.unwrap()];
        reader.read_exact(&mut body).unwrap();
        Some(serde_json::from_slice(&body).unwrap())
    }

    fn fixture(mode: &str, root: &Path, arguments: &[String]) {
        if mode == "unpolled" {
            fs::write(root.join("unpolled.pid"), std::process::id().to_string()).unwrap();
        }
        let mut descendant = None;
        if mode == "tree" {
            // Intentionally orphaned when the leader exits; the native job
            // must terminate it and its inherited output handles.
            #[allow(clippy::zombie_processes)]
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["--fixture", "silent-hold"])
                .arg(root)
                .env("XDG_CONFIG_HOME", root.join("descendant-config"))
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            descendant = Some(child.id());
        }
        if mode == "echo" {
            std::io::stderr().write_all(&vec![b'x'; 32 * 1024]).unwrap();
            std::io::stderr().write_all(b"TAIL_DONE").unwrap();
            std::io::stdout()
                .write_all(b"Content-Length: 3\r\n\r\nnot")
                .unwrap();
        }
        if mode != "silent-hold" {
            frame(
                &json!({"ready": std::process::id(), "descendant": descendant,
            "args": arguments, "cwd": std::env::current_dir().unwrap(),
            "config": PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap())}),
            );
        }
        match mode {
            "blocked" => {
                use windows_sys::Win32::System::{
                    Console::{GetStdHandle, STD_INPUT_HANDLE},
                    Pipes::PeekNamedPipe,
                };
                let deadline = Instant::now() + Duration::from_secs(5);
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
                    if available >= 32 * 1024 {
                        break;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "writer never filled the native pipe"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
                frame(&json!({"blocked": true}));
                loop {
                    std::thread::park();
                }
            }
            "silent-hold" | "unpolled" => loop {
                std::thread::park();
            },
            "flood" => loop {
                frame(&json!({"method":"flood", "data": "x".repeat(1024)}));
            },
            _ => {
                let mut stdin = std::io::BufReader::new(std::io::stdin().lock());
                while let Some(value) = receive(&mut stdin) {
                    if value.get("method").and_then(Value::as_str) == Some("exit") {
                        return;
                    }
                    frame(&value);
                }
            }
        }
    }

    type Inbox = mpsc::Receiver<(String, u64, Incoming)>;
    fn launch(root: &Path, mode: &str, capacity: usize) -> (Connection, Inbox) {
        let mut args = vec![
            "--fixture".to_owned(),
            mode.to_owned(),
            root.display().to_string(),
        ];
        args.extend(["", "café 😀", "a\"b", r"C:\folder with spaces\", "*.rs"].map(str::to_owned));
        let (sender, receiver) = mpsc::channel(capacity);
        let connection = transport::spawn(
            "fixture".into(),
            7,
            &std::env::current_exe().unwrap(),
            &args,
            root,
            sender,
        )
        .unwrap();
        (connection, receiver)
    }

    async fn incoming(events: &mut Inbox) -> Incoming {
        let (key, generation, event) = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(key, "fixture");
        assert_eq!(generation, 7);
        event
    }

    async fn ready(events: &mut Inbox) -> Value {
        match incoming(events).await {
            Incoming::Message(value) => *value,
            event => panic!("expected fixture readiness, got {event:?}"),
        }
    }

    async fn closed(events: &mut Inbox) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while events.recv().await.is_some() {}
        })
        .await
        .expect("framing tasks retained a full inbox after connection teardown");
    }

    fn run_case(case: &str, root: &Path) {
        if case == "unpolled_runtime_shutdown_owns_the_connection" {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let (connection, _events) = {
                let _entered = runtime.enter();
                launch(root, "unpolled", 1)
            };
            let deadline = Instant::now() + Duration::from_secs(5);
            let pid = loop {
                if let Ok(text) = fs::read_to_string(root.join("unpolled.pid"))
                    && let Ok(pid) = text.parse::<u32>()
                {
                    break pid;
                }
                assert!(Instant::now() < deadline, "unpolled fixture did not start");
                std::thread::sleep(Duration::from_millis(1));
            };
            let child = process(pid);
            // No spawned future has ever been polled on this current-thread
            // runtime. Its captured monitor guard must still own cancellation.
            drop(runtime);
            ended(&child);
            drop(connection);
            return;
        }
        let runtime = Runtime::new().unwrap();
        match case {
            "framing_arguments_directory_and_stderr" => runtime.block_on(async {
                let (connection, mut events) = launch(root, "echo", 8);
                assert!(matches!(
                    incoming(&mut events).await,
                    Incoming::Malformed(_)
                ));
                let notice = ready(&mut events).await;
                let child = process(notice["ready"].as_u64().unwrap() as u32);
                assert_eq!(
                    notice["args"],
                    json!(["", "café 😀", "a\"b", r"C:\folder with spaces\", "*.rs"])
                );
                assert_eq!(
                    PathBuf::from(notice["cwd"].as_str().unwrap())
                        .canonicalize()
                        .unwrap(),
                    root.canonicalize().unwrap()
                );
                assert_eq!(notice["config"], json!(root.join("config")));
                let message = json!({"id": 5, "result": "unicode: café 😀"});
                assert!(connection.send(message.clone()));
                assert_eq!(ready(&mut events).await, message);
                tokio::time::timeout(Duration::from_secs(5), async {
                    while !connection.stderr_tail().ends_with("TAIL_DONE") {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await
                .unwrap();
                assert!(connection.stderr_tail().len() <= 8 * 1024);
                assert!(connection.send(json!({"method": "exit"})));
                connection.stop().await;
                ended(&child);
                closed(&mut events).await;
            }),
            "blocked_stdin_stop_and_restart" => runtime.block_on(async {
                for _ in 0..3 {
                    let (connection, mut events) = launch(root, "blocked", 2);
                    let child = process(ready(&mut events).await["ready"].as_u64().unwrap() as u32);
                    assert!(connection.send(json!({"body": "x".repeat(16 * 1024 * 1024)})));
                    assert_eq!(ready(&mut events).await, json!({"blocked": true}));
                    let began = Instant::now();
                    connection.stop().await;
                    assert!(began.elapsed() < Duration::from_secs(2));
                    ended(&child);
                    closed(&mut events).await;
                }
            }),
            "full_inbox_and_connection_drop" => runtime.block_on(async {
                let (connection, mut events) = launch(root, "flood", 1);
                let child = process(ready(&mut events).await["ready"].as_u64().unwrap() as u32);
                tokio::time::timeout(Duration::from_secs(5), async {
                    while events.is_empty() {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await
                .unwrap();
                drop(connection);
                ended(&child);
                closed(&mut events).await;
            }),
            "leader_exit_ends_descendants" => runtime.block_on(async {
                let (connection, mut events) = launch(root, "tree", 4);
                let notice = ready(&mut events).await;
                let leader = process(notice["ready"].as_u64().unwrap() as u32);
                let descendant = process(notice["descendant"].as_u64().unwrap() as u32);
                assert!(connection.send(json!({"method": "exit"})));
                assert!(matches!(
                    incoming(&mut events).await,
                    Incoming::Closed { .. }
                ));
                ended(&leader);
                // Keep the Connection alive: observing leader exit alone must
                // trigger job cleanup, without relying on manager teardown.
                ended(&descendant);
                drop(connection);
            }),
            "runtime_shutdown_owns_the_connection" => {
                let (connection, child) = runtime.block_on(async {
                    let (connection, mut events) = launch(root, "blocked", 1);
                    let child = process(ready(&mut events).await["ready"].as_u64().unwrap() as u32);
                    assert!(connection.send(json!({"body": "x".repeat(16 * 1024 * 1024)})));
                    assert_eq!(ready(&mut events).await, json!({"blocked": true}));
                    (connection, child)
                });
                let began = Instant::now();
                drop(runtime);
                assert!(
                    began.elapsed() < Duration::from_secs(2),
                    "runtime waited on blocked pipe I/O"
                );
                ended(&child);
                drop(connection);
            }
            _ => panic!("unknown case {case}"),
        }
    }
}
