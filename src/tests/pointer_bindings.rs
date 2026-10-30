// SPDX-License-Identifier: MPL-2.0
use super::*;

fn compiled(source: &str) -> (PaneGestures, Vec<String>) {
    let value = serde_yaml::from_str(source).unwrap();
    let mut errors = Vec::new();
    (compile(Some(&value), &mut errors), errors)
}

#[test]
fn invalid_owner_rolls_back_atomically_and_does_not_affect_keyboard() {
    for source in [
        "{pane-move: [Alt-LeftDrag], pane-extend: [Alt-LeftDrag]}",
        "{pane-move: [Shift-Alt-LeftDrag]}",
        "{pane-move: [Ctrl-LeftDrag], pane-extend: [RightClick]}",
        "{pane-move: [Ctrl-LeftDrag, Ctrl-LeftDrag]}",
        "{pane-move: null}",
        "{pane-move: LeftDrag}",
        "{pane-move: [Ctrl-Foo-LeftDrag]}",
        "{unknown: []}",
        "[]",
    ] {
        let (gestures, errors) = compiled(source);
        assert_eq!(gestures, PaneGestures::default(), "{source}");
        assert_eq!(errors.len(), 1, "{source}");
    }
    let section = serde_yaml::from_str(
        "pointer: {pane-extend: [Alt-LeftDrag]}\nbind: {normal: {F9: move-right}}",
    )
    .unwrap();
    let result = crate::keymap::configured::compile(&section, crate::keymap::default_keymap());
    assert_eq!(result.errors.len(), 1);
    assert!(matches!(
        result.keymap.lookup(
            crate::command::Mode::Normal,
            &crate::keymap::KeySequence::parse("F9").unwrap()
        ),
        crate::keymap::Lookup::Exact(_)
    ));
}

#[test]
fn modifier_combinations_are_exact_and_non_down_events_are_not_new_gestures() {
    let (gestures, errors) =
        compiled("{pane-move: [LeftDrag, Super-LeftDrag], pane-extend: [Ctrl-Alt-LeftDrag]}");
    assert!(errors.is_empty());
    for bits in 0..64 {
        let event = PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: 4,
            row: 9,
            modifiers: Modifiers::from_bits(bits).unwrap(),
        };
        assert_eq!(
            gestures.action(event),
            match bits {
                0 | 8 => Some(PaneAction::Move),
                6 => Some(PaneAction::Extend),
                _ => None,
            }
        );
        for kind in [
            PointerEventKind::Up(PointerButton::Left),
            PointerEventKind::Drag(PointerButton::Left),
            PointerEventKind::Down(PointerButton::Right),
            PointerEventKind::ScrollDown,
        ] {
            assert_eq!(gestures.action(PointerEvent { kind, ..event }), None);
        }
    }
    assert_eq!(gestures.labels(PaneAction::Extend), "Ctrl-Alt-LeftDrag");
    assert_eq!(
        compiled("{pane-extend: []}").0.labels(PaneAction::Extend),
        "unbound"
    );
}

#[test]
fn pointer_diagnostics_and_help_are_bounded_and_use_effective_bindings() {
    for source in [
        format!("? {}\n: []", "x".repeat(10_000)),
        format!("{{pane-move: [{}LeftDrag]}}", "Ctrl-".repeat(10_000)),
    ] {
        let (gestures, errors) = compiled(&source);
        assert_eq!(gestures, PaneGestures::default());
        assert!(errors[0].len() < 160);
    }
    let section =
        serde_yaml::from_str("pointer: {pane-move: [], pane-extend: [Ctrl-Alt-LeftDrag]}").unwrap();
    let result = crate::keymap::configured::compile(&section, crate::keymap::default_keymap());
    let help = crate::help::render(
        crate::help::HelpTopic::Text,
        crate::command::GrammarKind::Runyte,
        crate::keymap::BindingScope::Global,
        &result.keymap,
        false,
    );
    assert!(help.contains("unbound rearranges panes; Ctrl-Alt-LeftDrag extends"));
    assert!(!help.contains("Shift-Alt-LeftDrag"));
}

#[test]
fn every_button_has_independent_exact_modifiers_and_wire_validation() {
    let (gestures, errors) =
        compiled("{pane-move: [Alt-RightDrag, Ctrl-MiddleDrag], pane-extend: [Shift-RightDrag]}");
    assert!(errors.is_empty());
    for (button, mask, action) in [
        (PointerButton::Right, Modifiers::ALT, PaneAction::Move),
        (PointerButton::Middle, Modifiers::CONTROL, PaneAction::Move),
        (PointerButton::Right, Modifiers::SHIFT, PaneAction::Extend),
    ] {
        assert_eq!(
            gestures.action(PointerEvent {
                kind: PointerEventKind::Down(button),
                column: 0,
                row: 0,
                modifiers: mask
            }),
            Some(action)
        );
    }
    assert_eq!(
        gestures.labels(PaneAction::Move),
        "Ctrl-MiddleDrag / Alt-RightDrag"
    );
    assert_eq!(PaneGestures::from_masks(gestures.masks()), Some(gestures));
    for button in 0..3 {
        let mut masks = [0; 6];
        masks[2 * button] = 1 << 63;
        masks[2 * button + 1] = 1 << 63;
        assert!(PaneGestures::from_masks(masks).is_none());
    }
    let (rolled_back, errors) = compiled("{pane-move: [RightDrag], pane-extend: [RightDrag]}");
    assert_eq!(rolled_back, PaneGestures::default());
    assert_eq!(errors.len(), 1);
}
