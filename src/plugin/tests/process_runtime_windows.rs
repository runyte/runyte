// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::{
    io::{Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use windows_sys::Win32::{
    Foundation::WAIT_OBJECT_0,
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
};

const FIXTURE: &str = "plugin::process::runtime::tests::child_fixture";

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "runyte-helper-{}-{}",
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

#[test]
#[ignore = "launched only by the native helper runtime tests"]
fn child_fixture() {
    if Path::new("park-helper").exists() {
        std::fs::write("helper.pid", std::process::id().to_string()).unwrap();
        loop {
            std::thread::park();
        }
    }
    if Path::new("flood-output").exists() {
        for _ in 0..1024 {
            std::io::stdout().write_all(&[b'x'; 1024]).unwrap();
        }
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input).unwrap();
    std::io::stdout().write_all(b"helper-output:\0").unwrap();
    std::io::stdout().write_all(&input).unwrap();
    std::io::stdout().flush().unwrap();
    std::io::stderr().write_all(b"helper-error:\xff").unwrap();
    std::io::stderr().flush().unwrap();
}

fn fixture(root: &Path, capture_stderr: bool) -> Start {
    Start {
        label: "test helper".into(),
        executable: std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        args: vec![
            "--exact".into(),
            FIXTURE.into(),
            "--ignored".into(),
            "--nocapture".into(),
            "--test-threads=1".into(),
        ],
        cwd: Some(root.to_string_lossy().into_owned()),
        capture_stderr,
    }
}

async fn next(receiver: &mut mpsc::Receiver<plugin::Event>) -> Event {
    let message = tokio::time::timeout(Duration::from_secs(8), receiver.recv())
        .await
        .expect("bounded helper completion")
        .expect("helper event channel");
    let plugin::ClientMessage::Process(event) = message.result.unwrap() else {
        panic!("expected helper event")
    };
    event
}

#[tokio::test]
async fn helper_binary_pipes_eof_and_natural_exit() {
    let root = Root::new();
    let (sender, mut receiver) = mpsc::channel(128);
    let handle = spawn(
        1,
        "generation".into(),
        "helper".into(),
        root.path().to_owned(),
        fixture(root.path(), true),
        sender,
    )
    .unwrap();
    let first = next(&mut receiver).await;
    assert!(matches!(first.kind, Kind::Started), "{first:?}");
    let bytes = b"binary\0input\xff".to_vec();
    handle.write("write".into(), bytes.clone(), true).unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut acknowledged = false;
    loop {
        match next(&mut receiver).await.kind {
            Kind::Output {
                stream: Stream::Stdout,
                bytes,
            } => out.extend(bytes),
            Kind::Output {
                stream: Stream::Stderr,
                bytes,
            } => err.extend(bytes),
            Kind::Written { request, result } => {
                assert_eq!(request, "write");
                let result = result.unwrap();
                assert_eq!(result.written, bytes.len());
                assert!(result.eof);
                acknowledged = true;
            }
            Kind::Exited {
                code,
                signal,
                error,
                stdout,
                stderr,
                write,
                ..
            } => {
                out.extend(stdout);
                err.extend(stderr);
                if let Some((request, result)) = write {
                    assert_eq!(request, "write");
                    assert_eq!(result.unwrap().written, bytes.len());
                    acknowledged = true;
                }
                assert_eq!(code, Some(0));
                assert_eq!(signal, None);
                assert!(error.is_none());
                break;
            }
            other => panic!("unexpected helper event {other:?}"),
        }
    }
    assert!(acknowledged);
    assert!(
        out.windows(b"helper-output:\0".len())
            .any(|w| w == b"helper-output:\0")
    );
    assert!(out.windows(bytes.len()).any(|w| w == bytes));
    assert!(
        err.windows(b"helper-error:\xff".len())
            .any(|w| w == b"helper-error:\xff")
    );
}

#[tokio::test]
async fn outside_workspace_and_missing_executable_fail_without_a_handle() {
    let root = Root::new();
    for start in [
        Start {
            cwd: Some(std::env::temp_dir().to_string_lossy().into_owned()),
            ..fixture(root.path(), false)
        },
        Start {
            executable: root
                .path()
                .join("missing.exe")
                .to_string_lossy()
                .into_owned(),
            ..fixture(root.path(), false)
        },
    ] {
        let (sender, mut receiver) = mpsc::channel(16);
        let _handle = spawn(
            1,
            "generation".into(),
            "helper".into(),
            root.path().to_owned(),
            start,
            sender,
        )
        .unwrap();
        match next(&mut receiver).await.kind {
            Kind::Exited {
                error: Some(error), ..
            } => assert!(matches!(
                error.code,
                ErrorCode::InvalidArgument | ErrorCode::Unavailable
            )),
            other => panic!("unexpected helper event {other:?}"),
        }
    }
}

#[tokio::test]
async fn failed_first_cleanup_retains_child_until_background_reap() {
    let root = Root::new();
    let start = fixture(root.path(), false);
    let path = root.path().to_owned();
    let (sender, mut receiver) = mpsc::channel(128);
    let handle = spawn_with_launcher(
        1,
        "generation".into(),
        "helper".into(),
        sender,
        Box::new(move |guard| {
            spawn_child(path, start, guard)?;
            guard.fail_reap_once = true;
            Ok(())
        }),
    )
    .unwrap();
    assert!(matches!(next(&mut receiver).await.kind, Kind::Started));
    handle.close();
    let mut delayed = false;
    loop {
        let event = next(&mut receiver).await;
        match event.kind {
            Kind::CleanupDelayed => {
                assert!(event._lifetime.is_none());
                delayed = true;
            }
            Kind::Exited { error, .. } => {
                assert!(
                    delayed,
                    "settlement must follow the retained cleanup notice"
                );
                assert!(error.is_none());
                assert!(event._lifetime.is_some());
                break;
            }
            Kind::Output { .. } => {}
            other => panic!("unexpected cleanup event {other:?}"),
        }
    }
}

#[tokio::test]
async fn cancellation_before_spawn_settlement_still_reaps_late_child() {
    let root = Root::new();
    std::fs::write(root.path().join("park-helper"), b"").unwrap();
    let start = fixture(root.path(), false);
    let path = root.path().to_owned();
    let (release, gate) = blocking::channel();
    let (sender, mut receiver) = mpsc::channel(16);
    let handle = spawn_with_launcher(
        1,
        "generation".into(),
        "helper".into(),
        sender,
        Box::new(move |guard| {
            gate.recv().unwrap();
            spawn_child(path, start, guard)
        }),
    )
    .unwrap();
    handle.close();
    release.send(()).unwrap();
    let terminal = next(&mut receiver).await;
    assert!(matches!(terminal.kind, Kind::Exited { .. }));
    assert!(terminal._lifetime.is_some());
}

#[tokio::test]
async fn full_ordinary_slots_cannot_hold_final_cleanup() {
    let root = Root::new();
    std::fs::write(root.path().join("flood-output"), b"").unwrap();
    let (sender, mut receiver) = mpsc::channel(16);
    let handle = spawn(
        1,
        "generation".into(),
        "helper".into(),
        root.path().to_owned(),
        fixture(root.path(), false),
        sender,
    )
    .unwrap();
    let started = next(&mut receiver).await;
    let output1 = next(&mut receiver).await;
    let output2 = next(&mut receiver).await;
    assert!(matches!(started.kind, Kind::Started));
    assert!(matches!(output1.kind, Kind::Output { .. }));
    assert!(matches!(output2.kind, Kind::Output { .. }));
    handle.close();
    let terminal = next(&mut receiver).await;
    assert!(matches!(terminal.kind, Kind::Exited { .. }));
    assert!(terminal._lifetime.is_some());
    drop((started, output1, output2));
}

#[tokio::test]
async fn blocked_stdin_times_out_with_unknown_delivery_and_confirmed_cleanup() {
    let root = Root::new();
    std::fs::write(root.path().join("park-helper"), b"").unwrap();
    let (sender, mut receiver) = mpsc::channel(128);
    let handle = spawn(
        1,
        "generation".into(),
        "helper".into(),
        root.path().to_owned(),
        fixture(root.path(), false),
        sender,
    )
    .unwrap();
    assert!(matches!(next(&mut receiver).await.kind, Kind::Started));
    let deadline = Instant::now() + Duration::from_secs(8);
    let pid: u32 = loop {
        if let Ok(value) = std::fs::read_to_string(root.path().join("helper.pid")) {
            break value.parse().unwrap();
        }
        assert!(Instant::now() < deadline, "helper did not publish its PID");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!raw.is_null());
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut accepted = 0;
    handle
        .write(
            "blocked".into(),
            vec![b'x'; super::super::MAX_IO_BYTES],
            false,
        )
        .unwrap();
    let began = Instant::now();
    let mut unknown = false;
    loop {
        let event = next(&mut receiver).await;
        match event.kind {
            Kind::Written { result, .. } => match result {
                Ok(_) => {
                    accepted += 1;
                    assert!(accepted <= 32, "helper pipe did not reach backpressure");
                    handle
                        .write(
                            "blocked".into(),
                            vec![b'x'; super::super::MAX_IO_BYTES],
                            false,
                        )
                        .unwrap();
                }
                Err(error) => {
                    assert_eq!(error.code, ErrorCode::OutcomeUnknown);
                    unknown = true;
                }
            },
            Kind::Exited { error, write, .. } => {
                if let Some((_, result)) = write {
                    assert_eq!(result.unwrap_err().code, ErrorCode::OutcomeUnknown);
                    unknown = true;
                }
                assert!(unknown);
                assert_eq!(error.unwrap().code, ErrorCode::OutcomeUnknown);
                assert!(event._lifetime.is_some());
                break;
            }
            Kind::Output { .. } => {}
            other => panic!("unexpected blocked input event {other:?}"),
        }
    }
    assert!(began.elapsed() >= WRITE_TIMEOUT);
    assert_eq!(
        unsafe { WaitForSingleObject(process.as_raw_handle(), 0) },
        WAIT_OBJECT_0
    );
}

#[test]
fn runtime_shutdown_keeps_reaper_ownership_until_child_exits() {
    let root = Root::new();
    std::fs::write(root.path().join("park-helper"), b"").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (handle, receiver, process) = runtime.block_on(async {
        let (sender, mut receiver) = mpsc::channel(16);
        let handle = spawn(
            1,
            "generation".into(),
            "helper".into(),
            root.path().to_owned(),
            fixture(root.path(), false),
            sender,
        )
        .unwrap();
        assert!(matches!(next(&mut receiver).await.kind, Kind::Started));
        let pid: u32 = loop {
            if let Ok(value) = std::fs::read_to_string(root.path().join("helper.pid")) {
                break value.parse().unwrap();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!raw.is_null());
        (handle, receiver, unsafe {
            OwnedHandle::from_raw_handle(raw)
        })
    });
    drop(runtime);
    assert_eq!(
        unsafe { WaitForSingleObject(process.as_raw_handle(), 8000) },
        WAIT_OBJECT_0,
        "runtime shutdown left the helper running"
    );
    drop((handle, receiver));
}

#[test]
fn cancelled_supervisor_keeps_failed_reap_result_owned() {
    let root = Root::new();
    std::fs::write(root.path().join("park-helper"), b"").unwrap();
    let path = root.path().to_owned();
    let start = fixture(root.path(), false);
    let reaped = Arc::new(AtomicBool::new(false));
    let witness = reaped.clone();
    let (ready, at_reap) = blocking::channel();
    let (release, gate) = blocking::channel();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (handle, receiver, process) = runtime.block_on(async {
        let (sender, mut receiver) = mpsc::channel(16);
        let handle = spawn_with_launcher(
            1,
            "generation".into(),
            "helper".into(),
            sender,
            Box::new(move |guard| {
                spawn_child(path, start, guard)?;
                guard.fail_reap_once = true;
                guard.reap_gate = Some((ready, gate));
                guard.reaped = Some(reaped);
                Ok(())
            }),
        )
        .unwrap();
        assert!(matches!(next(&mut receiver).await.kind, Kind::Started));
        let deadline = Instant::now() + Duration::from_secs(8);
        let pid: u32 = loop {
            if let Ok(value) = std::fs::read_to_string(root.path().join("helper.pid")) {
                break value.parse().unwrap();
            }
            assert!(Instant::now() < deadline, "helper did not publish its PID");
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!raw.is_null());
        (handle, receiver, unsafe {
            OwnedHandle::from_raw_handle(raw)
        })
    });
    handle.close();
    at_reap.recv_timeout(Duration::from_secs(8)).unwrap();
    // The blocking reap returns its RAII result after Tokio has abandoned the
    // supervisor. Dropping that result must queue the child for the reaper.
    runtime.shutdown_background();
    release.send(()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while !witness.load(Ordering::Acquire) {
        assert!(
            std::time::Instant::now() < deadline,
            "failed reap result was not handed to the background reaper"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        unsafe { WaitForSingleObject(process.as_raw_handle(), 0) },
        WAIT_OBJECT_0
    );
    drop((handle, receiver));
}
