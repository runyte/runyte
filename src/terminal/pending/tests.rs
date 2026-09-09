// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use std::time::{Duration, Instant};

fn fixture(behavior: &str) -> (TestRuntimeRoot, TerminalRequest) {
    let root = TestRuntimeRoot::new("terminal-pending").unwrap();
    let program = root.path().join("helper");
    std::os::unix::fs::symlink(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/fixtures/stand-in"),
        &program,
    )
    .unwrap();
    std::fs::write(root.path().join("helper.behavior"), behavior).unwrap();
    let request = TerminalRequest {
        program: program.into(),
        arguments: vec![],
        directory: root.path().into(),
        label: "Prepared terminal".into(),
    };
    (root, request)
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < until, "bounded terminal cleanup");
        std::thread::sleep(Duration::from_millis(2));
    }
}

struct Finished(mpsc::Sender<()>);
impl Drop for Finished {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[test]
fn pending_terminal_fast_output_is_gated_until_install_and_arguments_remain_literal() {
    let _guard = pending_test_guard();
    let (_root, mut request) = fixture("printf '<%s>\\n' \"$@\"\n");
    request.arguments = vec![
        "two words".into(),
        "$(touch must-not-exist)".into(),
        "猫".into(),
        "".into(),
    ];
    let mut sessions = TerminalSessions::new();
    let output = sessions.take_events().unwrap();
    let preparation = sessions.prepare_open(request, 100, 10).unwrap();
    let cancellation = preparation.cancellation();
    let id = cancellation.id;
    let pending = std::thread::spawn(move || preparation.spawn().unwrap())
        .join()
        .unwrap();
    assert!(sessions.get(id).is_none());
    assert!(output.try_recv().is_err());
    assert!(!sessions.events.0.state.lock().unwrap().sessions[&id].active);
    assert_eq!(sessions.install_prepared(pending).unwrap(), id);
    // Owner stop after handoff must leave the native terminal and its output.
    cancellation.cancel();
    wait_until(|| {
        while let Ok(message) = output.try_recv() {
            sessions.apply(message);
        }
        !sessions.get(id).unwrap().live()
    });
    let text = sessions.get(id).unwrap().plain_text();
    for value in ["<two words>", "<$(touch must-not-exist)>", "<猫>", "<>"] {
        assert!(text.contains(value), "{text:?}");
    }
}

#[test]
fn pending_terminal_failed_and_cancelled_preparations_release_the_gate_and_lease() {
    let _guard = pending_test_guard();
    let (root, mut request) = fixture("printf 'unexpected'\nsleep 30\n");
    let mut sessions = TerminalSessions::new();
    let output = sessions.take_events().unwrap();
    request.program = root.path().join("missing").into();
    let preparation = sessions.prepare_open(request.clone(), 80, 24).unwrap();
    let id = preparation.cancellation().id;
    assert!(preparation.spawn().is_err());
    assert!(
        !sessions
            .events
            .0
            .state
            .lock()
            .unwrap()
            .sessions
            .contains_key(&id)
    );
    request.program = root.path().join("helper").into();
    let mut preparation = sessions.prepare_open(request, 80, 24).unwrap();
    let cancellation = preparation.cancellation();
    let (finished, completion) = mpsc::channel();
    preparation.retain_until_settled(Box::new(Finished(finished)));
    cancellation.cancel();
    assert!(preparation.spawn().is_err());
    completion.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(output.try_recv().is_err());
    assert!(sessions.sessions.is_empty());
}

#[test]
fn pending_terminal_cleanup_kills_descendants_after_the_unreaped_leader_exits() {
    let _guard = pending_test_guard();
    // Preserve the descendant across the controlling terminal's leader-exit
    // hangup, so only explicit pending cleanup can end it.
    let (root, request) = fixture(
        "trap '' HUP\nsleep 30 &\nprintf '%s' \"$!\" > descendant\nprintf 'ready'\nexit 0\n",
    );
    let mut sessions = TerminalSessions::new();
    let output = sessions.take_events().unwrap();
    let mut preparation = sessions.prepare_open(request, 80, 24).unwrap();
    let cancellation = preparation.cancellation();
    let (finished, completion) = mpsc::channel();
    preparation.retain_until_settled(Box::new(Finished(finished)));
    let pending = preparation.spawn().unwrap();
    let child = pending.session.as_ref().unwrap().pty.as_ref().unwrap();
    // A non-reaping observation gives a deterministic exited-leader barrier.
    wait_until(|| child.unpublished_completed().unwrap());
    let descendant: i32 = std::fs::read_to_string(root.path().join("descendant"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(unsafe { libc::kill(descendant, 0) } == 0);
    cancellation.cancel();
    drop(pending);
    completion.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(output.try_recv().is_err());
    // The group is gone or contains only the killed descendant's zombie;
    // descriptor liveness proves it cannot keep the reader or child alive.
    #[cfg(target_os = "linux")]
    wait_until(
        || match std::fs::read_to_string(format!("/proc/{descendant}/stat")) {
            Ok(stat) => stat.rsplit_once(')').unwrap().1.starts_with(" Z"),
            Err(_) => true,
        },
    );
    #[cfg(target_os = "macos")]
    wait_until(|| {
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>();
        // Include zombies: either disappearance or SZOMB proves termination.
        let read = unsafe {
            libc::proc_pidinfo(
                descendant,
                libc::PROC_PIDTBSDINFO,
                1,
                info.as_mut_ptr().cast(),
                size as libc::c_int,
            )
        };
        read <= 0
            || (read == size as i32 && unsafe { info.assume_init() }.pbi_status == libc::SZOMB)
    });
    assert!(sessions.events.0.state.lock().unwrap().sessions.is_empty());
}

#[test]
fn pending_terminal_cancel_releases_reader_accounting_while_an_external_slave_stays_open() {
    let _guard = pending_test_guard();
    let (root, request) = fixture("tty > slave\nprintf ready\nsleep 30\n");
    let mut sessions = TerminalSessions::new();
    let output = sessions.take_events().unwrap();
    let mut preparation = sessions.prepare_open(request, 80, 24).unwrap();
    let (finished, completion) = mpsc::channel();
    preparation.retain_until_settled(Box::new(Finished(finished)));
    let pending = preparation.spawn().unwrap();
    let slave_path = root.path().join("slave");
    wait_until(|| std::fs::read_to_string(&slave_path).is_ok_and(|path| path.ends_with('\n')));
    let slave_path = std::fs::read_to_string(slave_path).unwrap();
    // Retain the slave outside the child group, as an escaped descendant
    // could. No extra child is spawned and no unrelated PID is signalled.
    let slave = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(slave_path.trim())
        .unwrap();
    assert!(completion.try_recv().is_err());
    drop(pending);
    // The accounting lease covers both reap and the gated reader descriptor.
    // Completion cannot depend on this unrelated slave owner closing first.
    completion.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(output.try_recv().is_err());
    assert!(sessions.events.0.state.lock().unwrap().sessions.is_empty());
    drop(slave);
}

#[test]
fn pending_terminal_refuses_unbounded_geometry_and_cross_workspace_installation() {
    let _guard = pending_test_guard();
    let (_root, request) = fixture("sleep 30\n");
    let mut sessions = TerminalSessions::new();
    assert!(
        sessions
            .prepare_open(request.clone(), usize::MAX, 2)
            .is_err()
    );
    assert!(sessions.prepare_open(request.clone(), 1024, 1024).is_err());
    assert!(sessions.prepare_open(request.clone(), 1, 32768).is_err());
    let mut preparation = sessions.prepare_open(request.clone(), 80, 24).unwrap();
    let (finished, completion) = mpsc::channel();
    preparation.retain_until_settled(Box::new(Finished(finished)));
    let cancellation = preparation.cancellation();
    let pending = preparation.spawn().unwrap();
    let mut other = TerminalSessions::new();
    assert!(other.install_prepared(pending).is_err());
    assert!(cancellation.is_cancelled());
    assert!(other.sessions.is_empty());
    assert!(sessions.sessions.is_empty());
    completion.recv_timeout(Duration::from_secs(5)).unwrap();
    let reserved = (0..MAX_PENDING)
        .map(|_| sessions.prepare_open(request.clone(), 80, 24).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        sessions
            .prepare_open(request.clone(), 80, 24)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    drop(reserved);
    let preparation = sessions.prepare_open(request, 80, 24).unwrap();
    drop(sessions);
    assert_eq!(
        preparation.spawn().unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}
