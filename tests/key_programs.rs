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
        if binding.program.has_conditions() {
            assert!(matches!(
                binding.target,
                BindingTarget::Editor(
                    C::EnterSelectMode
                        | C::RecordDefaultMacro
                        | C::DirectoryTreeCollapse
                        | C::DirectoryTreeExpand
                        | C::DirectoryTreeOpen
                        | C::DirectoryTreeVertical
                        | C::DirectoryTreeHorizontal
                )
            ));
            assert!(!binding.is_plain_action());
            continue;
        }
        if binding.program.len() > 1 {
            let BindingTarget::Editor(command) = binding.target else {
                panic!("unexpected composed default")
            };
            let expected = match command {
                C::EnterNormalMode => vec![C::FitSelectionForNormalMode, C::ModeEnter],
                C::SearchSetFromSelection => vec![command, C::SearchSelectAllMatches, C::ModeEnter],
                C::SelectAll
                | C::CollapseSelection
                | C::ToggleCase
                | C::ReplaceChar
                | C::CutSelection
                | C::CutSelectionForChange => vec![command, C::ModeEnter],
                _ => panic!("unclassified default program: {command:?}"),
            };
            assert_eq!(
                binding
                    .program
                    .iter()
                    .map(|step| step.target())
                    .collect::<Vec<_>>(),
                expected
                    .into_iter()
                    .map(BindingTarget::Editor)
                    .collect::<Vec<_>>()
            );
            let Action::Invocation(mode) = binding.program.last() else {
                panic!("expected mode step")
            };
            assert_eq!(mode.id(), CommandId::Editor(C::ModeEnter));
            let name = match command {
                C::SelectAll | C::SearchSetFromSelection | C::SearchNext | C::SearchPrevious => {
                    "select"
                }
                C::CutSelectionForChange => "insert",
                _ => "normal",
            };
            assert_eq!(
                mode.parameters(),
                &runyte::command::InvocationParameters::OptionalText(Some(name.into()))
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
            BindingTarget::Editor(C::Yank)
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
                program.iter().next().unwrap().target(),
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

#[test]
fn removed_action_names_report_migrations_in_both_configuration_forms() {
    use runyte::{command::parse_named_command, keymap::configured};
    for (old, replacement) in [
        ("yank-delete-selection", "cut-selection"),
        ("yank-change-selection", "cut-selection-for-change"),
        ("set-search-from-selection", "search-set-from-selection"),
        ("search-selection", "search-select-all-matches"),
    ] {
        assert!(
            parse_named_command(old, None)
                .unwrap_err()
                .to_string()
                .contains(replacement)
        );
        for source in [
            format!("bind: {{normal: {{F6: {old}}}}}"),
            format!("scopes: {{terminal: {{normal: {{F6: {old}}}}}}}"),
        ] {
            let value = serde_yaml::from_str(&source).unwrap();
            let result = configured::compile(&value, default_keymap());
            assert!(
                result
                    .errors
                    .iter()
                    .any(|error| error.contains(replacement)),
                "{:?}",
                result.errors
            );
        }
    }
}

#[test]
fn paste_yank_cut_and_search_classification_names_postponed_stage_4_bundles() {
    use runyte::command::descriptor::ActionEffect;
    // Decision 34: stage 4 must empty this list when terminal and tree modes
    // let these defaults use ordinary programs with explicit mode steps.
    const BUNDLED_ACTIONS_PENDING_STAGE_4: &[C] = &[
        C::YankCollapseNormalMode,
        C::YankLineCollapseNormalMode,
        C::PasteAfterOrReplace,
        C::PasteBefore,
        C::ClipboardPasteAfter,
        C::ClipboardPasteBefore,
        C::SearchNext,
        C::SearchPrevious,
    ];
    for command in BUNDLED_ACTIONS_PENDING_STAGE_4.iter().copied().chain([
        C::Yank,
        C::YankLine,
        C::PasteAfter,
        C::CutSelection,
        C::CutSelectionForChange,
        C::SearchSetFromSelection,
        C::SearchSelectAllMatches,
        C::SearchExtendNext,
        C::SearchExtendPrevious,
    ]) {
        assert_eq!(
            command
                .action_policy()
                .effects
                .contains(&ActionEffect::Mode),
            BUNDLED_ACTIONS_PENDING_STAGE_4.contains(&command),
            "unclassified bundled action: {command:?}"
        );
    }
    for &command in BUNDLED_ACTIONS_PENDING_STAGE_4 {
        assert!(
            default_keymap()
                .bindings()
                .iter()
                .any(|b| b.target == command.into())
        );
        for binding in default_keymap()
            .bindings()
            .iter()
            .filter(|b| b.target == command.into())
        {
            assert!(binding.is_plain_action(), "{command:?}");
        }
    }
}

#[test]
fn conditional_program_bounds_scope_and_continuation_cover_every_branch() {
    use runyte::keymap::{
        BindingScope as S,
        program::{Condition as Cnd, MAX_DEPTH, Step},
    };
    let empty = || Program::from_steps(vec![]).unwrap();
    let leaf = || Program::single(C::MoveRight.into());
    assert!(empty().validate(S::Global).is_err());
    let conditional = Program::conditional(Cnd::Mode(Mode::Select), leaf(), empty());
    assert!(conditional.validate(S::Global).is_ok());
    assert_eq!(conditional.iter().count(), 1);
    for condition in [
        Cnd::TreeEntryExpanded,
        Cnd::TreeEntryIsDirectory,
        Cnd::TreeEntryIsRoot,
    ] {
        let program = Program::conditional(condition, leaf(), empty());
        assert!(program.validate(S::Global).is_err());
        assert!(program.validate(S::DirectoryTree).is_ok());
    }
    let mut nested = leaf();
    for _ in 0..MAX_DEPTH {
        nested = Program::conditional(Cnd::RecordingMacro, nested, empty());
    }
    assert!(
        Program::from_steps(vec![Step::If {
            condition: Cnd::RecordingMacro,
            then: Box::new(nested),
            otherwise: Box::new(empty())
        }])
        .is_err()
    );
    let eight = Program::new(vec![Action::Editor(C::Yank); 8]).unwrap();
    assert!(
        Program::from_steps(vec![Step::If {
            condition: Cnd::RecordingMacro,
            then: Box::new(eight.clone()),
            otherwise: Box::new(eight)
        }])
        .is_err()
    );
    for stop in [C::Search, C::ReplayDefaultMacro] {
        let branch =
            Program::conditional(Cnd::RecordingMacro, Program::single(stop.into()), leaf());
        assert!(branch.validate(S::Global).is_ok());
        let mut steps = branch.steps().to_vec();
        steps.push(Step::Action(Action::Editor(C::MoveRight)));
        assert!(
            Program::from_steps(steps)
                .unwrap()
                .validate(S::Global)
                .is_err()
        );
    }
}

#[test]
fn conditional_discovery_shows_both_branches_and_refuses_incomplete_exports() {
    use runyte::{command::discovery, keymap::BindingScope};
    let map = default_keymap();
    let binding = map
        .bindings()
        .iter()
        .find(|b| b.target == C::EnterSelectMode.into())
        .unwrap();
    let rendered = discovery::binding_steps(binding, BindingScope::Global);
    for expected in [
        "mode: select",
        "collapse-selection",
        "mode-enter",
        "enter-select-mode",
        "else",
    ] {
        assert!(rendered.contains(expected), "{rendered}");
    }
    assert!(
        discovery::binding_yaml(binding, Mode::Normal, None, map)
            .unwrap_err()
            .contains("conditional")
    );
    let only_conditional = Keymap::new(vec![binding.clone()]).unwrap();
    for command in [C::EnterSelectMode, C::CollapseSelection, C::ModeEnter] {
        assert!(
            discovery::details(
                CommandId::Editor(command),
                &only_conditional,
                Mode::Normal,
                BindingScope::Global
            )
            .contains("mode: select")
        );
    }
}

#[test]
fn scoped_configuration_and_discovery_admit_branch_only_tree_actions() {
    use runyte::{
        command::discovery,
        keymap::{BindingScope, configured},
    };
    for command in [
        C::DirectoryTreeSelectParent,
        C::DirectoryTreeSelectFirstChild,
    ] {
        let name = command.metadata().name;
        let source = format!(
            "scopes: {{directory-tree: {{normal: {{F6: {name}}}, select: {{F6: {name}}}}}}}"
        );
        let result = configured::compile(&serde_yaml::from_str(&source).unwrap(), default_keymap());
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        for mode in [Mode::Normal, Mode::Select] {
            assert!(discovery::admitted_here(
                CommandId::Editor(command),
                mode,
                BindingScope::DirectoryTree
            ));
        }
        assert!(!discovery::admitted_here(
            CommandId::Editor(command),
            Mode::Normal,
            BindingScope::Global
        ));
        assert!(discovery::guide_reference().contains(&format!("- `{name}`: normal, select")));
    }
}

#[test]
fn scoped_admission_preserves_representative_actions_rewritten_as_programs() {
    use runyte::{
        command::discovery,
        keymap::{BindingScope, configured},
    };
    let source =
        serde_yaml::from_str("scopes: {media: {normal: {F6: enter-normal-mode}}}").unwrap();
    let result = configured::compile(&source, default_keymap());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(discovery::admitted_here(
        CommandId::Editor(C::EnterNormalMode),
        Mode::Normal,
        BindingScope::Media
    ));
}
