// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::time::{Duration, Instant};

const FIXTURE: &str = "terminal::pty::tests::native_console_fixture";

fn request(arguments: &[&str]) -> TerminalRequest {
    TerminalRequest {
        program: std::env::current_exe().unwrap().into_os_string(),
        arguments: ["--exact", FIXTURE, "--ignored", "--nocapture"]
            .into_iter()
            .chain(arguments.iter().copied())
            .map(str::to_owned)
            .collect(),
        directory: std::env::temp_dir(),
        label: "Prepared native terminal".into(),
    }
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "bounded native terminal cleanup");
        std::thread::yield_now();
    }
}

struct Settled(mpsc::Sender<()>);
impl Drop for Settled {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[test]
fn native_pending_terminal_stays_unpublished_until_install_and_keeps_literal_arguments() {
    let _guard = pending_test_guard();
    let mut sessions = TerminalSessions::new();
    let output = sessions.take_events().unwrap();
    let preparation = sessions
        .prepare_open(
            request(&[
                "probe-two words",
                "probe-$(must-not-run)",
                "probe-猫",
                "probe-",
            ]),
            100,
            10,
        )
        .unwrap();
    let id = preparation.cancellation().id;
    let pending = preparation.spawn().unwrap();
    assert!(sessions.get(id).is_none());
    assert!(output.try_recv().is_err());

    assert_eq!(sessions.install_prepared(pending).unwrap(), id);
    wait_until(|| {
        while let Ok(message) = output.try_recv() {
            sessions.apply(message);
        }
        let text = sessions.get(id).unwrap().plain_text();
        [
            "ARGUMENT:two words:END",
            "ARGUMENT:$(must-not-run):END",
            "ARGUMENT:猫:END",
            "ARGUMENT::END",
        ]
        .iter()
        .all(|value| text.contains(value))
    });
    assert!(sessions.close(id));
}

#[test]
fn native_pending_cancellation_retains_lease_through_complete_conpty_cleanup() {
    let _guard = pending_test_guard();
    let mut sessions = TerminalSessions::new();
    let mut preparation = sessions.prepare_open(request(&[]), 80, 24).unwrap();
    let cancellation = preparation.cancellation();
    let (finished, completion) = mpsc::channel();
    preparation.retain_until_settled(Box::new(Settled(finished)));
    let pending = preparation.spawn().unwrap();
    let child = pending.session.as_ref().unwrap().pty.as_ref().unwrap();
    assert!(!child.unpublished_completed().unwrap());
    cancellation.cancel();
    drop(pending);
    completion.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(sessions.sessions.is_empty());
    assert!(sessions.events.0.state.lock().unwrap().sessions.is_empty());
}

#[test]
fn native_pending_capacity_is_eight_and_install_transfers_ownership() {
    let _guard = pending_test_guard();
    let mut sessions = TerminalSessions::new();
    let reserved = (0..MAX_PENDING)
        .map(|_| sessions.prepare_open(request(&[]), 80, 24).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        sessions
            .prepare_open(request(&[]), 80, 24)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    drop(reserved);

    let mut releases = Vec::new();
    for _ in 0..MAX_PENDING {
        let preparation = sessions.prepare_open(request(&[]), 80, 24).unwrap();
        let cancellation = preparation.cancellation();
        let pending = preparation.spawn().unwrap();
        releases.push(
            pending
                .session
                .as_ref()
                .unwrap()
                .pty
                .as_ref()
                .unwrap()
                .hold_cleanup_for_test(),
        );
        cancellation.cancel();
        drop(pending);
    }
    assert_eq!(
        sessions
            .prepare_open(request(&[]), 80, 24)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    for release in releases {
        release();
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut recovered = Vec::new();
    while recovered.len() < MAX_PENDING {
        match sessions.prepare_open(request(&[]), 80, 24) {
            Ok(preparation) => recovered.push(preparation),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "pending capacity recovery");
                std::thread::yield_now();
            }
            Err(error) => panic!("unexpected capacity error: {error}"),
        }
    }
    drop(recovered);

    let preparation = sessions.prepare_open(request(&[]), 80, 24).unwrap();
    let cancellation = preparation.cancellation();
    let pending = preparation.spawn().unwrap();
    let id = sessions.install_prepared(pending).unwrap();
    cancellation.cancel();
    assert!(sessions.get(id).is_some_and(TerminalSession::live));
    assert!(sessions.close(id));
}

#[test]
fn native_pending_terminal_cannot_cross_workspace_ownership() {
    let _guard = pending_test_guard();
    let mut source = TerminalSessions::new();
    let preparation = source.prepare_open(request(&[]), 80, 24).unwrap();
    let pending = preparation.spawn().unwrap();
    let cleanup = pending
        .session
        .as_ref()
        .unwrap()
        .pty
        .as_ref()
        .unwrap()
        .cleanup_waiter();
    let mut destination = TerminalSessions::new();
    assert_eq!(
        destination.install_prepared(pending).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    cleanup();
    assert!(source.sessions.is_empty());
    assert!(destination.sessions.is_empty());
}
