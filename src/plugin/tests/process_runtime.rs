// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::os::unix::fs::symlink;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "runyte-helper-runtime-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(behavior: &str, capture_stderr: bool) -> (Root, Start) {
    let dir = Root::new();
    let executable = dir.path().join("helper");
    symlink(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/fixtures/stand-in"),
        &executable,
    )
    .unwrap();
    std::fs::write(dir.path().join("helper.behavior"), behavior).unwrap();
    let start = Start {
        label: "test helper".into(),
        executable: executable.to_string_lossy().into(),
        args: Vec::new(),
        cwd: None,
        capture_stderr,
    };
    (dir, start)
}
async fn event(receiver: &mut mpsc::Receiver<plugin::Event>) -> Event {
    let event = tokio::time::timeout(Duration::from_secs(8), receiver.recv())
        .await
        .expect("bounded helper completion")
        .expect("event channel");
    assert_eq!(event.plugin, 1);
    let plugin::ClientMessage::Process(event) = event.result.unwrap() else {
        panic!("process event")
    };
    assert_eq!(event.generation, "generation");
    assert_eq!(event.process, "helper");
    event
}
fn launch(dir: &Root, start: Start) -> (Handle, mpsc::Receiver<plugin::Event>) {
    let (sender, receiver) = mpsc::channel(128);
    (
        spawn(
            1,
            "generation".into(),
            "helper".into(),
            dir.path().into(),
            start,
            sender,
        )
        .unwrap(),
        receiver,
    )
}

#[tokio::test]
async fn binary_output_and_fast_exit_are_observed_without_a_poll_timer() {
    let (dir, start) = fixture("printf '\\000hello\\377'\nexit 7\n", false);
    let (_handle, mut receiver) = launch(&dir, start);
    assert!(matches!(event(&mut receiver).await.kind, Kind::Started));
    let mut bytes = Vec::new();
    loop {
        match event(&mut receiver).await.kind {
            Kind::Output {
                stream: Stream::Stdout,
                bytes: part,
            } => bytes.extend(part),
            Kind::Exited {
                code,
                signal,
                error,
                stdout,
                ..
            } => {
                bytes.extend(stdout);
                assert_eq!(code, Some(7));
                assert_eq!(signal, None);
                assert!(error.is_none());
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(bytes, b"\0hello\xff");
}

#[tokio::test]
async fn actual_stdin_write_and_eof_ack_precede_terminal_event() {
    let (dir, start) = fixture("cat\n", false);
    let (handle, mut receiver) = launch(&dir, start);
    assert!(matches!(event(&mut receiver).await.kind, Kind::Started));
    let input = b"binary\0input\xff".to_vec();
    handle.write("write-1".into(), input.clone(), true).unwrap();
    assert_eq!(
        handle
            .write("write-2".into(), vec![], true)
            .unwrap_err()
            .code,
        ErrorCode::Busy
    );
    let mut output = Vec::new();
    let mut written = false;
    loop {
        match event(&mut receiver).await.kind {
            Kind::Output { bytes, .. } => output.extend(bytes),
            Kind::Written { request, result } => {
                assert_eq!(request, "write-1");
                let result = result.unwrap();
                assert_eq!(result.written, input.len());
                assert!(result.eof);
                written = true;
            }
            Kind::Exited {
                code,
                error,
                stdout,
                write,
                ..
            } => {
                output.extend(stdout);
                if let Some((request, result)) = write {
                    assert_eq!(request, "write-1");
                    let result = result.unwrap();
                    assert_eq!(result.written, input.len());
                    assert!(result.eof);
                    written = true;
                }
                assert!(written);
                assert_eq!(code, Some(0));
                assert!(error.is_none());
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(output, input);
}

#[tokio::test]
async fn stderr_is_discarded_unless_capture_is_explicit() {
    for capture in [false, true] {
        let (dir, start) = fixture("printf 'private' >&2\nprintf 'public'\n", capture);
        let (_handle, mut receiver) = launch(&dir, start);
        let mut stderr = Vec::new();
        loop {
            match event(&mut receiver).await.kind {
                Kind::Output {
                    stream: Stream::Stderr,
                    bytes,
                } => stderr.extend(bytes),
                Kind::Exited { stderr: tail, .. } => {
                    stderr.extend(tail);
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(stderr, if capture { b"private".as_slice() } else { &[] });
    }
}

#[tokio::test]
async fn startup_failure_and_outside_directory_do_not_emit_started_or_private_diagnostics() {
    let (dir, mut start) = fixture("exit 0\n", false);
    start.executable = dir
        .path()
        .join("private-missing-program")
        .to_string_lossy()
        .into();
    start.args = vec!["secret-canary".into()];
    let (_handle, mut receiver) = launch(&dir, start.clone());
    let terminal = event(&mut receiver).await;
    let Kind::Exited {
        error: Some(error), ..
    } = terminal.kind
    else {
        panic!("failed startup")
    };
    assert_eq!(error.code, ErrorCode::Unavailable);
    assert!(!error.message.contains("private"));
    assert!(!error.message.contains("canary"));
    assert!(terminal._lifetime.is_some());
    start.cwd = Some("/".into());
    let (_handle, mut receiver) = launch(&dir, start);
    let Kind::Exited {
        error: Some(error), ..
    } = event(&mut receiver).await.kind
    else {
        panic!("outside cwd")
    };
    assert_eq!(error.code, ErrorCode::InvalidArgument);
}

#[tokio::test]
async fn close_and_handle_drop_clean_up_before_final_delivery() {
    for drop_handle in [false, true] {
        let (dir, start) = fixture("sleep 30 &\nwait\n", false);
        let (handle, mut receiver) = launch(&dir, start);
        assert!(matches!(event(&mut receiver).await.kind, Kind::Started));
        if drop_handle {
            drop(handle);
        } else {
            handle.close();
            handle.close();
        }
        loop {
            if let Kind::Exited { signal, .. } = event(&mut receiver).await.kind {
                assert_eq!(signal, Some(libc::SIGKILL));
                break;
            }
        }
    }
}

#[tokio::test]
async fn exited_leader_descendant_cannot_hold_output_or_cleanup_open() {
    let (dir, start) = fixture("sleep 30 &\nprintf 'done'\nexit 0\n", false);
    let (_handle, mut receiver) = launch(&dir, start);
    let start = Instant::now();
    let mut output = Vec::new();
    loop {
        match event(&mut receiver).await.kind {
            Kind::Output { bytes, .. } => output.extend(bytes),
            Kind::Exited { code, stdout, .. } => {
                output.extend(stdout);
                assert_eq!(code, Some(0));
                break;
            }
            _ => {}
        }
    }
    assert_eq!(output, b"done");
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn cancellation_before_spawn_settlement_still_receives_final_cleanup() {
    let (dir, start) = fixture("sleep 30\n", false);
    let (handle, mut receiver) = launch(&dir, start);
    handle.close();
    loop {
        if matches!(event(&mut receiver).await.kind, Kind::Exited { .. }) {
            break;
        }
    }
}

#[tokio::test]
async fn blocked_stdin_has_finite_unknown_delivery_and_reaps_the_helper() {
    let (dir, start) = fixture("sleep 30\n", false);
    let (handle, mut receiver) = launch(&dir, start);
    assert!(matches!(event(&mut receiver).await.kind, Kind::Started));
    handle
        .write(
            "blocked".into(),
            vec![b'x'; super::super::MAX_IO_BYTES],
            false,
        )
        .unwrap();
    let started = Instant::now();
    let mut unknown = false;
    let mut accepted = 0;
    loop {
        match event(&mut receiver).await.kind {
            Kind::Written { request, result } => {
                assert_eq!(request, "blocked");
                match result {
                    Ok(_) => {
                        accepted += 1;
                        assert!(
                            accepted <= 17,
                            "fixture pipe exceeded the bounded fill budget"
                        );
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
                }
            }
            Kind::Exited {
                signal,
                error,
                write,
                ..
            } => {
                if let Some((request, result)) = write {
                    assert_eq!(request, "blocked");
                    assert_eq!(result.unwrap_err().code, ErrorCode::OutcomeUnknown);
                    unknown = true;
                }
                assert!(unknown);
                assert_eq!(signal, Some(libc::SIGKILL));
                assert_eq!(error.unwrap().code, ErrorCode::OutcomeUnknown);
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(started.elapsed() >= WRITE_TIMEOUT);
}

#[tokio::test]
async fn full_ordinary_slots_do_not_block_close_or_reserved_terminal_delivery() {
    let (dir, start) = fixture(
        "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done\n",
        false,
    );
    let (handle, mut receiver) = launch(&dir, start);
    let started = event(&mut receiver).await;
    assert!(matches!(started.kind, Kind::Started));
    // Hold every ordinary permit outside the channel, just as a busy host can.
    let output1 = event(&mut receiver).await;
    let output2 = event(&mut receiver).await;
    assert!(matches!(output1.kind, Kind::Output { .. }));
    assert!(matches!(output2.kind, Kind::Output { .. }));
    handle.close();
    let final_event = event(&mut receiver).await;
    assert!(matches!(
        final_event.kind,
        Kind::Exited {
            signal: Some(libc::SIGKILL),
            ..
        }
    ));
    assert!(final_event._lifetime.is_some());
    drop((started, output1, output2));
}

#[test]
fn runtime_shutdown_keeps_cleanup_ownership_until_the_leader_is_reaped() {
    let (dir, start) = fixture("printf '%s\\n' $$\nsleep 30 &\nwait\n", false);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (handle, pid) = runtime.block_on(async {
        let (handle, mut receiver) = launch(&dir, start);
        assert!(matches!(event(&mut receiver).await.kind, Kind::Started));
        let Kind::Output { bytes, .. } = event(&mut receiver).await.kind else {
            panic!("leader identity")
        };
        let pid: i32 = std::str::from_utf8(&bytes).unwrap().trim().parse().unwrap();
        // Keep the host endpoint alive while runtime shutdown cancels the
        // supervisor itself. Guard Drop, not Handle Drop, must own this reap.
        (handle, (pid, receiver))
    });
    let (pid, receiver) = pid;
    drop(runtime);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        // SAFETY: signal zero only observes this known child and never reaps it.
        if unsafe { libc::kill(pid, 0) } == -1 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "supervisor Drop abandoned its child"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    drop((handle, receiver));
}

#[test]
fn missing_async_runtime_is_a_typed_refusal_before_helper_admission() {
    let (dir, start) = fixture("exit 0\n", false);
    let (sender, _receiver) = mpsc::channel(1);
    let error = spawn(
        1,
        "generation".into(),
        "helper".into(),
        dir.path().into(),
        start,
        sender,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Unavailable);
}

#[tokio::test]
async fn startup_deadline_refuses_late_spawn_but_retains_cleanup_ownership() {
    let (dir, start) = fixture("sleep 30\n", false);
    let root = dir.path().to_owned();
    let (release, gate) = blocking::channel();
    let (sender, mut receiver) = mpsc::channel(8);
    let _handle = spawn_with_launcher(
        1,
        "generation".into(),
        "helper".into(),
        sender,
        Box::new(move |guard| {
            let _ = gate.recv();
            spawn_child(root, start, guard)
        }),
    )
    .unwrap();
    let failed = event(&mut receiver).await;
    let Kind::StartFailed { error } = &failed.kind else {
        panic!("startup must refuse before the gate opens")
    };
    assert_eq!(error.code, ErrorCode::Unavailable);
    assert!(failed._lifetime.is_none());
    assert!(receiver.try_recv().is_err());
    release.send(()).unwrap();
    let terminal = event(&mut receiver).await;
    assert!(matches!(
        terminal.kind,
        Kind::Exited {
            signal: Some(libc::SIGKILL),
            ..
        }
    ));
    assert!(terminal._lifetime.is_some());
}

#[tokio::test]
async fn natural_exit_carries_completed_eof_when_every_ordinary_permit_is_held() {
    let (dir, start) = fixture(
        "printf A\nread first\nprintf B\nread second\nexit 0\n",
        false,
    );
    let (handle, mut receiver) = launch(&dir, start);
    let started = event(&mut receiver).await;
    let first_output = event(&mut receiver).await;
    assert!(matches!(started.kind, Kind::Started));
    assert!(matches!(first_output.kind, Kind::Output { .. }));
    handle.write("line".into(), b"\n".to_vec(), false).unwrap();
    let written = event(&mut receiver).await;
    assert!(matches!(written.kind, Kind::Written { .. }));
    handle.write("eof".into(), Vec::new(), true).unwrap();
    let final_event = event(&mut receiver).await;
    let Kind::Exited {
        code,
        write: Some((request, result)),
        stdout,
        ..
    } = final_event.kind
    else {
        panic!("reserved terminal write settlement")
    };
    assert_eq!(code, Some(0));
    assert_eq!(request, "eof");
    let result = result.unwrap();
    assert_eq!(result.written, 0);
    assert!(result.eof);
    assert_eq!(stdout, b"B");
    drop((started, first_output, written));
}

#[tokio::test]
async fn final_drain_deadline_is_shared_and_never_reset_by_slow_bytes() {
    let (mut writer, reader) = tokio::io::duplex(16);
    let producing = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(10)).await;
            if writer.write_all(b"x").await.is_err() {
                break;
            }
        }
    });
    let mut pipe = Some(reader);
    let started = Instant::now();
    let mut remaining = FINAL_DRAIN;
    let (bytes, truncated) = drain(
        &mut pipe,
        &mut remaining,
        started + Duration::from_millis(45),
    )
    .await;
    producing.abort();
    assert!(truncated);
    assert!(bytes.len() < FINAL_DRAIN);
    assert!(started.elapsed() < Duration::from_millis(250));
}
