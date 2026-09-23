// SPDX-License-Identifier: MPL-2.0

#![cfg(windows)]

//! Required native acceptance, run explicitly after CI provisions rust-analyzer.
use runyte::{
    config::{LanguageServerConfig, LspConfig},
    lsp::{self, LspCommand, LspEvent, LspEvents, RequestKind, Response},
    terminal::{
        emulator::Emulator,
        pty::{Pty, PtyEvent},
    },
    test_support::TestRuntimeRoot,
};
use std::{
    collections::HashMap,
    fs,
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    ptr,
    sync::mpsc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::WAIT_OBJECT_0,
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOBOBJECT_BASIC_PROCESS_ID_LIST,
            JobObjectBasicProcessIdList, QueryInformationJobObject,
        },
        Threading::{GetCurrentProcess, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    },
};

struct Fixture(Child);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "required Windows CI acceptance; needs the installed rust-analyzer component"]
fn real_rust_analyzer_permission_diagnostics_edits_restart_and_cleanup() {
    let root = TestRuntimeRoot::new("real-native-lsp").unwrap();
    let analyzer = Command::new("rustup")
        .args(["which", "rust-analyzer"])
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .output()
        .expect("rustup must be installed for required acceptance");
    assert!(
        analyzer.status.success(),
        "rust-analyzer must be provisioned"
    );
    let analyzer = PathBuf::from(String::from_utf8(analyzer.stdout).unwrap().trim());
    assert!(analyzer.is_absolute() && analyzer.is_file());
    let output = fs::File::create(root.join("fixture.log")).unwrap();
    let mut child = Fixture(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "native_lsp_fixture", "--ignored", "--nocapture"])
            .env("RUNYTE_NATIVE_LSP_ROOT", root.path())
            .env("RUNYTE_NATIVE_LSP_SERVER", analyzer)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("RUNYTE_CONTEXT_HOME", root.join("context"))
            .env("CARGO_HOME", root.join("cargo-home"))
            .env("CARGO_TARGET_DIR", root.join("cargo-target"))
            .env("CARGO_NET_OFFLINE", "true")
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            let output = fs::read_to_string(root.join("fixture.log")).unwrap();
            assert!(status.success(), "{output}");
            assert!(
                output.contains("NATIVE_REAL_LSP_ACCEPTANCE_COMPLETE"),
                "{output}"
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "native LSP fixture timed out: {}",
            fs::read_to_string(root.join("fixture.log")).unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A fixture-only enclosing job observes exactly this helper's descendants.
/// It neither identifies nor terminates somebody else's language servers.
struct Descendants(OwnedHandle);
impl Descendants {
    fn new() -> Self {
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        assert!(!handle.is_null(), "{}", std::io::Error::last_os_error());
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        assert_ne!(
            unsafe { AssignProcessToJobObject(handle.as_raw_handle(), GetCurrentProcess()) },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        Self(handle)
    }

    fn processes(&self) -> Vec<OwnedHandle> {
        // The native variable-sized record is DWORD counts followed by ULONG_PTRs.
        let mut storage = vec![0usize; 258];
        assert_ne!(
            unsafe {
                QueryInformationJobObject(
                    self.0.as_raw_handle(),
                    JobObjectBasicProcessIdList,
                    storage.as_mut_ptr().cast(),
                    (storage.len() * size_of::<usize>()) as u32,
                    ptr::null_mut(),
                )
            },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        let list = unsafe { &*storage.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>() };
        assert!(list.NumberOfProcessIdsInList < 256);
        let ids = unsafe {
            std::slice::from_raw_parts(
                list.ProcessIdList.as_ptr(),
                list.NumberOfProcessIdsInList as usize,
            )
        };
        ids.iter()
            .filter(|&&id| id != std::process::id() as usize)
            .filter_map(|&id| {
                let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, id as u32) };
                (!handle.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(handle) })
            })
            .collect()
    }
}

fn ended(processes: &[OwnedHandle]) {
    assert!(!processes.is_empty(), "the real server must have started");
    for process in processes {
        assert_eq!(
            unsafe { WaitForSingleObject(process.as_raw_handle(), 10_000) },
            WAIT_OBJECT_0,
            "an owned server or descendant survived cleanup"
        );
    }
}

async fn event<T>(events: &mut LspEvents, mut accept: impl FnMut(LspEvent) -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(40), async {
        loop {
            let next = events.recv().await.expect("manager stopped unexpectedly");
            if matches!(
                &next,
                LspEvent::Stopped { .. } | LspEvent::Status { error: true, .. }
            ) {
                panic!("unexpected LSP failure: {next:?}");
            }
            if let Some(value) = accept(next) {
                return value;
            }
        }
    })
    .await
    .expect("expected real language-server event within deadline")
}

fn options() -> serde_json::Value {
    serde_json::json!({
        "cargo": {"buildScripts": {"enable": false}, "sysroot": null},
        "procMacro": {"enable": false},
        "checkOnSave": false
    })
}

async fn manager(project: &Path, server: &Path, descendants: &Descendants) {
    let path = project.join("src/main.rs").canonicalize().unwrap();
    let config = LspConfig {
        enable: true,
        servers: HashMap::from([(
            "rust".into(),
            LanguageServerConfig {
                command: server.into(),
                args: Vec::new(),
                initialization_options: Some(options()),
            },
        )]),
    };
    let (handle, mut events) = lsp::spawn(config, project.canonicalize().unwrap());
    assert!(handle.send(LspCommand::Ensure {
        language: "rust".into()
    }));
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(
        descendants.processes().is_empty(),
        "denied manager launched a server"
    );
    assert!(events.try_recv().is_err());
    handle.set_allowed(true);
    assert!(handle.send(LspCommand::Ensure {
        language: "rust".into()
    }));
    let generation = event(&mut events, |event| match event {
        LspEvent::Ready {
            generation,
            capabilities,
            ..
        } => {
            assert!(capabilities.supports(&RequestKind::Format {
                tab_size: 4,
                insert_spaces: true
            }));
            Some(generation)
        }
        _ => None,
    })
    .await;
    let first = descendants.processes();
    assert!(!first.is_empty());
    assert!(handle.send(LspCommand::Open {
        language: "rust".into(),
        path: path.clone(),
        version: 1,
        text: "fn main( {\n".into(),
    }));
    event(&mut events, |event| match event {
        LspEvent::Diagnostics {
            path: returned,
            diagnostics,
            ..
        } if !diagnostics.is_empty() && returned.canonicalize().ok().as_ref() == Some(&path) => {
            Some(())
        }
        _ => None,
    })
    .await;
    assert!(handle.send(LspCommand::Change {
        language: "rust".into(),
        path: path.clone(),
        version: 2,
        changes: vec![lsp_types::TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: "fn main(){let value=1;println!(\"{value}\");}\n".into(),
        }],
    }));
    assert!(handle.send(LspCommand::Request {
        token: 1,
        language: "rust".into(),
        path: path.clone(),
        kind: Box::new(RequestKind::Format {
            tab_size: 4,
            insert_spaces: true
        }),
    }));
    event(&mut events, |event| match event {
        LspEvent::Response {
            token: 1,
            response: Response::Edits { edits, skipped, .. },
        } => {
            assert_eq!(skipped, 0);
            assert!(
                edits
                    .iter()
                    .any(|document| document.path.as_os_str().is_empty()
                        && document.edits.iter().any(|edit| !edit.new_text.is_empty()))
            );
            Some(())
        }
        LspEvent::Response { token: 1, response } => panic!("formatting failed: {response:?}"),
        _ => None,
    })
    .await;
    assert!(handle.send(LspCommand::Restart(Some("rust".into()))));
    event(&mut events, |event| {
        matches!(event, LspEvent::Restarted { .. }).then_some(())
    })
    .await;
    ended(&first);
    assert!(handle.send(LspCommand::Ensure {
        language: "rust".into()
    }));
    event(&mut events, |event| match event {
        LspEvent::Ready {
            generation: next, ..
        } => {
            assert!(next > generation);
            Some(())
        }
        _ => None,
    })
    .await;
    let second = descendants.processes();
    handle.set_allowed(false);
    // The manager's permission channel retires live servers independently of
    // the ordinary command queue; held process handles prevent PID reuse.
    tokio::task::yield_now().await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while second
        .iter()
        .any(|process| unsafe { WaitForSingleObject(process.as_raw_handle(), 0) } != WAIT_OBJECT_0)
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "revocation did not stop real server"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    ended(&second);
    assert!(handle.send(LspCommand::Shutdown));
    drop(handle);
    tokio::time::timeout(Duration::from_secs(10), async {
        while events.recv().await.is_some() {}
    })
    .await
    .unwrap();
}

fn until(events: &mpsc::Receiver<PtyEvent>, screen: &mut Emulator, marker: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !screen.plain_text().contains(marker) {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(PtyEvent::Output(bytes)) => {
                screen.feed(&bytes);
            }
            other => panic!("waiting for {marker:?}: {other:?}: {}", screen.plain_text()),
        }
    }
}

fn frontend(root: &Path, project: &Path) {
    let config = root.join("config/editor.yaml");
    fs::write(&config, "workspace:\n  mode: standalone\nlsp:\n  enable: true\n  rust:\n    command: runyte-intentionally-missing-language-server\n").unwrap();
    let (send, events) = mpsc::channel();
    let terminal = Pty::spawn(
        Path::new(env!("CARGO_BIN_EXE_runyte")).as_os_str(),
        &[
            "--config".into(),
            config.to_str().unwrap().into(),
            "--project-root".into(),
            project.to_str().unwrap().into(),
            "src/main.rs".into(),
        ],
        project,
        120,
        35,
        move |event| {
            let _ = send.send(event);
        },
    )
    .unwrap();
    let mut screen = Emulator::new(120, 35);
    until(&events, &mut screen, "Allow LSP once");
    until(&events, &mut screen, "Keep LSP disabled");
    // Escape keeps the default denial, leaving ordinary editing available.
    assert!(terminal.write(b"\x1b".to_vec()));
    assert!(terminal.write(b":lsp-trust\r".to_vec()));
    until(&events, &mut screen, "Allow LSP once");
    assert!(terminal.write(b"\x1b[B\r".to_vec()));
    until(&events, &mut screen, "E1");
    assert!(terminal.write(b":notifications\r".to_vec()));
    until(
        &events,
        &mut screen,
        "cannot start runyte-intentionally-missing-language-server",
    );
    assert!(terminal.write(b":quit!\r".to_vec()));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(PtyEvent::Output(_)) => {}
            Ok(PtyEvent::Exited(code)) => {
                assert_eq!(code, Some(0));
                break;
            }
            other => panic!("editor did not exit: {other:?}"),
        }
    }
}

#[test]
#[ignore = "compiled child fixture; launched by the required acceptance test"]
fn native_lsp_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_NATIVE_LSP_ROOT").unwrap());
    let server = PathBuf::from(std::env::var_os("RUNYTE_NATIVE_LSP_SERVER").unwrap());
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME").unwrap(),
        root.join("config")
    );
    assert_eq!(
        std::env::var_os("XDG_CACHE_HOME").unwrap(),
        root.join("cache")
    );
    assert_eq!(
        std::env::var_os("RUNYTE_CONTEXT_HOME").unwrap(),
        root.join("context")
    );
    fs::create_dir(root.join("config")).unwrap();
    let project = root.join("project café");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"native_lsp_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();
    let descendants = Descendants::new();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(manager(&project, &server, &descendants));
    frontend(&root, &project);
    println!("NATIVE_REAL_LSP_ACCEPTANCE_COMPLETE");
}
