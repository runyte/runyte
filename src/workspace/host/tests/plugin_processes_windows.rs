// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::{HostPorts, plugin_workflows::Instance},
    clipboard::SystemClipboard,
    plugin::{self, ClientMessage, HostMessage, PluginConfig},
};
use std::{
    io::{Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::sync::{Semaphore, mpsc};
use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

const FIXTURE: &str = "workspace::host::plugin_processes::tests::child_fixture";

#[test]
#[ignore = "launched by the native process host test"]
fn child_fixture() {
    let grandchild = if std::env::current_dir()
        .unwrap()
        .join("spawn-descendant")
        .exists()
    {
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "workspace::host::plugin_processes::tests::grandchild_fixture",
                "--ignored",
            ])
            .spawn()
            .unwrap();
        std::fs::write("grandchild.pid", child.id().to_string()).unwrap();
        Some(child)
    } else {
        None
    };
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input).unwrap();
    if let Some(mut grandchild) = grandchild {
        let _ = grandchild.kill();
        let _ = grandchild.wait();
    }
    std::io::stdout().write_all(b"native-helper:").unwrap();
    std::io::stdout().write_all(&input).unwrap();
    std::io::stdout().flush().unwrap();
}

#[test]
#[ignore = "launched only as a managed helper descendant"]
fn grandchild_fixture() {
    loop {
        std::thread::park();
    }
}

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "runyte-helper-host-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Clipboard;
impl SystemClipboard for Clipboard {
    fn read(&mut self) -> anyhow::Result<String> {
        anyhow::bail!("inert clipboard")
    }
    fn write(&mut self, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

fn fixture_start() -> process::Start {
    process::Start {
        label: "helper".into(),
        executable: std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        args: vec![
            "--exact".into(),
            FIXTURE.into(),
            "--ignored".into(),
            "--nocapture".into(),
        ],
        cwd: None,
        capture_stderr: false,
    }
}

fn host(
    root: &Root,
) -> (
    WorkspaceHost,
    mpsc::Receiver<plugin::Event>,
    mpsc::Receiver<HostMessage>,
) {
    let app = crate::app::App::new_in_isolated_project(
        root.path(),
        HostPorts::isolated(Box::new(Clipboard)),
    )
    .unwrap();
    let mut host = WorkspaceHost::new(app);
    let (events, received) = mpsc::channel(128);
    host.plugin_events_sender = Some(events);
    let (sender, replies) = mpsc::channel(32);
    let state = api::Instance {
        generation: "native-generation".into(),
        capabilities: ["processes".into()].into(),
        ..Default::default()
    };
    host.app.plugins.instances.insert(
        0,
        Instance {
            config: PluginConfig {
                settings: Default::default(),
                id: "native".into(),
                api: api::VERSION.into(),
                runyte: format!("={}", plugin::compatibility::HOST_VERSION),
                capabilities: vec!["processes".into()],
                enabled: true,
                executable: std::env::current_exe().unwrap(),
                args: vec![],
                bindings: Default::default(),
            },
            sender: plugin::Sender::new(sender),
            registered: true,
            application: state,
        },
    );
    (host, received, replies)
}

async fn pump(host: &mut WorkspaceHost, events: &mut mpsc::Receiver<plugin::Event>) -> bool {
    let event = tokio::time::timeout(std::time::Duration::from_secs(8), events.recv())
        .await
        .expect("bounded process completion")
        .expect("process event channel");
    let ClientMessage::Process(process) = event.result.unwrap() else {
        panic!("expected process event")
    };
    let started = matches!(process.kind, runtime::Kind::Started);
    host.application_process_event(event.plugin, process)
        .unwrap();
    started
}

#[tokio::test]
async fn native_process_requests_retain_binary_output_and_release_handle() {
    let root = Root::new();
    let (mut host, mut events, mut replies) = host(&root);
    assert!(
        host.application_process_request(0, "start", Request::ProcessStart(fixture_start()))
            .unwrap()
            .is_none()
    );
    assert!(pump(&mut host, &mut events).await);
    let handle = host.plugin_processes.keys().next().unwrap().clone();
    assert_eq!(
        host.app.plugins.instances[&0].application.processes.len(),
        1
    );
    assert!(
        host.application_process_request(
            0,
            "write",
            Request::ProcessWrite {
                process: handle.clone(),
                data: process::encode(b"binary\0\xff"),
                eof: true
            }
        )
        .unwrap()
        .is_none()
    );
    while host.plugin_processes[&handle].info.state != process::State::Exited {
        pump(&mut host, &mut events).await;
    }
    let result = host
        .application_process_request(
            0,
            "read",
            Request::ProcessRead {
                process: handle.clone(),
                stream: process::Stream::Stdout,
                offset: 0,
                limit: process::MAX_IO_BYTES,
            },
        )
        .unwrap()
        .unwrap();
    let ResultValue::ProcessRead(read) = result else {
        panic!("process read")
    };
    let bytes = process::decode(&read.data).unwrap();
    assert!(
        bytes
            .windows(b"native-helper:binary\0\xff".len())
            .any(|w| w == b"native-helper:binary\0\xff")
    );
    assert!(read.eof);
    assert!(
        host.application_process_request(
            0,
            "close",
            Request::ProcessClose {
                process: handle.clone()
            }
        )
        .unwrap()
        .is_some()
    );
    assert!(host.plugin_processes.is_empty());
    assert!(
        host.application_process_request(
            0,
            "close-again",
            Request::ProcessClose { process: handle }
        )
        .unwrap()
        .is_some()
    );
    while replies.try_recv().is_ok() {}
}

#[tokio::test]
async fn closing_a_running_helper_settles_its_descendant_before_reply() {
    let root = Root::new();
    std::fs::write(root.path().join("spawn-descendant"), b"").unwrap();
    let (mut host, mut events, mut replies) = host(&root);
    assert!(
        host.application_process_request(0, "start", Request::ProcessStart(fixture_start()))
            .unwrap()
            .is_none()
    );
    assert!(pump(&mut host, &mut events).await);
    let handle = host.plugin_processes.keys().next().unwrap().clone();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
    let pid: u32 = loop {
        if let Ok(value) = std::fs::read_to_string(root.path().join("grandchild.pid")) {
            break value.parse().unwrap();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "descendant did not start"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    };
    let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!raw.is_null());
    let child = unsafe { OwnedHandle::from_raw_handle(raw) };
    assert!(
        host.application_process_request(
            0,
            "close",
            Request::ProcessClose {
                process: handle.clone()
            }
        )
        .unwrap()
        .is_none()
    );
    while host.plugin_processes.contains_key(&handle) {
        pump(&mut host, &mut events).await;
    }
    assert_eq!(
        unsafe { WaitForSingleObject(child.as_raw_handle(), 0) },
        WAIT_OBJECT_0
    );
    let mut closed = false;
    while let Ok(message) = replies.try_recv() {
        if let HostMessage::Application(api::HostMessage::Response { id, outcome }) = message
            && id == "close"
        {
            assert!(matches!(outcome, api::Response::Success { .. }));
            closed = true;
        }
    }
    assert!(closed, "close was not acknowledged");
}

#[tokio::test]
async fn delayed_cleanup_keeps_handle_and_refuses_late_close() {
    let root = Root::new();
    let (mut host, mut events, _replies) = host(&root);
    assert!(
        host.application_process_request(0, "start", Request::ProcessStart(fixture_start()))
            .unwrap()
            .is_none()
    );
    assert!(pump(&mut host, &mut events).await);
    let handle = host.plugin_processes.keys().next().unwrap().clone();
    host.application_process_event(
        0,
        runtime::Event {
            generation: "native-generation".into(),
            process: handle.clone(),
            kind: runtime::Kind::CleanupDelayed,
            _permit: Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap(),
            _lifetime: None,
        },
    )
    .unwrap();
    assert!(host.plugin_processes[&handle].cleanup_pending);
    assert_eq!(host.pending_process_requests(Some(0)), 1);
    let error = host
        .application_process_request(
            0,
            "late-close",
            Request::ProcessClose {
                process: handle.clone(),
            },
        )
        .unwrap_err();
    assert_eq!(error.code, Code::Unavailable);
    host.plugin_processes[&handle]
        .control
        .as_ref()
        .unwrap()
        .close();
    while host.plugin_processes[&handle].info.state != process::State::Exited {
        pump(&mut host, &mut events).await;
    }
    assert_eq!(host.pending_process_requests(Some(0)), 0);
    assert!(
        host.application_process_request(
            0,
            "settled-close",
            Request::ProcessClose { process: handle }
        )
        .unwrap()
        .is_some()
    );
    assert!(host.plugin_processes.is_empty());
}
