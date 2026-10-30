// SPDX-License-Identifier: MPL-2.0
use super::*;

fn configured(source: &str) -> (Bindings, Vec<String>) {
    let mut errors = Vec::new();
    let value = serde_yaml::from_str(source).unwrap();
    (compile(Some(&value), &mut errors), errors)
}

#[test]
fn default_pointer_owners_preserve_all_modifier_combinations() {
    let bindings = Bindings::default();
    for bits in 0..64 {
        let modifiers = Modifiers::from_bits(bits).unwrap();
        for button in [
            PointerButton::Left,
            PointerButton::Middle,
            PointerButton::Right,
        ] {
            let event = PointerEvent {
                kind: PointerEventKind::Down(button),
                column: 0,
                row: 0,
                modifiers,
            };
            let left = button == PointerButton::Left;
            assert_eq!(
                bindings.press(Action::Select, event),
                left && !modifiers.contains(Modifiers::SHIFT)
            );
            assert_eq!(
                bindings.press(Action::Extend, event),
                left && modifiers.contains(Modifiers::SHIFT)
            );
            for action in [
                Action::Resize,
                Action::TreeResize,
                Action::TreeSelect,
                Action::SessionActivate,
            ] {
                assert_eq!(bindings.press(action, event), left);
            }
            assert_eq!(
                bindings.press(Action::Copy, event),
                button == PointerButton::Right
            );
            assert_eq!(
                bindings.matches(Action::TreeOpen, Trigger::double_click(button), modifiers),
                left
            );
            assert_eq!(
                bindings.press(Action::OpenLink, event),
                left && modifiers
                    == if cfg!(target_os = "macos") {
                        Modifiers::SUPER
                    } else {
                        Modifiers::CONTROL
                    }
            );
        }
        for (kind, action) in [
            (PointerEventKind::ScrollUp, Action::ScrollUp),
            (PointerEventKind::ScrollDown, Action::ScrollDown),
            (PointerEventKind::ScrollLeft, Action::ScrollLeft),
            (PointerEventKind::ScrollRight, Action::ScrollRight),
        ] {
            let event = PointerEvent {
                kind,
                column: 0,
                row: 0,
                modifiers,
            };
            assert_eq!(bindings.wheel(event), Some(action));
            for action in Action::ALL {
                assert!(!bindings.press(action, event));
            }
        }
    }
    assert_eq!(Bindings::from_masks(*bindings.masks()), Some(bindings));
}

#[test]
fn invalid_owner_rolls_back_without_losing_unrelated_remaps() {
    let (bindings, errors) = configured(
        "{extend: [Ctrl-LeftDrag], copy: [Alt-MiddleClick], pane-extend: [Ctrl-RightDrag]}",
    );
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("extend and select overlap"));
    assert!(bindings.is_default(Action::Select));
    assert!(bindings.is_default(Action::Extend));
    assert_eq!(bindings.label(Action::Copy), "Alt-MiddleClick");
    assert_eq!(
        bindings
            .pane_gestures()
            .labels(super::super::pointer::PaneAction::Extend),
        "Ctrl-RightDrag"
    );
    let (bindings, errors) = configured(
        "{select: [LeftDrag], extend: [Ctrl-LeftDrag], scroll-up: [WheelDown], scroll-down: [WheelUp], open-link: []}",
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(bindings.label(Action::Extend), "Ctrl-LeftDrag");
    assert_eq!(bindings.label(Action::OpenLink), "unbound");
    assert!(bindings.matches(Action::ScrollUp, Trigger::WheelDown, Modifiers::NONE));
    assert!(!bindings.matches(Action::ScrollUp, Trigger::WheelDown, Modifiers::SHIFT));
}

#[test]
fn malformed_bindings_and_wire_masks_are_rejected_atomically() {
    for source in [
        "{select: [LeftClick]}",
        "{tree-open: [LeftDrag]}",
        "{copy: [WheelUp]}",
        "{scroll-up: [RightClick]}",
        "{select: [RightDrag, RightDrag]}",
    ] {
        let (bindings, errors) = configured(source);
        assert_eq!(bindings, Bindings::default(), "{source}");
        assert_eq!(errors.len(), 1);
    }
    let mut masks = *Bindings::default().masks();
    masks[Action::Select as usize][Trigger::RightClick as usize] = 1;
    assert!(Bindings::from_masks(masks).is_none());
    let mut masks = *Bindings::default().masks();
    masks[Action::Extend as usize][Trigger::LeftDrag as usize] |= 1;
    assert!(Bindings::from_masks(masks).is_none());
}
