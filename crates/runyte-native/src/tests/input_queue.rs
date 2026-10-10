// SPDX-License-Identifier: MPL-2.0
use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent};

fn input(event: Event, frame: u64) -> NativeInput {
    NativeInput {
        attachment: 1,
        event,
        presented: Some(runyte::protocol::FrameId::from_raw(frame).into()),
        presentation_only: false,
        routing_serial: 0,
    }
}
fn key(c: char, frame: u64) -> NativeInput {
    input(
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)),
        frame,
    )
}
fn drag(x: u16, frame: u64) -> NativeInput {
    input(
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: x,
            row: 2,
            modifiers: KeyModifiers::NONE,
        }),
        frame,
    )
}

#[tokio::test]
async fn acknowledgements_do_not_displace_physical_input_and_overflow_stays_visible() {
    let (send, mut receive) = channel(2);
    send.try_send(key('a', 1)).unwrap();
    for frame in 1..100 {
        let mut ack = input(Event::FocusGained, frame);
        ack.presentation_only = true;
        send.try_send(ack).unwrap();
    }
    send.try_send(key('b', 2)).unwrap();
    assert!(matches!(send.try_send(key('c', 3)), Err(SendError::Full)));
    assert!(send.overflowed());
    let a = receive.recv().await.unwrap();
    let b = receive.recv().await.unwrap();
    assert_eq!(a.event, key('a', 1).event);
    assert_eq!(a.presented, key('a', 1).presented);
    assert_eq!(b.event, key('b', 2).event);
    assert_eq!(b.presented, key('b', 2).presented);
    let ack = receive.recv().await.unwrap();
    assert!(ack.presentation_only);
    assert_eq!(ack.presented, input(Event::FocusGained, 99).presented);
    send.try_send(key('d', 4)).unwrap();
    assert!(
        send.overflowed(),
        "successful input cannot silently dismiss the loss warning"
    );
    send.dismiss_overflow();
    assert!(!send.overflowed());
}

#[tokio::test]
async fn drags_coalesce_only_across_matching_identity_button_and_modifiers() {
    let (send, mut receive) = channel(4);
    send.try_send(key('a', 1)).unwrap();
    for x in 0..10_000 {
        send.try_send(drag(x, 1)).unwrap();
    }
    send.try_send(key('b', 1)).unwrap();
    send.try_send(drag(12, 1)).unwrap();
    assert!(!send.overflowed());
    assert_eq!(receive.recv().await.unwrap().event, key('a', 1).event);
    assert_eq!(receive.recv().await.unwrap().event, drag(9999, 1).event);
    assert_eq!(receive.recv().await.unwrap().event, key('b', 1).event);
    assert_eq!(receive.recv().await.unwrap().event, drag(12, 1).event);
    for variant in 0..4 {
        send.try_send(drag(1, 1)).unwrap();
        let mut changed = drag(2, 1);
        match variant {
            0 => changed.attachment = 2,
            1 => changed.presented = key('x', 2).presented,
            2 => {
                if let Event::Mouse(event) = &mut changed.event {
                    event.modifiers = KeyModifiers::SHIFT;
                }
            }
            _ => {
                if let Event::Mouse(event) = &mut changed.event {
                    event.kind = MouseEventKind::Drag(MouseButton::Right);
                }
            }
        }
        send.try_send(changed).unwrap();
        assert_eq!(receive.recv().await.unwrap().event, drag(1, 1).event);
        receive.recv().await.unwrap();
    }
}

#[tokio::test]
async fn queued_paste_bytes_are_bounded_and_released_on_receive() {
    let (send, mut receive) = channel(32);
    let text = "x".repeat(1024 * 1024);
    for _ in 0..8 {
        send.try_send(input(Event::Paste(text.clone()), 1)).unwrap();
    }
    assert!(matches!(
        send.try_send(input(Event::Paste(text.clone()), 1)),
        Err(SendError::Full)
    ));
    receive.recv().await.unwrap();
    send.try_send(input(Event::Paste(text), 1)).unwrap();
    assert_eq!(send.0.state.lock().unwrap().bytes, MAX_QUEUED_TEXT_BYTES);
    drop(receive);
    assert_eq!(send.0.state.lock().unwrap().bytes, 0);
    assert!(matches!(send.try_send(key('a', 1)), Err(SendError::Closed)));
}

#[tokio::test]
async fn queue_wakes_without_polling_and_drains_before_sender_close() {
    let (send, mut receive) = channel(2);
    let writer = send.clone();
    let task = tokio::spawn(async move {
        tokio::task::yield_now().await;
        writer.try_send(key('a', 1)).unwrap();
    });
    let value = tokio::time::timeout(std::time::Duration::from_secs(1), receive.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(value.event, key('a', 1).event);
    task.await.unwrap();
    send.try_send(key('b', 1)).unwrap();
    drop(send);
    assert_eq!(receive.recv().await.unwrap().event, key('b', 1).event);
    assert!(receive.recv().await.is_none());
}
