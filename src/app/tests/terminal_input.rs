// SPDX-License-Identifier: MPL-2.0
use super::*;

fn terminal_app(source: &str) -> (App, TerminalId) {
    let mut config = Config::default();
    config.merge_keys_source(source).unwrap();
    let mut app = App::new(config, None).unwrap();
    let terminal = app.terminals.insert_test_session(80, 24);
    app.panes.get_mut(&app.active_pane).unwrap().terminal = Some(terminal);
    app.mode = Mode::Insert;
    (app, terminal)
}

fn send(app: &mut App, spelling: &str) {
    app.handle_key(KeyStroke::parse(spelling).unwrap()).unwrap();
}

#[test]
fn scoped_escape_removal_forwards_old_keys_and_remapped_escape_changes_mode() {
    let (mut app, terminal) = terminal_app(
        "scopes: {terminal: {insert: {F6: enter-normal-mode, 'Ctrl-4': null, 'Ctrl-\\': null}}}",
    );
    for spelling in ["Ctrl-4", "Ctrl-\\", "Esc", "a", "Shift-F6"] {
        let before = app.terminals.get(terminal).unwrap().input_generation();
        send(&mut app, spelling);
        assert_eq!(app.mode, Mode::Insert, "{spelling}");
        assert_eq!(
            app.terminals.get(terminal).unwrap().input_generation(),
            before + 1,
            "{spelling}"
        );
    }
    let before = app.terminals.get(terminal).unwrap().input_generation();
    send(&mut app, "F6");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.terminals.get(terminal).unwrap().input_generation(),
        before
    );
}

#[test]
fn scoped_terminal_prefix_executes_and_cancels_without_leaking_suffixes() {
    let (mut app, terminal) = terminal_app("scopes: {terminal: {insert: {'F7 z': toggle-zen}}}");
    let before = app.terminals.get(terminal).unwrap().input_generation();
    send(&mut app, "F7");
    assert!(!app.grammar.pending_sequence().is_empty());
    send(&mut app, "Esc");
    assert!(app.grammar.pending_sequence().is_empty());
    assert_eq!(
        app.terminals.get(terminal).unwrap().input_generation(),
        before
    );
    assert!(app.maximized.is_none());
    send(&mut app, "F7");
    send(&mut app, "z");
    assert!(app.maximized.is_some());
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(
        app.terminals.get(terminal).unwrap().input_generation(),
        before
    );
    send(&mut app, "z");
    assert_eq!(
        app.terminals.get(terminal).unwrap().input_generation(),
        before + 1
    );
}

#[test]
#[cfg(any(unix, windows))]
fn explicit_session_action_in_standalone_reports_unavailable_without_child_input() {
    let (mut app, terminal) =
        terminal_app("scopes: {terminal: {insert: {F8: next-running-session}}}");
    let before = app.terminals.get(terminal).unwrap().input_generation();
    send(&mut app, "F8");
    assert_eq!(app.status, "session navigation needs mux mode");
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(
        app.terminals.get(terminal).unwrap().input_generation(),
        before
    );
}

/// What help leaves out of the terminal page is exactly what review refuses
/// with "needs a buffer"; every other command is either consumed by review or
/// passed on to ordinary dispatch, never refused.
#[test]
fn terminal_review_refuses_exactly_what_help_leaves_out() {
    use crate::keymap::admission::admits_command;
    let mut mismatches = Vec::new();
    let mut refused = 0;
    let (mut app, terminal) = terminal_app("{}");
    for &command in EditorCommand::ALL {
        // Only the terminal handler runs, so one app serves every command.
        app.mode = Mode::Normal;
        app.status.clear();
        app.panes.get_mut(&app.active_pane).unwrap().terminal = Some(terminal);
        assert_eq!(app.key_binding_scope(), BindingScope::Terminal);
        let consumed = app.execute_terminal_command(terminal, command, false);
        let refusal = app.status.contains("needs a buffer");
        if admits_command(BindingScope::Terminal, command) {
            if refusal {
                mismatches.push(format!("{command:?} admitted but refused: {}", app.status));
            }
        } else {
            refused += 1;
            if !(consumed && refusal) {
                mismatches.push(format!("{command:?} not refused: {}", app.status));
            }
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
    assert!(refused > 100, "only {refused} refused");
}
