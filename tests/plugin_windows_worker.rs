// SPDX-License-Identifier: MPL-2.0
// A harness-free executable provides exact NDJSON stdout without libtest text.

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    native::main();
}

#[cfg(windows)]
mod native {
    use runyte::{
        plugin::{
            self, ClientMessage, Event, HostMessage, PluginConfig, application, compatibility,
        },
        terminal::{
            emulator::Emulator,
            pty::{Pty, PtyEvent},
        },
        test_support::TestRuntimeRoot,
    };
    use std::{
        fs,
        io::{BufRead, Read, Write},
        os::windows::{
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
            process::CommandExt,
        },
        path::Path,
        process::{Child, Command, Stdio},
        sync::mpsc as blocking,
        time::{Duration, Instant},
    };
    use tokio::sync::mpsc;
    use windows_sys::Win32::{
        Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{
            CREATE_NO_WINDOW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
    };

    const CASES: &[&str] = &[
        "fragmented_coalesced_and_exit_order",
        "saturated_output_preserves_final",
        "malformed_and_oversized",
        "write_failure_drains_final_reply",
        "cancellation_interrupts_blocked_write",
        "leader_exit_settles_descendant",
        "runtime_shutdown_owns_job",
        "public_plugin_lifecycle",
    ];

    pub fn main() {
        let arguments: Vec<_> = std::env::args().collect();
        match arguments.get(1).map(String::as_str) {
            Some("--fixture") => fixture(&arguments[2], Path::new(&arguments[3])),
            Some("--case") => run_case(&arguments[2], Path::new(&arguments[3])),
            _ => run_all(),
        }
    }

    fn run_all() {
        for case in CASES {
            let root = TestRuntimeRoot::new("native-plugin-worker-acceptance").unwrap();
            let output = fs::File::create(root.join("case-output")).unwrap();
            let mut process = Process(
                Command::new(std::env::current_exe().unwrap())
                    .args(["--case", case])
                    .arg(root.path())
                    .env("XDG_CONFIG_HOME", root.join("config"))
                    .env("XDG_CACHE_HOME", root.join("cache"))
                    .env("XDG_RUNTIME_DIR", root.join("runtime"))
                    .env("RUNYTE_CONTEXT_HOME", root.join("context"))
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
        println!("native plugin worker: {} passed", CASES.len());
    }

    struct Process(Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            unsafe { WaitForSingleObject(self.0.as_raw_handle(), 5000) };
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn run_case(case: &str, root: &Path) {
        match case {
            "fragmented_coalesced_and_exit_order" => runtime().block_on(frames(root)),
            "saturated_output_preserves_final" => runtime().block_on(saturated(root)),
            "malformed_and_oversized" => runtime().block_on(invalid_frames(root)),
            "write_failure_drains_final_reply" => runtime().block_on(write_exit(root)),
            "cancellation_interrupts_blocked_write" => {
                runtime().block_on(blocked_cancellation(root))
            }
            "leader_exit_settles_descendant" => runtime().block_on(descendant(root)),
            "runtime_shutdown_owns_job" => runtime_shutdown(root),
            "public_plugin_lifecycle" => public_lifecycle(root),
            other => panic!("unknown native plugin worker case: {other}"),
        }
    }

    fn config(root: &Path, mode: &str) -> PluginConfig {
        PluginConfig {
            settings: Default::default(),
            id: format!("native-{mode}"),
            api: application::VERSION.to_owned(),
            runyte: format!("={}", compatibility::HOST_VERSION),
            capabilities: vec![],
            enabled: true,
            executable: std::env::current_exe().unwrap(),
            args: vec![
                "--fixture".into(),
                mode.into(),
                root.as_os_str().to_owned().into_string().unwrap(),
            ],
            bindings: Default::default(),
        }
    }

    fn payload(id: &str, bytes: usize) -> HostMessage {
        HostMessage::Application(application::HostMessage::Response {
            id: id.into(),
            outcome: application::Response::Failure {
                error: application::Error::new(
                    application::ErrorCode::Unavailable,
                    &"x".repeat(bytes),
                ),
            },
        })
    }

    fn request_id(message: ClientMessage) -> Option<String> {
        let ClientMessage::Queued { message, .. } = message else {
            return None;
        };
        let ClientMessage::Application(application::ClientMessage::Request { id, .. }) = *message
        else {
            return None;
        };
        Some(id)
    }

    async fn next(receiver: &mut mpsc::Receiver<Event>) -> Event {
        tokio::time::timeout(Duration::from_secs(7), receiver.recv())
            .await
            .expect("native plugin worker event timed out")
            .expect("native plugin worker event channel closed")
    }

    async fn frames(root: &Path) {
        let (events, mut receiver) = mpsc::channel(plugin::EVENT_CAPACITY);
        let (_worker, _sender) = plugin::spawn(config(root, "frames"), root.into(), 0, events);
        let mut ids = Vec::new();
        loop {
            let event = next(&mut receiver).await;
            match event.result {
                Ok(ClientMessage::WorkerStopped {
                    failure: Some(failure),
                    reaped: true,
                }) => {
                    assert_eq!(failure, "Plugin process exited");
                    break;
                }
                Ok(message) => match request_id(message) {
                    Some(id) => ids.push(id),
                    None => panic!("unexpected non-request worker event"),
                },
                other => panic!("unexpected frame event: {other:?}"),
            }
        }
        assert_eq!(ids, vec!["p:1".to_owned(), "p:2".to_owned()]);
        assert!(receiver.try_recv().is_err());
    }

    async fn saturated(root: &Path) {
        let (events, mut receiver) = mpsc::channel(plugin::EVENT_CAPACITY);
        let (_worker, _sender) = plugin::spawn(config(root, "saturated"), root.into(), 0, events);
        let mut held = Vec::new();
        for expected in 1..=16 {
            let event = next(&mut receiver).await;
            let ClientMessage::Queued { message, .. } = event.result.as_ref().unwrap() else {
                panic!("ordinary output did not precede the terminal event");
            };
            let ClientMessage::Application(application::ClientMessage::Request { id, .. }) =
                message.as_ref()
            else {
                panic!("unexpected saturated output event");
            };
            assert_eq!(id, &format!("p:{expected}"));
            held.push(event);
        }
        let stopped = next(&mut receiver).await;
        assert!(matches!(
            stopped.result,
            Ok(ClientMessage::WorkerStopped {
                failure: Some(ref failure),
                reaped: true
            }) if failure == "Plugin output exceeded its quota"
        ));
        drop(held);
    }

    async fn invalid_frames(root: &Path) {
        for (owner, mode) in ["malformed", "partial", "oversized"]
            .into_iter()
            .enumerate()
        {
            let (events, mut receiver) = mpsc::channel(plugin::EVENT_CAPACITY);
            let (_worker, _sender) = plugin::spawn(config(root, mode), root.into(), owner, events);
            let event = next(&mut receiver).await;
            assert!(matches!(
                event.result,
                Ok(ClientMessage::WorkerStopped {
                    failure: Some(ref failure),
                    reaped: true
                }) if failure == "Plugin protocol or IO failed"
            ));
        }
    }

    async fn write_exit(root: &Path) {
        let (events, mut receiver) = mpsc::channel(plugin::EVENT_CAPACITY);
        let (_worker, sender) = plugin::spawn(config(root, "write-exit"), root.into(), 0, events);
        assert_eq!(
            request_id(next(&mut receiver).await.result.unwrap()).as_deref(),
            Some("p:1")
        );
        for id in 0..4 {
            sender
                .try_send(payload(&format!("p:host-{id}"), 900_000))
                .unwrap();
        }
        let final_reply = next(&mut receiver).await;
        assert_eq!(
            request_id(final_reply.result.unwrap()).as_deref(),
            Some("p:2")
        );
        let stopped = next(&mut receiver).await;
        match stopped.result {
            Ok(ClientMessage::WorkerStopped {
                failure: Some(failure),
                reaped: true,
            }) => assert!(
                matches!(
                    failure.as_str(),
                    "Plugin protocol or IO failed" | "Plugin stopped reading host messages"
                ),
                "expected a write failure after the drained reply, got {failure:?}"
            ),
            other => panic!("expected reaped WorkerStopped after write failure, got {other:?}"),
        }
    }

    async fn blocked_cancellation(root: &Path) {
        let (events, mut receiver) = mpsc::channel(plugin::EVENT_CAPACITY);
        let (worker, sender) = plugin::spawn(config(root, "blocked"), root.into(), 0, events);
        assert_eq!(
            request_id(next(&mut receiver).await.result.unwrap()).as_deref(),
            Some("p:1")
        );
        for id in 0..3 {
            sender
                .try_send(payload(&format!("p:{id}"), 900_000))
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        worker.stop();
        let stopped = next(&mut receiver).await;
        assert!(matches!(
            stopped.result,
            Ok(ClientMessage::WorkerStopped {
                failure: None,
                reaped: true
            })
        ));
    }

    async fn descendant(root: &Path) {
        let (events, mut receiver) = mpsc::channel(plugin::EVENT_CAPACITY);
        let (_worker, sender) = plugin::spawn(config(root, "descendant"), root.into(), 0, events);
        assert_eq!(
            request_id(next(&mut receiver).await.result.unwrap()).as_deref(),
            Some("p:1")
        );
        let pid = fs::read_to_string(root.join("descendant.pid"))
            .unwrap()
            .parse()
            .unwrap();
        let process = open_process(pid);
        sender.try_send(payload("p:continue", 0)).unwrap();
        let stopped = next(&mut receiver).await;
        assert!(matches!(
            stopped.result,
            Ok(ClientMessage::WorkerStopped {
                failure: Some(_),
                reaped: true
            })
        ));
        // `reaped` proves that the leader is signaled and job accounting is
        // empty. Windows may signal an externally retained historical process
        // object just after that accounting transition, so prove its eventual
        // settlement with one bounded kernel wait rather than timing sleeps.
        assert_eq!(
            unsafe { WaitForSingleObject(process.as_raw_handle(), 5000) },
            WAIT_OBJECT_0,
            "descendant process object did not settle after its job became empty"
        );
    }

    fn runtime_shutdown(root: &Path) {
        let runtime = runtime();
        let (worker, process) = runtime.block_on(async {
            let (events, mut receiver) = mpsc::channel(plugin::EVENT_CAPACITY);
            let (worker, _sender) = plugin::spawn(config(root, "runtime"), root.into(), 0, events);
            assert_eq!(
                request_id(next(&mut receiver).await.result.unwrap()).as_deref(),
                Some("p:1")
            );
            let pid = fs::read_to_string(root.join("worker.pid"))
                .unwrap()
                .parse()
                .unwrap();
            (worker, open_process(pid))
        });
        drop(runtime);
        drop(worker);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match unsafe { WaitForSingleObject(process.as_raw_handle(), 0) } {
                WAIT_OBJECT_0 => break,
                WAIT_TIMEOUT => assert!(
                    Instant::now() < deadline,
                    "runtime shutdown did not close the plugin job"
                ),
                _ => panic!("cannot observe runtime-shutdown fixture"),
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn open_process(pid: u32) -> OwnedHandle {
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        assert!(!handle.is_null());
        unsafe { OwnedHandle::from_raw_handle(handle) }
    }

    fn frame(id: &str) -> Vec<u8> {
        format!(
            "{{\"type\":\"request\",\"id\":\"{id}\",\"method\":\"settings.get\",\"params\":{{}}}}\n"
        )
        .into_bytes()
    }

    fn public_fixture(mode: &str, root: &Path) {
        let mut input = std::io::stdin().lock();
        let mut line = String::new();
        input.read_line(&mut line).unwrap();
        let hello: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(hello["type"], "hello");
        assert!(
            !hello["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|cap| cap == "processes")
        );
        let id = if mode == "public-optional" {
            "optional"
        } else {
            "required"
        };
        let pid = std::process::id();
        fs::write(root.join(format!("{id}-{pid}.started")), "").unwrap();
        if id == "required" {
            // Let the parent retain a process handle before this deliberately
            // rejected registration can be killed and reaped by the host.
            let deadline = Instant::now() + Duration::from_secs(5);
            while !root.join("allow-required").exists() {
                assert!(
                    Instant::now() < deadline,
                    "required fixture was not released"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let registration = serde_json::json!({
            "type": "register",
            "version": application::VERSION,
            "runyte": format!("={}", compatibility::HOST_VERSION),
            "name": id,
            "commands": [{"name": "ping", "description": "Exercise plugin registration", "context": "workspace"}],
            "required_capabilities": if id == "required" { vec!["processes"] } else { vec![] },
            "optional_capabilities": if id == "optional" { vec!["processes"] } else { vec![] },
            "required_features": [],
            "optional_features": []
        });
        println!("{registration}");
        if id == "optional" {
            line.clear();
            input.read_line(&mut line).unwrap();
            let answer: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(answer["type"], "registered");
            assert!(
                !answer["capabilities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|cap| cap == "processes")
            );
            fs::write(root.join(format!("{id}-{pid}.answered")), "").unwrap();
        }
        while input.read_line(&mut line).unwrap() != 0 {
            line.clear();
        }
    }

    fn public_lifecycle(root: &Path) {
        let project = root.join("project");
        fs::create_dir(&project).unwrap();
        let executable =
            serde_json::to_string(&std::env::current_exe().unwrap().to_string_lossy()).unwrap();
        let fixture_root = serde_json::to_string(&root.to_string_lossy()).unwrap();
        let config = root.join("config/config.yaml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, format!(
            "lsp:\n  enable: false\nplugins:\n  - id: optional\n    enabled: true\n    api: runyte-1\n    runyte: '={version}'\n    executable: {executable}\n    args: ['--fixture', 'public-optional', {fixture_root}]\n    capabilities: [processes]\n  - id: required\n    enabled: true\n    api: runyte-1\n    runyte: '={version}'\n    executable: {executable}\n    args: ['--fixture', 'public-required', {fixture_root}]\n    capabilities: [processes]\n",
            version = compatibility::HOST_VERSION,
        )).unwrap();
        let (sender, events) = blocking::channel();
        let args = vec![
            "--standalone".into(),
            "--config".into(),
            config.display().to_string(),
        ];
        let editor = Pty::spawn(
            Path::new(env!("CARGO_BIN_EXE_runyte")).as_os_str(),
            &args,
            &project,
            120,
            30,
            move |event| {
                let _ = sender.send(event);
            },
        )
        .unwrap();
        let mut console = PublicConsole {
            editor,
            events,
            screen: Emulator::new(120, 30),
        };
        console.until_text("Project directory [");
        console.send("\r");
        console.until_text("[y/N]:");
        console.send("y\r");
        console.until_text("NOR");
        console.send(":plugins\r");
        console.until_text("Plugins");
        let first = console.until_answer(root, "optional", None);
        let rejected = console.until_marker(root, "required", "started", None);
        assert_ne!(first, rejected);
        let first_process = open_process(first);
        let rejected_process = open_process(rejected);
        fs::write(root.join("allow-required"), "").unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(rejected_process.as_raw_handle(), 5000) },
            WAIT_OBJECT_0
        );
        console.until_text("optional");
        console.send("\x1b[B");
        // The manager preview clips long diagnostics at the visible column.
        console.until_text("Host does not support required capability");
        console.send("\x1b");
        console.send(":plugin-stop optional\r");
        assert_eq!(
            unsafe { WaitForSingleObject(first_process.as_raw_handle(), 5000) },
            WAIT_OBJECT_0
        );
        console.send(":plugin-restart optional\r");
        let second = console.until_answer(root, "optional", Some(first));
        assert_ne!(first, second);
        let second_process = open_process(second);
        console.send(":quit\r");
        console.until_exit();
        assert_eq!(
            unsafe { WaitForSingleObject(second_process.as_raw_handle(), 5000) },
            WAIT_OBJECT_0
        );
    }

    struct PublicConsole {
        editor: Pty,
        events: blocking::Receiver<PtyEvent>,
        screen: Emulator,
    }
    impl PublicConsole {
        fn send(&self, text: &str) {
            assert!(self.editor.write(text.as_bytes().to_vec()));
        }
        fn pump(&mut self, deadline: Instant, stage: &str) -> bool {
            match self.events.recv_timeout(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(50)),
            ) {
                Ok(PtyEvent::Output(bytes)) => {
                    self.screen.feed(&bytes);
                    false
                }
                Ok(PtyEvent::Exited(code)) => {
                    assert_eq!(code, Some(0));
                    true
                }
                event => {
                    assert!(
                        Instant::now() < deadline,
                        "waiting for {stage}: editor event {event:?}; screen: {}",
                        self.screen_text()
                    );
                    false
                }
            }
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
        fn until_text(&mut self, text: &str) {
            let deadline = Instant::now() + Duration::from_secs(15);
            while !self.screen_text().replace('\n', "").contains(text) {
                assert!(
                    !self.pump(deadline, text),
                    "editor exited waiting for {text}"
                );
            }
        }
        fn until_answer(&mut self, root: &Path, id: &str, exclude: Option<u32>) -> u32 {
            self.until_marker(root, id, "answered", exclude)
        }
        fn until_marker(
            &mut self,
            root: &Path,
            id: &str,
            suffix: &str,
            exclude: Option<u32>,
        ) -> u32 {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                for entry in fs::read_dir(root).unwrap().flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if let Some(pid) = name
                        .strip_prefix(&format!("{id}-"))
                        .and_then(|rest| rest.strip_suffix(&format!(".{suffix}")))
                        .and_then(|pid| pid.parse::<u32>().ok())
                        && Some(pid) != exclude
                    {
                        return pid;
                    }
                }
                assert!(
                    !self.pump(deadline, id),
                    "editor exited waiting for {id} registration"
                );
            }
        }
        fn until_exit(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(15);
            while !self.pump(deadline, "editor exit") {}
        }
    }

    fn fixture(mode: &str, root: &Path) {
        assert_eq!(
            Path::new(&std::env::var_os("XDG_CONFIG_HOME").unwrap()),
            root.join("config")
        );
        assert_eq!(
            Path::new(&std::env::var_os("XDG_CACHE_HOME").unwrap()),
            root.join("cache")
        );
        assert_eq!(
            Path::new(&std::env::var_os("XDG_RUNTIME_DIR").unwrap()),
            root.join("runtime")
        );
        assert_eq!(
            Path::new(&std::env::var_os("RUNYTE_CONTEXT_HOME").unwrap()),
            root.join("context")
        );
        if mode.starts_with("public-") {
            public_fixture(mode, root);
            return;
        }
        fs::write(root.join("worker.pid"), std::process::id().to_string()).unwrap();
        let ready = frame("p:1");
        match mode {
            "frames" => {
                let second = frame("p:2");
                let mut output = std::io::stdout().lock();
                output.write_all(&ready[..17]).unwrap();
                output.flush().unwrap();
                std::thread::sleep(Duration::from_millis(10));
                output.write_all(&ready[17..]).unwrap();
                output.write_all(&second).unwrap();
                output.flush().unwrap();
            }
            "saturated" => {
                let mut output = std::io::stdout().lock();
                for id in 1..=17 {
                    output.write_all(&frame(&format!("p:{id}"))).unwrap();
                }
                output.flush().unwrap();
                loop {
                    std::thread::park();
                }
            }
            "malformed" => std::io::stdout().write_all(b"{]\n").unwrap(),
            "partial" => {
                std::io::stdout().write_all(b"{\"type\":").unwrap();
                std::io::stdout().flush().unwrap();
            }
            "oversized" => {
                let mut output = std::io::stdout().lock();
                output.write_all(&vec![b'x'; plugin::MAX_BYTES]).unwrap();
                output.write_all(b"\n").unwrap();
                output.flush().unwrap();
            }
            "write-exit" => {
                let mut output = std::io::stdout().lock();
                output.write_all(&ready).unwrap();
                output.flush().unwrap();
                // Reading a complete first frame and one byte of the second
                // proves the host entered another oversized write. The small
                // BufReader can retain only a fraction of that second frame.
                let mut input = std::io::BufReader::with_capacity(4096, std::io::stdin().lock());
                let mut first = Vec::new();
                input.read_until(b'\n', &mut first).unwrap();
                assert!(first.len() > 64 * 1024 && first.ends_with(b"\n"));
                let mut byte = [0];
                input.read_exact(&mut byte).unwrap();
                output.write_all(&frame("p:2")).unwrap();
                output.flush().unwrap();
            }
            "blocked" | "runtime" => {
                let mut output = std::io::stdout().lock();
                output.write_all(&ready).unwrap();
                output.flush().unwrap();
                loop {
                    std::thread::park();
                }
            }
            "descendant" => {
                #[allow(clippy::zombie_processes)]
                let child = Command::new(std::env::current_exe().unwrap())
                    .args(["--fixture", "hold-descendant"])
                    .arg(root)
                    .env("XDG_CONFIG_HOME", root.join("config"))
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                fs::write(root.join("descendant.pid"), child.id().to_string()).unwrap();
                std::io::stdout().write_all(&ready).unwrap();
                std::io::stdout().flush().unwrap();
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line).unwrap();
            }
            "hold-descendant" => loop {
                std::thread::park();
            },
            other => panic!("unknown native plugin worker fixture: {other}"),
        }
    }
}
