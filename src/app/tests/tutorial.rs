// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn tutorial_opens_two_native_panes_and_advances_from_editor_state() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute_command("tutorial").unwrap();

    let state = app.tutorial_state().unwrap();
    assert_eq!(app.panes.len(), 2);
    assert!(app.buffers[state.instruction_buffer].is_read_only());
    assert_eq!(app.buffers[state.scratch_buffer].to_string(), "");
    assert!(app.list.is_some(), "the motion preference picker is open");

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let state = app.tutorial_state().unwrap();
    assert_eq!(state.motion_hints, Some(MotionHints::HelixLike));
    assert_eq!(state.lesson, 1);
    assert_eq!(app.active_pane, state.exercise_pane);
    assert_eq!(app.active_buffer().to_string(), "hello\n");

    press(&mut app, 'i');
    type_text(&mut app, "Hi ");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    let state = app.tutorial_state().unwrap();
    assert_eq!(state.lesson, 2);
    assert_eq!(app.active_buffer().to_string(), "alpha beta\n");
    assert!(
        app.buffers[state.instruction_buffer]
            .to_string()
            .contains("Press gh to move to the start of the line")
    );
}

#[test]
fn every_tutorial_lesson_has_at_least_ten_lines_and_ends_with_next_steps() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute_command("tutorial").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let mut state = app.tutorial_state().unwrap().clone();

    for lesson in 1..=crate::tutorial::LAST_LESSON {
        state.lesson = lesson;
        state.awaiting_reattach = false;
        let document = crate::tutorial::render(&state, false);
        let body = document
            .split_once("\n\n")
            .unwrap()
            .1
            .split_once("\n\nExercises happen")
            .unwrap()
            .0;
        let content_lines = body.lines().filter(|line| !line.trim().is_empty()).count();
        assert!(
            content_lines >= 10,
            "lesson {lesson} has only {content_lines} content lines"
        );
    }

    state.lesson = crate::tutorial::LAST_LESSON;
    for (persistent, awaiting_reattach) in [(true, false), (true, true)] {
        state.awaiting_reattach = awaiting_reattach;
        let document = crate::tutorial::render(&state, persistent);
        let body = document
            .split_once("\n\n")
            .unwrap()
            .1
            .split_once("\n\nExercises happen")
            .unwrap()
            .0;
        assert!(body.lines().filter(|line| !line.trim().is_empty()).count() >= 10);
    }

    state.lesson = crate::tutorial::LAST_LESSON + 1;
    state.awaiting_reattach = false;
    let completed = crate::tutorial::render(&state, true);
    assert!(completed.contains("NEXT STEPS"));
    assert!(completed.contains("Run :help"));
    assert!(completed.contains("Press Space ? in each view"));
}

#[test]
fn tutorial_curriculum_advances_through_the_real_input_grammar() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute_command("tutorial").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    press(&mut app, 'i');
    type_text(&mut app, "Hi ");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.tutorial_state().unwrap().lesson, 2);

    press(&mut app, 'g');
    press(&mut app, 'h');
    assert_eq!(app.tutorial_state().unwrap().lesson, 3);

    press(&mut app, 'v');
    press(&mut app, 'e');
    press(&mut app, 'd');
    assert_eq!(app.tutorial_state().unwrap().lesson, 4);

    press(&mut app, 'x');
    press(&mut app, 'x');
    press(&mut app, 'X');
    press(&mut app, 'd');
    assert_eq!(app.tutorial_state().unwrap().lesson, 5);

    press(&mut app, 's');
    type_text(&mut app, "cat");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.tutorial_state().unwrap().lesson, 6);

    press(&mut app, 'c');
    type_text(&mut app, "fox");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.tutorial_state().unwrap().lesson, 7);

    press(&mut app, 'C');
    press(&mut app, 'C');
    press(&mut app, 'i');
    type_text(&mut app, "> ");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.tutorial_state().unwrap().lesson, 8);

    press(&mut app, ' ');
    press(&mut app, 's');
    press(&mut app, 'c');
    assert_eq!(app.tutorial_state().unwrap().lesson, 9);

    app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 100,
            height: 32,
            ..Rect::default()
        },
        editor: Rect {
            width: 100,
            height: 30,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    });

    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    press(&mut app, 'h');
    assert_eq!(app.tutorial_state().unwrap().lesson, 10);
    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    press(&mut app, 'l');
    assert_eq!(app.tutorial_state().unwrap().lesson, 11);

    press(&mut app, ' ');
    press(&mut app, 'e');
    assert!(app.active_buffer().is_directory());
    assert_eq!(app.tutorial_state().unwrap().lesson, 12);
    key(&mut app, KeyCode::Char('o'), Modifiers::ALT);
    assert_eq!(app.tutorial_state().unwrap().lesson, 13);

    press(&mut app, ' ');
    press(&mut app, 't');
    press(&mut app, 'n');
    assert!(app.active_terminal().is_some());
    assert_eq!(app.tutorial_state().unwrap().lesson, 14);
    key(&mut app, KeyCode::Char('\\'), Modifiers::CONTROL);
    press(&mut app, ' ');
    press(&mut app, 't');
    press(&mut app, 't');
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.active_terminal().is_none());
    assert_eq!(app.tutorial_state().unwrap().lesson, 15);

    press(&mut app, 'g');
    press(&mut app, 'e');
    assert_eq!(app.tutorial_state().unwrap().lesson, 16);
    key(&mut app, KeyCode::Char('o'), Modifiers::CONTROL);
    assert_eq!(app.tutorial_state().unwrap().lesson, 17);
    key(&mut app, KeyCode::Char('i'), Modifiers::CONTROL);
    assert_eq!(app.tutorial_state().unwrap().lesson, 18);
}

fn prepare_two_pane_geometry(app: &mut App) {
    app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 100,
            height: 32,
            ..Rect::default()
        },
        editor: Rect {
            width: 100,
            height: 30,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    });
}

#[test]
fn closing_either_tutorial_pane_retires_progress_without_panicking() {
    for close_instructions in [false, true] {
        let mut app = App::new(Config::default(), None).unwrap();
        app.execute_command("tutorial").unwrap();
        key(&mut app, KeyCode::Enter, Modifiers::NONE);
        if close_instructions {
            let instruction = app.tutorial_state().unwrap().instruction_pane;
            app.activate_pane(instruction);
        }
        prepare_two_pane_geometry(&mut app);

        key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
        press(&mut app, 'c');

        assert_eq!(app.panes.len(), 1);
        assert!(app.tutorial_state().is_none());
    }
}

#[test]
fn tutorial_start_is_atomic_while_the_active_pane_is_maximized() {
    let mut app = App::new(Config::default(), None).unwrap();
    let original_buffer = app.active().buffer;
    app.toggle_maximized(MaximizedView::Zen);

    let outcome = app.execute_command("tutorial").unwrap();

    assert!(matches!(
        outcome,
        CommandOutcome::UserError(message) if message.contains("leave zen mode")
    ));
    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.active().buffer, original_buffer);
    assert!(app.tutorial_state().is_none());
}

#[test]
fn live_tutorial_refuses_resume_and_reset_while_its_pane_is_maximized() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute_command("tutorial").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    app.toggle_maximized(MaximizedView::Fullscreen);

    for command in ["tutorial", "tutorial reset"] {
        let outcome = app.execute_command(command).unwrap();
        assert!(matches!(
            outcome,
            CommandOutcome::UserError(ref message) if message.contains("full-screen view")
        ));
        assert_eq!(app.tutorial_state().unwrap().lesson, 1);
        assert!(app.list.is_none());
    }
}

#[test]
fn scratch_lesson_cannot_complete_from_an_unrelated_buffer() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute_command("tutorial").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    press(&mut app, 'i');
    type_text(&mut app, "Hi ");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.tutorial_state().unwrap().lesson, 2);

    app.open_scratch_buffer();
    press(&mut app, '0');

    assert_eq!(app.tutorial_state().unwrap().lesson, 2);
}

#[test]
fn reopening_tutorial_restores_its_pane_buffers_after_navigation() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute_command("tutorial").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let state = app.tutorial_state().unwrap().clone();

    app.activate_pane(state.exercise_pane);
    app.open_scratch_buffer();
    app.activate_pane(state.instruction_pane);
    app.open_scratch_buffer();
    app.execute_command("tutorial").unwrap();

    assert_eq!(
        app.panes[&state.instruction_pane].buffer,
        state.instruction_buffer
    );
    assert_eq!(app.panes[&state.exercise_pane].buffer, state.scratch_buffer);
    assert_eq!(app.active_pane, state.exercise_pane);
}

#[test]
fn reopening_explorer_history_lesson_restores_the_alt_o_back_step() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute_command("tutorial").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    app.tutorial.as_mut().unwrap().lesson = 11;
    press(&mut app, ' ');
    press(&mut app, 'e');
    let state = app.tutorial_state().unwrap().clone();
    app.open_scratch_buffer();

    app.execute_command("tutorial").unwrap();
    assert_eq!(app.active().buffer, state.explorer_buffer.unwrap());
    key(&mut app, KeyCode::Char('o'), Modifiers::ALT);

    assert_eq!(app.tutorial_state().unwrap().lesson, 13);
    assert_eq!(app.active().buffer, state.scratch_buffer);
}

#[test]
fn reopening_jump_lessons_reconstructs_backward_and_forward_history() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute_command("tutorial").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    app.tutorial.as_mut().unwrap().lesson = 16;
    app.reset_tutorial_scratch("first\nsecond\nthird\n", 0, true);
    app.prepare_tutorial_jump_history(16);

    app.open_scratch_buffer();
    app.execute_command("tutorial").unwrap();
    key(&mut app, KeyCode::Char('o'), Modifiers::CONTROL);
    assert_eq!(app.tutorial_state().unwrap().lesson, 17);

    app.open_scratch_buffer();
    app.execute_command("tutorial").unwrap();
    key(&mut app, KeyCode::Char('i'), Modifiers::CONTROL);
    assert_eq!(app.tutorial_state().unwrap().lesson, 18);
}

#[test]
fn both_motion_preference_shows_both_aliases_without_changing_the_keymap() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute_command("tutorial").unwrap();
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(
        app.tutorial_state().unwrap().motion_hints,
        Some(MotionHints::Both)
    );
    press(&mut app, 'i');
    type_text(&mut app, "Hi ");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    let instruction = app.tutorial_state().unwrap().instruction_buffer;
    let rendered = app.buffers[instruction].to_string();
    assert!(rendered.contains("gh (Helix-like) or 0 (Vim-like)"));
    assert!(matches!(
        app.keymap.lookup_in(
            Mode::Normal,
            BindingScope::Global,
            &KeySequence::from([crate::keymap::Key::char('0')])
        ),
        crate::keymap::Lookup::Exact(binding)
            if binding.target == BindingTarget::Editor(EditorCommand::MoveLineStart)
    ));
}

#[test]
fn file_end_lesson_honors_vim_like_and_both_motion_preferences() {
    for (down, expected, key_sequence) in [
        (
            1,
            "Press G to jump",
            vec![(KeyCode::Char('G'), Modifiers::NONE)],
        ),
        (
            2,
            "ge (Helix-like) or G (Vim-like)",
            vec![
                (KeyCode::Char('g'), Modifiers::NONE),
                (KeyCode::Char('e'), Modifiers::NONE),
            ],
        ),
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        app.execute_command("tutorial").unwrap();
        for _ in 0..down {
            key(&mut app, KeyCode::Down, Modifiers::NONE);
        }
        key(&mut app, KeyCode::Enter, Modifiers::NONE);
        app.tutorial.as_mut().unwrap().lesson = 15;
        app.reset_tutorial_scratch("first\nsecond\nthird\n", 0, true);
        app.refresh_tutorial_document();
        let instructions = app.tutorial_state().unwrap().instruction_buffer;
        assert!(app.buffers[instructions].to_string().contains(expected));

        for (code, modifiers) in key_sequence {
            key(&mut app, code, modifiers);
        }
        assert_eq!(app.tutorial_state().unwrap().lesson, 16);
    }
}

#[test]
fn persistent_tutorial_finishes_only_after_detach_and_reattach() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.enable_persistent_session();
    app.execute_command("tutorial sessions").unwrap();
    assert_eq!(app.tutorial_state().unwrap().lesson, 18);
    assert!(app.list.is_none());

    app.execute_command("detach").unwrap();
    assert!(app.should_quit);
    assert!(app.tutorial_state().unwrap().awaiting_reattach);

    app.should_quit = false;
    app.note_frontend_attached();
    let state = app.tutorial_state().unwrap();
    assert_eq!(state.lesson, 19);
    assert!(
        app.buffers[state.instruction_buffer]
            .to_string()
            .starts_with("Runyte tutorial · complete")
    );
    assert!(
        app.active_buffer()
            .to_string()
            .contains("persistent tutorial token")
    );
}

#[test]
fn standalone_session_lesson_states_the_real_boundary() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute_command("tutorial sessions").unwrap();
    let state = app.tutorial_state().unwrap();
    let instructions = app.buffers[state.instruction_buffer].to_string();
    assert!(instructions.contains("standalone workspace"));
    assert!(instructions.contains("runyte --persistent"));
    assert!(instructions.contains("not crash, reboot, or machine-failure storage"));
}
