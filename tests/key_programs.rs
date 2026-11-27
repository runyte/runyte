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
fn programs_are_nonempty_bounded_and_migrated_defaults_have_explicit_modes() {
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
        assert!((1..=MAX_STEPS).contains(&binding.program.len()));
        if binding.program.len() == 2 {
            let BindingTarget::Editor(command) = binding.target else {
                panic!("unexpected composed default")
            };
            let first = match command {
                C::EnterNormalMode => C::FitSelectionForNormalMode,
                C::SelectAll | C::CollapseSelection | C::ToggleCase | C::ReplaceChar => command,
                _ => panic!("unclassified default program: {command:?}"),
            };
            assert_eq!(binding.program.get(0), Some(&Action::Editor(first)));
            let Action::Invocation(mode) = binding.program.last() else {
                panic!("expected pure mode step")
            };
            assert_eq!(mode.id(), CommandId::Editor(C::ModeEnter));
            assert_eq!(
                mode.parameters(),
                &runyte::command::InvocationParameters::OptionalText(Some(
                    if command == C::SelectAll {
                        "select"
                    } else {
                        "normal"
                    }
                    .into()
                ))
            );
            assert!(!binding.is_plain_action());
        } else {
            assert_eq!(binding.program.len(), 1);
            assert_eq!(binding.program.last().target(), binding.target);
            assert!(binding.is_plain_action());
        }
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
    let output = acknowledge(output, &mut grammar);
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

#[test]
fn every_binding_kind_silently_drops_a_count_when_no_step_accepts_it() {
    use runyte::command::{ColonCommand, parse_named_command};
    let programs = [
        Program::single(C::Yank.into()),
        Program::single(BindingTarget::Colon(ColonCommand::LspStatus)),
        Program::single(BindingTarget::Plugin(42)),
        Program::new(vec![Action::Invocation(
            parse_named_command("help", Some("key-actions")).unwrap(),
        )])
        .unwrap(),
        Program::new(vec![Action::Editor(C::SelectAll), Action::Editor(C::Yank)]).unwrap(),
    ];
    for program in programs {
        for configured in [false, true] {
            let mut binding = Binding::implemented(
                &[Mode::Normal],
                Key::char('x'),
                program.as_slice()[0].target(),
            );
            binding.program = program.clone();
            binding.configured_actions = configured;
            let map = Keymap::new(vec![
                binding,
                Binding::implemented(&[Mode::Normal], Key::char('l'), C::MoveRight),
            ])
            .unwrap();
            let context =
                GrammarContext::new(Mode::Normal, runyte::keymap::BindingScope::Global, &map);
            let mut grammar = RunyteGrammar::default();
            let uncounted = grammar
                .translate(runyte::input::InputEvent::Key(Key::char('x')), context)
                .unwrap();
            let uncounted = acknowledge(uncounted, &mut grammar);
            grammar
                .translate(runyte::input::InputEvent::Key(Key::char('3')), context)
                .unwrap();
            let counted = grammar
                .translate(runyte::input::InputEvent::Key(Key::char('x')), context)
                .unwrap();
            let counted = acknowledge(counted, &mut grammar);
            assert_eq!(
                counted.intents, uncounted.intents,
                "{program:?}, configured={configured}"
            );
            assert!(
                counted
                    .intents
                    .iter()
                    .all(|intent| matches!(intent, EditorIntent::Command(_)))
            );
            let next = grammar
                .translate(runyte::input::InputEvent::Key(Key::char('l')), context)
                .unwrap();
            let next = acknowledge(next, &mut grammar);
            let [EditorIntent::Command(next)] = next.intents.as_slice() else {
                panic!("expected movement")
            };
            assert_eq!(
                next.execution().count(),
                None,
                "dropped count must not leak"
            );
        }
    }
}

#[test]
fn singleton_and_composed_parameterized_steps_receive_the_same_accepted_count() {
    use runyte::command::{CommandExecutionContext, CommandInvocation};
    for prefix in [false, true] {
        let invocation =
            CommandInvocation::editor(C::MoveRight, CommandExecutionContext::default()).unwrap();
        let mut steps = Vec::new();
        if prefix {
            steps.push(Action::Editor(C::Yank));
        }
        steps.push(Action::Invocation(invocation));
        let mut binding = Binding::implemented(&[Mode::Normal], Key::char('x'), steps[0].target());
        binding.program = Program::new(steps).unwrap();
        let map = Keymap::new(vec![binding]).unwrap();
        let context = GrammarContext::new(Mode::Normal, runyte::keymap::BindingScope::Global, &map);
        let mut grammar = RunyteGrammar::default();
        grammar
            .translate(runyte::input::InputEvent::Key(Key::char('3')), context)
            .unwrap();
        let output = grammar
            .translate(runyte::input::InputEvent::Key(Key::char('x')), context)
            .unwrap();
        let output = acknowledge(output, &mut grammar);
        let EditorIntent::Command(invocation) = output.intents.last().unwrap() else {
            panic!("expected movement")
        };
        assert_eq!(invocation.execution().count(), Some(3));
    }
}

fn acknowledge(
    mut output: runyte::input_grammar::GrammarOutput,
    grammar: &mut RunyteGrammar,
) -> runyte::input_grammar::GrammarOutput {
    while let Some(intent) = grammar.next_program_intent().unwrap() {
        output.intents.push(intent);
    }
    output
}
