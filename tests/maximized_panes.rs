// SPDX-License-Identifier: MPL-2.0

//! Maximizing a pane is presentation: the editable buffer and split tree below
//! it keep their identities while one pane temporarily fills the UI. `:zen`
//! centres its text at a fixed width; `:fullscreen` gives the pane the whole
//! area and leaves its content laid out exactly as it is in a split.

use ratatui::layout::Rect;
use runyte::{
    app::{App, CommandOutcome, PreparedView},
    command::{CommandExecutionContext, CommandInvocation, EditorCommand, parse_colon_command},
    config::Config,
    input::{InputEvent, KeyCode, KeyStroke, Modifiers},
    ui,
};

fn run(app: &mut App, command: EditorCommand) {
    app.execute(CommandInvocation::editor(command, CommandExecutionContext::default()).unwrap())
        .unwrap();
}

fn finish_macro_replay(app: &mut App) {
    while app.macro_replay_pending() {
        app.advance_macro_replay().unwrap();
    }
}

fn press(app: &mut App, code: KeyCode, modifiers: Modifiers) {
    app.handle_key(KeyStroke::new(code, modifiers)).unwrap();
}

fn prepare(app: &mut App, width: u16, height: u16) -> PreparedView {
    app.prepare_view(ui::frame_geometry(Rect::new(0, 0, width, height)))
}

#[test]
fn zen_maximizes_the_active_pane_and_the_second_toggle_restores_the_split_tree() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();
    run(&mut app, EditorCommand::SplitVertical);
    let zen_pane = app.active_pane;
    let layout = format!("{:?}", app.layout);

    app.execute(parse_colon_command("zen").unwrap()).unwrap();
    let focused = prepare(&mut app, 220, 40);

    assert_eq!(focused.panes.len(), 1);
    assert_eq!(focused.panes[0].pane_id, zen_pane);
    assert_eq!(focused.panes[0].area, focused.geometry.editor);
    assert_eq!(focused.panes[0].text_width, 100);
    assert_eq!(focused.panes[0].content_indent, 59);
    assert_eq!(app.panes.len(), 2, "the hidden pane remains alive");
    assert_eq!(format!("{:?}", app.layout), layout);

    app.execute(parse_colon_command("zen").unwrap()).unwrap();
    let restored = prepare(&mut app, 220, 40);

    assert_eq!(restored.panes.len(), 2);
    assert_eq!(app.panes.len(), 2);
    assert_eq!(format!("{:?}", app.layout), layout);
}

#[test]
fn zen_keeps_the_buffer_editable_and_does_not_move_as_text_changes() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();
    app.execute(parse_colon_command("zen").unwrap()).unwrap();
    let before = prepare(&mut app, 160, 30).panes[0].clone();

    run(&mut app, EditorCommand::EnterInsertMode);
    app.handle_input(InputEvent::Text(
        "A sentence written in zen mode.".to_owned(),
    ))
    .unwrap();
    let after = prepare(&mut app, 160, 30).panes[0].clone();

    assert_eq!(
        app.active_buffer().text().to_string(),
        "A sentence written in zen mode."
    );
    assert!(app.active_buffer().dirty);
    assert_eq!(after.content_indent, before.content_indent);
    assert_eq!(after.text_width, before.text_width);
}

#[test]
fn zen_width_is_configurable_and_narrow_panes_use_every_available_cell() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    config.editor.zen_width = 72;
    let mut app = App::new(config, None).unwrap();
    app.execute(parse_colon_command("zen").unwrap()).unwrap();

    let wide = prepare(&mut app, 120, 30).panes[0].clone();
    assert_eq!(wide.text_width, 72);
    assert_eq!(wide.content_indent, 23);

    let narrow = prepare(&mut app, 60, 30).panes[0].clone();
    assert_eq!(narrow.text_width, 58);
    assert_eq!(narrow.content_indent, 0);
}

#[test]
fn window_structure_stays_stable_until_zen_is_toggled_off() {
    let mut app = App::new(Config::default(), None).unwrap();
    run(&mut app, EditorCommand::SplitVertical);
    let pane = app.active_pane;
    let panes = app.panes.len();
    app.execute(parse_colon_command("zen").unwrap()).unwrap();

    let split = app
        .execute(
            CommandInvocation::editor(
                EditorCommand::SplitHorizontal,
                CommandExecutionContext::default(),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(
        matches!(split, CommandOutcome::UserError(message) if message.contains("leave zen mode"))
    );

    run(&mut app, EditorCommand::CloseWindow);
    assert_eq!(app.active_pane, pane);
    assert_eq!(app.panes.len(), panes);
}

#[test]
fn fullscreen_maximizes_the_active_pane_without_narrowing_the_text() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();
    run(&mut app, EditorCommand::SplitVertical);
    let full_pane = app.active_pane;
    let layout = format!("{:?}", app.layout);

    app.execute(parse_colon_command("fullscreen").unwrap())
        .unwrap();
    let focused = prepare(&mut app, 220, 40);

    assert_eq!(focused.panes.len(), 1);
    assert_eq!(focused.panes[0].pane_id, full_pane);
    assert_eq!(focused.panes[0].area, focused.geometry.editor);
    assert_eq!(
        focused.panes[0].text_width, 218,
        "the whole pane body is text: no zen width is enforced"
    );
    assert_eq!(focused.panes[0].content_indent, 0);
    assert_eq!(app.panes.len(), 2, "the hidden pane remains alive");
    assert_eq!(format!("{:?}", app.layout), layout);

    app.execute(parse_colon_command("fullscreen").unwrap())
        .unwrap();
    let restored = prepare(&mut app, 220, 40);

    assert_eq!(restored.panes.len(), 2);
    assert_eq!(format!("{:?}", app.layout), layout);
}

#[test]
fn asking_for_the_other_maximized_view_switches_to_it_rather_than_stacking() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();

    app.execute(parse_colon_command("zen").unwrap()).unwrap();
    assert_eq!(prepare(&mut app, 220, 40).panes[0].text_width, 100);

    app.execute(parse_colon_command("fullscreen").unwrap())
        .unwrap();
    let full = prepare(&mut app, 220, 40).panes[0].clone();
    assert_eq!(full.text_width, 218);
    assert_eq!(full.content_indent, 0);

    app.execute(parse_colon_command("zen").unwrap()).unwrap();
    assert_eq!(prepare(&mut app, 220, 40).panes[0].text_width, 100);

    // Only the view that is showing toggles off, so one more `:zen` leaves the
    // editor with no maximized pane at all.
    app.execute(parse_colon_command("zen").unwrap()).unwrap();
    run(&mut app, EditorCommand::SplitVertical);
    assert_eq!(app.panes.len(), 2);
}

#[test]
fn window_structure_stays_stable_until_fullscreen_is_toggled_off() {
    let mut app = App::new(Config::default(), None).unwrap();
    run(&mut app, EditorCommand::SplitVertical);
    let pane = app.active_pane;
    let panes = app.panes.len();
    app.execute(parse_colon_command("fullscreen").unwrap())
        .unwrap();

    let split = app
        .execute(
            CommandInvocation::editor(
                EditorCommand::SplitHorizontal,
                CommandExecutionContext::default(),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(
        matches!(split, CommandOutcome::UserError(message) if message.contains("leave the full-screen view"))
    );

    run(&mut app, EditorCommand::CloseWindow);
    assert_eq!(app.active_pane, pane);
    assert_eq!(app.panes.len(), panes);
}

#[test]
fn ctrl_w_toggles_both_maximized_views() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();
    run(&mut app, EditorCommand::SplitVertical);

    press(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    press(&mut app, KeyCode::Char('f'), Modifiers::NONE);
    let full = prepare(&mut app, 220, 40);
    assert_eq!(full.panes.len(), 1);
    assert_eq!(full.panes[0].text_width, 218);

    press(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    press(&mut app, KeyCode::Char('z'), Modifiers::NONE);
    let zen = prepare(&mut app, 220, 40);
    assert_eq!(zen.panes.len(), 1);
    assert_eq!(zen.panes[0].text_width, 100);

    press(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    press(&mut app, KeyCode::Char('z'), Modifiers::NONE);
    assert_eq!(prepare(&mut app, 220, 40).panes.len(), 2);
}

#[test]
fn space_w_reaches_the_same_two_maximized_views() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();
    run(&mut app, EditorCommand::SplitVertical);

    for key in [' ', 'w', 'z'] {
        press(&mut app, KeyCode::Char(key), Modifiers::NONE);
    }
    let zen = prepare(&mut app, 220, 40);
    assert_eq!(zen.panes.len(), 1);
    assert_eq!(zen.panes[0].text_width, 100);

    for key in [' ', 'w', 'f'] {
        press(&mut app, KeyCode::Char(key), Modifiers::NONE);
    }
    let full = prepare(&mut app, 220, 40);
    assert_eq!(full.panes.len(), 1);
    assert_eq!(full.panes[0].text_width, 218);

    // The Ctrl-w spelling toggles off the view the Space sequence turned on:
    // both reach one command and one state.
    press(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    press(&mut app, KeyCode::Char('f'), Modifiers::NONE);
    assert_eq!(prepare(&mut app, 220, 40).panes.len(), 2);
}

/// Focus is asked to move with the pane rectangles of the *previous* frame
/// still in hand, which is the state a replayed macro leaves the editor in:
/// nothing has rendered since the view was maximized, so those rectangles
/// still describe the split. A refusal that depended on the geometry rather
/// than on the state would move focus to a pane nobody can see.
#[test]
fn a_maximized_pane_refuses_directional_focus_in_every_spelling() {
    let mut app = App::new(Config::default(), None).unwrap();
    run(&mut app, EditorCommand::SplitVertical);
    let maximized = app.active_pane;

    for view in ["zen", "fullscreen"] {
        assert_eq!(prepare(&mut app, 220, 40).panes.len(), 2);
        app.execute(parse_colon_command(view).unwrap()).unwrap();

        for (code, modifiers) in [
            (KeyCode::Char('h'), Modifiers::NONE),
            (KeyCode::Char('h'), Modifiers::CONTROL),
            (KeyCode::Left, Modifiers::NONE),
        ] {
            press(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
            press(&mut app, code, modifiers);
            assert_eq!(
                app.active_pane, maximized,
                "Ctrl-w moved focus under :{view}"
            );
        }

        for step in [' ', 'w', 'h'] {
            press(&mut app, KeyCode::Char(step), Modifiers::NONE);
        }
        assert_eq!(
            app.active_pane, maximized,
            "Space w moved focus under :{view}"
        );

        app.execute(parse_colon_command(view).unwrap()).unwrap();
    }

    // The refusal is the maximized view's alone: the same key still moves once
    // the split is back on screen.
    assert_eq!(prepare(&mut app, 220, 40).panes.len(), 2);
    press(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    press(&mut app, KeyCode::Char('h'), Modifiers::NONE);
    assert_ne!(app.active_pane, maximized);
}

#[test]
fn a_maximized_pane_refuses_content_swapping_in_both_views() {
    let mut app = App::new(Config::default(), None).unwrap();
    run(&mut app, EditorCommand::SplitVertical);
    let active = app.active_pane;
    let buffers = app
        .panes
        .iter()
        .map(|(pane, state)| (*pane, state.buffer))
        .collect::<std::collections::HashMap<_, _>>();

    for view in ["zen", "fullscreen"] {
        app.execute(parse_colon_command(view).unwrap()).unwrap();
        let outcome = app
            .execute(
                CommandInvocation::editor(
                    EditorCommand::SwapWindow,
                    CommandExecutionContext::default(),
                )
                .unwrap(),
            )
            .unwrap();
        assert!(matches!(
            outcome,
            CommandOutcome::UserError(message) if message.contains("keeps the current pane maximized")
        ));
        assert_eq!(app.active_pane, active);
        assert!(
            app.panes
                .iter()
                .all(|(pane, state)| buffers.get(pane) == Some(&state.buffer))
        );
        app.execute(parse_colon_command(view).unwrap()).unwrap();
    }
}

#[test]
fn a_macro_cannot_focus_a_hidden_pane_by_maximizing_before_the_next_frame() {
    let mut app = App::new(Config::default(), None).unwrap();
    run(&mut app, EditorCommand::SplitVertical);
    let maximized = app.active_pane;
    // One rendered frame, so the rectangles the focus search consults are the
    // ordinary split ones rather than empty.
    assert_eq!(prepare(&mut app, 220, 40).panes.len(), 2);

    // Recorded and replayed as one run, the maximizing key and the focus key
    // are handled with no frame between them.
    run(&mut app, EditorCommand::RecordDefaultMacro);
    press(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    press(&mut app, KeyCode::Char('f'), Modifiers::NONE);
    press(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    press(&mut app, KeyCode::Char('h'), Modifiers::NONE);
    run(&mut app, EditorCommand::RecordDefaultMacro);
    assert_eq!(app.active_pane, maximized);
    let frame = prepare(&mut app, 220, 40);
    assert_eq!(frame.panes.len(), 1);
    assert_eq!(frame.panes[0].pane_id, maximized);

    // Replaying it turns the view back off and must still never leave focus on
    // a pane the frame is not showing.
    run(&mut app, EditorCommand::ReplayDefaultMacro);
    finish_macro_replay(&mut app);
    assert_eq!(app.active_pane, maximized);
    let frame = prepare(&mut app, 220, 40);
    assert!(
        frame.pane(app.active_pane).is_some(),
        "focus left the visible panes"
    );
}
