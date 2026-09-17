// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::terminal::{
    proposal::{Delivery, DeliveryState, Text},
    tests::session,
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn proposal_input(text: &str) -> (Input, Delivery) {
    let delivery = Delivery::queued();
    (
        Input {
            bytes: text.as_bytes().to_vec(),
            delivery: Some(delivery.clone()),
        },
        delivery,
    )
}

#[test]
fn cancellation_before_claim_sends_nothing() {
    let (input, delivery) = proposal_input("echo approved");
    assert!(delivery.cancel());
    input
        .deliver(|_| panic!("cancelled input reached writer"))
        .unwrap();
    assert_eq!(delivery.state(), DeliveryState::Cancelled);
    assert!(!delivery.cancel());
}

#[test]
fn acknowledgment_requires_every_byte_and_claim_prevents_cancellation() {
    let (input, delivery) = proposal_input(&"x".repeat(WRITE_CHUNK + 1));
    let mut written = 0;
    input
        .deliver(|bytes| {
            assert_eq!(delivery.state(), DeliveryState::Writing);
            assert!(!delivery.cancel());
            written += bytes.len();
            Ok(())
        })
        .unwrap();
    assert_eq!(written, WRITE_CHUNK + 1);
    assert_eq!(delivery.state(), DeliveryState::Delivered);
    drop(input);
    assert_eq!(delivery.state(), DeliveryState::Delivered);
}

#[test]
fn abandoned_queued_input_is_cancelled_and_partial_failure_is_unknown() {
    let (queued, delivery) = proposal_input("queued");
    drop(queued);
    assert_eq!(delivery.state(), DeliveryState::Cancelled);
    let (input, delivery) = proposal_input(&"x".repeat(WRITE_CHUNK + 1));
    let mut calls = 0;
    assert!(
        input
            .deliver(|_| {
                calls += 1;
                if calls == 1 {
                    Ok(())
                } else {
                    Err(io::ErrorKind::BrokenPipe.into())
                }
            })
            .is_err()
    );
    drop(input);
    assert_eq!(calls, 2);
    assert_eq!(delivery.state(), DeliveryState::OutcomeUnknown);
}

#[test]
fn native_input_intents_and_mode_changes_cancel_queued_proposals() {
    let mut session = session(12, 3);
    for input in 0..4 {
        let delivery = Delivery::queued();
        session.proposal_deliveries.push(delivery.clone());
        let generation = session.input_generation();
        match input {
            0 => {
                session.send_text("");
            }
            1 => {
                session.send_key(crate::input::KeyStroke::char('x'));
            }
            2 => {
                session.undo_sent_text();
            }
            _ => {
                session.send_mouse(
                    crate::input::PointerEvent {
                        kind: crate::input::PointerEventKind::Moved,
                        column: 0,
                        row: 0,
                        modifiers: crate::input::Modifiers::NONE,
                    },
                    0,
                    0,
                );
            }
        }
        assert_eq!(session.input_generation(), generation + 1);
        assert_eq!(delivery.state(), DeliveryState::Cancelled);
    }
    let delivery = Delivery::queued();
    session.proposal_deliveries.push(delivery.clone());
    let signature = session.proposal_input_signature();
    session.feed(b"\x1b[?2004h");
    assert_ne!(session.proposal_input_signature(), signature);
    assert_eq!(delivery.state(), DeliveryState::Cancelled);
    let delivery = Delivery::queued();
    session.proposal_deliveries.push(delivery.clone());
    drop(session);
    assert_eq!(delivery.state(), DeliveryState::Cancelled);
}

#[test]
fn refused_proposal_preserves_review_scroll_and_attention() {
    let mut session = session(12, 3);
    session.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n");
    session.begin_review();
    session.scroll_back(2);
    let scroll = session.scroll();
    let unread = session.unread_activity;
    let revision = session.revision();
    assert!(
        session
            .enqueue_proposal(&Text::new("literal").unwrap())
            .is_err()
    );
    assert!(session.review.is_some());
    assert_eq!(session.scroll(), scroll);
    assert_eq!(session.unread_activity, unread);
    assert_eq!(session.revision(), revision);
    session.exit = Some(Some(0));
    assert!(
        session
            .enqueue_proposal(&Text::new("literal").unwrap())
            .is_err()
    );
}

fn wait_until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "PTY condition timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn real_pty_inserts_without_submitting_and_preserves_native_view() {
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let captured = output.clone();
    let pty = Pty::spawn(OsStr::new("/bin/sh"), &["-c".into(),
        "stty -echo; printf READY; IFS= read -r line; printf 'SUBMITTED:%s' \"$line\"; IFS= read -r hold".into()],
        Path::new("/tmp"), 40, 10, move |event| {
            if let PtyEvent::Output(bytes) = event { captured.lock().unwrap().extend(bytes); }
        }).unwrap();
    wait_until(|| {
        output
            .lock()
            .unwrap()
            .windows(5)
            .any(|bytes| bytes == b"READY")
    });
    let mut session = session(40, 10);
    session.pty = Some(pty);
    session.feed(b"history\r\n");
    session.begin_review();
    let revision = session.revision();
    let attention = session.unread_activity;
    let delivery = session
        .enqueue_proposal(&Text::new("echo 界").unwrap())
        .unwrap();
    wait_until(|| delivery.state() == DeliveryState::Delivered);
    assert_eq!(&*output.lock().unwrap(), b"READY");
    assert!(session.review.is_some());
    assert_eq!(session.revision(), revision);
    assert_eq!(session.unread_activity, attention);
    // Only this independent native input supplies the line terminator.
    assert!(session.pty.as_ref().unwrap().write(b"\n".to_vec()));
    wait_until(|| String::from_utf8_lossy(&output.lock().unwrap()).contains("SUBMITTED:echo 界"));
}

#[test]
fn real_pty_receives_only_generated_paste_framing() {
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let captured = output.clone();
    // Raw echo sends exactly the bytes received, including generated brackets.
    let pty = Pty::spawn(
        OsStr::new("/bin/sh"),
        &["-c".into(), "stty raw -echo; printf READY; cat".into()],
        Path::new("/tmp"),
        40,
        10,
        move |event| {
            if let PtyEvent::Output(bytes) = event {
                captured.lock().unwrap().extend(bytes);
            }
        },
    )
    .unwrap();
    wait_until(|| {
        output
            .lock()
            .unwrap()
            .windows(5)
            .any(|bytes| bytes == b"READY")
    });
    let delivery = pty
        .enqueue_proposal(&Text::new("界 e\u{301}").unwrap(), true)
        .unwrap();
    let expected = "READY\x1b[200~界 e\u{301}\x1b[201~".as_bytes();
    wait_until(|| output.lock().unwrap().len() >= expected.len());
    assert_eq!(&*output.lock().unwrap(), expected);
    assert_eq!(delivery.state(), DeliveryState::Delivered);
}

#[test]
fn exited_retained_terminal_cancels_queued_input_before_releasing_pty() {
    use crate::terminal::{TerminalOutput, TerminalSessions};
    let mut sessions = TerminalSessions::new();
    let id = sessions.insert_test_session(12, 3);
    let delivery = Delivery::queued();
    sessions
        .sessions
        .get_mut(&id)
        .unwrap()
        .proposal_deliveries
        .push(delivery.clone());
    sessions.apply(TerminalOutput::Exited { id, code: Some(0) });
    assert_eq!(delivery.state(), DeliveryState::Cancelled);
    assert!(!sessions.sessions.get(&id).unwrap().live());
    assert!(sessions.sessions.contains_key(&id));
}
