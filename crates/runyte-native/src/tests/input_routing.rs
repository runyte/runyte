// SPDX-License-Identifier: MPL-2.0
use super::*;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[test]
fn review_overlay_claims_wheel_input_above_native_media() {
    let overlays = [runyte::layout::Rect {
        x: 20,
        y: 12,
        width: 160,
        height: 36,
    }];
    assert!(overlay_owns_scroll(&overlays, 20.0, 12.0));
    assert!(overlay_owns_scroll(&overlays, 179.9, 47.9));
    assert!(!overlay_owns_scroll(&overlays, 180.0, 30.0));
    assert!(!overlay_owns_scroll(&overlays, 40.0, 48.0));
    assert!(!overlay_owns_scroll(&[], 40.0, 30.0));
}

fn key(code: char, attachment: u64) -> NativeInput {
    let mut key = KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE);
    key.kind = KeyEventKind::Repeat;
    NativeInput {
        attachment,
        event: Event::Key(key),
        presented: Some(runyte::protocol::FrameId::from_raw(42).into()),
        presentation_only: false,
        pointer_bindings: Default::default(),
        window_bindings: Default::default(),
        preview_bindings: Default::default(),
        preview_target: None,
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
        pointer_bindings: Default::default(),
        window_bindings: Default::default(),
        preview_bindings: Default::default(),
        preview_target: None,
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

#[test]
fn alt_pane_gesture_bypasses_native_media_until_release() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut queue: Queue = Queue::default();
    let mouse = |kind, modifiers, attachment| {
        let mut input = key('x', attachment);
        input.event = Event::Mouse(MouseEvent {
            kind,
            column: 5,
            row: 5,
            modifiers,
        });
        input
    };
    assert!(default_pane_gesture(
        &mut queue,
        &mouse(
            MouseEventKind::Down(MouseButton::Left),
            KeyModifiers::ALT,
            1
        )
    ));
    assert!(default_pane_gesture(
        &mut queue,
        &mouse(
            MouseEventKind::Drag(MouseButton::Left),
            KeyModifiers::NONE,
            1
        )
    ));
    let mut escape = key('x', 1);
    escape.event = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(default_pane_gesture(&mut queue, &escape));
    assert!(default_pane_gesture(
        &mut queue,
        &mouse(MouseEventKind::Up(MouseButton::Left), KeyModifiers::NONE, 1)
    ));
    assert!(!default_pane_gesture(
        &mut queue,
        &mouse(
            MouseEventKind::Down(MouseButton::Left),
            KeyModifiers::NONE,
            1
        )
    ));
    assert!(default_pane_gesture(
        &mut queue,
        &mouse(
            MouseEventKind::Down(MouseButton::Left),
            KeyModifiers::ALT,
            1
        )
    ));
    assert!(!default_pane_gesture(
        &mut queue,
        &mouse(
            MouseEventKind::Drag(MouseButton::Left),
            KeyModifiers::ALT,
            2
        )
    ));
    assert!(default_pane_gesture(
        &mut queue,
        &mouse(
            MouseEventKind::Down(MouseButton::Left),
            KeyModifiers::ALT,
            2
        )
    ));
    queue.clear();
    assert!(!default_pane_gesture(
        &mut queue,
        &mouse(MouseEventKind::Up(MouseButton::Left), KeyModifiers::NONE, 2)
    ));
}

#[test]
fn lost_pane_button_cancels_without_synthesizing_a_drop() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut queue: Queue = Queue::default();
    let mut input = key('x', 1);
    input.event = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 5,
        row: 5,
        modifiers: KeyModifiers::ALT,
    });
    assert!(default_pane_gesture(&mut queue, &input));
    input.event = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 8,
        row: 8,
        modifiers: KeyModifiers::NONE,
    });
    queue.cancel_lost_pane_button(&mut input, Some(MouseButton::Left));
    assert!(matches!(input.event, Event::Mouse(_)));
    queue.cancel_lost_pane_button(&mut input, None);
    assert!(matches!(input.event, Event::FocusLost));
    assert!(queue.pane_gesture.is_none());
    assert!(!default_pane_gesture(&mut queue, &key('x', 1)));
}

#[test]
fn focus_loss_clears_native_pane_capture() {
    let mut queue: Queue = Queue {
        pane_gesture: Some((1, crossterm::event::MouseButton::Left)),
        ..Queue::default()
    };
    let mut input = key('x', 1);
    input.event = Event::FocusLost;
    assert!(!default_pane_gesture(&mut queue, &input));
    assert!(queue.pane_gesture.is_none());
}

#[test]
fn shift_alt_expansion_bypasses_preview_and_media_until_release() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut queue: Queue = Queue::default();
    let mut input = key('x', 1);
    input.event = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 5,
        row: 5,
        modifiers: KeyModifiers::ALT | KeyModifiers::SHIFT,
    });
    assert!(default_pane_gesture(&mut queue, &input));
    for kind in [
        MouseEventKind::Drag(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        input.event = Event::Mouse(MouseEvent {
            kind,
            column: 8,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });
        assert!(default_pane_gesture(&mut queue, &input));
    }
    assert!(!default_pane_gesture(&mut queue, &key('x', 1)));
    input.event = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 5,
        row: 5,
        modifiers: KeyModifiers::ALT | KeyModifiers::SHIFT | KeyModifiers::CONTROL,
    });
    assert!(
        !default_pane_gesture(&mut queue, &input),
        "extra modifiers do not claim a pane gesture"
    );
}

fn default_pane_gesture(queue: &mut Queue, input: &NativeInput) -> bool {
    queue.editor_pane_gesture(input)
}

#[test]
fn configured_pane_gesture_owns_native_routing_until_release() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let gestures = runyte::keymap::pointer::PaneGestures {
        move_pane: [0; 3],
        extend_pane: [1 << 6, 0, 0],
    };
    let disabled = runyte::keymap::pointer::PaneGestures {
        move_pane: [0; 3],
        extend_pane: [0; 3],
    };
    let mut queue: Queue = Queue::default();
    let mut input = key('x', 1);
    input.pointer_bindings = with_pane_gestures(gestures);
    let mut event = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 5,
        row: 5,
        modifiers: KeyModifiers::ALT | KeyModifiers::SHIFT,
    };
    input.event = Event::Mouse(event);
    assert!(
        !queue.editor_pane_gesture(&input),
        "old binding must reach ordinary native handling"
    );
    event.modifiers = KeyModifiers::CONTROL | KeyModifiers::ALT;
    input.event = Event::Mouse(event);
    assert!(queue.editor_pane_gesture(&input));
    input.pointer_bindings = with_pane_gestures(disabled);
    event.kind = MouseEventKind::Drag(MouseButton::Left);
    event.modifiers = KeyModifiers::NONE;
    input.event = Event::Mouse(event);
    assert!(
        queue.editor_pane_gesture(&input),
        "reload preserves captured routing"
    );
    event.kind = MouseEventKind::Up(MouseButton::Left);
    input.event = Event::Mouse(event);
    assert!(queue.editor_pane_gesture(&input));
    assert!(!queue.editor_pane_gesture(&input));
}

#[test]
fn queued_pane_press_keeps_the_painted_configuration() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let enabled = runyte::keymap::pointer::PaneGestures::default();
    let disabled = runyte::keymap::pointer::PaneGestures {
        move_pane: [0; 3],
        extend_pane: [0; 3],
    };
    for (captured, later, expected) in [(enabled, disabled, true), (disabled, enabled, false)] {
        let mut queue: Queue = Queue::default();
        let mut input = key('x', 1);
        input.pointer_bindings = with_pane_gestures(captured);
        input.event = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: KeyModifiers::ALT,
        });
        queue.push(input, ());
        // A newer frame has different mappings when the physical event drains.
        assert_ne!(later, captured);
        let (input, ()) = queue.next(1, 0, true).unwrap();
        assert_eq!(queue.editor_pane_gesture(&input), expected);
    }
}

#[test]
fn queued_window_shortcuts_retain_the_painted_map_across_replacement() {
    use runyte::{
        input::{KeyCode as CoreCode, KeyStroke},
        keymap::native_window::{Action as WindowAction, Bindings},
    };
    let stroke = KeyStroke::plain(CoreCode::Function(8));
    let mut first = key('x', 1);
    first.event = Event::Key(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE));
    first.window_bindings =
        Bindings::from_entries(vec![(stroke, WindowAction::FontSize(1))]).unwrap();
    let mut replacement = first.clone();
    replacement.presented = Some(runyte::protocol::FrameId::from_raw(43).into());
    replacement.window_bindings =
        Bindings::from_entries(vec![(stroke, WindowAction::PasteText)]).unwrap();
    let mut queue = Queue::default();
    queue.push(first, ());
    queue.push(replacement, ());
    let first = queue.next(1, 0, true).unwrap().0;
    let second = queue.next(1, 0, true).unwrap().0;
    assert_eq!(first.presented.unwrap().get(), 42);
    assert_eq!(
        first.window_bindings.lookup(stroke),
        Some(WindowAction::FontSize(1))
    );
    assert_eq!(second.presented.unwrap().get(), 43);
    assert_eq!(
        second.window_bindings.lookup(stroke),
        Some(WindowAction::PasteText)
    );
}

#[test]
fn queued_window_operand_waits_for_host_and_keeps_captured_configuration() {
    use runyte::{
        input::KeyStroke,
        keymap::native_window::{Action as WindowAction, Bindings},
    };
    let stroke = KeyStroke::ctrl('x');
    let map = Bindings::from_entries(vec![(stroke, WindowAction::FontSize(1))])
        .unwrap()
        .with_policy(1, 0)
        .unwrap();
    let mut prefix = key('f', 1);
    prefix.window_bindings = map.clone();
    let mut operand = key('x', 1);
    operand.event = Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
    operand.window_bindings = map.clone();
    let mut queue: Queue = Queue::default();
    queue.push(prefix, ());
    queue.push(operand, ());
    let (mut prefix, ()) = queue.next(1, 0, true).unwrap();
    assert!(window_requires_barrier(&prefix.window_bindings, Some(&map)));
    queue.forward(&mut prefix);
    assert!(queue.next(1, 0, true).is_none());
    // The host acknowledges f and publishes an operand owner. A simultaneous
    // config reload removes the chord, but cannot change its captured identity.
    let current = Bindings::from_entries(vec![])
        .unwrap()
        .with_policy(0, 1)
        .unwrap();
    let (operand, ()) = queue.next(1, prefix.routing_serial, true).unwrap();
    assert!(
        operand
            .window_bindings
            .with_current_guard(&current)
            .lookup(stroke)
            .is_none()
    );
    let released = current.with_policy(0, 0).unwrap();
    assert_eq!(
        operand
            .window_bindings
            .with_current_guard(&released)
            .lookup(stroke),
        Some(WindowAction::FontSize(1))
    );
    assert!(!window_requires_barrier(
        &Bindings::default(),
        Some(&released)
    ));
}

#[test]
fn queued_preview_navigation_keeps_painted_map_and_document_identity() {
    use runyte::{
        input::{KeyCode as CoreCode, KeyStroke},
        keymap::{
            KeySequence,
            native_preview::{Action as PreviewAction, Bindings},
        },
    };
    let stroke = KeyStroke::plain(CoreCode::Function(8));
    let map =
        |action| Bindings::from_entries(vec![(KeySequence::from(stroke), action)], vec![]).unwrap();
    let mut first = key('x', 1);
    first.event = Event::Key(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE));
    first.preview_bindings = map(PreviewAction::ZoomIn);
    first.preview_target = Some(super::super::preview::Target {
        pane: 2,
        source: 7,
        generation: 11,
    });
    let mut later = first.clone();
    later.preview_bindings = map(PreviewAction::Back);
    later.preview_target.as_mut().unwrap().generation = 12;
    let mut queue: Queue = Queue::default();
    queue.push(first, ());
    queue.push(later, ());
    let first = queue.next(1, 0, true).unwrap().0;
    let later = queue.next(1, 0, true).unwrap().0;
    assert_eq!(
        first.preview_bindings.lookup(&KeySequence::from(stroke)),
        runyte::keymap::native_preview::Lookup::Action(PreviewAction::ZoomIn)
    );
    assert_eq!(
        later.preview_bindings.lookup(&KeySequence::from(stroke)),
        runyte::keymap::native_preview::Lookup::Action(PreviewAction::Back)
    );
    assert_eq!(first.preview_target.unwrap().generation, 11);
    assert_eq!(later.preview_target.unwrap().generation, 12);
}

#[test]
fn physical_pointer_motion_and_remapped_capture_retain_button_identity() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    assert_eq!(pointer_motion(None), MouseEventKind::Moved);
    for (native, button, index) in [
        (super::super::MouseButton::Left, MouseButton::Left, 0),
        (super::super::MouseButton::Middle, MouseButton::Middle, 1),
        (super::super::MouseButton::Right, MouseButton::Right, 2),
    ] {
        assert_eq!(pointer_motion(Some(native)), MouseEventKind::Drag(button));
        let mut queue: Queue = Queue::default();
        let mut input = key('x', 1);
        let mut gestures = runyte::keymap::pointer::PaneGestures {
            move_pane: [0; 3],
            extend_pane: [0; 3],
        };
        gestures.extend_pane[index] = 1 << KeyModifiers::CONTROL.bits();
        input.pointer_bindings = with_pane_gestures(gestures);
        input.event = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(button),
            column: 5,
            row: 5,
            modifiers: KeyModifiers::CONTROL,
        });
        assert!(queue.editor_pane_gesture(&input));
        gestures.extend_pane = [0; 3];
        input.pointer_bindings = with_pane_gestures(gestures);
        input.event = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Drag(button),
            column: 8,
            row: 8,
            modifiers: KeyModifiers::NONE,
        });
        queue.cancel_lost_pane_button(&mut input, Some(button));
        assert!(queue.editor_pane_gesture(&input));
        queue.cancel_lost_pane_button(&mut input, None);
        assert!(matches!(input.event, Event::FocusLost));
        assert!(queue.pane_gesture.is_none());
    }
}

#[test]
fn unmatched_pane_release_keeps_the_initiating_buttons_tail_owned() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut queue: Queue = Queue::default();
    let mut input = key('x', 1);
    input.pointer_bindings = with_pane_gestures(runyte::keymap::pointer::PaneGestures {
        move_pane: [0; 3],
        extend_pane: [0, 0, 1],
    });
    for kind in [
        MouseEventKind::Down(MouseButton::Right),
        MouseEventKind::Up(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Right),
        MouseEventKind::Up(MouseButton::Right),
    ] {
        input.event = Event::Mouse(MouseEvent {
            kind,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        assert!(queue.editor_pane_gesture(&input));
    }
    assert!(queue.pane_gesture.is_none());
}

#[test]
fn passive_motion_coalesces_without_consuming_room_for_real_input() {
    use crossterm::event::{MouseEvent, MouseEventKind};
    let mut queue: Queue<u8> = Queue::default();
    let mut input = key('x', 1);
    input.event = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: 5,
        row: 5,
        modifiers: KeyModifiers::NONE,
    });
    for _ in 0..1000 {
        assert!(queue.push_coalesced(input.clone(), 0, |a, b| a == b));
    }
    assert_eq!(queue.inputs.len(), 1);
    assert!(queue.push_coalesced(input.clone(), 1, |a, b| a == b));
    input.presented = Some(runyte::protocol::FrameId::from_raw(999).into());
    assert!(queue.push_coalesced(input.clone(), 1, |a, b| a == b));
    input.attachment += 1;
    assert!(queue.push_coalesced(input.clone(), 1, |a, b| a == b));
    if let Event::Mouse(event) = &mut input.event {
        event.modifiers = KeyModifiers::ALT;
    }
    assert!(queue.push_coalesced(input, 1, |a, b| a == b));
    assert!(queue.push(key('x', 2), 1));
    assert_eq!(queue.inputs.len(), 6);
}

fn with_pane_gestures(
    gestures: runyte::keymap::pointer::PaneGestures,
) -> runyte::keymap::pointer_actions::Bindings {
    let mut masks = *runyte::keymap::pointer_actions::Bindings::default().masks();
    masks[0][3..6].copy_from_slice(&gestures.move_pane);
    masks[1][3..6].copy_from_slice(&gestures.extend_pane);
    runyte::keymap::pointer_actions::Bindings::from_masks(masks).unwrap()
}

#[test]
fn middle_double_click_initiators_reach_the_host_without_single_click_actions() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use runyte::keymap::pointer_actions::{Action, Bindings, Trigger};
    let mut masks = *Bindings::default().masks();
    masks[Action::TreeSelect as usize] = [0; 14];
    masks[Action::TreeOpen as usize] = [0; 14];
    masks[Action::TreeOpen as usize][Trigger::MiddleDoubleClick as usize] = 1;
    let mut input = key('x', 1);
    input.pointer_bindings = Bindings::from_masks(masks).unwrap();
    input.event = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Middle),
        column: 2,
        row: 2,
        modifiers: KeyModifiers::NONE,
    });
    assert!(host_pointer_press(&input));
}

#[test]
fn remapped_host_selection_routes_nonleft_tails_after_reload() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use runyte::keymap::pointer_actions::{Action, Bindings, Trigger};
    let mut queue: Queue = Queue::default();
    let mut input = key('x', 1);
    input.event = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 3,
        row: 3,
        modifiers: KeyModifiers::NONE,
    });
    queue.capture_host_drag(&input);
    assert!(
        queue.pane_gesture.is_none(),
        "factory left-selection keeps native key precedence"
    );
    let mut masks = *Bindings::default().masks();
    masks[Action::Select as usize] = [0; 14];
    masks[Action::Select as usize][Trigger::MiddleDrag as usize] = 1;
    input.pointer_bindings = Bindings::from_masks(masks).unwrap();
    if let Event::Mouse(event) = &mut input.event {
        event.kind = MouseEventKind::Down(MouseButton::Middle);
    }
    assert!(host_pointer_press(&input));
    assert!(!queue.editor_pane_gesture(&input));
    queue.capture_host_drag(&input);
    input.pointer_bindings = Bindings::default();
    for kind in [
        MouseEventKind::Drag(MouseButton::Middle),
        MouseEventKind::Up(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Middle),
    ] {
        if let Event::Mouse(event) = &mut input.event {
            event.kind = kind;
            event.modifiers = KeyModifiers::ALT;
        }
        assert!(queue.editor_pane_gesture(&input));
    }
    assert!(queue.pane_gesture.is_none());
}

#[test]
fn remapped_copy_claims_its_tail_without_changing_factory_right_click_routing() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use runyte::keymap::pointer_actions::{Action, Bindings, Trigger};
    let mut queue: Queue = Queue::default();
    let mut input = key('x', 1);
    input.event = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: 3,
        row: 3,
        modifiers: KeyModifiers::NONE,
    });
    queue.capture_host_drag(&input);
    assert!(queue.pane_gesture.is_none());
    let mut masks = *Bindings::default().masks();
    masks[Action::Copy as usize] = [0; 14];
    masks[Action::Copy as usize][Trigger::MiddleClick as usize] = 1;
    input.pointer_bindings = Bindings::from_masks(masks).unwrap();
    if let Event::Mouse(event) = &mut input.event {
        event.kind = MouseEventKind::Down(MouseButton::Middle);
    }
    assert!(host_pointer_press(&input));
    queue.capture_host_drag(&input);
    assert!(queue.pane_gesture.is_some());
    if let Event::Mouse(event) = &mut input.event {
        event.kind = MouseEventKind::Up(MouseButton::Middle);
    }
    assert!(queue.editor_pane_gesture(&input));
    assert!(queue.pane_gesture.is_none());
}
