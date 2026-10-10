// SPDX-License-Identifier: MPL-2.0
use super::*;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

fn key(code: char, attachment: u64) -> NativeInput {
    let mut key = KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE);
    key.kind = KeyEventKind::Repeat;
    NativeInput {
        attachment,
        event: Event::Key(key),
        presented: Some(runyte::protocol::FrameId::from_raw(42).into()),
        presentation_only: false,
        routing_serial: 0,
    }
}

#[test]
fn command_burst_waits_for_processed_frame_and_preserves_physical_identity() {
    let mut queue = Queue::default();
    queue.push(key(':', 1), ());
    queue.push(key('h', 1), ());
    queue.push(key('s', 1), ());
    let mut colon = queue.next(1, 0, true).unwrap().0;
    queue.forward(&mut colon);
    assert!(queue.next(1, 0, true).is_none());
    let mut h = queue.next(1, colon.routing_serial, true).unwrap().0;
    assert_eq!(h.presented.unwrap().get(), 42);
    assert!(matches!(
        h.event,
        Event::Key(KeyEvent {
            code: KeyCode::Char('h'),
            kind: KeyEventKind::Repeat,
            ..
        })
    ));
    queue.forward(&mut h);
    assert!(queue.next(1, colon.routing_serial, true).is_none());
    assert!(matches!(
        queue.next(1, h.routing_serial, true).unwrap().0.event,
        Event::Key(KeyEvent {
            code: KeyCode::Char('s'),
            ..
        })
    ));
}

#[test]
fn attachment_changes_discard_old_inputs_without_reusing_old_acknowledgements() {
    let mut queue = Queue::default();
    let mut first = key(':', 1);
    queue.forward(&mut first);
    queue.push(key('h', 1), ());
    queue.push(key(':', 2), ());
    let mut second = queue.next(2, 0, true).unwrap().0;
    assert_eq!(second.attachment, 2);
    queue.forward(&mut second);
    queue.push(key('h', 2), ());
    assert!(queue.next(2, first.routing_serial, true).is_none());
    assert!(queue.next(2, second.routing_serial, true).is_some());
}

#[test]
fn queue_is_bounded_and_send_failure_can_clear_waiting_inputs() {
    let mut queue = Queue::default();
    for _ in 0..256 {
        assert!(queue.push(key('x', 1), ()));
    }
    assert!(!queue.push(key('x', 1), ()));
    let mut input = queue.next(1, 0, true).unwrap().0;
    queue.forward(&mut input);
    queue.clear();
    assert!(queue.next(1, 0, true).is_none());
    assert!(queue.push(key(':', 1), ()));
    assert!(queue.next(1, 0, true).is_some());
}

#[test]
fn text_and_pointer_input_follow_queued_keys_with_their_original_identity() {
    use crossterm::event::{MouseEvent, MouseEventKind};
    let mut queue = Queue::default();
    let mut colon = key(':', 1);
    queue.forward(&mut colon);
    queue.push(key('h', 1), ());
    let mut text = key('x', 1);
    text.event = Event::Paste("split".into());
    queue.push(text, ());
    let mut pointer = key('x', 1);
    pointer.event = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: 7,
        row: 9,
        modifiers: KeyModifiers::NONE,
    });
    queue.push(pointer, ());
    assert!(queue.next(1, 0, true).is_none());
    for expected in ["key", "text", "pointer"] {
        let (mut input, ()) = queue.next(1, colon.routing_serial, true).unwrap();
        assert_eq!(input.presented.unwrap().get(), 42);
        assert_eq!(
            match &input.event {
                Event::Key(_) => "key",
                Event::Paste(_) => "text",
                Event::Mouse(_) => "pointer",
                _ => unreachable!(),
            },
            expected
        );
        queue.forward(&mut input);
        assert!(queue.next(1, colon.routing_serial, true).is_none());
        colon = input;
    }
}

#[test]
fn queued_text_is_bounded_and_accounted_when_discarded_or_drained() {
    let mut queue = Queue::default();
    let mut text = key('x', 1);
    text.event = Event::Paste("x".repeat(8 * 1024 * 1024));
    assert!(queue.push(text.clone(), ()));
    assert!(!queue.push(text.clone(), ()));
    assert!(queue.next(2, 0, true).is_none()); // old attachment releases retained bytes
    assert!(queue.push(text.clone(), ()));
    assert!(queue.next(1, 0, true).is_some());
    assert!(queue.push(text.clone(), ()));
    queue.clear();
    assert!(queue.push(text, ()));
}

#[test]
fn consecutive_drags_coalesce_without_crossing_physical_input_boundaries() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut queue = Queue::default();
    let mut pending = key(':', 1);
    queue.forward(&mut pending);
    let drag = |column, attachment, frame| NativeInput {
        attachment,
        presented: Some(runyte::protocol::FrameId::from_raw(frame).into()),
        event: Event::Mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column,
            row: 9,
            modifiers: KeyModifiers::NONE,
        }),
        presentation_only: false,
        routing_serial: 0,
    };
    for column in 0..300 {
        assert!(queue.push_coalesced(drag(column, 1, 42), 0, |a, b| a == b));
    }
    assert_eq!(queue.inputs.len(), 1);
    assert!(queue.push(key('h', 1), 0));
    assert!(queue.push_coalesced(drag(300, 1, 42), 0, |a, b| a == b));
    assert!(queue.push_coalesced(drag(301, 1, 42), 1, |a, b| a == b)); // different button state
    assert!(queue.push_coalesced(drag(302, 1, 43), 1, |a, b| a == b)); // different painted frame
    assert!(queue.push_coalesced(drag(303, 2, 43), 1, |a, b| a == b)); // different attachment
    assert_eq!(queue.inputs.len(), 6);
    let (input, action) = queue.next(1, pending.routing_serial, true).unwrap();
    assert_eq!(action, 0);
    assert!(matches!(
        input.event,
        Event::Mouse(MouseEvent { column: 299, .. })
    ));
    assert!(matches!(
        queue.next(1, pending.routing_serial, true).unwrap().0.event,
        Event::Key(_)
    ));
}

#[test]
fn acknowledged_input_waits_for_preview_preparation_without_losing_its_barrier() {
    let mut queue = Queue::default();
    let mut focus = key('l', 1);
    queue.forward(&mut focus);
    queue.push(key('j', 1), ());
    assert!(queue.next(1, focus.routing_serial, false).is_none());
    assert_eq!(queue.pending, Some((1, focus.routing_serial)));
    assert!(queue.next(1, 0, true).is_none());
    let (navigation, ()) = queue.next(1, focus.routing_serial, true).unwrap();
    assert_eq!(navigation.presented.unwrap().get(), 42);
    assert!(matches!(
        navigation.event,
        Event::Key(KeyEvent {
            code: KeyCode::Char('j'),
            ..
        })
    ));
}
