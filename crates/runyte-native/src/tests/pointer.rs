// SPDX-License-Identifier: MPL-2.0
use super::*;
fn media(strict: bool) -> Capture {
    Capture {
        attachment: 1,
        target: Target::Media {
            pane: 2,
            path: "image.png".into(),
            page: 0,
            source: Some((None, 123)),
        },
        button: PointerButton::Middle,
        strict,
        cancelled: false,
    }
}
#[test]
fn replacement_or_lost_button_cancels_but_retains_the_physical_tail() {
    for (attachment, pressed, current) in [
        (2, Some(PointerButton::Middle), true),
        (1, Some(PointerButton::Middle), false),
        (1, None, true),
        (1, Some(PointerButton::Left), true),
    ] {
        let mut capture = media(true);
        assert_eq!(capture.motion(attachment, pressed, current), Step::Cancel);
        assert_eq!(
            capture.motion(1, Some(PointerButton::Middle), true),
            Step::Consume
        );
        assert_eq!(capture.release(PointerButton::Right), (Step::Cancel, false));
        assert_eq!(
            capture.release(PointerButton::Middle),
            (Step::Consume, true)
        );
    }
}
#[test]
fn factory_media_keeps_any_held_button_motion_but_remaps_are_exact() {
    let mut factory = media(false);
    assert_eq!(
        factory.motion(1, Some(PointerButton::Left), true),
        Step::Move
    );
    assert_eq!(factory.release(PointerButton::Left), (Step::Cancel, false));
    assert_eq!(
        factory.release(PointerButton::Middle),
        (Step::Consume, true)
    );
    let mut remapped = media(true);
    assert_eq!(
        remapped.motion(1, Some(PointerButton::Middle), true),
        Step::Move
    );
    assert_eq!(
        remapped.release(PointerButton::Middle),
        (Step::Release, true)
    );
}
#[test]
fn default_preview_ignores_other_releases_but_never_follows_another_button() {
    let mut capture = media(false);
    capture.target = Target::Preview(super::super::preview::Target {
        pane: 2,
        source: 3,
        generation: 4,
    });
    capture.button = PointerButton::Left;
    assert_eq!(
        capture.release(PointerButton::Right),
        (Step::Consume, false)
    );
    assert_eq!(
        capture.motion(1, Some(PointerButton::Left), true),
        Step::Move
    );
    assert_eq!(
        capture.motion(1, Some(PointerButton::Right), true),
        Step::Cancel
    );
    assert_eq!(capture.release(PointerButton::Left), (Step::Consume, true));
}

#[test]
fn fresh_host_press_and_attachment_change_clear_local_ownership() {
    let mut capture = Some(media(false));
    assert!(!reset_capture(&mut capture, 1, false));
    assert!(capture.is_some());
    assert!(
        reset_capture(&mut capture, 1, true),
        "host and local fresh presses both cancel the old drag"
    );
    assert!(capture.is_none());
    capture = Some(media(true));
    assert!(reset_capture(&mut capture, 2, false));
    assert!(capture.is_none());
}

#[test]
fn factory_preview_right_click_keeps_host_copy_but_removed_gestures_are_inert() {
    use runyte::keymap::pointer_actions::{Action, Bindings, Trigger};
    let target = Target::Preview(super::super::preview::Target {
        pane: 1,
        source: 2,
        generation: 3,
    });
    let defaults = Bindings::default();
    assert!(factory_preview_fallback(
        &target,
        PointerButton::Right,
        &defaults
    ));
    assert!(!factory_preview_fallback(
        &target,
        PointerButton::Middle,
        &defaults
    ));
    assert!(!factory_preview_fallback(
        &media(false).target,
        PointerButton::Right,
        &defaults
    ));
    for alias in [None, Some(Trigger::MiddleDrag), Some(Trigger::RightDrag)] {
        let mut masks = *defaults.masks();
        masks[Action::PreviewSelect as usize] = [0; 14];
        if let Some(alias) = alias {
            masks[Action::PreviewSelect as usize][alias as usize] = 1;
        }
        let bindings = Bindings::from_masks(masks).unwrap();
        assert!(!factory_preview_fallback(
            &target,
            PointerButton::Right,
            &bindings
        ));
    }
}

#[test]
fn loading_media_admits_focus_only_even_if_raster_arrives_before_dispatch() {
    let pane = super::super::MediaPane {
        pane: 2,
        path: "image.png".into(),
        page: 0,
        body: Default::default(),
    };
    let target = Target::Media {
        pane: 2,
        path: pane.path.clone(),
        page: 0,
        source: None,
    };
    for loaded in [None, Some((None, 123))] {
        assert_eq!(
            target.media_admission(&pane, loaded),
            MediaAdmission::FocusOnly
        );
    }
    let mut capture = media(true);
    assert_eq!(
        capture.target.media_admission(&pane, Some((None, 123))),
        MediaAdmission::Start
    );
    for replacement in [
        super::super::MediaPane {
            page: 1,
            ..pane.clone()
        },
        super::super::MediaPane {
            pane: 3,
            ..pane.clone()
        },
        super::super::MediaPane {
            path: "other.png".into(),
            ..pane.clone()
        },
    ] {
        assert_eq!(
            capture
                .target
                .media_admission(&replacement, Some((None, 123))),
            MediaAdmission::Discard
        );
    }
    let current = capture.target.media_admission(&pane, Some((None, 124))) == MediaAdmission::Start;
    assert_eq!(
        capture.motion(1, Some(PointerButton::Middle), current),
        Step::Cancel
    );
    assert_eq!(
        capture.release(PointerButton::Middle),
        (Step::Consume, true)
    );
}

#[test]
fn overflow_banner_defaults_remaps_removal_and_hint_share_one_policy() {
    use crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use runyte::keymap::pointer_actions::{Action, Bindings, Trigger};
    let (queue, _receiver) = super::super::input_queue::channel(2);
    let defaults = Bindings::default();
    let event = |button, modifiers| {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(button),
            column: 0,
            row: 0,
            modifiers,
        })
    };
    queue.note_overflow();
    assert!(!dismiss_overflow(
        &queue,
        &defaults,
        event(MouseButton::Right, KeyModifiers::NONE)
    ));
    assert!(queue.overflowed());
    assert!(dismiss_overflow(
        &queue,
        &defaults,
        event(
            MouseButton::Left,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )
    ));
    assert!(!queue.overflowed());
    assert_eq!(
        overflow_hint(&defaults),
        "Some input was lost. Click to dismiss."
    );
    let mut masks = *defaults.masks();
    masks[Action::InputOverflowDismiss as usize] = [0; 14];
    let removed = Bindings::from_masks(masks).unwrap();
    queue.note_overflow();
    assert!(!dismiss_overflow(
        &queue,
        &removed,
        event(MouseButton::Left, KeyModifiers::NONE)
    ));
    assert!(queue.overflowed());
    assert!(overflow_hint(&removed).contains("unbound"));
    masks[Action::InputOverflowDismiss as usize][Trigger::RightClick as usize] =
        1 << runyte::input::Modifiers::ALT.bits();
    let remapped = Bindings::from_masks(masks).unwrap();
    assert!(!dismiss_overflow(
        &queue,
        &remapped,
        event(MouseButton::Right, KeyModifiers::NONE)
    ));
    assert!(dismiss_overflow(
        &queue,
        &remapped,
        event(MouseButton::Right, KeyModifiers::ALT)
    ));
    assert!(!queue.overflowed());
    assert!(overflow_hint(&remapped).contains("Alt-RightClick"));
}
