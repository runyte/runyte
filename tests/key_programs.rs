// SPDX-License-Identifier: MPL-2.0

use runyte::{
    command::{CommandId, EditorCommand as C, Mode},
    input_grammar::{EditorIntent, GrammarContext, InputGrammar, RunyteGrammar},
    keymap::{
        Binding, BindingTarget, Key, KeySequence, Keymap,
        actions::Action,
        default_keymap,
        program::{MAX_STEPS, Program},
    },
};

#[test]
fn programs_are_nonempty_bounded_and_all_defaults_are_plain_singletons() {
    assert!(Program::new(vec![]).is_err());
    assert!(Program::new(vec![Action::Editor(C::Yank); MAX_STEPS + 1]).is_err());
    assert_eq!(
        Program::new(vec![Action::Editor(C::Yank); MAX_STEPS])
            .unwrap()
            .len(),
        MAX_STEPS
    );
    assert_eq!(
        Program::new(vec![Action::Editor(C::Yank)]).unwrap(),
        Program::single(C::Yank.into())
    );
    for binding in default_keymap().bindings() {
        assert_eq!(binding.program.len(), 1, "{}", binding.sequence);
        assert_eq!(binding.program.last().target(), binding.target);
        assert!(binding.is_plain_action());
        assert!(!binding.configured_actions);
    }
}

#[test]
fn count_recipient_comes_from_descriptors_in_program_order() {
    for &command in C::ALL {
        let program = Program::new(vec![
            Action::Editor(C::Yank),
            Action::Editor(command),
            Action::Editor(C::MoveRight),
        ])
        .unwrap();
        assert_eq!(
            program.count_step(),
            Some(if command.action_policy().count.accepts() {
                1
            } else {
                2
            }),
            "{command:?}"
        );
    }
}

#[test]
fn authored_programs_execute_all_steps_without_configuration_provenance() {
    let mut binding = Binding::implemented(&[Mode::Normal], Key::char('x'), C::Yank);
    binding.program = Program::new(vec![
        Action::Editor(C::Yank),
        Action::Editor(C::MoveRight),
        Action::Editor(C::MoveLeft),
    ])
    .unwrap();
    let map = Keymap::new(vec![binding]).unwrap();
    let mut grammar = RunyteGrammar::default();
    let context = GrammarContext::new(Mode::Normal, runyte::keymap::BindingScope::Global, &map);
    grammar
        .translate(runyte::input::InputEvent::Key(Key::char('3')), context)
        .unwrap();
    let output = grammar
        .translate(runyte::input::InputEvent::Key(Key::char('x')), context)
        .unwrap();
    let commands = output
        .intents
        .iter()
        .map(|intent| match intent {
            EditorIntent::Command(invocation) => (invocation.id(), invocation.execution().count()),
            other => panic!("unexpected intent: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        commands,
        vec![
            (CommandId::Editor(C::Yank), None),
            (CommandId::Editor(C::MoveRight), Some(3)),
            (CommandId::Editor(C::MoveLeft), None)
        ]
    );
    assert_eq!(
        output.resolved_binding,
        Some((
            KeySequence::parse("3 x").unwrap(),
            BindingTarget::Editor(C::MoveLeft)
        ))
    );
}
